//! The thin Client's core: everything the iOS app is but its screen (ADR-0080).
//!
//! Swift makes one [`ThinClient`] from a directory in the app's container and
//! the signer of this phone's identity key: the device's core, opened once
//! however many windows the app shows. Each window asks it for a
//! [`ThinClientWindow`], a Client of its own over it, and reaches through
//! that what it holds for itself.
//! What a screen shows crosses as a finished model, whole on every change, to
//! a listener Swift hands in, which hears the current value at once; an act is
//! an async call that returns once it is done, with nothing but success, what
//! it changed arriving by the listener. Everything the core keeps is in that
//! one directory, which Swift keeps out of every backup, and all of it is of
//! the one key the signer signs with.
//!
//! This is the one crate that knows of UniFFI. What it hands Swift carries no
//! compatibility promise: the app and its core are always built together.

use std::{
    collections::HashMap,
    fmt,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use shaman_client::SessionCommandTimings;
use shaman_pairing::{GivenIdentity, IdentityMaterial, PairingProtocol, Redeemer, RedeemerTimings};
use tokio::{
    runtime::Runtime,
    sync::{Mutex as AsyncMutex, watch},
    task::JoinHandle,
};

mod acts;
mod answers;
mod beginning;
mod behind;
mod blocks;
mod conversation;
mod device;
mod drafts;
mod frame;
mod identity;
mod intervention;
mod listing;
mod notices;
mod pairing;
mod picker;
mod reach;
mod relays;
mod remotes;
mod session;
mod tree;
mod window;
mod work;

pub use answers::{HalfAnswered, HalfAnsweredPage};
pub use beginning::{Landing, LandingWorkspace, WorktreeChoice};
pub use blocks::AgentBlockKind;
pub use conversation::{
    ConversationItem, ItemKind, Tone, TurnCard, TurnOutcome, agent_message_items,
};
pub use device::ThisDevice;
use device::{DeviceRecord, Records};
pub use drafts::{Draft, GivenBack};
pub use frame::{Bar, BarKind, Composer, QueuedPrompt, Review, ReviewOpens, SessionFrame};
pub use identity::{KeySigner, SignError};
pub use intervention::{
    AnswerField, ChoiceCard, DecisionKind, DecisionOffer, GivenAnswer, Intervention,
    PendingApproval, PendingQuestion, PendingQuestionnaire,
};
use listing::Listings;
pub use listing::{
    ActiveRow, CheckoutLine, FoundRow, Listing, ScopeChoice, ScopeKind, ScopeSection, SettledRow,
    Slot, Standing, UnreachableHint, UnreachableRow, UnreachableSheet,
};
pub use notices::Notice;
pub use pairing::{InviteReading, OfferedWay, PairingOutcome, ReadInvite, ScanWords, TryInstead};
pub use picker::{WorkspacePicker, WorkspaceRow, WorkspaceSection};
use relays::Relays;
pub use relays::{DeviceLogin, LoginOffer, RelayRow, RelayRowState};
use remotes::Remotes;
pub use remotes::{BeginsIn, Outlook, ServerRow, Servers, WayRow, WayState};
pub use session::{
    ConversationChange, ConversationListener, DetailListener, FrameListener, OpenSession,
    TreeListener, WorkListener,
};
pub use tree::{Place, SubagentTree, TreeButton, TreeRow};
pub use window::ThinClientWindow;
pub use work::{
    ActivityDetail, BodyStyle, DetailSection, Fact, Tap, TurnWork, WorkItem, WorkKind, WorkRow,
};

uniffi::setup_scaffolding!();

/// How long the core gives what it waits on: its own, so a test injects
/// milliseconds.
#[derive(Clone, Copy, Debug)]
pub struct CoreTimings {
    /// How the redeemer reaches each Remote.
    pub redeemer: RedeemerTimings,
    /// How long each Relay is given to answer what a login, a join or a
    /// removal asks of it.
    pub relays: shaman_pairing::relay::RelayClientTimings,
    /// How long confirming a pairing keeps dialling while no way answers,
    /// so that allowing the local network prompt is followed by success.
    pub pairing_keeps_dialling: Duration,
    /// How long confirming a pairing waits between one dial that no way
    /// answered and the next.
    pub pairing_redial: Duration,
    /// How each Remote's readers wait between one failed attempt and the
    /// next.
    pub readers: SessionCommandTimings,
    /// How long an act waits on its Server's answer before it fails as
    /// Unreachable.
    pub act_bound: Duration,
    /// How long a Session is left alone before it settles on its own: three
    /// days, the schema's default, which a phone offers no Setting for.
    pub settle_after: Duration,
    /// How long each connection to a Remote is given to answer the check
    /// made as the app comes to the front or the network path changes,
    /// before it is given up and redialled.
    pub check_deadline: Duration,
    /// How long a Remote reached anew — as the core opens, as the app comes
    /// to the front, as its stream drops — holds its last reading without an
    /// answer before it reads Unreachable, where every way has not failed
    /// sooner.
    pub unreachable_after: Duration,
    /// How long a draft waits after its last change before it is written to
    /// the core's directory: a moment after typing stops.
    pub draft_written_after: Duration,
}

impl Default for CoreTimings {
    fn default() -> Self {
        Self {
            redeemer: RedeemerTimings::default(),
            relays: shaman_pairing::relay::RelayClientTimings::default(),
            pairing_keeps_dialling: Duration::from_secs(8),
            pairing_redial: Duration::from_millis(500),
            readers: SessionCommandTimings {
                initial_recovery_backoff: Duration::from_millis(50),
                max_recovery_backoff: Duration::from_secs(5),
                attachment_fetch_timeout: Duration::from_secs(30),
            },
            act_bound: Duration::from_secs(30),
            settle_after: listing::SETTLE_AFTER,
            check_deadline: Duration::from_secs(2),
            unreachable_after: Duration::from_secs(3),
            draft_written_after: Duration::from_secs(1),
        }
    }
}

/// The device's core of a thin Client, kept in one directory: what is the
/// phone's, one however many windows it shows.
#[derive(uniffi::Object)]
pub struct ThinClient {
    /// Every act and every listener runs here, so nothing Swift calls needs
    /// a runtime of its own, and UniFFI's `tokio` feature is not used. Shut
    /// down in the background as the core goes, wherever it goes from.
    runtime: Option<Runtime>,
    inner: Arc<Inner>,
}

/// Everything the core holds, shared by the acts running on its runtime.
pub(crate) struct Inner {
    records: Records,
    /// What the thin Client's own file holds, held to the end of each change
    /// to it, so one change never undoes another and the last asked is kept.
    record: AsyncMutex<DeviceRecord>,
    identity: IdentityMaterial,
    device: watch::Sender<ThisDevice>,
    /// Redeems Invites and holds the Remotes they form, in `remotes.json`.
    redeemer: Redeemer,
    /// The Relays this phone stands at, in `relays.json`, which the
    /// redeemer dials Relay ways through.
    relays: Relays,
    /// The Relays the Invite last read offers, at which a login may be asked
    /// for before its Server is a Remote.
    invited_relays: Mutex<Vec<String>>,
    remotes: Remotes,
    /// Every Remote's Sessions, merged into the one listing each window
    /// narrows as it will.
    listing: Listings,
    /// Held through each pairing, so one pairing at a time is made.
    pairing: AsyncMutex<()>,
    /// Each composer's unsent text, kept in the directory.
    drafts: drafts::Drafts,
    /// What was half given each Questionnaire, for the core's life.
    half_answers: answers::HalfAnswers,
    /// The acts under way, which Swift holds background time for.
    acts: acts::Acts,
    /// The trees the open Sessions belong to, each read once.
    trees: tree::Trees,
    /// Every window the core has handed out, while it lives.
    windows: Mutex<Vec<Weak<window::Window>>>,
    /// How many windows the core has handed out.
    windows_made: std::sync::atomic::AtomicU64,
    /// Each Pairing that ended while no window lived, for a window whose
    /// scene kept its Outlook there to say.
    ended_unclaimed: Mutex<Vec<EndedUnclaimed>>,
    timings: CoreTimings,
}

#[uniffi::export]
impl ThinClient {
    /// The core kept in `directory`, made where it is absent, proving this
    /// phone by the key `signer` signs with. What the directory holds of
    /// another key is discarded first, the name alone staying, and the phone
    /// is told it must pair again; a key that will not sign discards nothing
    /// and opens nothing.
    #[uniffi::constructor]
    pub fn open(directory: String, signer: Arc<dyn KeySigner>) -> Result<Arc<Self>, OpenError> {
        Self::open_with(directory, signer, CoreTimings::default())
    }

    /// A window of the app, a Client of its own over this core: its Outlook
    /// begins where the phone was last turned, by any window, at the
    /// Workspace remembered there, its listing at Everywhere, its Landing
    /// saying what the Landing's draft kept says, and it says for itself
    /// every notice there is to say; or, handed `kept`, what its scene kept
    /// of its Outlook, as Swift hands it back after a cold start, standing
    /// there, as [`ThinClientWindow::kept_outlook`] says. Handed back, an
    /// Outlook is not kept as the phone's latest turn, and one whose Remote
    /// is no longer held turns nothing, so the window stands at the Remote
    /// the phone was most recently turned toward that is still held; where
    /// that Remote's Pairing ended as no window lived, the window says so,
    /// and where it now stands. It lives for as long as Swift holds it.
    pub fn window(self: Arc<Self>, kept: Option<String>) -> Arc<ThinClientWindow> {
        let inner = &self.inner;
        // A beginning whose answer never came, as the app was last ended or
        // in another window, may have begun.
        let landing_note = inner
            .drafts
            .maybe_sent(beginning::LANDING_DRAFT)
            .then(|| beginning::MAYBE_BEGUN_LEFT.to_owned());
        let window = Arc::new(window::Window {
            id: inner
                .windows_made
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            outlook: inner.remotes.window(),
            view: inner.listing.view(),
            notices: inner.listing.notices().window(),
            landing_note: Arc::new(watch::Sender::new(landing_note)),
            workspace_query: watch::Sender::new(String::new()),
            handed_back: kept.is_some(),
        });
        // Handed back as it is made, so nothing that ends meanwhile finds it
        // standing elsewhere.
        if let Some(remote) = kept.and_then(|kept| inner.remotes.restore(&window.outlook, &kept)) {
            inner.handed_back(&window, &remote);
        }
        {
            let mut windows = inner
                .windows
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            windows.retain(|window| window.strong_count() > 0);
            windows.push(Arc::downgrade(&window));
        }
        ThinClientWindow::new(self, window)
    }

    /// Tells `listener` what "This device" shows now, and again on every
    /// change, until the subscription is cancelled.
    pub fn watch_device(&self, listener: Arc<dyn DeviceListener>) -> Arc<Subscription> {
        self.watch(self.inner.device.subscribe(), move |device| {
            listener.changed(device);
        })
    }

    /// Tells `listener` what the Servers section of the sheet shows now, and
    /// again on every change, until the subscription is cancelled.
    pub fn watch_servers(&self, listener: Arc<dyn ServersListener>) -> Arc<Subscription> {
        self.watch(self.inner.remotes.servers(), move |servers| {
            listener.changed(servers);
        })
    }

    /// What was last half given the Questionnaire `questionnaire` of the
    /// Session `session` names, by a sheet of it in any window, where
    /// anything was: what a sheet opening starts from.
    pub fn half_answered(&self, session: String, questionnaire: String) -> Option<HalfAnswered> {
        self.inner.half_answers.get(&session, &questionnaire)
    }

    /// Keeps `answers` as what was last half given the Questionnaire
    /// `questionnaire` of the Session `session` names, for as long as the
    /// core runs and never in its directory.
    pub fn keep_half_answered(
        &self,
        session: String,
        questionnaire: String,
        answers: HalfAnswered,
    ) {
        self.inner
            .half_answers
            .keep(&session, &questionnaire, answers);
    }

    /// Forgets what was half given the Questionnaire `questionnaire` of the
    /// Session `session` names, as it is answered, declined or waits no
    /// more.
    pub fn forget_half_answered(&self, session: String, questionnaire: String) {
        self.inner.half_answers.forget(&session, &questionnaire);
    }

    /// Tells `listener` whether any act is under way now, and again each
    /// time that changes, until the subscription is cancelled: Swift holds
    /// iOS background time only while one is.
    pub fn watch_acts(&self, listener: Arc<dyn ActsListener>) -> Arc<Subscription> {
        self.watch(self.inner.acts.busy(), move |under_way| {
            listener.changed(under_way);
        })
    }

    /// Sets the Session `session` names aside as done for now.
    pub async fn settle(&self, session: String) -> Result<(), ActError> {
        self.settling(session, true).await
    }

    /// Brings the Session `session` names back among the active.
    pub async fn unsettle(&self, session: String) -> Result<(), ActError> {
        self.settling(session, false).await
    }

    /// Hears that the app has come to the front: every Remote is checked,
    /// each connection to it asked something cheap, what answers kept and
    /// the rest redialled at once, skipping any backoff, and every stream
    /// resumes by a fresh snapshot. Each Remote holds its last state while
    /// that settles.
    pub fn came_to_front(&self) {
        self.check();
    }

    /// Hears that the app has left the front. Nothing is done to its
    /// connections: a short switch away comes back to live streams, and the
    /// check on return finds whatever did not last. Every login under way is
    /// given up, as the connection it waits on cannot outlast the app being
    /// suspended. Every draft is written before it returns, so it is there
    /// however iOS ends the app.
    pub fn left_front(&self) {
        self.inner.drafts.write_now();
        self.inner.relays.leave();
    }

    /// Logs in at the Relay `relay` names, which a Remote of this phone's,
    /// or the Invite last read, offers: tells `listener` how the login stands
    /// until it ends — the code and the address to open, then done or
    /// failed. Cancelling the subscription first gives the login up, as
    /// leaving the front does. Once the Relay confirms it, every Remote is
    /// tried again at once.
    pub fn log_in_at_relay(
        &self,
        relay: String,
        listener: Arc<dyn DeviceLoginListener>,
    ) -> Arc<Subscription> {
        let offered = shaman_pairing::relay::holdable_address(&relay)
            .is_some_and(|address| self.inner.offers_relay(&address));
        if !offered {
            listener.changed(DeviceLogin::Failed {
                title: format!("Log in at {}", relays::shown(&relay)),
                message: "This phone logs in only at a relay one of its servers offers".to_owned(),
                again: "Try again".to_owned(),
            });
            return Subscription::of(self.runtime().spawn(async {}));
        }
        let inner = self.inner.clone();
        let tell = listener.clone();
        self.inner.relays.log_in(
            self.runtime().handle(),
            relay,
            move |login| tell.changed(login),
            move || {
                let asked = inner.clone();
                tokio::spawn(async move {
                    for name in asked.remotes.names() {
                        let asking = asked.clone();
                        tokio::spawn(async move { asking.remotes.ask_ways(&name).await });
                    }
                    asked.listing.check(asked.timings.check_deadline).await;
                });
            },
        )
    }

    /// Removes the Relay entry a row of the Relays section hands back,
    /// giving up this phone's Login there: the Relay is asked to forget it,
    /// and one that does not answer is forgotten here all the same.
    pub async fn remove_relay(&self, relay: String) -> Result<(), ActError> {
        let inner = self.inner.clone();
        self.run(async move {
            inner.relays.remove(&relay).await?;
            // A way that answered through it, or beside it, is asked anew,
            // so none reads answering while its Remote no longer does.
            for name in inner.remotes.names() {
                let asking = inner.clone();
                tokio::spawn(async move { asking.remotes.ask_ways(&name).await });
            }
            Ok(())
        })
        .await?
    }

    /// Renames the Remote a row of the Servers section hands back to `name`,
    /// as the shared rule for a Remote's name allows and no other Remote
    /// holds, refusing one it would not with why. Its Pairing stands as it
    /// was, and what was remembered for it goes with the name; the Server
    /// never hears what this phone calls it.
    pub async fn rename_remote(&self, remote: String, name: String) -> Result<(), ActError> {
        let inner = self.inner.clone();
        self.run(async move { inner.rename_remote(&remote, name.trim()).await })
            .await?
    }

    /// Removes the Remote a row of the Servers section hands back, ending
    /// its Pairing: the Remote is asked to withdraw this phone's Peer, and
    /// one that does not answer within a moment is forgotten here all the
    /// same, which is said. Its drafts and what was remembered for it go
    /// with it, and each window's Outlook that stood there turns to the
    /// Remote the phone was most recently turned toward that is still held,
    /// saying so in that window, or to none.
    pub async fn remove_remote(&self, remote: String) -> Result<(), ActError> {
        let inner = self.inner.clone();
        self.run(async move { inner.remove_remote(&remote).await })
            .await?
    }

    /// Hears that the network path changed, and whether there is one at
    /// all: with none nothing is dialled and every Remote reads Unreachable;
    /// with one every Remote is checked, as on a return to the front.
    pub fn path_changed(&self, has_path: bool) {
        self.inner.listing.network_path(has_path);
        if has_path {
            self.check();
        }
    }

    /// Tries the Remote `remote` names again, as an Unreachable Remote's
    /// row offers, answering once it answers, or failing as Unreachable
    /// where it still does not.
    pub async fn try_again(&self, remote: String) -> Result<(), ActError> {
        let inner = self.inner.clone();
        self.act(async move {
            inner
                .listing
                .try_again(&remote, inner.timings.check_deadline)
                .await
        })
        .await
    }

    /// Renames this device to `name`, as the Serving side would keep it,
    /// refusing a name it would not, with why.
    pub async fn rename_device(&self, name: String) -> Result<(), ActError> {
        let name = shaman_pairing::shown_peer_name(&name).map_err(|refusal| ActError::Invalid {
            message: refusal.to_string(),
        })?;
        let inner = self.inner.clone();
        self.run(async move { inner.change_record(move |record| record.name = name).await })
            .await?
    }

    /// Says no more that this phone must pair again.
    pub async fn dismiss_pair_again(&self) -> Result<(), ActError> {
        let inner = self.inner.clone();
        self.run(async move {
            inner
                .change_record(|record| record.pair_again = false)
                .await
        })
        .await?
    }

    /// What the screen where pairing begins says to do.
    pub fn pairing_prompt(&self) -> String {
        pairing::PROMPT.to_owned()
    }

    /// What the scanner's screen and the offer to scan say, of which the app
    /// shows whichever the camera calls for.
    pub fn scan_words(&self) -> ScanWords {
        pairing::ScanWords::new()
    }

    /// Whether `text`, as a scanner read it, is an Invite, by its form alone:
    /// the scanner stops at the first that is and goes on to the
    /// confirmation screen, which says what is wrong with a malformed one or
    /// one from a newer Shaman, and passes over anything else.
    pub fn is_invite(&self, text: String) -> bool {
        shaman_pairing::has_invite_form(&text)
    }

    /// Reads `invite` for the confirmation screen, dialling nothing.
    pub fn read_invite(&self, invite: String) -> InviteReading {
        pairing::read(&self.inner, &invite)
    }
}

impl ThinClient {
    /// [`Self::open`], waiting on what it waits as `timings` say.
    pub fn open_with(
        directory: String,
        signer: Arc<dyn KeySigner>,
        timings: CoreTimings,
    ) -> Result<Arc<Self>, OpenError> {
        Self::open_speaking(directory, signer, timings, PairingProtocol::CURRENT)
    }

    /// [`Self::open_with`], speaking `protocol` to every Server: stating its
    /// version, and working with one at or above its floor. Injectable, so a
    /// test can stand the phone at any release.
    pub fn open_speaking(
        directory: String,
        signer: Arc<dyn KeySigner>,
        timings: CoreTimings,
        protocol: PairingProtocol,
    ) -> Result<Arc<Self>, OpenError> {
        let signer = identity::taken_up(signer).map_err(OpenError::unusable_key)?;
        let records = Records::open(PathBuf::from(directory)).map_err(OpenError::from)?;
        let identity::Reconciled { record, identity } = identity::reconcile(&records, signer)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("shaman-core")
            .enable_all()
            .build()
            .map_err(|error| OpenError::from(anyhow::Error::from(error)))?;
        let redeemer = Redeemer::new(
            records.directory(),
            protocol,
            Arc::new(GivenIdentity(identity.clone())),
            timings.redeemer,
        )
        .map_err(OpenError::from)?;
        let (device, _) = watch::channel(ThisDevice::of(&record, protocol));
        let relays = Relays::open(
            records.directory(),
            Arc::new(GivenIdentity(identity.clone())),
            device.subscribe(),
            timings.relays,
            runtime.handle(),
        )
        .map_err(OpenError::from)?;
        redeemer.reach_relays_through(relays.ways());
        // A pairing the app was ended in the middle of keeps nothing, kept
        // so or not.
        redeemer.drop_uncommitted();
        let drafts = drafts::Drafts::open(
            records.directory(),
            timings.draft_written_after,
            runtime.handle(),
        );
        let remotes = Remotes::new(
            redeemer.clone(),
            relays.clone(),
            &record.turned,
            &record.workspaces,
        );
        let listing = Listings::new(
            runtime.handle().clone(),
            redeemer.clone(),
            relays.clone(),
            timings.readers,
            timings.settle_after,
            timings.unreachable_after,
            drafts.clone(),
        );
        listing.follow(&remotes.held());
        let trees = tree::Trees::new(runtime.handle().clone(), listing.notices());
        let inner = Arc::new(Inner {
            records,
            record: AsyncMutex::new(record),
            identity,
            device,
            redeemer,
            relays,
            invited_relays: Mutex::default(),
            remotes,
            listing,
            pairing: AsyncMutex::new(()),
            acts: {
                let drafts = drafts.clone();
                acts::Acts::settling_with(move || drafts.write_now())
            },
            drafts,
            half_answers: answers::HalfAnswers::default(),
            trees,
            windows: Mutex::default(),
            windows_made: std::sync::atomic::AtomicU64::new(0),
            ended_unclaimed: Mutex::default(),
            timings,
        });
        for name in inner.remotes.names() {
            let inner = inner.clone();
            runtime.spawn(async move { inner.remotes.ask_ways(&name).await });
        }
        // A Remote that refuses this phone's key has ended the Pairing on its
        // side, and is forgotten here too.
        let (ended, mut endings) = tokio::sync::mpsc::unbounded_channel();
        inner.redeemer.attach(Arc::new(remotes::Endings::new(
            ended,
            inner.listing.downgrade(),
        )));
        let forgetting = Arc::downgrade(&inner);
        runtime.spawn(async move {
            while let Some((name, generation)) = endings.recv().await {
                let Some(inner) = forgetting.upgrade() else {
                    return;
                };
                inner.pairing_ended(&name, generation).await;
            }
        });
        // Whatever shows how this phone stands at a Relay shows it again as
        // that changes: a login done, a Login refused, an entry removed.
        let mut relays_changed = inner.relays.changes();
        let shown = Arc::downgrade(&inner);
        runtime.spawn(async move {
            while relays_changed.changed().await.is_ok() {
                let Some(inner) = shown.upgrade() else {
                    return;
                };
                inner.remotes.refresh();
                inner.listing.relays_changed();
            }
        });
        // Whatever shows a Remote's ways shows them again as what it tells of
        // the Relays it Serves through changes them: one turned on reads as
        // this phone stands there, and is asked whether it answers; one
        // turned off goes, and is dialled no more.
        let mut told = inner.redeemer.told_changes();
        let mut seen = told_ways(&inner.remotes.held());
        let shown = Arc::downgrade(&inner);
        runtime.spawn(async move {
            while told.changed().await.is_ok() {
                let Some(inner) = shown.upgrade() else {
                    return;
                };
                let now = told_ways(&inner.remotes.held());
                inner.remotes.refresh();
                inner.listing.relays_changed();
                for (name, ways) in &now {
                    if seen.get(name) != Some(ways) {
                        let inner = inner.clone();
                        let name = name.clone();
                        tokio::spawn(async move { inner.remotes.ask_ways_again(&name).await });
                    }
                }
                seen = now;
            }
        });
        Ok(Arc::new(Self {
            runtime: Some(runtime),
            inner,
        }))
    }

    /// What proves this phone to the Servers it pairs with: what signs with
    /// its key, and the certificate of the key the core keeps.
    pub fn identity(&self) -> &IdentityMaterial {
        &self.inner.identity
    }

    /// How an open Session waits between attempts to read it.
    fn pacing(&self) -> session::Pacing {
        session::Pacing {
            initial_backoff: self.inner.timings.readers.initial_recovery_backoff,
            max_backoff: self.inner.timings.readers.max_recovery_backoff,
        }
    }

    /// What the core hands each Session `window` opens for its acts.
    fn asked(&self, window: &window::Window) -> session::Asked {
        let pacing = self.pacing();
        session::Asked {
            drafts: self.inner.drafts.clone(),
            half_answers: self.inner.half_answers.clone(),
            acts: self.inner.acts.clone(),
            asking: acts::Asking {
                initial_backoff: pacing.initial_backoff,
                max_backoff: pacing.max_backoff,
                bound: self.inner.timings.act_bound,
                // Every way's dial, the last begun after its head start.
                dialling: self.inner.timings.redeemer.direct_head_start
                    + self.inner.timings.redeemer.handshake_timeout * 2,
            },
            trees: self.inner.trees.clone(),
            landing_note: window.landing_note.clone(),
        }
    }

    /// Checks every Remote, in the background.
    fn check(&self) {
        let inner = self.inner.clone();
        self.runtime().spawn(async move {
            inner.listing.check(inner.timings.check_deadline).await;
        });
    }

    fn runtime(&self) -> &Runtime {
        self.runtime
            .as_ref()
            .expect("the runtime stands until the core goes")
    }

    async fn settling(&self, session: String, settled: bool) -> Result<(), ActError> {
        let inner = self.inner.clone();
        self.act(async move { inner.listing.settle(&session, settled).await })
            .await
    }

    /// Runs `act`, which asks a Server, on the core's runtime, failing it as
    /// Unreachable where no answer came within an act's bound.
    async fn act(
        &self,
        act: impl Future<Output = Result<(), ActError>> + Send + 'static,
    ) -> Result<(), ActError> {
        let bound = self.inner.timings.act_bound;
        let under_way = self.inner.acts.begin();
        self.run(async move {
            let _under_way = under_way;
            tokio::time::timeout(bound, act).await.unwrap_or_else(|_| {
                Err(ActError::Unreachable {
                    message: "The server did not answer in time".to_owned(),
                })
            })
        })
        .await?
    }

    /// Runs `act` on the core's runtime, to its end.
    async fn run<T: Send + 'static>(
        &self,
        act: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, ActError> {
        self.runtime()
            .spawn(act)
            .await
            .map_err(|error| ActError::failed(anyhow::Error::from(error)))
    }

    /// Tells `tell` what `model` holds now, and again on every change, until
    /// the subscription is cancelled.
    fn watch<T: Clone + Send + Sync + 'static>(
        &self,
        model: watch::Receiver<T>,
        tell: impl Fn(T) + Send + 'static,
    ) -> Arc<Subscription> {
        watching(self.runtime().handle(), model, tell)
    }
}

/// Tells `tell`, on `runtime`, what `model` holds now, and again on every
/// change, until the subscription is cancelled or the model goes.
pub(crate) fn watching<T: Clone + Send + Sync + 'static>(
    runtime: &tokio::runtime::Handle,
    mut model: watch::Receiver<T>,
    tell: impl Fn(T) + Send + 'static,
) -> Arc<Subscription> {
    let task = runtime.spawn(async move {
        loop {
            let current = model.borrow_and_update().clone();
            tell(current);
            if model.changed().await.is_err() {
                break;
            }
        }
    });
    Subscription::of(task)
}

impl Drop for ThinClient {
    fn drop(&mut self) {
        self.inner.relays.shutdown();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// The ways of each Remote of `held`, by its name.
fn told_ways(held: &[shaman_pairing::StoredRemote]) -> HashMap<String, Vec<shaman_protocol::Way>> {
    held.iter()
        .map(|stored| (stored.remote.name.clone(), stored.remote.ways.clone()))
        .collect()
}

impl Inner {
    /// Whether a login may be asked for at the Relay at `address`: one a
    /// Remote held offers, or the Invite last read.
    fn offers_relay(&self, address: &str) -> bool {
        let offered = |way: &shaman_protocol::Way| matches!(way, shaman_protocol::Way::Relay(relay) if relay == address);
        self.remotes
            .held()
            .iter()
            .any(|stored| stored.remote.ways.iter().any(offered))
            || self
                .invited_relays
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .iter()
                .any(|relay| relay == address)
    }

    /// Whether the phone's own Outlook stands at `stored`, where a new window
    /// would begin, and every window that lives, each with whether its
    /// Outlook stands there.
    fn windows_at(&self, stored: &shaman_pairing::StoredRemote) -> StoodThere {
        let at = |outlook: &Outlook| matches!(outlook, Outlook::Remote { name, .. } if *name == stored.remote.name);
        StoodThere {
            phone: at(&self.remotes.phone_outlook()),
            windows: self
                .windows
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .iter()
                .filter_map(Weak::upgrade)
                .map(|window| {
                    let there = at(&window.outlook.outlook().borrow());
                    (window, there)
                })
                .collect(),
        }
    }

    /// The Remote held, its Pairing made, whose key's fingerprint is
    /// `fingerprint`.
    fn held_remote(&self, fingerprint: &str) -> Option<shaman_pairing::StoredRemote> {
        self.remotes
            .held()
            .into_iter()
            .find(|stored| stored.remote.fingerprint == fingerprint)
    }

    /// Renames the Remote whose key's fingerprint is `remote` to `name`.
    async fn rename_remote(&self, remote: &str, name: &str) -> Result<(), ActError> {
        // One change to the Remotes at a time, a pairing's included.
        let _pairing = self.pairing.lock().await;
        let stored = self.held_remote(remote).ok_or_else(remotes::gone)?;
        shaman_pairing::validate_remote_name(name).map_err(|failure| ActError::Invalid {
            message: failure.message,
        })?;
        let was = stored.remote.name;
        if was == name {
            return Ok(());
        }
        if self
            .redeemer
            .records()
            .iter()
            .any(|other| other.remote.name == name)
        {
            return Err(ActError::Invalid {
                message: format!("This phone holds another server as {name}; choose another name"),
            });
        }
        self.redeemer
            .rename_remote(&was, name)
            .map_err(|failure| match failure.code {
                shaman_protocol::SessionErrorCode::RemoteNotFound => remotes::gone(),
                _ => ActError::Invalid {
                    message: failure.message,
                },
            })?;
        let (from, to) = (was.clone(), name.to_owned());
        // Each window remembers of it what it remembered, under its new name.
        self.remotes.rename_turned(&was, name);
        // Renamed, it is renamed wherever it is shown, kept or not: a record
        // that could not be written now is written whole by the next change.
        let shown = self
            .change_record_shown(move |record| record.rename(&from, &to))
            .await;
        if let Err(error) = shown {
            tracing::warn!("a Remote was renamed but this phone's records were not kept: {error}");
        }
        // Held a moment, so what reads the Remote again under its new name
        // goes on over the connections it was read over, dialling nothing.
        let client = self.redeemer.client_of(name);
        let held_for = self.timings.check_deadline;
        tokio::spawn(async move {
            tokio::time::sleep(held_for).await;
            drop(client);
        });
        self.listing.follow(&self.remotes.held());
        self.remotes.renamed(&was, name);
        Ok(())
    }

    /// Removes the Remote whose key's fingerprint is `remote`, asking it to
    /// withdraw this phone's Peer first.
    async fn remove_remote(&self, remote: &str) -> Result<(), ActError> {
        let _pairing = self.pairing.lock().await;
        let stored = self.held_remote(remote).ok_or_else(remotes::gone)?;
        let stood_there = self.windows_at(&stored);
        let removal = self
            .redeemer
            .remove_remote(&stored.remote.name)
            .await
            .map_err(|failure| match failure.code {
                shaman_protocol::SessionErrorCode::RemoteNotFound => remotes::gone(),
                _ => ActError::failed(anyhow::anyhow!(failure.message)),
            })?;
        self.forgotten(
            &stored,
            remotes::Ending::Removed {
                acknowledged: removal.acknowledged,
            },
            stood_there,
        )
        .await;
        Ok(())
    }

    /// Forgets the Remote `name`, whose Pairing `generation` its Server
    /// ended by refusing this phone's key, where that Pairing still stands
    /// here.
    async fn pairing_ended(&self, name: &str, generation: u64) {
        let _pairing = self.pairing.lock().await;
        let Some(stored) = self
            .remotes
            .held()
            .into_iter()
            .find(|stored| stored.remote.name == name && stored.generation == generation)
        else {
            return;
        };
        let stood_there = self.windows_at(&stored);
        if let Err(failure) = self.redeemer.delete_remote(name) {
            tracing::warn!(
                "{name} ended its pairing and was not forgotten: {}",
                failure.message
            );
            return;
        }
        self.forgotten(&stored, remotes::Ending::EndedThere, stood_there)
            .await;
    }

    /// Forgets everything held of `stored`, whose record is gone, as its
    /// Pairing ended by `ending`: what was remembered for it, its drafts,
    /// its Sessions and its ways, turning each window's Outlook that stood
    /// there, as `stood_there` says of each window, to the Remote the phone
    /// was most recently turned toward that is still held, and saying so in
    /// that window alone. Where no window's Outlook stood there, why it ended
    /// is said once, where there is anything to say, by the oldest window
    /// that stands; and where no window lives, by the first to stand, as the
    /// phone's own Outlook stood.
    async fn forgotten(
        &self,
        stored: &shaman_pairing::StoredRemote,
        ending: remotes::Ending,
        stood_there: StoodThere,
    ) {
        let name = stored.remote.name.clone();
        // No window lives to have stood there: it is kept, before anything is
        // awaited, so a window made meanwhile finds it, for the window whose
        // scene kept its Outlook there to say, or else the first to stand.
        if stood_there.windows.is_empty() {
            self.ended_unclaimed
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(EndedUnclaimed {
                    fingerprint: stored.remote.fingerprint.clone(),
                    name: name.clone(),
                    ending,
                    phone_stood_there: stood_there.phone,
                    told: false,
                    said: None,
                });
        }
        let forgotten = name.clone();
        let shown = self
            .change_record_shown(move |record| record.forget(&forgotten))
            .await;
        if let Err(error) = shown {
            tracing::warn!("{name} was forgotten but this phone's records were not kept: {error}");
        }
        // Following the Remotes held drops its drafts, the Landing's among
        // them where it was written for it.
        self.listing.follow(&self.remotes.held());
        self.half_answers.forget_remote(&stored.remote.fingerprint);
        self.remotes.forget_ways(&name);
        self.remotes.forget_turned(&name);
        // A window whose Outlook stood elsewhere is turned by nothing and says
        // nothing, and no window says a turn that is not its own. Where no
        // window's stood there, what the phone is still to hear, that the
        // Remote removed it or may still list it, is said once, by one window.
        // Where no window lived, it was kept as it was forgotten, above.
        if stood_there.windows.iter().any(|(_, there)| *there) {
            for (window, _) in stood_there.windows.iter().filter(|(_, there)| *there) {
                let turned = match &*window.outlook.outlook().borrow() {
                    Outlook::Remote { begins_in, .. } => Some(begins_in.said.clone()),
                    Outlook::Nowhere => None,
                };
                if let Some(words) = ending.words(&name, turned.as_deref()) {
                    window.notices.say(words);
                }
            }
        } else if !stood_there.windows.is_empty()
            && let Some(words) = ending.words(&name, None)
        {
            self.listing.notices().say_unclaimed(words);
        }
    }

    /// Hears that `window`'s Outlook was handed back where its scene kept it,
    /// at the Remote whose key's fingerprint is `remote`: where that Remote's
    /// Pairing ended as no window lived, the window says why, and where its
    /// Outlook now stands, in place of what the first window to stand said.
    fn handed_back(&self, window: &window::Window, remote: &str) {
        let mut ended = self.ended_unclaimed();
        let Some(ended) = ended.iter_mut().find(|ended| ended.fingerprint == remote) else {
            return;
        };
        ended.told = true;
        if let Some((said, id)) = ended.said.take()
            && let Some(said) = said.upgrade()
        {
            said.notices.seen(id);
        }
        if let Some(words) = ended
            .ending
            .words(&ended.name, Self::turned(window).as_deref())
        {
            window.notices.say(words);
        }
    }

    /// Hears that `window` stands, as it follows its notices: the first to
    /// stand says why each Pairing that ended as no window lived ended, where
    /// no window handed back an Outlook kept there has; with where it stands
    /// only where that turn was its own, as it was handed back nothing and
    /// begins where the phone stood, which was there.
    fn stands(&self, window: &Arc<window::Window>) {
        let mut ended = self.ended_unclaimed();
        for ended in ended.iter_mut().filter(|ended| !ended.told) {
            ended.told = true;
            let turned = (!window.handed_back && ended.phone_stood_there)
                .then(|| Self::turned(window))
                .flatten();
            if let Some(words) = ended.ending.words(&ended.name, turned.as_deref()) {
                ended.said = Some((Arc::downgrade(window), window.notices.say(words)));
            }
        }
    }

    /// Each Pairing that ended as no window lived, but those whose Remote is
    /// held again, paired since, which no window says.
    fn ended_unclaimed(&self) -> std::sync::MutexGuard<'_, Vec<EndedUnclaimed>> {
        let mut ended = self
            .ended_unclaimed
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let held = self.remotes.held();
        ended.retain(|ended| {
            held.iter()
                .all(|stored| stored.remote.fingerprint != ended.fingerprint)
        });
        ended
    }

    /// Where `window`'s Outlook stands, as a turn toward it is said.
    fn turned(window: &window::Window) -> Option<String> {
        match &*window.outlook.outlook().borrow() {
            Outlook::Remote { begins_in, .. } => Some(begins_in.said.clone()),
            Outlook::Nowhere => None,
        }
    }

    /// The Landing of `window`, where its Outlook stands now.
    fn landing(&self, window: &window::Window) -> Landing {
        let standing = self.remotes.standing(&window.outlook);
        let known =
            standing
                .as_ref()
                .map_or_else(beginning::Known::default, |(remote, workspace)| {
                    self.listing.landing(
                        remote,
                        workspace.as_ref().map(|workspace| workspace.path.as_path()),
                    )
                });
        beginning::landing(
            standing
                .as_ref()
                .map(|(remote, workspace)| (remote.as_str(), workspace.as_ref())),
            &known,
            window.landing_note.borrow().clone(),
        )
    }

    /// Changes the thin Client's own file by `change`, keeping it before
    /// "This device" and every window's Outlook are told: where it cannot be
    /// kept, nothing changes.
    async fn change_record(
        &self,
        change: impl FnOnce(&mut DeviceRecord) + Send + 'static,
    ) -> Result<(), ActError> {
        self.changing_record(change, false).await
    }

    /// Changes the thin Client's own file by `change`, telling "This device"
    /// and every window's Outlook whether or not it could be kept, as what
    /// the change
    /// records has been done already: where it could not, it is held, and
    /// kept by the next change, which writes the file whole.
    async fn change_record_shown(
        &self,
        change: impl FnOnce(&mut DeviceRecord) + Send + 'static,
    ) -> Result<(), ActError> {
        self.changing_record(change, true).await
    }

    async fn changing_record(
        &self,
        change: impl FnOnce(&mut DeviceRecord) + Send + 'static,
        shown_unkept: bool,
    ) -> Result<(), ActError> {
        let mut held = self.record.lock().await;
        let mut changed = held.clone();
        change(&mut changed);
        let written = changed.clone();
        let records = self.records.clone();
        let kept = tokio::task::spawn_blocking(move || records.write(&written))
            .await
            .map_err(|error| ActError::failed(anyhow::Error::from(error)))
            .and_then(|kept| kept.map_err(ActError::failed));
        if kept.is_ok() || shown_unkept {
            self.device
                .send_replace(ThisDevice::of(&changed, self.redeemer.protocol()));
            self.remotes.show(&changed.turned, &changed.workspaces);
            *held = changed;
        }
        kept
    }
}

/// A Pairing that ended while no window lived: the Remote, by its key's
/// fingerprint and its name, how it ended, and whether the phone's own
/// Outlook stood there; whether a window has said it, and the window and
/// notice the first to stand said it in, until a window whose scene kept its
/// Outlook there says it in its place.
struct EndedUnclaimed {
    fingerprint: String,
    name: String,
    ending: remotes::Ending,
    phone_stood_there: bool,
    told: bool,
    said: Option<(Weak<window::Window>, u64)>,
}

/// Where the Outlooks stood as a Pairing ended: the phone's own, and each
/// window's that lives.
struct StoodThere {
    phone: bool,
    windows: Vec<(Arc<window::Window>, bool)>,
}

/// Hears what "This device" shows, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait DeviceListener: Send + Sync {
    fn changed(&self, device: ThisDevice);
}

/// Hears what the Servers section of the sheet shows, at once and on every
/// change.
#[uniffi::export(with_foreign)]
pub trait ServersListener: Send + Sync {
    fn changed(&self, servers: Servers);
}

/// Hears what the listing shows, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait ListingListener: Send + Sync {
    fn changed(&self, listing: Listing);
}

/// Hears where the Outlook stands, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait OutlookListener: Send + Sync {
    fn changed(&self, outlook: Outlook);
}

/// Hears what the Landing shows, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait LandingListener: Send + Sync {
    fn changed(&self, landing: Landing);
}

/// Hears what the Workspace Picker shows, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait WorkspacesListener: Send + Sync {
    fn changed(&self, picker: WorkspacePicker);
}

/// Hears a composer's draft, at once and each time the core changes it on its
/// own.
#[uniffi::export(with_foreign)]
pub trait DraftListener: Send + Sync {
    fn changed(&self, draft: Draft);
}

/// Hears how a login at a Relay stands, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait DeviceLoginListener: Send + Sync {
    fn changed(&self, login: DeviceLogin);
}

/// Hears what the core says of its own accord, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait NoticeListener: Send + Sync {
    fn changed(&self, notice: Option<Notice>);
}

/// Hears whether any act is under way, at once and each time that changes.
#[uniffi::export(with_foreign)]
pub trait ActsListener: Send + Sync {
    fn changed(&self, under_way: bool);
}

/// What keeps a listener hearing; cancelled, or dropped, it hears no more.
#[derive(uniffi::Object)]
pub struct Subscription {
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Subscription {
    fn of(task: JoinHandle<()>) -> Arc<Self> {
        Arc::new(Self {
            task: Mutex::new(Some(task)),
        })
    }
}

#[uniffi::export]
impl Subscription {
    pub fn cancel(&self) {
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Why the core could not be opened, in words for whoever opened it.
#[derive(Debug, PartialEq, Eq, uniffi::Error)]
pub enum OpenError {
    /// The records could not be read or kept, or the core could not start.
    Failed { message: String },
    /// The key the app handed in cannot prove this phone: it is of another
    /// kind, or it would not sign. Nothing was discarded; starting again
    /// under a new key is the user's to choose.
    KeyRefused { message: String },
}

impl OpenError {
    /// The key the app handed in is not one this phone can be proved by.
    fn unusable_key(error: anyhow::Error) -> Self {
        Self::KeyRefused {
            message: format!("This phone's identity key could not be used: {error:#}"),
        }
    }

    /// The key would not sign, in the words of whatever holds it.
    fn unsigned(error: &anyhow::Error) -> Self {
        Self::KeyRefused {
            message: format!(
                "This phone's identity key would not sign: {}",
                error.root_cause()
            ),
        }
    }

    /// The records could not be kept as they are to be.
    fn unkept(error: anyhow::Error) -> Self {
        Self::Failed {
            message: format!("This phone's records could not be kept: {error:#}"),
        }
    }
}

impl From<anyhow::Error> for OpenError {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed {
            message: format!("This phone's records could not be read: {error:#}"),
        }
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Self::Failed { message } | Self::KeyRefused { message }) = self;
        formatter.write_str(message)
    }
}

impl std::error::Error for OpenError {}

/// Why an act failed, each case with words the core wrote. Whatever a screen
/// has a place for is a state of its model instead, and never one of these.
#[derive(Debug, PartialEq, Eq, uniffi::Error)]
pub enum ActError {
    /// No way to the Remote answered.
    Unreachable { message: String },
    /// The Server said no, in its own words.
    Refused { message: String },
    /// The Session or Remote no longer exists.
    Gone { message: String },
    /// The core refused the input, shown beside it.
    Invalid { message: String },
}

impl ActError {
    /// A failure of the phone's own, which no Server said and no input
    /// caused.
    fn failed(error: anyhow::Error) -> Self {
        Self::Refused {
            message: format!("This phone could not do that: {error:#}"),
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Unreachable { message }
            | Self::Refused { message }
            | Self::Gone { message }
            | Self::Invalid { message } => message,
        }
    }
}

impl fmt::Display for ActError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ActError {}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc,
        },
        time::Duration,
    };

    use shaman_pairing::signer::{Signer, testing::CallbackSigner};

    use super::*;

    /// Every line of the core's source whose literal runs spaces together
    /// within its words, between the line's first and last quote: where a
    /// lost line continuation joins a long literal's lines.
    fn literal_runs_of_spaces() -> Vec<String> {
        fn visit(directory: &std::path::Path, found: &mut Vec<String>) {
            for entry in std::fs::read_dir(directory).expect("read the core's source") {
                let path = entry.expect("read the core's source").path();
                if path.is_dir() {
                    visit(&path, found);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    let source = std::fs::read_to_string(&path).expect("read a source file");
                    for (number, line) in source.lines().enumerate() {
                        let (Some(first), Some(last)) = (line.find('"'), line.rfind('"')) else {
                            continue;
                        };
                        if first < last && joins_words(&line[first + 1..last]) {
                            found.push(format!("{}:{}: {line}", path.display(), number + 1));
                        }
                    }
                }
            }
        }
        let mut found = Vec::new();
        visit(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut found,
        );
        found
    }

    /// Whether `quoted` runs spaces together between two words, as a lost
    /// line continuation does: not at its edges, nor beside an escape such
    /// as `\n`, where a test pads its input on purpose.
    fn joins_words(quoted: &str) -> bool {
        let characters = quoted.chars().collect::<Vec<_>>();
        let mut at = 0;
        while at < characters.len() {
            if characters[at] != ' ' {
                at += 1;
                continue;
            }
            let start = at;
            while at < characters.len() && characters[at] == ' ' {
                at += 1;
            }
            let escaped_before = start >= 2 && characters[start - 2] == '\\';
            let worded = start > 0 && at < characters.len() && characters[at] != '\\';
            if at - start >= 2 && worded && !escaped_before {
                return true;
            }
        }
        false
    }

    /// No word the core writes runs spaces together, as one does where a
    /// long literal's line continuation was lost.
    #[test]
    fn no_literal_of_the_core_runs_spaces_together() {
        let found = literal_runs_of_spaces();
        assert!(found.is_empty(), "{}", found.join("\n"));
    }

    /// How long a test waits for the listener to hear something it must.
    const HEARD_WITHIN: Duration = Duration::from_secs(5);

    /// A key held apart from the core, as the app holds a phone's, which
    /// counts what it signs.
    struct TestKey(CallbackSigner);

    impl TestKey {
        fn new() -> Arc<Self> {
            Arc::new(Self(CallbackSigner::elsewhere()))
        }

        /// A key that will not sign, as a chip that refuses its blob.
        fn refusing() -> Arc<Self> {
            let public_key = CallbackSigner::elsewhere().public_key();
            Arc::new(Self(CallbackSigner::new(public_key, |_| {
                Err(anyhow::anyhow!("the chip will not take the key"))
            })))
        }

        fn signed(&self) -> Arc<AtomicU64> {
            self.0.signed()
        }

        fn fingerprint(&self) -> String {
            shaman_pairing::fingerprint(&self.0.public_key())
        }
    }

    impl KeySigner for TestKey {
        fn public_key(&self) -> Vec<u8> {
            self.0.public_key()
        }

        fn sign(&self, message: Vec<u8>) -> Result<Vec<u8>, SignError> {
            self.0.sign(&message).map_err(|refused| SignError::Refused {
                message: format!("{refused:#}"),
            })
        }
    }

    fn open(directory: &std::path::Path, key: &Arc<TestKey>) -> Result<Arc<ThinClient>, OpenError> {
        ThinClient::open(directory.to_string_lossy().into_owned(), key.clone())
    }

    fn opened(directory: &std::path::Path, key: &Arc<TestKey>) -> Arc<ThinClient> {
        open(directory, key).expect("open the core")
    }

    /// Runs an act to its end from outside the core's runtime, as Swift does.
    fn act<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime to wait on")
            .block_on(future)
    }

    struct Heard(Mutex<mpsc::Sender<ThisDevice>>);

    impl DeviceListener for Heard {
        fn changed(&self, device: ThisDevice) {
            let _ = self.0.lock().unwrap().send(device);
        }
    }

    fn listening(core: &ThinClient) -> (Arc<Subscription>, mpsc::Receiver<ThisDevice>) {
        let (sender, heard) = mpsc::channel();
        let subscription = core.watch_device(Arc::new(Heard(Mutex::new(sender))));
        (subscription, heard)
    }

    /// What "This device" shows at once over the core kept in `directory`.
    fn shown(directory: &std::path::Path, key: &Arc<TestKey>) -> ThisDevice {
        let core = opened(directory, key);
        let (_subscription, heard) = listening(&core);
        heard.recv_timeout(HEARD_WITHIN).expect("heard at once")
    }

    /// What the directory holds, by name.
    fn held(directory: &std::path::Path) -> Vec<String> {
        let mut names = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    /// Records of the key the core was last opened with: a Remote, a
    /// Relay entry and drafts, in a directory of their own.
    fn keep_records(directory: &std::path::Path) {
        std::fs::write(
            directory.join(shaman_pairing::REMOTES_FILE),
            br#"[{"name":"laptop"}]"#,
        )
        .unwrap();
        std::fs::write(
            directory.join(shaman_pairing::relay::RELAYS_FILE),
            br#"[{"url":"https://relay.example.com"}]"#,
        )
        .unwrap();
        std::fs::write(
            directory.join("drafts.json"),
            br#"{"session":"half a thought"}"#,
        )
        .unwrap();
        std::fs::create_dir(directory.join("kept")).unwrap();
        std::fs::write(directory.join("kept").join("more.json"), b"{}").unwrap();
    }

    #[test]
    fn a_new_phone_is_first_offered_as_iphone_with_its_key_and_the_protocol_it_speaks() {
        let directory = tempfile::tempdir().unwrap();
        let key = TestKey::new();
        let device = shown(&directory.path().join("core"), &key);
        assert_eq!(device.name, "iPhone");
        assert_eq!(device.fingerprint, key.fingerprint());
        assert_eq!(device.pair_again, None);
        assert_eq!(device.protocol_version, shaman_protocol::PROTOCOL_VERSION);
        assert!(
            device.name_note.contains("paired from now on"),
            "{}",
            device.name_note
        );
    }

    #[test]
    fn the_fingerprint_shown_is_the_one_a_serving_server_knows_this_phone_by() {
        let directory = tempfile::tempdir().unwrap();
        let key = TestKey::new();
        let core = opened(directory.path(), &key);
        let (_subscription, heard) = listening(&core);
        let device = heard.recv_timeout(HEARD_WITHIN).unwrap();

        // A Serving Server knows a Peer by the key of the certificate it
        // presents, and shows the fingerprint of that key whole.
        let presented = shaman_pairing::public_key_from_certificate(
            &core.identity().certificate.clone().into(),
        )
        .expect("a certificate of a key");
        assert_eq!(device.fingerprint, shaman_pairing::fingerprint(&presented));
        assert_eq!(presented, key.public_key(), "the certificate is of the key");
        assert_eq!(device.fingerprint.len(), 64, "{}", device.fingerprint);
    }

    #[test]
    fn the_core_signs_through_the_signer_once_and_keeps_what_it_signed() {
        let directory = tempfile::tempdir().unwrap();
        let key = TestKey::new();
        let signed = key.signed();

        let core = opened(directory.path(), &key);
        assert_eq!(signed.load(Ordering::Acquire), 1, "the certificate");
        act(core.rename_device("Pocket".to_owned())).expect("renamed");
        drop(core);

        let reopened = shown(directory.path(), &key);
        assert_eq!(reopened.name, "Pocket");
        assert_eq!(reopened.fingerprint, key.fingerprint());
        assert_eq!(
            signed.load(Ordering::Acquire),
            1,
            "a later start, and a rename, sign nothing"
        );
    }

    #[test]
    fn records_without_their_key_are_discarded_drafts_included_and_the_name_stays() {
        let directory = tempfile::tempdir().unwrap();
        let (lost, new) = (TestKey::new(), TestKey::new());
        let core = opened(directory.path(), &lost);
        act(core.rename_device("Pocket".to_owned())).expect("renamed");
        drop(core);
        keep_records(directory.path());

        let device = shown(directory.path(), &new);
        assert_eq!(device.name, "Pocket", "the name stays");
        assert_eq!(device.fingerprint, new.fingerprint());
        let said = device.pair_again.expect("says this phone must pair again");
        assert!(said.contains("Pair with each of them again"), "{said}");
        assert_eq!(held(directory.path()), [device::RECORD_FILE]);
    }

    #[test]
    fn that_this_phone_must_pair_again_is_said_until_dismissed_and_never_after() {
        let directory = tempfile::tempdir().unwrap();
        let (lost, new) = (TestKey::new(), TestKey::new());
        drop(opened(directory.path(), &lost));
        keep_records(directory.path());
        assert!(shown(directory.path(), &new).pair_again.is_some());
        assert!(
            shown(directory.path(), &new).pair_again.is_some(),
            "said again on a later start until it is dismissed"
        );

        let core = opened(directory.path(), &new);
        let (_subscription, heard) = listening(&core);
        assert!(
            heard
                .recv_timeout(HEARD_WITHIN)
                .unwrap()
                .pair_again
                .is_some()
        );
        act(core.dismiss_pair_again()).expect("dismissed");
        assert_eq!(heard.recv_timeout(HEARD_WITHIN).unwrap().pair_again, None);
        drop(core);

        assert_eq!(shown(directory.path(), &new).pair_again, None);
    }

    #[test]
    fn records_of_another_key_that_held_no_server_are_discarded_without_a_word() {
        let directory = tempfile::tempdir().unwrap();
        drop(opened(directory.path(), &TestKey::new()));
        std::fs::write(directory.path().join(shaman_pairing::REMOTES_FILE), b"[]").unwrap();
        std::fs::write(directory.path().join("drafts.json"), b"{}").unwrap();

        let device = shown(directory.path(), &TestKey::new());
        assert_eq!(device.pair_again, None);
        assert_eq!(held(directory.path()), [device::RECORD_FILE]);
    }

    #[test]
    fn a_key_without_records_is_reused() {
        let directory = tempfile::tempdir().unwrap();
        let core = directory.path().join("core");
        let key = TestKey::new();
        drop(opened(&core, &key));
        // The app deleted and installed again: its directory went, and the
        // Keychain kept the key.
        std::fs::remove_dir_all(&core).unwrap();

        let device = shown(&core, &key);
        assert_eq!(device.fingerprint, key.fingerprint());
        assert_eq!(device.pair_again, None, "nothing was lost to pair again");
    }

    #[test]
    fn a_key_that_will_not_sign_is_refused_in_its_words_and_discards_nothing() {
        let directory = tempfile::tempdir().unwrap();
        drop(opened(directory.path(), &TestKey::new()));
        keep_records(directory.path());
        let before = std::fs::read(directory.path().join(device::RECORD_FILE)).unwrap();

        let refused = open(directory.path(), &TestKey::refusing())
            .err()
            .expect("opened over a key that will not sign");
        assert_eq!(
            refused,
            OpenError::KeyRefused {
                message: "This phone's identity key would not sign: the chip will not take the key"
                    .to_owned()
            }
        );
        assert_eq!(
            std::fs::read(directory.path().join(device::RECORD_FILE)).unwrap(),
            before
        );
        assert!(directory.path().join(shaman_pairing::REMOTES_FILE).exists());
        assert!(directory.path().join("drafts.json").exists());
    }

    #[test]
    fn a_key_of_another_kind_is_refused() {
        struct NotAKey;
        impl KeySigner for NotAKey {
            fn public_key(&self) -> Vec<u8> {
                b"not a key".to_vec()
            }
            fn sign(&self, _: Vec<u8>) -> Result<Vec<u8>, SignError> {
                unreachable!("nothing is signed")
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let Err(OpenError::KeyRefused { message }) = ThinClient::open(
            directory.path().to_string_lossy().into_owned(),
            Arc::new(NotAKey),
        ) else {
            panic!("opened over no key");
        };
        assert!(
            message.starts_with("This phone's identity key could not be used: "),
            "{message}"
        );
        assert!(message.contains("P-256"), "{message}");
    }

    #[test]
    fn a_kept_certificate_not_of_the_key_is_signed_again_and_kept_in_its_place() {
        let directory = tempfile::tempdir().unwrap();
        let key = TestKey::new();
        let signed = key.signed();
        drop(opened(directory.path(), &key));

        // The certificate kept beside the key's fingerprint is another key's.
        let elsewhere = tempfile::tempdir().unwrap();
        let other = opened(elsewhere.path(), &TestKey::new());
        let path = directory.path().join(device::RECORD_FILE);
        let mut kept: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        kept["key"]["certificate"] = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &other.identity().certificate,
        )
        .into();
        std::fs::write(&path, serde_json::to_vec(&kept).unwrap()).unwrap();

        let core = opened(directory.path(), &key);
        let presented = shaman_pairing::public_key_from_certificate(
            &core.identity().certificate.clone().into(),
        )
        .unwrap();
        assert_eq!(presented, key.public_key(), "the certificate is of the key");
        assert_eq!(signed.load(Ordering::Acquire), 2, "signed again");
        drop(core);
        drop(opened(directory.path(), &key));
        assert_eq!(signed.load(Ordering::Acquire), 2, "and kept");
    }

    #[test]
    fn a_rename_is_heard_and_kept_from_one_run_to_the_next() {
        let directory = tempfile::tempdir().unwrap();
        let key = TestKey::new();
        let core = opened(directory.path(), &key);
        let (_subscription, heard) = listening(&core);
        assert_eq!(heard.recv_timeout(HEARD_WITHIN).unwrap().name, "iPhone");

        act(core.rename_device("  Pocket phone ".to_owned())).expect("renamed");
        assert_eq!(
            heard.recv_timeout(HEARD_WITHIN).unwrap().name,
            "Pocket phone",
            "kept as the Serving side would keep it"
        );
        drop(core);

        assert_eq!(shown(directory.path(), &key).name, "Pocket phone");
    }

    #[test]
    fn a_name_the_serving_side_would_not_keep_is_refused_in_its_words_and_changes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let key = TestKey::new();
        let core = opened(directory.path(), &key);
        let (_subscription, heard) = listening(&core);
        heard.recv_timeout(HEARD_WITHIN).unwrap();

        let refused = act(core.rename_device("everywhere".to_owned())).expect_err("refused");
        assert_eq!(
            refused,
            ActError::Invalid {
                message: shaman_pairing::PeerNameRefusal::Everywhere.to_string()
            }
        );
        let refused = act(core.rename_device("n".repeat(65))).expect_err("refused");
        assert!(matches!(refused, ActError::Invalid { .. }), "{refused:?}");

        assert!(
            heard.recv_timeout(Duration::from_millis(50)).is_err(),
            "nothing changed to be heard"
        );
        drop(core);
        assert_eq!(shown(directory.path(), &key).name, "iPhone");
    }

    #[test]
    fn a_cancelled_listener_hears_no_more() {
        let directory = tempfile::tempdir().unwrap();
        let core = opened(directory.path(), &TestKey::new());
        let (subscription, heard) = listening(&core);
        heard.recv_timeout(HEARD_WITHIN).unwrap();
        subscription.cancel();

        act(core.rename_device("phone".to_owned())).expect("renamed");
        assert!(heard.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn an_act_its_server_does_not_answer_fails_as_unreachable_within_its_bound() {
        let directory = tempfile::tempdir().unwrap();
        let core = ThinClient::open_with(
            directory.path().to_string_lossy().into_owned(),
            TestKey::new(),
            CoreTimings {
                act_bound: Duration::from_millis(20),
                ..CoreTimings::default()
            },
        )
        .expect("open the core");

        let unanswered = act(core.act(std::future::pending())).expect_err("bounded");
        assert_eq!(
            unanswered,
            ActError::Unreachable {
                message: "The server did not answer in time".to_owned()
            }
        );
        assert_eq!(act(core.act(async { Ok(()) })), Ok(()));
    }

    /// The core kept in `directory`, writing a draft `after` its last
    /// change.
    fn writing_drafts_after(
        directory: &std::path::Path,
        key: &Arc<TestKey>,
        after: Duration,
    ) -> Arc<ThinClient> {
        ThinClient::open_with(
            directory.to_string_lossy().into_owned(),
            key.clone(),
            CoreTimings {
                draft_written_after: after,
                ..CoreTimings::default()
            },
        )
        .expect("open the core")
    }

    /// Ends the core as iOS ends the app, with nothing more done.
    fn ended(core: Arc<ThinClient>) {
        drop(core);
    }

    /// The Landing's key, which a draft is kept under whatever Remotes the
    /// phone holds.
    fn landing() -> String {
        beginning::LANDING_DRAFT.to_owned()
    }

    #[test]
    fn a_draft_is_there_after_the_app_is_ended_a_moment_after_typing_stops() {
        let directory = tempfile::tempdir().unwrap();
        let key = TestKey::new();
        let core = writing_drafts_after(directory.path(), &key, Duration::from_millis(20));
        core.clone()
            .window(None)
            .set_draft(landing(), "Tidy".to_owned(), 0);
        core.clone()
            .window(None)
            .set_draft(landing(), "Tidy the build".to_owned(), 0);
        let drafts = directory.path().join(drafts::DRAFTS_FILE);
        let deadline = std::time::Instant::now() + HEARD_WITHIN;
        while !drafts.exists() {
            assert!(std::time::Instant::now() < deadline, "never written");
            std::thread::sleep(Duration::from_millis(2));
        }
        ended(core);

        let core = opened(directory.path(), &key);
        assert_eq!(
            core.clone().window(None).draft(landing()),
            Draft {
                text: "Tidy the build".to_owned(),
                version: 0,
                given: Vec::new(),
                written_here: true,
            }
        );
    }

    #[test]
    fn a_draft_is_written_as_the_app_leaves_the_front_and_not_before_typing_stops() {
        let directory = tempfile::tempdir().unwrap();
        let key = TestKey::new();
        let core = writing_drafts_after(directory.path(), &key, Duration::from_secs(3_600));
        core.clone()
            .window(None)
            .set_draft(landing(), "still typing".to_owned(), 0);
        ended(core);
        let core = writing_drafts_after(directory.path(), &key, Duration::from_secs(3_600));
        assert_eq!(
            core.clone().window(None).draft(landing()).text,
            "",
            "ended before a moment passed"
        );

        core.clone()
            .window(None)
            .set_draft(landing(), "Tidy the build".to_owned(), 0);
        core.left_front();
        ended(core);
        assert_eq!(
            opened(directory.path(), &key)
                .window(None)
                .draft(landing())
                .text,
            "Tidy the build"
        );
    }

    #[test]
    fn a_draft_written_for_a_remote_not_held_is_dropped_as_the_core_opens() {
        let directory = tempfile::tempdir().unwrap();
        let key = TestKey::new();
        let session = format!("desk-key/{}", shaman_protocol::SessionId::new());
        drop(opened(directory.path(), &key));
        let keep = |landing_for: serde_json::Value| {
            std::fs::write(
                directory.path().join(drafts::DRAFTS_FILE),
                serde_json::to_vec(&serde_json::json!({
                    session.clone(): {"text": "half a thought"},
                    landing(): landing_for,
                }))
                .unwrap(),
            )
            .unwrap();
        };

        keep(serde_json::json!({"text": "a new one", "remote": "desk-key"}));
        let core = opened(directory.path(), &key);
        assert_eq!(
            core.clone().window(None).draft(session.clone()),
            Draft::default()
        );
        assert_eq!(
            core.clone().window(None).draft(landing()),
            Draft::default(),
            "the Landing's, written where the Outlook stood at a Remote not held"
        );
        drop(core);

        keep(serde_json::json!({"text": "a new one"}));
        let core = opened(directory.path(), &key);
        assert_eq!(
            core.clone().window(None).draft(landing()).text,
            "a new one",
            "written for no Remote"
        );
    }

    #[test]
    fn a_draft_kept_under_a_lost_key_is_not_there_under_the_new_one() {
        let directory = tempfile::tempdir().unwrap();
        let (lost, new) = (TestKey::new(), TestKey::new());
        let core = opened(directory.path(), &lost);
        core.clone()
            .window(None)
            .set_draft(landing(), "half a thought".to_owned(), 0);
        core.left_front();
        drop(core);
        assert_eq!(
            opened(directory.path(), &lost)
                .window(None)
                .draft(landing())
                .text,
            "half a thought"
        );

        let core = opened(directory.path(), &new);
        assert_eq!(core.clone().window(None).draft(landing()), Draft::default());
        assert!(!directory.path().join(drafts::DRAFTS_FILE).exists());
    }

    #[test]
    fn records_it_cannot_read_are_said_so_and_not_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(device::RECORD_FILE);
        std::fs::write(&path, b"{ not json").unwrap();
        let Err(OpenError::Failed { message }) = open(directory.path(), &TestKey::new()) else {
            panic!("opened over records it cannot read, or said the key was refused");
        };
        assert!(
            message.starts_with("This phone's records could not be read"),
            "{message}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }
}
