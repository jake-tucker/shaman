//! An open Session: one for each screen of a Session pushed, read live over
//! its Remote's Pairing for as long as it is open.
//!
//! It crosses to Swift in two parts. Its frame is whole on each change: the
//! header, which stands at once from what the listing knows. Its
//! conversation crosses as items with stable ids, whole once and then the
//! items that changed and the order they stand in, so a Message streaming
//! many times a second crosses alone: until the Session has been read it is
//! the loading mark alone, and then the conversation lands in one piece.
//!
//! Closing it stops the reading and cancels nothing on the Server.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use shaman_client::{
    RecoveryBackoff, SessionCommands, SessionEvent, SessionStreamError, SessionSubscription, TryNow,
};
use shaman_protocol::{SessionId, SessionSnapshot};
use tokio::{runtime::Handle, sync::watch, task::JoinHandle};

use crate::{
    Subscription,
    conversation::{self, ConversationItem},
    listing::{Listed, Listings},
    watching,
};

/// What frames an open Session's screen, whole on each change.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SessionFrame {
    /// The Session's Title.
    pub title: String,
    /// Where it stands, beneath the Title: "origin · workspace · branch",
    /// the Origin named only where the phone holds more than one Remote.
    pub place: String,
    /// Said where the Session, or the Remote it is on, is gone.
    pub gone: Option<String>,
}

/// How an open Session's conversation changed.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ConversationChange {
    /// The conversation, whole: what a listener hears first.
    Whole { items: Vec<ConversationItem> },
    /// The items that changed or are new since the listener last heard, and
    /// the ids of every item, in the order they stand in now.
    Changed {
        items: Vec<ConversationItem>,
        order: Vec<String>,
    },
}

/// The conversation as the reader last made it: each item shared, so one
/// unchanged since is told apart without comparing it whole.
type Shown = Arc<Vec<Arc<ConversationItem>>>;

/// One Session open on a screen.
#[derive(uniffi::Object)]
pub struct OpenSession {
    runtime: Handle,
    frame: watch::Receiver<SessionFrame>,
    conversation: watch::Receiver<Shown>,
    /// Reads the Session until it is closed.
    reader: Mutex<Option<JoinHandle<()>>>,
}

#[uniffi::export]
impl OpenSession {
    /// Tells `listener` what frames the Session now, and again on every
    /// change, until the subscription is cancelled or the Session closed.
    pub fn watch_frame(&self, listener: Arc<dyn FrameListener>) -> Arc<Subscription> {
        watching(&self.runtime, self.frame.clone(), move |frame| {
            listener.changed(frame);
        })
    }

    /// Tells `listener` the conversation whole, and then each change to it,
    /// until the subscription is cancelled or the Session closed.
    pub fn watch_conversation(&self, listener: Arc<dyn ConversationListener>) -> Arc<Subscription> {
        let mut shown = self.conversation.clone();
        let task = self.runtime.spawn(async move {
            let mut told = Told::default();
            loop {
                let current = shown.borrow_and_update().clone();
                if let Some(change) = told.change_to(&current) {
                    listener.changed(change);
                }
                if shown.changed().await.is_err() {
                    break;
                }
            }
        });
        Subscription::of(task)
    }

    /// Ends the Session's reading; it cancels nothing on the Server.
    pub fn close(&self) {
        if let Some(reader) = self
            .reader
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            reader.abort();
        }
    }
}

impl OpenSession {
    /// Whether the Session is still being read: until it is closed, or its
    /// Session is gone.
    pub fn is_reading(&self) -> bool {
        self.reader
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .is_some_and(|reader| !reader.is_finished())
    }
}

impl Drop for OpenSession {
    fn drop(&mut self) {
        self.close();
    }
}

/// Hears what frames an open Session, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait FrameListener: Send + Sync {
    fn changed(&self, frame: SessionFrame);
}

/// Hears an open Session's conversation, whole at once and then by change.
#[uniffi::export(with_foreign)]
pub trait ConversationListener: Send + Sync {
    fn changed(&self, change: ConversationChange);
}

/// What one listener has been told of a conversation.
#[derive(Default)]
struct Told {
    items: Option<HashMap<String, Arc<ConversationItem>>>,
    order: Vec<String>,
}

impl Told {
    /// What the listener is to be told for it to hold `current`: whole the
    /// first time, then what changed since, and nothing where nothing did.
    fn change_to(&mut self, current: &Shown) -> Option<ConversationChange> {
        let order = current
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        let change = match &self.items {
            None => ConversationChange::Whole {
                items: current.iter().map(|item| (**item).clone()).collect(),
            },
            Some(told) => {
                let items = current
                    .iter()
                    .filter(|item| {
                        told.get(&item.id)
                            .is_none_or(|was| !Arc::ptr_eq(was, item) && was != *item)
                    })
                    .map(|item| (**item).clone())
                    .collect::<Vec<_>>();
                if items.is_empty() && order == self.order {
                    return None;
                }
                ConversationChange::Changed {
                    items,
                    order: order.clone(),
                }
            }
        };
        self.items = Some(
            current
                .iter()
                .map(|item| (item.id.clone(), item.clone()))
                .collect(),
        );
        self.order = order;
        Some(change)
    }
}

/// How an open Session waits between attempts to read it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pacing {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

/// Opens the Session `reference` names, read on `runtime` over the Pairing of
/// the Remote it is on, its frame standing at once from what the listing
/// knows of it.
pub(crate) fn open(
    runtime: &Handle,
    listings: &Listings,
    reference: &str,
    pacing: Pacing,
) -> Arc<OpenSession> {
    let opening = listings.opening(reference);
    let listed = opening.as_ref().map(|opening| opening.listed.clone());
    let (frame, frame_seen) = watch::channel(frame_of(listed.as_ref(), None, None));
    let (conversation, conversation_seen) = watch::channel(shown(None, conversation::loading()));
    let reader = opening.map(|opening| {
        let reader = Reader {
            listings: listings.clone(),
            reference: reference.to_owned(),
            session_id: opening.session_id,
            commands: opening.commands,
            try_now: opening.try_now,
            listed: opening.listed,
            snapshot: None,
            frame,
            conversation,
            pacing,
        };
        runtime.spawn(reader.read())
    });
    Arc::new(OpenSession {
        runtime: runtime.clone(),
        frame: frame_seen,
        conversation: conversation_seen,
        reader: Mutex::new(reader),
    })
}

/// What frames a Session: its Title and place as the listing and its
/// snapshot last said them, or that it is gone.
fn frame_of(
    listed: Option<&Listed>,
    snapshot: Option<&SessionSnapshot>,
    gone: Option<String>,
) -> SessionFrame {
    let Some(listed) = listed else {
        return SessionFrame {
            title: String::new(),
            place: String::new(),
            gone: Some(gone.unwrap_or_else(|| GONE_REMOTE.to_owned())),
        };
    };
    let title = snapshot
        .map(|snapshot| snapshot.title.clone())
        .filter(|title| !title.is_empty())
        .or_else(|| listed.title.clone())
        .unwrap_or_default();
    let workspace = listed
        .workspace
        .as_ref()
        .map(|(_, name)| name.clone())
        .or_else(|| {
            snapshot.map(|snapshot| listed.paths.workspace_name(&snapshot.session.workspace))
        });
    let place = listed
        .origin
        .iter()
        .cloned()
        .chain(workspace)
        .chain(listed.branch.clone())
        .collect::<Vec<_>>()
        .join(" · ");
    SessionFrame { title, place, gone }
}

/// Said where the Remote a Session was opened on is no longer held.
const GONE_REMOTE: &str = "This phone is no longer paired with the server this session is on";

/// Said where the Session's Server no longer holds it.
const GONE_SESSION: &str = "This session no longer exists";

/// `items` as shown, each sharing what `was` shows where it is unchanged.
fn shown(was: Option<&Shown>, items: Vec<ConversationItem>) -> Shown {
    let was = was
        .into_iter()
        .flat_map(|was| was.iter())
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();
    Arc::new(
        items
            .into_iter()
            .map(|item| match was.get(item.id.as_str()) {
                Some(was) if ***was == item => Arc::clone(was),
                _ => Arc::new(item),
            })
            .collect(),
    )
}

/// What reads one open Session, and what it last read.
struct Reader {
    listings: Listings,
    reference: String,
    session_id: SessionId,
    commands: SessionCommands,
    try_now: TryNow,
    listed: Listed,
    snapshot: Option<SessionSnapshot>,
    frame: watch::Sender<SessionFrame>,
    conversation: watch::Sender<Shown>,
    pacing: Pacing,
}

/// Why a Session stopped being read.
enum Ended {
    /// It is gone, as `0` says.
    Gone(String),
    /// Its stream dropped, and is to be opened again.
    Dropped,
}

impl Reader {
    async fn read(mut self) {
        let mut listing = self.listings.listing();
        let mut backoff =
            RecoveryBackoff::new(self.pacing.initial_backoff, self.pacing.max_backoff);
        loop {
            let ended = match self.commands.attach_session(self.session_id).await {
                Ok(subscription) => {
                    backoff.reset();
                    self.follow(subscription, &mut listing).await
                }
                Err(error) => match error.downcast_ref::<SessionStreamError>() {
                    Some(refused) if refused.deleted_session().is_some() => {
                        Ended::Gone(GONE_SESSION.to_owned())
                    }
                    _ => {
                        tracing::debug!("the session was not read: {error:#}");
                        Ended::Dropped
                    }
                },
            };
            if let Ended::Gone(why) = ended {
                self.show_gone(why);
                return;
            }
            let retry_in = backoff.next();
            tokio::select! {
                _ = self.try_now.or_after(retry_in) => {}
                // While it waits, the header still follows the listing.
                Ok(()) = listing.changed() => {
                    if !self.relist() {
                        return;
                    }
                }
            }
        }
    }

    /// Reads `subscription` until it ends, the frame following the listing
    /// as it changes meanwhile.
    async fn follow(
        &mut self,
        mut subscription: SessionSubscription,
        listing: &mut watch::Receiver<crate::Listing>,
    ) -> Ended {
        loop {
            tokio::select! {
                event = subscription.next() => {
                    let Some(event) = event else {
                        return Ended::Dropped;
                    };
                    if let Some(ended) = self.take(event) {
                        return ended;
                    }
                    // A streaming Turn arrives as a run of small events:
                    // each already here is taken before the conversation is
                    // made again, once.
                    while let Some(Some(event)) = subscription.try_next() {
                        if let Some(ended) = self.take(event) {
                            return ended;
                        }
                    }
                    self.show();
                }
                Ok(()) = listing.changed() => {
                    if !self.relist() {
                        return Ended::Gone(GONE_REMOTE.to_owned());
                    }
                }
            }
        }
    }

    /// Takes one event of the Session's stream into what it holds, or says
    /// why reading it ends.
    fn take(&mut self, event: Result<SessionEvent, SessionStreamError>) -> Option<Ended> {
        match event {
            Ok(SessionEvent::Snapshot(snapshot)) => {
                self.snapshot = Some(*snapshot);
                None
            }
            Ok(SessionEvent::Updated(update)) => {
                let snapshot = self.snapshot.as_mut()?;
                if let Err(error) =
                    shaman_protocol::session_projection::apply_update(snapshot, &update)
                {
                    // Read afresh: the stream opened again begins with a
                    // snapshot.
                    tracing::debug!("an update did not apply: {error:#}");
                    return Some(Ended::Dropped);
                }
                None
            }
            Err(error) if error.deleted_session().is_some() => {
                Some(Ended::Gone(GONE_SESSION.to_owned()))
            }
            Err(error) if error.remote_status().is_some() => Some(Ended::Gone(error.to_string())),
            Err(error) => {
                tracing::debug!("the session's stream ended: {error}");
                Some(Ended::Dropped)
            }
        }
    }

    /// Reads again what the listing knows of the Session, showing the frame
    /// by it; false where its Remote is no longer held.
    fn relist(&mut self) -> bool {
        let Some(opening) = self.listings.opening(&self.reference) else {
            return false;
        };
        self.listed = opening.listed;
        self.show_frame(None);
        true
    }

    fn show(&self) {
        self.show_frame(None);
        let items = self
            .snapshot
            .as_ref()
            .map_or_else(conversation::loading, conversation::conversation);
        self.conversation.send_if_modified(|current| {
            let next = shown(Some(current), items);
            let changed = next.len() != current.len()
                || next
                    .iter()
                    .zip(current.iter())
                    .any(|(next, current)| !Arc::ptr_eq(next, current));
            *current = next;
            changed
        });
    }

    fn show_frame(&self, gone: Option<String>) {
        let frame = frame_of(Some(&self.listed), self.snapshot.as_ref(), gone);
        self.frame.send_if_modified(|current| {
            let changed = *current != frame;
            *current = frame;
            changed
        });
    }

    fn show_gone(&self, why: String) {
        self.show_frame(Some(why));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ItemKind;

    fn item(id: &str, text: &str) -> ConversationItem {
        ConversationItem {
            id: id.to_owned(),
            kind: ItemKind::Error {
                text: text.to_owned(),
            },
        }
    }

    #[test]
    fn a_listener_is_told_the_conversation_whole_then_only_what_changed_and_the_order() {
        let mut told = Told::default();
        let first = shown(None, vec![item("a", "one"), item("b", "two")]);
        assert_eq!(
            told.change_to(&first),
            Some(ConversationChange::Whole {
                items: vec![item("a", "one"), item("b", "two")]
            })
        );
        assert_eq!(told.change_to(&first), None, "nothing changed");

        let second = shown(
            Some(&first),
            vec![
                item("a", "one"),
                item("b", "two, longer"),
                item("c", "three"),
            ],
        );
        assert!(
            Arc::ptr_eq(&first[0], &second[0]),
            "an unchanged item is shared"
        );
        assert_eq!(
            told.change_to(&second),
            Some(ConversationChange::Changed {
                items: vec![item("b", "two, longer"), item("c", "three")],
                order: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            })
        );

        let reordered = shown(Some(&second), vec![item("c", "three"), item("a", "one")]);
        assert_eq!(
            told.change_to(&reordered),
            Some(ConversationChange::Changed {
                items: Vec::new(),
                order: vec!["c".to_owned(), "a".to_owned()],
            }),
            "an item gone, and the rest moved, is the order alone"
        );
    }
}
