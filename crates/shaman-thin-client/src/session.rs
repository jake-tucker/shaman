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
//! queued for after it; a queued Prompt steered now or withdrawn; a stop; a
//! Decision on an Approval; and a Questionnaire answered or declined whole.
//! A Prompt sent stands at once as a bubble marked as sending; where it fails
//! it goes, and its text is given back to the composer's draft. Its own Interventions are dismissed and reviewed on the
//! screen alone, asking nothing of the Server.
//!
//! While a screen pushed from it is up, a Turn's work or an Activity's
//! detail is attached to it, filled again from the Session as it changes and
//! made no more once that screen goes.
//!
//! It holds the tree it belongs to for as long as it is open, which frames
//! it — the button that opens the tree's sheet, a Subagent's name and Model,
//! whether a Subagent's bar offers Stop — and fills the sheet while it is up;
//! a working Subagent is stopped from there.
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
    AdmitPromptRequest, InitialPrompt, InterruptOutcome, PromptDelivery, PromptId,
    QuestionnaireSubmission, SessionId, SessionSnapshot,
};
use tokio::{runtime::Handle, sync::watch, task::JoinHandle};

use crate::{
    ActError, Subscription,
    acts::{self, Acts, Asking, Unanswered},
    beginning::{self, Progress, Provisional},
    behind::Behind,
    conversation::{self, ConversationItem},
    drafts::Drafts,
    frame::{self, Cut, GONE_REMOTE, Local, Outgoing, SessionFrame, frame_of},
    intervention::{self, DecisionKind, GivenAnswer, InterventionId},
    listing::{Listed, Listings},
    tree::{self, InTree, Known, Lease, Pairings, SubagentTree, TreeState, Trees},
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
    /// The tree it belongs to, as it sees it, for as long as it is open.
    tree: watch::Receiver<TreeState>,
    /// The Remotes this phone holds a Pairing with, which the tree's entries
    /// on other Servers are opened through.
    pairings: watch::Receiver<Pairings>,
    /// The Remote it is on, by its key, and its identity there, where its
    /// reference names them.
    named: Option<(String, SessionId)>,
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
    pub landing_note: Arc<watch::Sender<Option<String>>>,
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

    /// Tells `listener` the tree the Session belongs to, as its sheet shows
    /// it, and again on every change, until the subscription is cancelled,
    /// as the sheet goes.
    pub fn watch_tree(&self, listener: Arc<dyn TreeListener>) -> Arc<Subscription> {
        let Some((remote, open)) = self.named.clone() else {
            // A Session never read belongs to no tree this phone can read.
            let none = TreeState {
                reading: tree::Reading::Gone,
                known: HashMap::new(),
            };
            listener.changed(tree::sheet(
                &none,
                "",
                SessionId::new(),
                &Pairings::default(),
            ));
            return Subscription::of(self.runtime.spawn(async {}));
        };
        let (mut tree, mut pairings) = (self.tree.clone(), self.pairings.clone());
        // Told again as the tree moves, and as the phone's Pairings do, which
        // say which of the tree's entries on other Servers it opens.
        let task = self.runtime.spawn(async move {
            loop {
                let state = tree.borrow_and_update().clone();
                let paired = pairings.borrow_and_update().clone();
                listener.changed(tree::sheet(&state, &remote, open, &paired));
                tokio::select! {
                    changed = tree.changed() => if changed.is_err() { break },
                    changed = pairings.changed() => if changed.is_err() { break },
                }
            }
        });
        Subscription::of(task)
    }

    /// The Session, as the core is handed it back: its Origin's key and its
    /// identity there, which for a Session being begun the core chose.
    pub fn reference(&self) -> String {
        self.acting.reference.clone()
    }

    /// Stops the working Subagent `subagent` names, as its entry in the tree
    /// hands it back, and whatever it delegated.
    pub async fn stop_subagent(&self, subagent: String) -> Result<(), ActError> {
        let acting = Acting {
            reference: subagent,
            ..self.acting.clone()
        };
        self.act(async move { acting.stop_alone().await }).await
    }

    /// Ends the Session's reading; it cancels nothing on the Server, and a
    /// Session being begun goes on being begun. One refused, or whose answer
    /// never came, is discarded, its Prompt's text kept as the Landing's
    /// draft, and the Landing saying where it may have begun all the same.
    pub fn close(&self) {
        let mut discarded = None;
        self.acting.local.send_modify(|local| {
            local.closed = true;
            if local
                .provisional
                .as_ref()
                .is_some_and(|provisional| !provisional.starting())
            {
                discarded = local.provisional.take().map(|provisional| {
                    let maybe = provisional.progress == Progress::MayHaveBegun;
                    (provisional.prompt.text, maybe)
                });
            }
        });
        if let Some((text, may_have_begun)) = discarded {
            self.acting.left(&text, may_have_begun);
        }
        if let Some(reader) = self
            .reader
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            reader.abort();
        }
    }

    /// Asks again to begin the Session being begun, whose Server refused it
    /// or never answered, with the same Prompt, which the Server answers as
    /// the same creation, answering once its Server has: the refusal, or the
    /// Session made, arrives by the frame and the conversation.
    pub async fn try_again(&self) -> Result<(), ActError> {
        let acting = self.acting.clone();
        self.act(async move { acting.begin_again(None).await })
            .await
    }

    /// Sends `text` as a Prompt: it steers the working Turn, or begins one.
    /// It stands at once as a bubble marked as sending; where the send fails
    /// the bubble goes and the text is given back to the composer's draft.
    /// On a Session being begun whose Server refused it, or never answered,
    /// it replaces the Prompt and asks again; where the Server holds the
    /// Session begun with the first after all, the screen turns into it and
    /// this one goes back to the composer, saying so.
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

    /// Answers the Questionnaire `questionnaire` names with `answers`, one
    /// for each of its Questions in their order, once its Server has. Answers
    /// whose delivery was lost are never sent again: the Questionnaire waits
    /// behind "Review", as one dismissed does, until the Session shows it
    /// answered.
    pub async fn answer(
        &self,
        questionnaire: String,
        answers: Vec<GivenAnswer>,
    ) -> Result<(), ActError> {
        let acting = self.acting.clone();
        let submission = QuestionnaireSubmission::Answer {
            answer: intervention::answer_of(answers),
        };
        self.act(async move {
            acting
                .submit(
                    &questionnaire,
                    submission,
                    "The server did not answer, so the answers may have been sent",
                )
                .await
        })
        .await
    }

    /// Declines the Questionnaire `questionnaire` names whole, once its
    /// Server has, as an answer is sent.
    pub async fn decline(&self, questionnaire: String) -> Result<(), ActError> {
        let acting = self.acting.clone();
        self.act(async move {
            acting
                .submit(
                    &questionnaire,
                    QuestionnaireSubmission::Decline,
                    "The server did not answer, so the questionnaire may have been declined",
                )
                .await
        })
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

/// Hears the tree an open Session belongs to, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait TreeListener: Send + Sync {
    fn changed(&self, tree: SubagentTree);
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
    /// What it is filled with from `snapshot`, of a Session on the Remote
    /// `remote` names, a Session it names on another Server opened through
    /// `pairings`.
    fn fill(
        &self,
        snapshot: Option<&SessionSnapshot>,
        remote: &str,
        pairings: &Pairings,
    ) -> Filled {
        match self {
            Self::Work(turn) => Filled::Work(work::turn_work(snapshot, turn, remote, pairings)),
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

    /// Fills every attachment from `snapshot`, of a Session on the Remote
    /// `remote` names, telling only those it changes.
    fn fill(&self, snapshot: Option<&SessionSnapshot>, remote: &str, pairings: &Pairings) {
        for (target, filling) in self.held().filling.values() {
            let filled = target.fill(snapshot, remote, pairings);
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
    opened(runtime, listings, reference, pacing, asked, None)
}

/// Begins the Session `provisional` says, on the Remote `reference` names
/// it under, and opens it at once as the Provisional Session, until its
/// Server answers with the Session it made, which replaces it in place. The
/// beginning goes on whatever becomes of the screen.
pub(crate) fn begin(
    runtime: &Handle,
    listings: &Listings,
    reference: &str,
    pacing: Pacing,
    asked: Asked,
    provisional: Provisional,
) -> Arc<OpenSession> {
    let session = opened(
        runtime,
        listings,
        reference,
        pacing,
        asked,
        Some(provisional),
    );
    let acting = session.acting.clone();
    let under_way = acting.acts.begin();
    runtime.spawn(async move {
        let _under_way = under_way;
        acting.create().await;
    });
    session
}

fn opened(
    runtime: &Handle,
    listings: &Listings,
    reference: &str,
    pacing: Pacing,
    asked: Asked,
    provisional: Option<Provisional>,
) -> Arc<OpenSession> {
    let opening = listings.opening(reference);
    let beginning = provisional.is_some();
    let listed = opening.as_ref().map(|opening| opening.listed.clone());
    let remote = reference
        .split_once('/')
        .map_or(reference, |(remote, _)| remote)
        .to_owned();
    // The tree it belongs to is held from the moment it opens: one already
    // read frames it at once. A Session being begun belongs to none until
    // its Server holds it.
    let lease = opening.as_ref().filter(|_| !beginning).map(|opening| {
        asked.trees.hold(
            &remote,
            opening.session_id,
            &opening.commands,
            opening.behind.clone(),
        )
    });
    let (tree_seeing, tree_seen) = match &lease {
        Some(lease) => (None, lease.seen()),
        None => {
            let (seeing, seen) = watch::channel(TreeState::default());
            (Some(seeing), seen)
        }
    };
    let named = opening
        .as_ref()
        .map(|opening| (remote.clone(), opening.session_id));
    let in_tree = named.as_ref().map_or_else(InTree::default, |(_, open)| {
        tree::in_tree(&tree_seen.borrow(), *open, None)
    });
    let held = Local {
        provisional,
        ..Local::default()
    };
    let (frame, frame_seen) = watch::channel(frame_of(
        &remote,
        listed.as_ref(),
        None,
        None,
        &held,
        None,
        &in_tree,
    ));
    let (conversation, conversation_seen) = watch::channel(shown(
        None,
        held.provisional
            .as_ref()
            .map_or_else(conversation::loading, frame::provisional_items),
    ));
    let local = Arc::new(watch::Sender::new(held));
    let reading = Arc::new(AtomicBool::new(true));
    let attached = Arc::new(if opening.is_some() {
        Attachments::default()
    } else {
        Attachments::unread()
    });
    let reader = opening.map(|opening| {
        let reader = Reader {
            tree: lease,
            tree_seen: tree_seen.clone(),
            beginning: tree_seeing.filter(|_| beginning),
            trees: asked.trees.clone(),
            remote,
            attached: attached.clone(),
            gone: None,
            drafts: asked.drafts.clone(),
            reading: reading.clone(),
            listings: listings.clone(),
            pairings: listings.pairings(),
            reference: reference.to_owned(),
            session_id: opening.session_id,
            commands: opening.commands,
            try_now: opening.try_now,
            listed: opening.listed,
            snapshot: None,
            local: Arc::downgrade(&local),
            unreachable: opening.unreachable,
            behind: opening.behind,
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
            landing_note: asked.landing_note,
        },
        attached,
        tree: tree_seen,
        pairings: listings.pairings(),
        named,
    })
}

/// What the core hands each open Session for its acts, and the trees it
/// holds the one it belongs to among.
pub(crate) struct Asked {
    pub drafts: Drafts,
    pub acts: Acts,
    pub asking: Asking,
    pub trees: Trees,
    /// What the Landing says of a beginning left before it was known.
    pub landing_note: Arc<watch::Sender<Option<String>>>,
}

/// Said where the Session's Server no longer holds it.
const GONE_SESSION: &str = "This session no longer exists";

/// Said where a queued Prompt an act names is no longer queued.
const GONE_PROMPT: &str = "That prompt is no longer queued";

/// Said where a Prompt with nothing in it is sent.
const EMPTY_PROMPT: &str = "Write something to send";

/// How long a Session being begun that the listing carries is given to be
/// read.
const LOOKING: Duration = Duration::from_secs(5);

/// Said where an act is asked of a Session its Server does not hold yet.
const NOT_YET_BEGUN: &str = "Nothing is sent until the session has begun";

/// Said where an Approval a Decision names no longer waits on one.
const GONE_APPROVAL: &str = "That approval is no longer waiting";

/// Said where a Questionnaire answers name no longer waits on any.
const GONE_QUESTIONNAIRE: &str = "That questionnaire is no longer waiting";

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

    /// Whether the Session is still being begun, its Server yet to be seen
    /// holding it.
    fn beginning(&self) -> bool {
        self.local.borrow().provisional.is_some()
    }

    /// Refuses an act on a Session its Server does not hold yet.
    fn begun(&self) -> Result<(), ActError> {
        if self.beginning() {
            return Err(ActError::Invalid {
                message: NOT_YET_BEGUN.to_owned(),
            });
        }
        Ok(())
    }

    /// Asks again to begin the Session being begun whose Server refused it,
    /// or never answered: with `text` in place of its Prompt where one is
    /// written instead, and otherwise as the same creation.
    async fn begin_again(&self, text: Option<String>) -> Result<(), ActError> {
        let mut asked = false;
        self.local.send_modify(|local| {
            let Some(provisional) = local
                .provisional
                .as_mut()
                .filter(|provisional| !provisional.starting())
            else {
                return;
            };
            match text.clone() {
                Some(text) => provisional.replace(text),
                None => provisional.ask_again(),
            }
            local.note = None;
            asked = true;
        });
        if !asked {
            if let Some(text) = text {
                self.drafts.give_back(&self.reference, &text);
            }
            return Err(ActError::Invalid {
                message: NOT_YET_BEGUN.to_owned(),
            });
        }
        self.create().await;
        Ok(())
    }

    /// Asks the Session's Server to begin it, as it is being begun, until it
    /// answers or the act's bound passes. The Server answers a repeat of the
    /// same creation, the same Session id and the same Prompt id, with the
    /// Session it made, so one whose answer was lost is asked again too.
    /// Where it is not answered with the Session, the Server is asked whether
    /// it holds the Session after all, as it does where an earlier asking
    /// began it and this one was refused for it; and where it does, that is
    /// the Session made. Otherwise it stands refused, in the Server's words,
    /// or, where any asking may have reached the Server, as having maybe
    /// begun. Where the screen has gone by then, its Prompt's text is kept as
    /// the Landing's draft.
    async fn create(&self) {
        let Some(session_id) = self
            .local
            .borrow()
            .provisional
            .as_ref()
            .map(|provisional| provisional.session_id)
        else {
            return;
        };
        let answered = match self.opening() {
            Err(gone) => Err(Progress::Refused(gone.to_string())),
            Ok(opening) => {
                let commands = opening.commands;
                let mut try_now = opening.try_now;
                if let Err(refused) = self.prepare(&commands, &mut try_now).await {
                    self.stand(Err(refused));
                    return;
                }
                let Some(request) = self
                    .local
                    .borrow()
                    .provisional
                    .as_ref()
                    .map(Provisional::request)
                else {
                    return;
                };
                let created = acts::deliver_again(self.asking, &mut try_now, || {
                    let (commands, request) = (commands.clone(), request.clone());
                    async move { commands.create_session(request).await }
                })
                .await;
                match created {
                    Ok(created) => Ok(created),
                    Err(unanswered) => {
                        // An asking no way carried reached no Server to ask.
                        let reached = !matches!(
                            unanswered,
                            Unanswered::NotDone(ActError::Unreachable { .. })
                        );
                        let held = if reached {
                            self.held(&commands, &mut try_now, session_id).await
                        } else {
                            None
                        };
                        match (held, unanswered) {
                            (Some(held), _) => Ok(held),
                            (None, Unanswered::MaybeDone) => Err(Progress::MayHaveBegun),
                            (None, Unanswered::NotDone(refused)) => {
                                Err(Progress::Refused(refused.to_string()))
                            }
                        }
                    }
                }
            }
        };
        self.stand(answered);
    }

    /// Prepares the new Worktree the Session being begun is to begin in,
    /// where it has one yet to be prepared, for the Provider of the Agent its
    /// Server says a Session would get: until the Server answers or the
    /// act's bound passes, a preparation that resumes however often it is
    /// asked. Once it stands ready the Session is begun there, its bar
    /// reading "Starting"; otherwise the beginning stands refused, why in
    /// the Server's words, nothing having begun.
    async fn prepare(
        &self,
        commands: &SessionCommands,
        try_now: &mut TryNow,
    ) -> Result<(), Progress> {
        if !self
            .local
            .borrow()
            .provisional
            .as_ref()
            .is_some_and(Provisional::preparing)
        {
            return Ok(());
        }
        let health = acts::deliver_again(self.asking, try_now, || {
            let commands = commands.clone();
            async move { commands.health().await }
        })
        .await
        .map_err(|unanswered| match unanswered {
            Unanswered::NotDone(refused) => Progress::Refused(refused.to_string()),
            Unanswered::MaybeDone => {
                Progress::Refused(beginning::PREPARATION_UNANSWERED.to_owned())
            }
        })?;
        let provider = beginning::worktree_provider(&health)
            .ok_or_else(|| Progress::Refused(beginning::NO_AGENT_FOR_WORKTREE.to_owned()))?;
        let Some(request) = self
            .local
            .borrow()
            .provisional
            .as_ref()
            .and_then(|provisional| provisional.preparation(provider))
        else {
            return Ok(());
        };
        let prepared = acts::deliver_again(self.asking, try_now, || {
            let (commands, request) = (commands.clone(), request.clone());
            async move { commands.prepare_checkout(request).await }
        })
        .await
        .map_err(|unanswered| match unanswered {
            Unanswered::NotDone(refused) => Progress::Refused(refused.to_string()),
            Unanswered::MaybeDone => {
                Progress::Refused(beginning::PREPARATION_UNANSWERED.to_owned())
            }
        })?;
        if let Some(error) = prepared.error {
            return Err(Progress::Refused(error));
        }
        let destination = prepared.preparation.destination.path;
        self.local.send_modify(|local| {
            if let Some(provisional) = local.provisional.as_mut() {
                provisional.prepared(destination);
            }
        });
        Ok(())
    }

    /// Stands the Session being begun as `answered` says: turning into the
    /// Session its Server made, or standing refused, or as having maybe
    /// begun. Where the screen has gone by then, its Prompt's text is kept as
    /// the Landing's draft.
    fn stand(&self, answered: Result<SessionSnapshot, Progress>) {
        let mut left = None;
        self.local.send_modify(|local| {
            let closed = local.closed;
            let Some(provisional) = local.provisional.as_mut() else {
                return;
            };
            match answered {
                Ok(created) => {
                    if closed && !holds(&created, provisional) {
                        // Begun with an earlier Prompt: this one was not sent.
                        left = Some((provisional.prompt.text.clone(), false));
                    }
                    provisional.created = Some(Box::new(created));
                }
                Err(standing) if closed => {
                    left = Some((
                        provisional.prompt.text.clone(),
                        standing == Progress::MayHaveBegun,
                    ));
                    local.provisional = None;
                }
                Err(standing) => provisional.progress = standing,
            }
        });
        if let Some((text, may_have_begun)) = left {
            self.left(&text, may_have_begun);
        }
    }

    /// The Session `session_id`, where its Server answers that it holds it.
    async fn held(
        &self,
        commands: &SessionCommands,
        try_now: &mut TryNow,
        session_id: SessionId,
    ) -> Option<SessionSnapshot> {
        let asking = Asking {
            // As long as a dial may take, twice: the Server is asked one
            // thing more, not waited on again.
            bound: self.asking.dialling * 2,
            ..self.asking
        };
        acts::deliver(asking, try_now, || {
            let commands = commands.clone();
            async move { commands.read_session(session_id).await }
        })
        .await
        .ok()
    }

    /// Keeps the text of a beginning whose screen has gone as the Landing's
    /// draft, the Landing saying so where it `may_have_begun`.
    fn left(&self, text: &str, may_have_begun: bool) {
        self.drafts.give_back(beginning::LANDING_DRAFT, text);
        if may_have_begun {
            self.landing_note
                .send_replace(Some(beginning::MAYBE_BEGUN_LEFT.to_owned()));
        }
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
        if self.beginning() {
            // A Prompt written on a refused beginning replaces its own.
            return self.begin_again(Some(text)).await;
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
        self.begun()?;
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
        self.begun()?;
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
        // A Session being begun offers no Stop, and nothing is stopped.
        self.begun()?;
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

    /// Stops the Subagent the act's reference names alone, and whatever it
    /// delegated.
    async fn stop_alone(&self) -> Result<(), ActError> {
        let mut opening = self.opening()?;
        self.deliver(&mut opening, |commands, session_id| async move {
            commands
                .interrupt_session_reporting_outcome(session_id)
                .await
        })
        .await
        .map(drop)
        .map_err(|unanswered| {
            said(
                unanswered,
                "The server did not answer, so the subagent may have been stopped",
            )
        })
    }
}

impl Acting {
    async fn decide(&self, reference: &str, decision: DecisionKind) -> Result<(), ActError> {
        self.begun()?;
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
                local.dismissed.insert(InterventionId::Approval(approval));
            });
        }
        decided.map_err(|unanswered| {
            said(
                unanswered,
                "The server did not answer, so the decision may have been sent",
            )
        })
    }

    /// Hands the Questionnaire `reference` names `submission`, saying
    /// `maybe` where it may have been delivered.
    async fn submit(
        &self,
        reference: &str,
        submission: QuestionnaireSubmission,
        maybe: &str,
    ) -> Result<(), ActError> {
        self.begun()?;
        // The Session stands; only the Questionnaire is no longer there.
        let questionnaire =
            intervention::questionnaire_referenced(reference).ok_or_else(|| ActError::Refused {
                message: GONE_QUESTIONNAIRE.to_owned(),
            })?;
        let mut opening = self.opening()?;
        let submitted = self
            .deliver(&mut opening, |commands, session_id| {
                let submission = submission.clone();
                async move {
                    commands
                        .submit_questionnaire(session_id, questionnaire, submission)
                        .await
                }
            })
            .await;
        if submitted == Err(Unanswered::MaybeDone) {
            // It may have been delivered: it waits out of sight, and is not
            // offered again until the reader asks.
            self.local.send_modify(|local| {
                local
                    .dismissed
                    .insert(InterventionId::Questionnaire(questionnaire));
            });
        }
        submitted.map_err(|unanswered| said(unanswered, maybe))
    }
}

/// Whether `snapshot` holds the Prompt `provisional` was being begun with,
/// rather than one an earlier asking began it with.
fn holds(snapshot: &SessionSnapshot, provisional: &Provisional) -> bool {
    snapshot
        .prompts
        .iter()
        .any(|prompt| prompt.id == provisional.prompt.id)
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
    /// Its hold on the tree it belongs to, let go as the reading ends.
    tree: Option<Lease>,
    /// The tree as it sees it.
    tree_seen: watch::Receiver<TreeState>,
    /// While the Session is being begun, what it is to see its tree through
    /// once its Server holds it.
    beginning: Option<watch::Sender<TreeState>>,
    trees: Trees,
    /// The Remote it is on, by its key.
    remote: String,
    /// What screens pushed from it have attached, which it fills.
    attached: Arc<Attachments>,
    /// Why the Session is gone, once it is.
    gone: Option<String>,
    /// Where a Prompt a stop withdrew goes back to.
    drafts: Drafts,
    /// Whether the Session's stream is still read.
    reading: Arc<AtomicBool>,
    listings: Listings,
    /// The Remotes this phone holds a Pairing with, which a row among a
    /// Turn's work naming a Session on another Server opens it through.
    pairings: watch::Receiver<Pairings>,
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
    /// The side too old for the other, where the Session's Remote and the
    /// phone can no longer work together, as the listing reads it.
    behind: watch::Receiver<Option<Behind>>,
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
/// this phone may read it no more. One refused as the phone or its Server
/// being too old for the other is not gone: the Pairing stands, so the
/// Session holds what it showed and is read again, as its stream dropping
/// is, until that side is updated.
fn refusal(refused: &SessionStreamError) -> Ended {
    if refused.deleted_session().is_some() {
        Ended::Gone(GONE_SESSION.to_owned())
    } else if refused
        .remote_status()
        .and_then(Behind::of_status)
        .is_some()
    {
        Ended::Dropped
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
        if let Some(seeing) = self.beginning.take()
            && let Err(why) = self
                .await_beginning(seeing, &mut listing, &mut local, &mut attaching)
                .await
        {
            return self.show_gone(why, local, attaching).await;
        }
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

    /// Waits for the Session being begun to be made, showing meanwhile what
    /// the screen holds of it, and looking for it in the listing whenever
    /// that moves or the beginning stands unanswered: then the Session its
    /// Server holds stands in its place, and its tree is seen through
    /// `seeing`. A Prompt written instead that the Session was not begun
    /// with goes back to the composer, saying so. Fails with why where its
    /// Remote is no longer held.
    async fn await_beginning(
        &mut self,
        seeing: watch::Sender<TreeState>,
        listing: &mut watch::Receiver<crate::Listing>,
        local: &mut watch::Receiver<Local>,
        attaching: &mut watch::Receiver<u64>,
    ) -> Result<(), String> {
        loop {
            let provisional = local.borrow_and_update().provisional.clone();
            let Some(provisional) = provisional else {
                // Discarded: nothing stands to be made.
                return Ok(());
            };
            if let Some(created) = provisional.created.clone() {
                let superseded = !holds(&created, &provisional);
                self.snapshot = Some(*created);
                if let Some(held) = self.local.upgrade() {
                    held.send_modify(|local| {
                        local.provisional = None;
                        if superseded {
                            local.note = Some(beginning::BEGUN_WITH_THE_FIRST.to_owned());
                        }
                    });
                }
                if superseded {
                    self.drafts
                        .give_back(&self.reference, &provisional.prompt.text);
                }
                self.tree = Some(self.trees.hold_through(
                    &self.remote,
                    self.session_id,
                    &self.commands,
                    seeing,
                    self.behind.clone(),
                ));
                self.show();
                return Ok(());
            }
            self.show();
            if provisional.progress == Progress::MayHaveBegun {
                self.look_for_it().await;
            }
            tokio::select! {
                changed = local.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                }
                Ok(()) = attaching.changed() => self.fill_attached(),
                Ok(()) = listing.changed() => {
                    if self.relist() == Relisted::Gone {
                        return Err(GONE_REMOTE.to_owned());
                    }
                    self.look_for_it().await;
                }
                Ok(()) = self.unreachable.changed() => self.show_frame(),
                Ok(()) = self.behind.changed() => self.show_frame(),
            }
        }
    }

    /// Where the listing carries the Session being begun, reads it, and
    /// takes it as the Session made.
    async fn look_for_it(&mut self) {
        let listed = self
            .listings
            .opening(&self.reference)
            .is_some_and(|opening| opening.listed.title.is_some());
        if !listed {
            return;
        }
        let read = tokio::time::timeout(LOOKING, self.commands.read_session(self.session_id)).await;
        let Ok(Ok(held)) = read else {
            tracing::debug!("a session being begun is listed and was not read");
            return;
        };
        if let Some(local) = self.local.upgrade() {
            local.send_modify(|local| {
                if let Some(provisional) = local.provisional.as_mut() {
                    provisional.created = Some(Box::new(held));
                }
            });
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
                Ok(()) = self.pairings.changed() => self.fill_attached(),
                Ok(()) = self.tree_seen.changed() => self.show_frame(),
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
        // Refused as the phone or its Server being behind, which the reading
        // may have heard before this Session asked: the screen says so now.
        if self.behind.borrow().is_some() {
            self.show();
        }
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
                Ok(()) = self.pairings.changed() => self.fill_attached(),
                Ok(()) = self.unreachable.changed() => self.show_frame(),
                Ok(()) = self.tree_seen.changed() => self.show_frame(),
                // The Remote works with the phone again, so the Session is
                // read again now; or no longer does, which the screen says.
                Ok(()) = self.behind.changed() => {
                    if self.behind.borrow().is_none() {
                        return Ok(());
                    }
                    self.show();
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
                Ok(()) = self.pairings.changed() => self.fill_attached(),
                Ok(()) = self.unreachable.changed() => self.show_frame(),
                Ok(()) = self.tree_seen.changed() => self.show_frame(),
                Ok(()) = self.behind.changed() => self.show(),
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
                // Passed over, as of a newer Server, which is said once,
                // whatever becomes of the rest.
                if update.skips_a_change() {
                    self.listings.notices().newer(&self.remote);
                }
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
        // What the tree does not say of the Session, its Provider and the
        // Subagents it spawned through the Broker, the tree is told.
        if let (Some(snapshot), Some(tree)) = (&self.snapshot, &self.tree) {
            tree.know(Known::of(snapshot));
        }
        self.show_frame();
        // A Prompt being sent stands at the foot even before the Session has
        // been read, beneath the loading mark.
        let local = self.local();
        let items = frame::with_outgoing(
            match (&self.snapshot, &local.provisional) {
                (Some(snapshot), _) => conversation::conversation(snapshot),
                (None, Some(provisional)) => frame::provisional_items(provisional),
                // Not read, and not to be until the side behind is updated:
                // why stands where the conversation would.
                (None, None) => match *self.behind.borrow() {
                    Some(behind) => conversation::unread(
                        &behind.unread(self.listed.origin.as_deref().unwrap_or("The server")),
                    ),
                    None => conversation::loading(),
                },
            },
            &local,
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
        let pairings = self.pairings.borrow().clone();
        self.attached
            .fill(self.snapshot.as_ref(), &self.remote, &pairings);
    }

    /// What keeps the Session apart for now, where anything does: its
    /// Remote being Unreachable, or it and the phone no longer working
    /// together.
    fn cut(&self) -> Option<Cut> {
        if self.unreachable.borrow().is_some() {
            return Some(Cut::Unreachable);
        }
        (*self.behind.borrow()).map(Cut::Behind)
    }

    fn show_frame(&self) {
        let in_tree = tree::in_tree(
            &self.tree_seen.borrow(),
            self.session_id,
            self.snapshot.as_ref(),
        );
        let frame = frame_of(
            &self.remote,
            Some(&self.listed),
            self.snapshot.as_ref(),
            self.gone.clone(),
            &self.local(),
            self.cut(),
            &in_tree,
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
        // A Session gone holds its tree no more.
        self.tree = None;
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
        TreeButton,
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
            "desk-key",
            Some(&opening.listed),
            None,
            None,
            &Local::default(),
            None,
            &InTree::default(),
        ));
        let (conversation, _) = watch::channel(shown(None, conversation::loading()));
        let try_now = transport.try_now();
        let reader = Reader {
            tree: None,
            tree_seen: watch::channel(TreeState::default()).1,
            beginning: None,
            trees: Trees::new(runtime.handle().clone(), listings.notices()),
            remote: reference.split_once('/').unwrap().0.to_owned(),
            attached,
            gone: None,
            drafts: Drafts::default(),
            reading: Arc::new(AtomicBool::new(true)),
            listings: listings.clone(),
            pairings: listings.pairings(),
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
            behind: opening.behind,
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
    fn a_refusal_as_either_side_being_behind_keeps_the_session_and_reads_it_again() {
        for code in [
            SessionErrorCode::PairingPeerBehind,
            SessionErrorCode::PairingServerBehind,
        ] {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let (listings, _directory) = listings(&runtime);
            let (desk, reference) = listed_on("desk", "Map the workspace");
            listings.hold(desk);
            let behind = SessionError {
                code,
                message: "update Shaman somewhere".to_owned(),
                unreachable: None,
            };
            let transport = Arc::new(ScriptedTransport::new(vec![
                Ok(shaman_client::testing::refusal(
                    http::StatusCode::CONFLICT,
                    &behind,
                )),
                Ok(shaman_client::testing::refusal(
                    http::StatusCode::CONFLICT,
                    &behind,
                )),
            ]));
            let (_reader, frame) = reading(
                &runtime,
                &listings,
                &reference,
                transport.clone(),
                Duration::from_millis(1),
                Arc::default(),
            );
            runtime.block_on(async {
                tokio::time::timeout(SHOWN_WITHIN, async {
                    while transport.asked() < 3 {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .expect("asked again");
            });
            assert_eq!(frame.borrow().gone, None, "{code:?} keeps the Session");
        }
    }

    #[test]
    fn a_change_or_activity_a_newer_server_sent_holds_its_place_and_is_said_once() {
        use shaman_protocol::{
            Activity, ActivityId, SESSION_SNAPSHOT_EVENT, SESSION_UPDATED_EVENT, TranscriptItem,
            Turn, TurnStatus,
        };

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (listings, _directory) = listings(&runtime);
        let (desk, reference) = listed_on("desk", "Map the workspace");
        listings.hold(desk);
        let session_id = listings.opening(&reference).unwrap().session_id;
        let mut snapshot = shaman_client::testing::session_snapshot(session_id, 3);
        let mut turn = Turn::unprompted(None);
        turn.status = TurnStatus::Completed;
        let turn_id = turn.id;
        snapshot.turns.push(turn);
        let placeholder = ActivityId::new();
        snapshot.activities.push(Activity::Unknown {
            id: placeholder,
            turn_id: Some(turn_id),
            kind: "deploy_preview".to_owned(),
        });
        snapshot.transcript.push(TranscriptItem::Activity {
            activity_id: placeholder,
        });
        let skipping = |revision: u64| {
            serde_json::json!({
                "session_id": session_id,
                "revision": revision,
                "changes": [{ "type": "session_mood_changed", "mood": "sunny" }],
            })
            .to_string()
        };
        let transport = Arc::new(ScriptedTransport::new(vec![Ok(
            shaman_client::testing::open_stream_of(&[
                (
                    SESSION_SNAPSHOT_EVENT,
                    Some(3),
                    serde_json::to_string(&snapshot).unwrap(),
                ),
                (SESSION_UPDATED_EVENT, Some(4), skipping(4)),
                (SESSION_UPDATED_EVENT, Some(5), skipping(5)),
            ]),
        )]));
        let attached = Arc::new(Attachments::default());
        let (_, mut filled) = attached.attach(Target::Work(turn_id.to_string()));
        let _reader = reading(
            &runtime,
            &listings,
            &reference,
            transport,
            Duration::from_secs(60),
            attached,
        );

        let mut notices = listings.notices().shown();
        let said = runtime.block_on(async {
            tokio::time::timeout(SHOWN_WITHIN, notices.wait_for(Option::is_some))
                .await
                .expect("said in time")
                .expect("the notices stand")
                .clone()
                .unwrap()
        });
        assert_eq!(
            said.words,
            "The server runs a newer Shaman; some things may not show"
        );
        listings.notices().seen(said.id);
        assert_eq!(*notices.borrow(), None, "said once, for both changes");

        let work = runtime.block_on(async {
            tokio::time::timeout(
                SHOWN_WITHIN,
                filled.wait_for(
                    |work| matches!(work, Some(Filled::Work(work)) if !work.items.is_empty()),
                ),
            )
            .await
            .expect("filled in time")
            .expect("the attachment stands")
            .clone()
        });
        let Some(Filled::Work(work)) = work else {
            panic!("a Turn's work");
        };
        let rows = work
            .items
            .iter()
            .filter_map(|item| match &item.kind {
                work::WorkKind::Row { row } => Some(row.title.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rows, ["Not shown: deploy preview, from a newer Shaman"]);
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
                trees: Trees::new(runtime.handle().clone(), listings.notices()),
                landing_note: Arc::new(watch::Sender::new(None)),
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
                trees: Trees::new(runtime.handle().clone(), listings.notices()),
                landing_note: Arc::new(watch::Sender::new(None)),
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
                trees: Trees::new(runtime.handle().clone(), listings.notices()),
                landing_note: Arc::new(watch::Sender::new(None)),
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

    /// Keeps the last tree it heard.
    struct Kept(Mutex<Option<SubagentTree>>);

    impl TreeListener for Kept {
        fn changed(&self, tree: SubagentTree) {
            *self.0.lock().unwrap() = Some(tree);
        }
    }

    #[test]
    fn a_session_never_read_has_a_tree_that_shows_nothing_and_never_says_loading() {
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
                trees: Trees::new(runtime.handle().clone(), listings.notices()),
                landing_note: Arc::new(watch::Sender::new(None)),
            },
        );
        let kept = Arc::new(Kept(Mutex::new(None)));
        let _subscription = opened.watch_tree(kept.clone());
        let tree = kept.0.lock().unwrap().clone().expect("told at once");
        assert_eq!((tree.loading, tree.failed), (None, None));
        assert!(tree.rows.is_empty());
        assert_eq!(opened.frame.borrow().tree, None);
    }

    /// The tree's sheet over a Session that has spawned a Subagent, read
    /// over a Server whose tree answers as `tree` does, once the frame
    /// offers the button that opens it, and once the sheet shows what
    /// `shown` looks for.
    fn sheet_over(
        tree: impl Fn() -> futures_util::future::BoxFuture<
            'static,
            Result<shaman_client::TransportResponse, SessionError>,
        > + Send
        + Sync
        + 'static,
        shown: impl Fn(&SubagentTree) -> bool,
    ) -> (SessionFrame, SubagentTree) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let session_id = Arc::new(Mutex::new(None::<SessionId>));
        let answers: Arc<Answers> = {
            let session_id = session_id.clone();
            Arc::new(move |_, path| {
                if path.ends_with("/subagent-tree") {
                    return tree();
                }
                if path.ends_with("/events") {
                    let id = session_id.lock().unwrap().expect("the Session is known");
                    let mut snapshot = shaman_client::testing::session_snapshot(id, 1);
                    snapshot
                        .activities
                        .push(shaman_protocol::Activity::Subagent {
                            id: shaman_protocol::ActivityId::new(),
                            turn_id: TurnId::new(),
                            status: shaman_protocol::ActivityStatus::Active,
                            name: "Explorer".to_owned(),
                            description: "Find what held the lock".to_owned(),
                            model: None,
                            session_id: SessionId::new(),
                            brokered: false,
                            duration_ms: None,
                            delegated_at: None,
                        });
                    let (more, rest) = tokio::sync::mpsc::unbounded_channel();
                    // Held open for as long as the stream is read.
                    std::mem::forget(more);
                    return Box::pin(async move { Ok(stream_from(&snapshot, rest)) });
                }
                Box::pin(std::future::pending())
            })
        };
        let (listings, _directory) = listings(&runtime);
        let (desk, reference) = listed_on("desk", "Map the workspace");
        *session_id.lock().unwrap() = Some(desk.sessions.as_ref().unwrap()[0].id());
        listings.hold_over(desk, answering(answers));
        let opened = open(
            runtime.handle(),
            &listings,
            &reference,
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
                trees: Trees::new(runtime.handle().clone(), listings.notices()),
                landing_note: Arc::new(watch::Sender::new(None)),
            },
        );
        let mut frame = opened.frame.clone();
        let framed = runtime.block_on(async {
            tokio::time::timeout(SHOWN_WITHIN, frame.wait_for(|frame| frame.tree.is_some()))
                .await
                .expect("the button offered in time")
                .expect("the frame stands")
                .clone()
        });
        let kept = Arc::new(Kept(Mutex::new(None)));
        let _subscription = opened.watch_tree(kept.clone());
        let sheet = runtime.block_on(async {
            tokio::time::timeout(SHOWN_WITHIN, async {
                loop {
                    if let Some(sheet) = kept.0.lock().unwrap().clone().filter(|sheet| shown(sheet))
                    {
                        return sheet;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("the sheet shown in time")
        });
        (framed, sheet)
    }

    #[test]
    fn a_session_with_subagents_offers_its_tree_and_the_sheet_says_loading_until_it_is_read() {
        let (framed, sheet) = sheet_over(
            || Box::pin(std::future::pending()),
            |sheet| sheet.loading.is_some(),
        );
        assert_eq!(
            framed.tree,
            Some(TreeButton {
                working: None,
                label: "Subagents".to_owned()
            }),
            "offered, uncounted, before its tree is read"
        );
        assert_eq!(sheet.loading.as_deref(), Some("Loading"));
        assert!(sheet.rows.is_empty());
    }

    #[test]
    fn a_session_whose_tree_cannot_be_read_offers_it_and_the_sheet_says_why() {
        let (framed, sheet) = sheet_over(
            || {
                Box::pin(async {
                    Ok(shaman_client::testing::refusal(
                        http::StatusCode::NOT_FOUND,
                        &SessionError {
                            code: SessionErrorCode::SessionNotFound,
                            message: "No such session".to_owned(),
                            unreachable: None,
                        },
                    ))
                })
            },
            |sheet| sheet.failed.is_some(),
        );
        assert!(framed.tree.is_some());
        let failed = sheet.failed.expect("why");
        assert!(
            failed.starts_with("Could not load subagents: ") && failed.contains("No such session"),
            "{failed}"
        );
    }

    #[test]
    fn a_stop_its_server_refuses_fails_in_the_servers_words() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (opened, _drafts, reference, _session, _directory) = opened_over(
            &runtime,
            answering(Arc::new(|method, path| {
                if method == http::Method::POST && path.ends_with("/interrupt") {
                    return Box::pin(async {
                        Ok(shaman_client::testing::refusal(
                            http::StatusCode::CONFLICT,
                            &SessionError {
                                code: SessionErrorCode::SubagentStopUnsupported,
                                message: "This provider cannot stop one subagent alone".to_owned(),
                                unreachable: None,
                            },
                        ))
                    });
                }
                Box::pin(std::future::pending())
            })),
        );
        let refused = runtime
            .block_on(opened.stop_subagent(reference))
            .expect_err("refused");
        let ActError::Refused { message } = refused else {
            panic!("refused in the Server's words: {refused:?}");
        };
        assert!(
            message.contains("This provider cannot stop one subagent alone"),
            "{message}"
        );
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

    /// The snapshot of the working Session `session`, as its stream sends
    /// it, waiting on the Approval `approval` names.
    fn waiting_on(session: SessionId, approval: shaman_protocol::ApprovalId) -> SessionSnapshot {
        let mut snapshot = shaman_client::testing::session_snapshot(session, 1);
        let mut turn = shaman_protocol::Turn::unprompted(None);
        turn.status = shaman_protocol::TurnStatus::Active;
        let turn_id = turn.id;
        snapshot.turns.push(turn);
        let activity_id = shaman_protocol::ActivityId::new();
        snapshot
            .activities
            .push(shaman_protocol::Activity::Approval {
                id: activity_id,
                turn_id,
                approval: shaman_protocol::Approval {
                    id: approval,
                    subject: shaman_protocol::ApprovalSubject::Command {
                        command: "cargo build".to_owned(),
                        cwd: None,
                        actions: Vec::new(),
                    },
                    reason: None,
                },
                tool_activity_id: None,
                detail_truncated: false,
                outcome: shaman_protocol::ApprovalOutcome::Pending,
                decision: None,
                follow_up_error: None,
                asked_at: None,
            });
        snapshot
            .transcript
            .push(shaman_protocol::TranscriptItem::Activity { activity_id });
        let revision = snapshot.revision;
        shaman_protocol::session_projection::close_revision(&mut snapshot, revision);
        snapshot
    }

    #[test]
    fn a_decision_whose_answer_was_lost_waits_behind_review_while_the_session_still_shows_it_pending()
     {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let approval = shaman_protocol::ApprovalId::new();
        let decided = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = decided.clone();
        // The stream stays open on the Approval pending, as a Server whose
        // answer to the Decision was lost has yet to say otherwise.
        let (more, held) = tokio::sync::mpsc::unbounded_channel();
        let held = Arc::new(Mutex::new(Some(held)));
        let (opened, _drafts, _reference, _session, _directory) = opened_over(
            &runtime,
            answering(Arc::new(move |method, path| {
                if method == http::Method::POST && path.ends_with("/decision") {
                    counted.fetch_add(1, Ordering::AcqRel);
                    return Box::pin(async {
                        Err(SessionError {
                            code: SessionErrorCode::PairingOutcomeUnknown,
                            message: "Remote API request was delivered, and its answer was lost"
                                .to_owned(),
                            unreachable: None,
                        })
                    });
                }
                if path.ends_with("/events")
                    && let Some(held) = held.lock().unwrap().take()
                {
                    // The Session the stream is asked of, as its path names it.
                    let session = path
                        .split('/')
                        .rev()
                        .nth(1)
                        .and_then(|id| serde_json::from_value(serde_json::json!(id)).ok())
                        .expect("a Session's stream");
                    let snapshot = waiting_on(session, approval);
                    return Box::pin(async move { Ok(stream_from(&snapshot, held)) });
                }
                Box::pin(std::future::pending())
            })),
        );
        let mut frame = opened.frame.clone();
        let presented = |frame: &SessionFrame| frame.intervention.is_some();
        runtime.block_on(async {
            tokio::time::timeout(SHOWN_WITHIN, frame.wait_for(presented))
                .await
                .expect("presented in time")
                .expect("the frame stands");
        });

        let failed =
            runtime.block_on(opened.decide(approval.to_string(), DecisionKind::AcceptOnce));
        assert_eq!(
            failed,
            Err(ActError::Unreachable {
                message: "The server did not answer, so the decision may have been sent".to_owned()
            })
        );
        assert_eq!(decided.load(Ordering::Acquire), 1, "never sent again");
        // It waits behind Review, as one dismissed does, and is not offered
        // again while it may have been answered.
        let waiting = |frame: &SessionFrame| {
            frame.intervention.is_none()
                && frame.bar.as_ref().is_some_and(|bar| {
                    bar.kind == frame::BarKind::Waiting
                        && bar.review.as_ref().is_some_and(|review| {
                            review.label == "Review"
                                && review.opens == frame::ReviewOpens::Interventions
                        })
                })
        };
        runtime.block_on(async {
            tokio::time::timeout(SHOWN_WITHIN, frame.wait_for(waiting))
                .await
                .unwrap_or_else(|_| {
                    panic!("still offered: {:?}", opened.frame.borrow().intervention)
                })
                .expect("the frame stands");
        });
        drop(more);
    }
}
