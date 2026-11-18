//! An open Session: one for each screen of a Session pushed, read live over
//! its Remote's Pairing for as long as it is open.
//!
//! It crosses to Swift in two parts. Its frame is whole on each change: the
//! header, which stands at once from what the listing knows, the bar, the
//! composer's state and the queued Prompts. Its conversation crosses as items
//! with stable ids, whole once and then the items that changed and the order
//! they stand in, so a Message streaming many times a second crosses alone:
//! until the Session has been read it is the loading mark alone, and then the
//! conversation lands in one piece.
//!
//! Its acts are a Prompt sent, to steer the working Turn or begin one, or
//! queued for after it; a queued Prompt steered now or withdrawn; and a stop.
//! A Prompt sent stands at once as a bubble marked as sending; where it fails
//! it goes, and its text is given back to the composer's draft.
//!
//! Closing it stops the reading and cancels nothing on the Server.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use shaman_client::{
    RecoveryBackoff, SessionCommands, SessionEvent, SessionStreamError, SessionSubscription, TryNow,
};
use shaman_protocol::{
    AdmitPromptRequest, InitialPrompt, InterruptOutcome, PromptDelivery, PromptId, SessionId,
    SessionSnapshot,
};
use tokio::{runtime::Handle, sync::watch, task::JoinHandle};

use crate::{
    ActError, Subscription,
    acts::{self, Acts, Asking, Unanswered},
    conversation::{self, ConversationItem},
    drafts::Drafts,
    frame::{self, GONE_REMOTE, Local, Outgoing, SessionFrame, frame_of},
    listing::{Listed, Listings},
    watching,
};

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
    /// Whether its stream is still read: until its Session is gone, when
    /// the reader only shows what the screen's own acts change.
    reading: Arc<AtomicBool>,
    /// What its acts ask over and keep.
    acting: Acting,
}

/// What an open Session's acts need: where to find the Session's Remote as
/// each is asked, the composer's draft, and what the screen holds of its own.
#[derive(Clone)]
pub(crate) struct Acting {
    pub reference: String,
    pub listings: Listings,
    pub drafts: Drafts,
    pub acts: Acts,
    pub asking: Asking,
    pub local: Arc<watch::Sender<Local>>,
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

    /// Sends `text` as a Prompt: it steers the working Turn, or begins one.
    /// It stands at once as a bubble marked as sending; where the send fails
    /// the bubble goes and the text is given back to the composer's draft.
    pub async fn send(&self, text: String) -> Result<(), ActError> {
        let acting = self.acting.clone();
        self.act(async move { acting.prompt(text, PromptDelivery::Steer).await })
            .await
    }

    /// Queues `text` as a Prompt for after the working Turn, failing as a
    /// send does.
    pub async fn queue(&self, text: String) -> Result<(), ActError> {
        let acting = self.acting.clone();
        self.act(async move { acting.prompt(text, PromptDelivery::Queue).await })
            .await
    }

    /// Delivers the queued Prompt `prompt` names now, steering the working
    /// Turn.
    pub async fn steer_now(&self, prompt: String) -> Result<(), ActError> {
        let acting = self.acting.clone();
        self.act(async move { acting.steer_now(&prompt).await })
            .await
    }

    /// Withdraws the queued Prompt `prompt` names.
    pub async fn cancel_queued(&self, prompt: String) -> Result<(), ActError> {
        let acting = self.acting.clone();
        self.act(async move { acting.cancel_queued(&prompt).await })
            .await
    }

    /// Stops the Session's work: its Turn, or its Watches while it is
    /// Monitoring. A Session Working only for a Prompt not yet delivered has
    /// that Prompt withdrawn instead, and its text given back to the
    /// composer's draft, ahead of anything written there.
    pub async fn stop(&self) -> Result<(), ActError> {
        let acting = self.acting.clone();
        self.act(async move { acting.stop().await }).await
    }
}

impl OpenSession {
    /// Runs `act` on the core's runtime to its end, marked under way while
    /// it runs, whatever becomes of the call that asked it.
    async fn act(
        &self,
        act: impl Future<Output = Result<(), ActError>> + Send + 'static,
    ) -> Result<(), ActError> {
        let under_way = self.acting.acts.begin();
        self.runtime
            .spawn(async move {
                let _under_way = under_way;
                act.await
            })
            .await
            .map_err(|error| ActError::failed(anyhow::Error::from(error)))?
    }

    /// Whether the Session is still being read: until it is closed, or its
    /// Session is gone.
    pub fn is_reading(&self) -> bool {
        self.reading.load(Ordering::Acquire)
            && self
                .reader
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
/// knows of it, and asking its acts as `acting` says.
pub(crate) fn open(
    runtime: &Handle,
    listings: &Listings,
    reference: &str,
    pacing: Pacing,
    asked: Asked,
) -> Arc<OpenSession> {
    let opening = listings.opening(reference);
    let listed = opening.as_ref().map(|opening| opening.listed.clone());
    let local = Arc::new(watch::Sender::new(Local::default()));
    let (frame, frame_seen) = watch::channel(frame_of(
        listed.as_ref(),
        None,
        None,
        &Local::default(),
        false,
    ));
    let (conversation, conversation_seen) = watch::channel(shown(None, conversation::loading()));
    let reading = Arc::new(AtomicBool::new(true));
    let reader = opening.map(|opening| {
        let reader = Reader {
            gone: None,
            reading: reading.clone(),
            listings: listings.clone(),
            reference: reference.to_owned(),
            session_id: opening.session_id,
            commands: opening.commands,
            try_now: opening.try_now,
            listed: opening.listed,
            snapshot: None,
            local: Arc::downgrade(&local),
            unreachable: opening.unreachable,
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
        reading,
        acting: Acting {
            reference: reference.to_owned(),
            listings: listings.clone(),
            drafts: asked.drafts,
            acts: asked.acts,
            asking: asked.asking,
            local,
        },
    })
}

/// What the core hands each open Session for its acts.
pub(crate) struct Asked {
    pub drafts: Drafts,
    pub acts: Acts,
    pub asking: Asking,
}

/// Said where the Session's Server no longer holds it.
const GONE_SESSION: &str = "This session no longer exists";

/// Said where a queued Prompt an act names is no longer queued.
const GONE_PROMPT: &str = "That prompt is no longer queued";

/// Said where a Prompt with nothing in it is sent.
const EMPTY_PROMPT: &str = "Write something to send";

impl Acting {
    /// What the Session's acts are asked over now: its Remote as the
    /// listing holds it, which may have been paired again since it opened.
    fn opening(&self) -> Result<crate::listing::Opening, ActError> {
        self.listings
            .opening(&self.reference)
            .ok_or_else(|| ActError::Gone {
                message: GONE_REMOTE.to_owned(),
            })
    }

    /// Asks `ask` of the Session's Server, as an act is asked: again while
    /// no way carries it, and never again once it may have reached it.
    async fn deliver<T, F>(
        &self,
        opening: &mut crate::listing::Opening,
        ask: impl FnMut(SessionCommands, SessionId) -> F,
    ) -> Result<T, Unanswered>
    where
        F: Future<Output = anyhow::Result<T>>,
    {
        let mut ask = ask;
        let (commands, session_id) = (opening.commands.clone(), opening.session_id);
        acts::deliver(self.asking, &mut opening.try_now, || {
            ask(commands.clone(), session_id)
        })
        .await
    }

    /// Sends `text` as a Prompt delivered as `delivery`, standing as a
    /// bubble marked as sending until its Server holds it, and given back to
    /// the composer's draft where it fails.
    async fn prompt(&self, text: String, delivery: PromptDelivery) -> Result<(), ActError> {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(ActError::Invalid {
                message: EMPTY_PROMPT.to_owned(),
            });
        }
        let mut opening = self.opening()?;
        let id = PromptId::new();
        self.local.send_modify(|local| {
            local.note = None;
            local.outgoing.push(Outgoing {
                id,
                text: text.clone(),
                delivery,
                answered: false,
            });
        });
        let request = AdmitPromptRequest {
            prompt: InitialPrompt {
                id,
                text: text.clone(),
                skill_invocations: Vec::new(),
                attachments: Vec::new(),
            },
            delivery,
        };
        let admitted = self
            .deliver(&mut opening, |commands, session_id| {
                let request = request.clone();
                async move { commands.admit_prompt(session_id, request).await }
            })
            .await;
        match admitted {
            Ok(_) => {
                // It stands as sent until the Server is seen holding it.
                self.local.send_modify(|local| {
                    if let Some(outgoing) = local.outgoing.iter_mut().find(|sent| sent.id == id) {
                        outgoing.answered = true;
                    }
                });
                Ok(())
            }
            Err(unanswered) => {
                let maybe_sent = unanswered == Unanswered::MaybeDone;
                self.local.send_modify(|local| {
                    local.outgoing.retain(|outgoing| outgoing.id != id);
                    if maybe_sent {
                        local.note = Some(frame::MAY_HAVE_BEEN_SENT.to_owned());
                    }
                });
                self.drafts.give_back(&self.reference, &text);
                Err(said(unanswered, "The server did not answer"))
            }
        }
    }

    async fn steer_now(&self, prompt: &str) -> Result<(), ActError> {
        let prompt = queued_prompt(prompt)?;
        let mut opening = self.opening()?;
        self.deliver(&mut opening, |commands, session_id| async move {
            commands.promote_prompt(session_id, prompt).await
        })
        .await
        .map(drop)
        .map_err(|unanswered| {
            said(
                unanswered,
                "The server did not answer, so the prompt may have been steered",
            )
        })
    }

    async fn cancel_queued(&self, prompt: &str) -> Result<(), ActError> {
        let prompt = queued_prompt(prompt)?;
        let mut opening = self.opening()?;
        self.deliver(&mut opening, |commands, session_id| async move {
            commands.cancel_prompt(session_id, prompt).await
        })
        .await
        .map(drop)
        .map_err(|unanswered| {
            said(
                unanswered,
                "The server did not answer, so the prompt may have been cancelled",
            )
        })
    }

    async fn stop(&self) -> Result<(), ActError> {
        let mut opening = self.opening()?;
        let outcome = self
            .deliver(&mut opening, |commands, session_id| async move {
                commands
                    .interrupt_session_reporting_outcome(session_id)
                    .await
            })
            .await
            .map_err(|unanswered| {
                said(
                    unanswered,
                    "The server did not answer, so the work may have been stopped",
                )
            })?;
        // A Prompt withdrawn, as the one thing to stop, is owed back to the
        // composer it was written in.
        if let InterruptOutcome::WithdrewPrompt { prompt } = outcome {
            self.drafts.give_back(&self.reference, &prompt.text);
        }
        Ok(())
    }
}

/// The queued Prompt `reference` names.
fn queued_prompt(reference: &str) -> Result<PromptId, ActError> {
    // The Session stands; only the Prompt is no longer there to act on.
    frame::prompt_referenced(reference).ok_or_else(|| ActError::Refused {
        message: GONE_PROMPT.to_owned(),
    })
}

/// An act's failure as the banner says it, `maybe` where it may have been
/// done.
fn said(unanswered: Unanswered, maybe: &str) -> ActError {
    match unanswered {
        Unanswered::NotDone(error) => error,
        Unanswered::MaybeDone => ActError::Unreachable {
            message: maybe.to_owned(),
        },
    }
}

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
    /// Why the Session is gone, once it is.
    gone: Option<String>,
    /// Whether the Session's stream is still read.
    reading: Arc<AtomicBool>,
    listings: Listings,
    reference: String,
    session_id: SessionId,
    commands: SessionCommands,
    try_now: TryNow,
    listed: Listed,
    snapshot: Option<SessionSnapshot>,
    /// What the screen holds of its own, which its acts change, for as long
    /// as the screen holds it.
    local: Weak<watch::Sender<Local>>,
    /// Whether no way to the Session's Remote answered the last request
    /// asked of it.
    unreachable: watch::Receiver<bool>,
    frame: watch::Sender<SessionFrame>,
    conversation: watch::Sender<Shown>,
    pacing: Pacing,
}

/// What reading the listing again found of the Session.
#[derive(Debug, PartialEq, Eq)]
enum Relisted {
    /// Its Remote is no longer held.
    Gone,
    /// What the listing knows of it changed.
    Changed,
    /// Nothing of it changed.
    Same,
}

/// How reading a Session ends on a refusal its Server gave that no reopening
/// gets past: that the Session is gone, or the Server's own words for why
/// this phone may read it no more.
fn refusal(refused: &SessionStreamError) -> Ended {
    if refused.deleted_session().is_some() {
        Ended::Gone(GONE_SESSION.to_owned())
    } else {
        Ended::Gone(refused.to_string())
    }
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
        // Where the screen holds nothing of its own, nothing it holds moves.
        let mut local = self.local.upgrade().map_or_else(
            || watch::channel(Local::default()).1,
            |local| local.subscribe(),
        );
        let mut backoff =
            RecoveryBackoff::new(self.pacing.initial_backoff, self.pacing.max_backoff);
        loop {
            let ended = match self.commands.attach_session(self.session_id).await {
                Ok(subscription) => {
                    backoff.reset();
                    self.follow(subscription, &mut listing, &mut local).await
                }
                Err(error) => match error.downcast_ref::<SessionStreamError>() {
                    // A refusal no reopening gets past ends the reading in its
                    // words, as one that comes mid-stream does.
                    Some(refused) if !refused.is_recoverable() => refusal(refused),
                    _ => {
                        tracing::debug!("the session was not read: {error:#}");
                        Ended::Dropped
                    }
                },
            };
            if let Ended::Gone(why) = ended {
                return self.show_gone(why, local).await;
            }
            if let Err(why) = self
                .back_off(backoff.next(), &mut listing, &mut local)
                .await
            {
                return self.show_gone(why, local).await;
            }
        }
    }

    /// Waits `retry_in` before the Session is read again, or less where the
    /// transport says to try now or what the listing knows of the Session
    /// changes; the header follows the listing meanwhile. Fails with why
    /// where the Session's Remote is no longer held.
    async fn back_off(
        &mut self,
        retry_in: Duration,
        listing: &mut watch::Receiver<crate::Listing>,
        local: &mut watch::Receiver<Local>,
    ) -> Result<(), String> {
        let until = tokio::time::Instant::now() + retry_in;
        loop {
            let left = until.saturating_duration_since(tokio::time::Instant::now());
            tokio::select! {
                _ = self.try_now.or_after(left) => return Ok(()),
                Ok(()) = listing.changed() => match self.relist() {
                    Relisted::Gone => return Err(GONE_REMOTE.to_owned()),
                    // What moved is this Session's, so it is read again now.
                    Relisted::Changed => return Ok(()),
                    // Another Session moved: the wait goes on.
                    Relisted::Same => {}
                },
                // A Prompt sent meanwhile stands as it is being sent.
                Ok(()) = local.changed() => self.show(),
                Ok(()) = self.unreachable.changed() => self.show_frame(),
            }
        }
    }

    /// Reads `subscription` until it ends, the frame following the listing
    /// as it changes meanwhile.
    async fn follow(
        &mut self,
        mut subscription: SessionSubscription,
        listing: &mut watch::Receiver<crate::Listing>,
        local: &mut watch::Receiver<Local>,
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
                    if self.relist() == Relisted::Gone {
                        return Ended::Gone(GONE_REMOTE.to_owned());
                    }
                }
                Ok(()) = local.changed() => self.show(),
                Ok(()) = self.unreachable.changed() => self.show_frame(),
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
            Err(error) if error.deleted_session().is_some() || error.remote_status().is_some() => {
                Some(refusal(&error))
            }
            Err(error) => {
                tracing::debug!("the session's stream ended: {error}");
                Some(Ended::Dropped)
            }
        }
    }

    /// Reads again what the listing knows of the Session, showing the frame
    /// by it, and says whether that changed, or that its Remote is no longer
    /// held.
    fn relist(&mut self) -> Relisted {
        let Some(opening) = self.listings.opening(&self.reference) else {
            return Relisted::Gone;
        };
        if opening.listed == self.listed {
            return Relisted::Same;
        }
        self.listed = opening.listed;
        self.show_frame();
        Relisted::Changed
    }

    fn show(&self) {
        // A Prompt sent from here that the Server now holds stands as the
        // Server says from now on.
        if let (Some(snapshot), Some(local)) = (&self.snapshot, self.local.upgrade()) {
            local.send_if_modified(|local| local.held_by(snapshot));
        }
        self.show_frame();
        let items = self
            .snapshot
            .as_ref()
            .map_or_else(conversation::loading, |snapshot| {
                frame::with_outgoing(conversation::conversation(snapshot), &self.local())
            });
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

    fn show_frame(&self) {
        let frame = frame_of(
            Some(&self.listed),
            self.snapshot.as_ref(),
            self.gone.clone(),
            &self.local(),
            *self.unreachable.borrow(),
        );
        self.frame.send_if_modified(|current| {
            let changed = *current != frame;
            *current = frame;
            changed
        });
    }

    /// What the screen holds of its own now.
    fn local(&self) -> Local {
        self.local
            .upgrade()
            .map(|local| local.borrow().clone())
            .unwrap_or_default()
    }

    /// Shows the Session gone, as `why` says, and reads it no more; what
    /// the screen's own acts change meanwhile is shown until it is closed,
    /// so a Prompt whose send fails goes from the conversation all the same.
    async fn show_gone(mut self, why: String, mut local: watch::Receiver<Local>) {
        self.reading.store(false, Ordering::Release);
        self.gone = Some(why);
        self.show_frame();
        while local.changed().await.is_ok() {
            self.show();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use shaman_client::{SessionCommandTimings, Transport as _, testing::ScriptedTransport};
    use shaman_protocol::{SessionError, SessionErrorCode};

    use super::*;
    use crate::{
        conversation::ItemKind,
        listing::tests::{listings, remote, session},
    };

    /// How long a test waits for the frame to show what it must.
    const SHOWN_WITHIN: Duration = Duration::from_secs(5);

    /// A reader of the Session `reference` names, read over `transport`,
    /// waiting `backoff` between one failed attempt and the next, and the
    /// frame it shows.
    fn reading(
        runtime: &tokio::runtime::Runtime,
        listings: &Listings,
        reference: &str,
        transport: Arc<ScriptedTransport>,
        backoff: Duration,
    ) -> (JoinHandle<()>, watch::Receiver<SessionFrame>) {
        let opening = listings.opening(reference).expect("its Remote is held");
        let (frame, seen) = watch::channel(frame_of(
            Some(&opening.listed),
            None,
            None,
            &Local::default(),
            false,
        ));
        let (conversation, _) = watch::channel(shown(None, conversation::loading()));
        let try_now = transport.try_now();
        let reader = Reader {
            gone: None,
            reading: Arc::new(AtomicBool::new(true)),
            listings: listings.clone(),
            reference: reference.to_owned(),
            session_id: opening.session_id,
            commands: SessionCommands::new(
                transport,
                SessionCommandTimings {
                    initial_recovery_backoff: backoff,
                    max_recovery_backoff: backoff,
                    attachment_fetch_timeout: SHOWN_WITHIN,
                },
            ),
            try_now,
            listed: opening.listed,
            snapshot: None,
            local: Weak::new(),
            unreachable: opening.unreachable,
            frame,
            conversation,
            pacing: Pacing {
                initial_backoff: backoff,
                max_backoff: backoff,
            },
        };
        (runtime.spawn(reader.read()), seen)
    }

    /// The reference of the one Session the Remote `name` holds.
    fn listed_on(name: &str, title: &str) -> (crate::listing::Answered, String) {
        let answered = remote(name, vec![session(title, "shaman", 1, 1)]);
        let id = answered.sessions.as_ref().unwrap()[0].id();
        let reference = format!("{}/{id}", answered.fingerprint);
        (answered, reference)
    }

    fn gone_within(
        runtime: &tokio::runtime::Runtime,
        frame: &mut watch::Receiver<SessionFrame>,
    ) -> String {
        runtime.block_on(async {
            tokio::time::timeout(SHOWN_WITHIN, frame.wait_for(|frame| frame.gone.is_some()))
                .await
                .expect("shown gone in time")
                .expect("the frame stands")
                .gone
                .clone()
                .unwrap()
        })
    }

    #[test]
    fn a_refusal_no_reopening_gets_past_on_the_first_attach_ends_the_reading_in_its_words() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (listings, _directory) = listings(&runtime);
        let (desk, reference) = listed_on("desk", "Map the workspace");
        listings.hold(desk);
        let revoked = SessionError {
            code: SessionErrorCode::PairingAuthenticationFailed,
            message: "unknown peer".to_owned(),
            unreachable: None,
        };
        let transport = Arc::new(ScriptedTransport::new(vec![Ok(
            shaman_client::testing::refusal(http::StatusCode::FORBIDDEN, &revoked),
        )]));
        let (reader, mut frame) = reading(
            &runtime,
            &listings,
            &reference,
            transport.clone(),
            Duration::from_millis(1),
        );
        assert_eq!(
            gone_within(&runtime, &mut frame),
            "Remote revoked this pairing",
            "in the words a refusal mid-stream is said in"
        );
        runtime.block_on(reader).unwrap();
        assert_eq!(transport.asked(), 1, "and it is asked no more");
    }

    #[test]
    fn a_remote_no_longer_held_while_the_reading_waits_to_try_again_shows_the_session_gone() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (listings, _directory) = listings(&runtime);
        let (desk, reference) = listed_on("desk", "Map the workspace");
        listings.hold(desk);
        let transport = Arc::new(ScriptedTransport::new(Vec::new()));
        let (reader, mut frame) = reading(
            &runtime,
            &listings,
            &reference,
            transport.clone(),
            Duration::from_secs(60),
        );
        runtime.block_on(async {
            while transport.asked() == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        listings.release("desk");
        assert_eq!(gone_within(&runtime, &mut frame), GONE_REMOTE);
        runtime.block_on(reader).unwrap();
    }

    #[test]
    fn only_what_moves_of_the_session_itself_cuts_the_wait_to_try_again_short() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (listings, _directory) = listings(&runtime);
        let (desk, reference) = listed_on("desk", "Map the workspace");
        let (lab, _) = listed_on("lab", "Tidy the build");
        listings.hold(desk);
        listings.hold(lab);
        let transport = Arc::new(ScriptedTransport::new(Vec::new()));
        let (_reader, frame) = reading(
            &runtime,
            &listings,
            &reference,
            transport.clone(),
            Duration::from_secs(60),
        );
        let asked_at_least = |times: usize| {
            runtime.block_on(async {
                tokio::time::timeout(SHOWN_WITHIN, async {
                    while transport.asked() < times {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
            })
        };
        asked_at_least(1).expect("asked once");

        // Another Remote's Session moves: the wait goes on.
        listings.answer("lab", |answered| {
            answered.sessions = Some(vec![crate::listing::tests::listed_summary(session(
                "Tidy the build, again",
                "shaman",
                1,
                2,
            ))]);
        });
        runtime.block_on(async { tokio::time::sleep(Duration::from_millis(50)).await });
        assert_eq!(
            transport.asked(),
            1,
            "another Remote's change cut nothing short"
        );

        // This Session's own row moves: it is read again at once.
        listings.answer("desk", |answered| {
            let Some(shaman_protocol::SessionListItem::Readable(summary)) = answered
                .sessions
                .as_mut()
                .and_then(|sessions| sessions.first_mut())
            else {
                panic!("a readable Session");
            };
            summary.title = "Map the workspace, renamed".to_owned();
        });
        asked_at_least(2).expect("asked again at once");
        assert_eq!(frame.borrow().title, "Map the workspace, renamed");
    }

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
