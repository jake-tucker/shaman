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
//! queued for after it; a queued Prompt steered now or withdrawn; a stop; and
//! a Decision on an Approval. A Prompt sent stands at once as a bubble marked
//! as sending; where it fails it goes, and its text is given back to the
//! composer's draft. Its own Interventions are dismissed and reviewed on the
//! screen alone, asking nothing of the Server.
//!
//! While a screen pushed from it is up, a Turn's work or an Activity's
//! detail is attached to it, filled again from the Session as it changes and
//! made no more once that screen goes.
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
    intervention::{self, DecisionKind},
    listing::{Listed, Listings},
    watching,
    work::{self, ActivityDetail, TurnWork},
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
    /// What screens pushed from it have attached.
    attached: Arc<Attachments>,
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

    /// Tells `listener` the work of the Turn `turn` names, as a card hands
    /// it, and again on every change, until the subscription is cancelled,
    /// as its screen goes.
    pub fn watch_work(&self, turn: String, listener: Arc<dyn WorkListener>) -> Arc<Subscription> {
        let (attachment, filled) = self.attached.attach(Target::Work(turn));
        self.told(attachment, filled, move |filled| {
            if let Filled::Work(work) = filled {
                listener.changed(work);
            }
        })
    }

    /// Tells `listener` the whole of the Activity `activity` names, as a row
    /// hands it, and again on every change, until the subscription is
    /// cancelled, as its screen goes.
    pub fn watch_detail(
        &self,
        activity: String,
        listener: Arc<dyn DetailListener>,
    ) -> Arc<Subscription> {
        let (attachment, filled) = self.attached.attach(Target::Detail(activity));
        self.told(attachment, filled, move |filled| {
            if let Filled::Detail(detail) = filled {
                listener.changed(detail);
            }
        })
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

    /// Answers the Approval `approval` names with `decision`, once its Server
    /// has. One whose answer was lost is never sent again: it waits behind
    /// "Review", as one dismissed does, until the Session shows it answered.
    pub async fn decide(&self, approval: String, decision: DecisionKind) -> Result<(), ActError> {
        let acting = self.acting.clone();
        self.act(async move { acting.decide(&approval, decision).await })
            .await
    }

    /// Dismisses every one of the Session's own Interventions pending now,
    /// as swiping its sheet away does, for as long as this screen stays
    /// open: the bar offers them again, and one asked after presents itself.
    pub fn dismiss(&self) {
        self.acting.local.send_modify(Local::dismiss);
    }

    /// Presents the Interventions dismissed again, oldest first.
    pub fn review(&self) {
        self.acting
            .local
            .send_modify(|local| local.dismissed.clear());
    }
}

impl OpenSession {
    /// Tells `tell` what the attachment `attachment` is filled with, once it
    /// is and on every change, until the subscription is cancelled, when it
    /// is filled no more.
    fn told(
        &self,
        attachment: u64,
        mut filled: watch::Receiver<Option<Filled>>,
        tell: impl Fn(Filled) + Send + 'static,
    ) -> Arc<Subscription> {
        let detached = Detached {
            attached: self.attached.clone(),
            attachment,
        };
        let task = self.runtime.spawn(async move {
            let _detached = detached;
            loop {
                let current = filled.borrow_and_update().clone();
                if let Some(current) = current {
                    tell(current);
                }
                if filled.changed().await.is_err() {
                    break;
                }
            }
        });
        Subscription::of(task)
    }

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

/// Hears a Turn's work, whole at once and on every change.
#[uniffi::export(with_foreign)]
pub trait WorkListener: Send + Sync {
    fn changed(&self, work: TurnWork);
}

/// Hears the whole of one Activity, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait DetailListener: Send + Sync {
    fn changed(&self, detail: ActivityDetail);
}

/// What a screen pushed from an open Session attached to it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    /// The work of the Turn named.
    Work(String),
    /// The whole of the Activity named.
    Detail(String),
}

/// What an attachment is filled with.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Filled {
    Work(TurnWork),
    Detail(ActivityDetail),
}

impl Target {
    fn fill(&self, snapshot: Option<&SessionSnapshot>) -> Filled {
        match self {
            Self::Work(turn) => Filled::Work(work::turn_work(snapshot, turn)),
            Self::Detail(activity) => Filled::Detail(work::activity_detail(snapshot, activity)),
        }
    }

    /// What it is filled with where its Session will never be read.
    fn unread(&self) -> Filled {
        match self {
            Self::Work(_) => Filled::Work(work::gone_work()),
            Self::Detail(_) => Filled::Detail(work::gone_detail()),
        }
    }
}

/// What the screens pushed from an open Session have attached to it, each
/// filled by its reader from the Session as it changes, and only while its
/// screen is up.
#[derive(Default)]
pub(crate) struct Attachments {
    held: Mutex<Held>,
    /// Moves on as each is attached, so the reader fills it at once.
    attached: watch::Sender<u64>,
    /// Whether no reader will ever fill them, the Session's Remote being
    /// held by none when it opened, so each is filled as it is attached.
    unread: bool,
}

#[derive(Default)]
struct Held {
    next: u64,
    filling: HashMap<u64, (Target, watch::Sender<Option<Filled>>)>,
}

impl Attachments {
    /// Attaches `target`, answering under what, and what it is filled with.
    fn attach(&self, target: Target) -> (u64, watch::Receiver<Option<Filled>>) {
        let (filling, filled) = watch::channel(self.unread.then(|| target.unread()));
        let attachment = {
            let mut held = self.held();
            held.next += 1;
            let attachment = held.next;
            held.filling.insert(attachment, (target, filling));
            attachment
        };
        self.attached.send_modify(|attached| *attached += 1);
        (attachment, filled)
    }

    /// Attachments no reader will ever fill.
    fn unread() -> Self {
        Self {
            unread: true,
            ..Self::default()
        }
    }

    fn detach(&self, attachment: u64) {
        self.held().filling.remove(&attachment);
    }

    /// Fills every attachment from `snapshot`, telling only those it changes.
    fn fill(&self, snapshot: Option<&SessionSnapshot>) {
        for (target, filling) in self.held().filling.values() {
            let filled = target.fill(snapshot);
            filling.send_if_modified(|current| {
                let changed = current.as_ref() != Some(&filled);
                if changed {
                    *current = Some(filled);
                }
                changed
            });
        }
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Held> {
        self.held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Hears each attachment made from now on.
    fn subscribe(&self) -> watch::Receiver<u64> {
        self.attached.subscribe()
    }
}

/// Detaches its attachment as it is dropped, which its subscription's end
/// does.
struct Detached {
    attached: Arc<Attachments>,
    attachment: u64,
}

impl Drop for Detached {
    fn drop(&mut self) {
        self.attached.detach(self.attachment);
    }
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
    let attached = Arc::new(if opening.is_some() {
        Attachments::default()
    } else {
        Attachments::unread()
    });
    let reader = opening.map(|opening| {
        let reader = Reader {
            attached: attached.clone(),
            gone: None,
            drafts: asked.drafts.clone(),
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
        attached,
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

/// Said where an Approval a Decision names no longer waits on one.
const GONE_APPROVAL: &str = "That approval is no longer waiting";

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
        let asked = self
            .deliver(&mut opening, |commands, session_id| async move {
                commands
                    .interrupt_session_reporting_outcome(session_id)
                    .await
            })
            .await;
        if asked == Err(Unanswered::MaybeDone) {
            // Whatever it withdrew is owed back once the Session shows it
            // withdrawn.
            self.local.send_modify(|local| {
                let withdrawable = local.withdrawable.clone();
                local.owed.extend(withdrawable);
            });
        }
        let outcome = asked.map_err(|unanswered| {
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

impl Acting {
    async fn decide(&self, reference: &str, decision: DecisionKind) -> Result<(), ActError> {
        // The Session stands; only the Approval is no longer there to answer.
        let approval =
            intervention::approval_referenced(reference).ok_or_else(|| ActError::Refused {
                message: GONE_APPROVAL.to_owned(),
            })?;
        let mut opening = self.opening()?;
        let decided = self
            .deliver(&mut opening, |commands, session_id| async move {
                commands
                    .submit_decision(session_id, approval, decision.into())
                    .await
            })
            .await;
        if decided == Err(Unanswered::MaybeDone) {
            // It may have been delivered: it waits out of sight, and is not
            // offered again until the reader asks.
            self.local.send_modify(|local| {
                local.dismissed.insert(approval);
            });
        }
        decided.map_err(|unanswered| {
            said(
                unanswered,
                "The server did not answer, so the decision may have been sent",
            )
        })
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
    /// What screens pushed from it have attached, which it fills.
    attached: Arc<Attachments>,
    /// Why the Session is gone, once it is.
    gone: Option<String>,
    /// Where a Prompt a stop withdrew goes back to.
    drafts: Drafts,
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
    /// Whether the Session's Remote reads Unreachable, as the listing reads
    /// it.
    unreachable: watch::Receiver<crate::reach::Reading>,
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
        let mut attaching = self.attached.subscribe();
        // Whatever was attached before the reader heard of attachments is
        // filled now, from nothing read yet.
        self.fill_attached();
        let mut backoff =
            RecoveryBackoff::new(self.pacing.initial_backoff, self.pacing.max_backoff);
        loop {
            let attached = self.attach(&mut local, &mut attaching).await;
            let ended = match attached {
                Ok(subscription) => {
                    backoff.reset();
                    self.follow(subscription, &mut listing, &mut local, &mut attaching)
                        .await
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
                return self.show_gone(why, local, attaching).await;
            }
            if let Err(why) = self
                .back_off(backoff.next(), &mut listing, &mut local, &mut attaching)
                .await
            {
                return self.show_gone(why, local, attaching).await;
            }
        }
    }

    /// Attaches to the Session's stream, showing meanwhile what the screen's
    /// own acts change: a Prompt sent before the stream answers stands at
    /// once.
    async fn attach(
        &mut self,
        local: &mut watch::Receiver<Local>,
        attaching: &mut watch::Receiver<u64>,
    ) -> anyhow::Result<SessionSubscription> {
        let commands = self.commands.clone();
        let reading = commands.attach_session(self.session_id);
        tokio::pin!(reading);
        loop {
            tokio::select! {
                attached = &mut reading => return attached,
                Ok(()) = local.changed() => self.show(),
                Ok(()) = attaching.changed() => self.fill_attached(),
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
        attaching: &mut watch::Receiver<u64>,
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
                Ok(()) = attaching.changed() => self.fill_attached(),
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
        attaching: &mut watch::Receiver<u64>,
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
                Ok(()) = attaching.changed() => self.fill_attached(),
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
            let mut withdrawn = Vec::new();
            local.send_if_modified(|local| {
                // What a stop would withdraw, and what one may have, change
                // nothing shown.
                withdrawn = local.reconcile(snapshot);
                local.held_by(snapshot)
            });
            // A Prompt a stop whose answer was lost did withdraw.
            for text in withdrawn {
                self.drafts.give_back(&self.reference, &text);
            }
        }
        self.show_frame();
        // A Prompt being sent stands at the foot even before the Session has
        // been read, beneath the loading mark.
        let items = frame::with_outgoing(
            self.snapshot
                .as_ref()
                .map_or_else(conversation::loading, conversation::conversation),
            &self.local(),
        );
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
        self.fill_attached();
    }

    /// Fills what screens pushed from the Session have attached, from what
    /// it last read.
    fn fill_attached(&self) {
        self.attached.fill(self.snapshot.as_ref());
    }

    fn show_frame(&self) {
        let frame = frame_of(
            Some(&self.listed),
            self.snapshot.as_ref(),
            self.gone.clone(),
            &self.local(),
            self.unreachable.borrow().is_some(),
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
    async fn show_gone(
        mut self,
        why: String,
        mut local: watch::Receiver<Local>,
        mut attaching: watch::Receiver<u64>,
    ) {
        self.reading.store(false, Ordering::Release);
        self.gone = Some(why);
        self.show_frame();
        loop {
            tokio::select! {
                changed = local.changed() => match changed {
                    Ok(()) => self.show(),
                    Err(_) => return,
                },
                // A screen pushed before it went still reads what it last
                // read.
                Ok(()) = attaching.changed() => self.fill_attached(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use shaman_client::{SessionCommandTimings, Transport as _, testing::ScriptedTransport};
    use shaman_protocol::{SessionError, SessionErrorCode, TurnId};

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
        attached: Arc<Attachments>,
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
            attached,
            gone: None,
            drafts: Drafts::default(),
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
            Arc::default(),
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
            Arc::default(),
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
            Arc::default(),
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

    /// What a test's Server answers each request it is asked over, by its
    /// method and path, and how many times each was asked.
    type Answers = dyn Fn(
            &http::Method,
            &str,
        ) -> futures_util::future::BoxFuture<
            'static,
            Result<shaman_client::TransportResponse, SessionError>,
        > + Send
        + Sync;

    fn answering(answer: Arc<Answers>) -> Arc<dyn shaman_client::Transport> {
        shaman_client::testing::Answering::new(move |request| {
            answer(request.method(), request.uri().path())
        })
    }

    /// A Session's stream as a Server sends it: `snapshot`, and whatever is
    /// sent down `more` after, open until `more` is dropped.
    fn stream_from(
        snapshot: &SessionSnapshot,
        more: tokio::sync::mpsc::UnboundedReceiver<bytes::Bytes>,
    ) -> shaman_client::TransportResponse {
        let first = bytes::Bytes::from(format!(
            "event: {}\nid: {}\ndata: {}\n\n",
            shaman_protocol::SESSION_SNAPSHOT_EVENT,
            snapshot.revision.0,
            serde_json::to_string(snapshot).unwrap()
        ));
        let rest = futures_util::stream::unfold(more, |mut more| async move {
            more.recv().await.map(|chunk| (chunk, more))
        });
        http::Response::new(shaman_client::testing::streamed(
            futures_util::stream::StreamExt::chain(futures_util::stream::iter([first]), rest),
        ))
    }

    /// An answer of success whose body was cut off part way.
    fn cut_off() -> shaman_client::TransportResponse {
        http::Response::new(shaman_client::full_body(br#"{"id":"#.to_vec()))
    }

    /// The Session `reference` names opened over `over`, its acts asked as
    /// a test's are, and the drafts they keep.
    fn opened_over(
        runtime: &tokio::runtime::Runtime,
        over: Arc<dyn shaman_client::Transport>,
    ) -> (
        Arc<OpenSession>,
        Drafts,
        String,
        SessionId,
        tempfile::TempDir,
    ) {
        let (listings, directory) = listings(runtime);
        let (desk, reference) = listed_on("desk", "Map the workspace");
        let session_id = desk.sessions.as_ref().unwrap()[0].id();
        listings.hold_over(desk, over);
        let drafts = Drafts::default();
        let pacing = Pacing {
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
        };
        let opened = open(
            runtime.handle(),
            &listings,
            &reference,
            pacing,
            Asked {
                drafts: drafts.clone(),
                acts: Acts::default(),
                asking: Asking {
                    initial_backoff: Duration::from_millis(1),
                    max_backoff: Duration::from_millis(5),
                    bound: SHOWN_WITHIN,
                    dialling: Duration::from_millis(50),
                },
            },
        );
        (opened, drafts, reference, session_id, directory)
    }

    fn conversation_until(
        runtime: &tokio::runtime::Runtime,
        opened: &OpenSession,
        shown: impl Fn(&[Arc<ConversationItem>]) -> bool,
    ) -> Vec<ConversationItem> {
        let mut conversation = opened.conversation.clone();
        runtime.block_on(async {
            tokio::time::timeout(SHOWN_WITHIN, conversation.wait_for(|items| shown(items)))
                .await
                .expect("shown in time")
                .expect("the conversation stands")
                .iter()
                .map(|item| (**item).clone())
                .collect()
        })
    }

    #[test]
    fn a_prompt_sent_before_the_session_is_read_stands_at_once_beneath_the_loading_mark() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        // The Session's stream never answers, and nor does the send.
        let (opened, _drafts, _reference, _session, _directory) = opened_over(
            &runtime,
            answering(Arc::new(|_, _| Box::pin(std::future::pending()))),
        );
        let sending = {
            let opened = opened.clone();
            runtime.spawn(async move { opened.send("Steer it".to_owned()).await })
        };
        let items = conversation_until(&runtime, &opened, |items| items.len() == 2);
        assert!(
            matches!(items[0].kind, ItemKind::Loading { .. }),
            "{items:?}"
        );
        assert_eq!(
            items[1].kind,
            ItemKind::UserMessage {
                text: "Steer it".to_owned(),
                note: None,
                sending: true,
            }
        );
        sending.abort();
    }

    #[test]
    fn a_send_whose_success_could_not_be_read_may_have_been_sent_and_is_not_sent_again() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let admitted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = admitted.clone();
        let (opened, drafts, reference, _session, _directory) = opened_over(
            &runtime,
            answering(Arc::new(move |method, path| {
                if method == http::Method::POST && path.ends_with("/prompts") {
                    counted.fetch_add(1, Ordering::AcqRel);
                    return Box::pin(async { Ok(cut_off()) });
                }
                Box::pin(std::future::pending())
            })),
        );
        let failed = runtime.block_on(opened.send("One more thing".to_owned()));
        assert_eq!(
            failed,
            Err(ActError::Unreachable {
                message: "The server did not answer".to_owned()
            }),
            "not refused: the Server may hold it"
        );
        assert_eq!(admitted.load(Ordering::Acquire), 1, "never sent again");
        assert_eq!(drafts.get(&reference).text, "One more thing");
        let mut frame = opened.frame.clone();
        let note = runtime.block_on(async {
            tokio::time::timeout(
                SHOWN_WITHIN,
                frame.wait_for(|frame| frame.composer.as_ref().is_some_and(|c| c.note.is_some())),
            )
            .await
            .expect("shown in time")
            .expect("the frame stands")
            .composer
            .clone()
            .and_then(|composer| composer.note)
        });
        assert_eq!(note.as_deref(), Some(frame::MAY_HAVE_BEEN_SENT));
    }

    #[test]
    fn a_stop_whose_answer_was_lost_gives_back_a_prompt_the_session_then_shows_withdrawn() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let session_id = Arc::new(Mutex::new(None::<SessionId>));
        let prompt = shaman_protocol::Prompt {
            id: PromptId::new(),
            text: "Tidy the build".to_owned(),
            skill_invocations: Vec::new(),
            attachments: Vec::new(),
            delivery: PromptDelivery::Steer,
            admission_order: shaman_protocol::PromptOrder(1),
            status: shaman_protocol::PromptStatus::Pending,
            withdrawal: None,
            author: None,
            taken: None,
        };
        let (more, first_more) = tokio::sync::mpsc::unbounded_channel();
        let first_more = Arc::new(Mutex::new(Some(first_more)));
        let streams = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let answers: Arc<Answers> = {
            let session_id = session_id.clone();
            let prompt = prompt.clone();
            Arc::new(move |method, path| {
                let id = session_id.lock().unwrap().expect("the Session is known");
                if method == http::Method::POST && path.ends_with("/interrupt") {
                    return Box::pin(async { Ok(cut_off()) });
                }
                if path.ends_with("/events") {
                    let mut snapshot = shaman_client::testing::session_snapshot(id, 1);
                    let mut prompt = prompt.clone();
                    let opened = streams.fetch_add(1, Ordering::AcqRel);
                    if opened == 0 {
                        snapshot.prompts = vec![prompt];
                        let more = first_more.lock().unwrap().take().unwrap();
                        return Box::pin(async move { Ok(stream_from(&snapshot, more)) });
                    }
                    // Read again after the stop: the Prompt was withdrawn.
                    prompt.status = shaman_protocol::PromptStatus::Cancelled;
                    snapshot.revision = shaman_protocol::SessionRevision(2);
                    snapshot.prompts = vec![prompt];
                    let data = serde_json::to_string(&snapshot).unwrap();
                    return Box::pin(async move {
                        Ok(shaman_client::testing::open_stream_of(&[(
                            shaman_protocol::SESSION_SNAPSHOT_EVENT,
                            Some(2),
                            data,
                        )]))
                    });
                }
                Box::pin(std::future::pending())
            })
        };
        let (listings, _directory) = listings(&runtime);
        let (desk, reference) = listed_on("desk", "Map the workspace");
        *session_id.lock().unwrap() = Some(desk.sessions.as_ref().unwrap()[0].id());
        listings.hold_over(desk, answering(answers));
        let drafts = Drafts::default();
        let opened = open(
            runtime.handle(),
            &listings,
            &reference,
            Pacing {
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(5),
            },
            Asked {
                drafts: drafts.clone(),
                acts: Acts::default(),
                asking: Asking {
                    initial_backoff: Duration::from_millis(1),
                    max_backoff: Duration::from_millis(5),
                    bound: SHOWN_WITHIN,
                    dialling: Duration::from_millis(50),
                },
            },
        );
        conversation_until(&runtime, &opened, |items| {
            items.iter().any(|item| {
                matches!(&item.kind, ItemKind::UserMessage { text, .. } if text == "Tidy the build")
            })
        });
        drafts.set(&reference, "and the docs".to_owned(), 0);

        let stopped = runtime.block_on(opened.stop());
        assert_eq!(
            stopped,
            Err(ActError::Unreachable {
                message: "The server did not answer, so the work may have been stopped".to_owned()
            })
        );
        assert_eq!(drafts.get(&reference).text, "and the docs", "not yet known");

        // The stream drops and is read again, showing the Prompt withdrawn.
        drop(more);
        runtime.block_on(async {
            tokio::time::timeout(SHOWN_WITHIN, async {
                while drafts.get(&reference).text == "and the docs" {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            })
            .await
            .expect("given back in time");
        });
        assert_eq!(drafts.get(&reference).text, "Tidy the build\nand the docs");
    }

    /// What `attached` fills its attachment `filled` with, once it does.
    fn filled_within(
        runtime: &tokio::runtime::Runtime,
        filled: &mut watch::Receiver<Option<Filled>>,
    ) -> Filled {
        runtime.block_on(async {
            tokio::time::timeout(SHOWN_WITHIN, filled.wait_for(Option::is_some))
                .await
                .expect("filled in time")
                .expect("the attachment stands")
                .clone()
                .expect("filled")
        })
    }

    #[test]
    fn an_attachment_made_before_the_reader_first_polls_is_filled_all_the_same() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (listings, _directory) = listings(&runtime);
        let (desk, reference) = listed_on("desk", "Map the workspace");
        listings.hold(desk);
        let attached = Arc::new(Attachments::default());
        // Attached before the reader is so much as spawned.
        let (_, mut filled) = attached.attach(Target::Work(TurnId::new().to_string()));
        let transport = Arc::new(ScriptedTransport::new(Vec::new()));
        let _reader = reading(
            &runtime,
            &listings,
            &reference,
            transport,
            Duration::from_secs(60),
            attached,
        );
        let Filled::Work(work) = filled_within(&runtime, &mut filled) else {
            panic!("a Turn's work");
        };
        assert_eq!(work.empty.as_deref(), Some("Loading"), "nothing read yet");
    }

    #[test]
    fn an_attachment_to_a_session_never_read_is_filled_at_once_with_it_gone() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (listings, _directory) = listings(&runtime);
        let opened = open(
            runtime.handle(),
            &listings,
            &format!("nobody/{}", SessionId::new()),
            Pacing {
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(5),
            },
            Asked {
                drafts: Drafts::default(),
                acts: Acts::default(),
                asking: Asking {
                    initial_backoff: Duration::from_millis(1),
                    max_backoff: Duration::from_millis(5),
                    bound: SHOWN_WITHIN,
                    dialling: Duration::from_millis(50),
                },
            },
        );
        let (_, mut work) = opened
            .attached
            .attach(Target::Work(TurnId::new().to_string()));
        let Filled::Work(work) = filled_within(&runtime, &mut work) else {
            panic!("a Turn's work");
        };
        assert_eq!(
            work.empty.as_deref(),
            Some("This turn is no longer in the session")
        );
        let (_, mut detail) = opened.attached.attach(Target::Detail(
            shaman_protocol::ActivityId::new().to_string(),
        ));
        let Filled::Detail(detail) = filled_within(&runtime, &mut detail) else {
            panic!("an Activity's detail");
        };
        assert_eq!(detail.sections[0].body, "This is no longer in the session");
    }

    /// Hears nothing it keeps.
    struct Deaf;

    impl WorkListener for Deaf {
        fn changed(&self, _: TurnWork) {}
    }

    impl DetailListener for Deaf {
        fn changed(&self, _: ActivityDetail) {}
    }

    #[test]
    fn an_attachment_is_filled_no_more_once_its_screen_goes() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (opened, _drafts, _reference, _session, _directory) = opened_over(
            &runtime,
            answering(Arc::new(|_, _| Box::pin(std::future::pending()))),
        );
        let attached = || opened.attached.held().filling.len();
        let work = opened.watch_work(TurnId::new().to_string(), Arc::new(Deaf));
        let detail = opened.watch_detail(
            shaman_protocol::ActivityId::new().to_string(),
            Arc::new(Deaf),
        );
        assert_eq!(attached(), 2, "attached while their screens are up");
        let detached_within = |count: usize| {
            runtime.block_on(async {
                tokio::time::timeout(SHOWN_WITHIN, async {
                    while attached() != count {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .expect("detached in time");
            });
        };
        detail.cancel();
        detached_within(1);
        // Dropped as Swift drops it when the stream ends, it detaches too.
        drop(work);
        detached_within(0);
    }
}
