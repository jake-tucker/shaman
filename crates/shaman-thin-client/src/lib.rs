//! The thin Client's core: everything the iOS app is but its screen (ADR-0080).
//!
//! Swift makes one [`ThinClient`] from a directory in the app's container and
//! the signer of this phone's identity key, and reaches everything through it.
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
    sync::{Arc, Mutex},
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
mod beginning;
mod behind;
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
mod work;

pub use beginning::{Landing, LandingWorkspace, WorktreeChoice};
pub use conversation::{ConversationItem, ItemKind, Tone, TurnCard, TurnOutcome};
pub use device::ThisDevice;
use device::{DeviceRecord, Records};
pub use drafts::Draft;
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
pub use pairing::{InviteReading, OfferedWay, PairingOutcome, ReadInvite, TryInstead};
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
        }
    }
}

/// The core of one thin Client, kept in one directory.
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
    /// Every Remote's Sessions, merged into the one listing.
    listing: Listings,
    /// Held through each pairing, so one pairing at a time is made.
    pairing: AsyncMutex<()>,
    /// Each composer's unsent text.
    drafts: drafts::Drafts,
    /// The acts under way, which Swift holds background time for.
    acts: acts::Acts,
    /// The trees the open Sessions belong to, each read once.
    trees: tree::Trees,
    /// What the Landing says of a Session begun from it and left before its
    /// Server was seen holding it.
    landing_note: Arc<watch::Sender<Option<String>>>,
    /// What the Workspace Picker is searched for, until it is opened again.
    workspace_query: watch::Sender<String>,
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

    /// Tells `listener` where the Outlook stands now, and again on every
    /// change, until the subscription is cancelled.
    pub fn watch_outlook(&self, listener: Arc<dyn OutlookListener>) -> Arc<Subscription> {
        self.watch(self.inner.remotes.outlook(), move |outlook| {
            listener.changed(outlook);
        })
    }

    /// Tells `listener` what the listing shows now, and again on every
    /// change, until the subscription is cancelled.
    pub fn watch_listing(&self, listener: Arc<dyn ListingListener>) -> Arc<Subscription> {
        self.watch(self.inner.listing.listing(), move |listing| {
            listener.changed(listing);
        })
    }

    /// Tells `listener` what the core says of its own accord now, beside what
    /// an act's failure says, and again on every change, until the
    /// subscription is cancelled: shown in the banner an act's failure is,
    /// until Swift says, by [`Self::notice_seen`], that it was seen.
    pub fn watch_notices(&self, listener: Arc<dyn NoticeListener>) -> Arc<Subscription> {
        self.watch(self.inner.listing.notices().shown(), move |notice| {
            listener.changed(notice);
        })
    }

    /// Hears that the notice `id` names has been seen, so the next is shown.
    /// It is never said again.
    pub fn notice_seen(&self, id: u64) {
        self.inner.listing.notices().seen(id);
    }

    /// Tells `listener` whether any act is under way now, and again each
    /// time that changes, until the subscription is cancelled: Swift holds
    /// iOS background time only while one is.
    pub fn watch_acts(&self, listener: Arc<dyn ActsListener>) -> Arc<Subscription> {
        self.watch(self.inner.acts.busy(), move |under_way| {
            listener.changed(under_way);
        })
    }

    /// The unsent text of the composer of the Session `session` names,
    /// asked for when its screen appears, with the version of it Swift hands
    /// back with each change.
    pub fn draft(&self, session: String) -> Draft {
        self.inner.drafts.get(&session)
    }

    /// Holds `text`, written over the version `version` of the draft, as the
    /// unsent text of the composer of the Session `session` names. Text the
    /// core gave back since that version stands ahead of it, and the
    /// draft's listeners are told.
    pub fn set_draft(&self, session: String, text: String, version: u64) {
        self.inner.drafts.set(&session, text, version);
    }

    /// Tells `listener` the draft of the composer of the Session `session`
    /// names now, and again each time the core changes it on its own: a
    /// Prompt whose send failed, or that a stop withdrew, given back ahead of
    /// what is written, which the field is to take.
    pub fn watch_draft(
        &self,
        session: String,
        listener: Arc<dyn DraftListener>,
    ) -> Arc<Subscription> {
        self.watch(self.inner.drafts.changes(&session), move |held| {
            listener.changed(held.draft());
        })
    }

    /// Tells `listener` what the Landing shows now, where the Outlook
    /// stands, and again on every change, until the subscription is
    /// cancelled.
    pub fn watch_landing(&self, listener: Arc<dyn LandingListener>) -> Arc<Subscription> {
        let inner = self.inner.clone();
        let mut outlook = inner.remotes.outlook();
        let mut moved = inner.listing.moved();
        let mut noted = inner.landing_note.subscribe();
        let task = self.runtime().spawn(async move {
            // Held for as long as the Landing is answered, so a Workspace is
            // resolved for it only while it is up.
            let _up = inner.listing.landing_up();
            let mut told = None;
            let mut asked = None;
            loop {
                outlook.borrow_and_update();
                moved.borrow_and_update();
                noted.borrow_and_update();
                // What a new Session would get, and what its Workspace may
                // begin on, are read again as the screen opens and as it
                // turns elsewhere; and as the Remote comes back, by its
                // reader.
                let standing = inner.remotes.standing();
                if standing != asked {
                    if let Some((remote, _)) = &standing
                        && asked.as_ref().is_none_or(|(was, _)| was != remote)
                    {
                        inner.listing.ask_for_agent(remote);
                    }
                    inner
                        .listing
                        .aim_landing(standing.as_ref().and_then(|(remote, workspace)| {
                            workspace
                                .as_ref()
                                .map(|workspace| (remote.as_str(), workspace.path.as_path()))
                        }));
                    asked = standing;
                }
                let landing = inner.landing();
                if told.as_ref() != Some(&landing) {
                    listener.changed(landing.clone());
                    told = Some(landing);
                }
                tokio::select! {
                    changed = outlook.changed() => if changed.is_err() { break },
                    changed = moved.changed() => if changed.is_err() { break },
                    changed = noted.changed() => if changed.is_err() { break },
                }
            }
        });
        Subscription::of(task)
    }

    /// Tells `listener` what the Workspace Picker shows now, searched for
    /// nothing, and again on every change, until the subscription is
    /// cancelled: every Remote's Workspaces, the Outlook's ticked.
    pub fn watch_workspaces(&self, listener: Arc<dyn WorkspacesListener>) -> Arc<Subscription> {
        let inner = self.inner.clone();
        // Each time the picker opens, it opens unsearched, and asks every
        // Remote for its Workspaces afresh.
        inner.workspace_query.send_replace(String::new());
        inner.listing.ask_for_workspaces();
        let mut outlook = inner.remotes.outlook();
        let mut moved = inner.listing.moved();
        let mut query = inner.workspace_query.subscribe();
        let task = self.runtime().spawn(async move {
            let mut told = None;
            loop {
                outlook.borrow_and_update();
                moved.borrow_and_update();
                let searched = query.borrow_and_update().clone();
                let standing = inner.remotes.standing();
                let picker = inner.listing.workspaces(
                    standing.as_ref().and_then(|(remote, workspace)| {
                        workspace
                            .as_ref()
                            .map(|workspace| (remote.as_str(), workspace.path.as_path()))
                    }),
                    &searched,
                );
                if told.as_ref() != Some(&picker) {
                    listener.changed(picker.clone());
                    told = Some(picker);
                }
                tokio::select! {
                    changed = outlook.changed() => if changed.is_err() { break },
                    changed = moved.changed() => if changed.is_err() { break },
                    changed = query.changed() => if changed.is_err() { break },
                }
            }
        });
        Subscription::of(task)
    }

    /// Searches the Workspace Picker by name for `text`, until it is opened
    /// again; an empty one searches for nothing.
    pub fn search_workspaces(&self, text: String) {
        self.inner.workspace_query.send_replace(text);
    }

    /// Turns the Outlook toward the Workspace a row of the Workspace Picker
    /// hands back, so the next Session begins there, leaving the listing's
    /// scope where it was. Refused where its Server cannot read it.
    pub async fn choose_workspace(&self, workspace: String) -> Result<(), ActError> {
        let turn = self.inner.listing.choosing(&workspace)?;
        let inner = self.inner.clone();
        self.run(async move {
            inner
                .change_record_shown(move |record| record.turn(&turn.remote, turn.workspace))
                .await
        })
        .await?
    }

    /// Begins a Session with the Prompt `text` where the Outlook stands, on
    /// its Workspace's main Worktree or, where `new_worktree`, in a new one
    /// its Server prepares first, under an identity the core chooses, and
    /// opens it at once as the Provisional Session for the Landing's screen
    /// to turn into: until its Server answers with the Session it made,
    /// which replaces it in place, or refuses it, which stands in its
    /// conversation. The beginning goes on whatever becomes of the screen,
    /// as an act under way. Refused at once where nothing is written, where
    /// no Workspace is remembered for the Outlook's Remote, where it is a
    /// bare Repository, or where a new Worktree is asked of a Workspace the
    /// Landing offers none in.
    pub fn begin_session(
        &self,
        text: String,
        new_worktree: bool,
    ) -> Result<Arc<OpenSession>, ActError> {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(ActError::Invalid {
                message: "Write something to send".to_owned(),
            });
        }
        let Some((remote, workspace)) = self.inner.remotes.standing() else {
            return Err(ActError::Gone {
                message: "This phone is paired with no server to begin a session on".to_owned(),
            });
        };
        let Some(workspace) = workspace else {
            let origin_named = self.inner.remotes.names().len() > 1;
            return Err(ActError::Invalid {
                message: beginning::asks(origin_named.then_some(remote.as_str())),
            });
        };
        let Some(fingerprint) = self.inner.listing.fingerprint(&remote) else {
            return Err(ActError::Gone {
                message: frame::GONE_REMOTE.to_owned(),
            });
        };
        let session_id = shaman_protocol::SessionId::new();
        let reference = format!("{fingerprint}/{session_id}");
        let origin = self
            .inner
            .listing
            .opening(&reference)
            .and_then(|opening| opening.listed.origin);
        let place = origin
            .into_iter()
            .chain([workspace.name.clone()])
            .collect::<Vec<_>>()
            .join(" · ");
        let landing = self.inner.landing();
        if let Some(working_copy) = landing.working_copy {
            return Err(ActError::Invalid {
                message: working_copy,
            });
        }
        if new_worktree && landing.worktree.is_none() {
            return Err(ActError::Invalid {
                message: "No new worktree can be made in this workspace".to_owned(),
            });
        }
        let provisional =
            beginning::Provisional::new(session_id, workspace.path.clone(), text, place);
        let provisional = if new_worktree {
            provisional.in_new_worktree()
        } else {
            provisional
        };
        // A Session begun anew says nothing more of one left before.
        self.inner.landing_note.send_replace(None);
        Ok(session::begin(
            self.runtime().handle(),
            &self.inner.listing,
            &reference,
            self.pacing(),
            self.asked(),
            provisional,
        ))
    }

    /// Opens the Session `session` names for a screen pushed to read it,
    /// framed at once by what the listing knows of it, and turns the Outlook
    /// toward its Origin and Workspace. Closing it cancels nothing on the
    /// Server.
    pub fn open_session(&self, session: String) -> Arc<OpenSession> {
        if let Some(opening) = self.inner.listing.opening(&session) {
            let inner = self.inner.clone();
            let listed = opening.listed;
            self.runtime().spawn(async move {
                let turned = inner
                    .change_record_shown(move |record| record.turn_toward(&listed))
                    .await;
                if let Err(error) = turned {
                    tracing::warn!("the Outlook was turned and not kept: {error}");
                }
            });
        }
        session::open(
            self.runtime().handle(),
            &self.inner.listing,
            &session,
            self.pacing(),
            self.asked(),
        )
    }

    /// Sets the Session `session` names aside as done for now.
    pub async fn settle(&self, session: String) -> Result<(), ActError> {
        self.settling(session, true).await
    }

    /// Brings the Session `session` names back among the active.
    pub async fn unsettle(&self, session: String) -> Result<(), ActError> {
        self.settling(session, false).await
    }

    /// Brings up the next of the settled shelf's rows, until the core is
    /// opened again.
    pub fn show_more_settled(&self) {
        self.inner.listing.show_more();
    }

    /// Narrows the listing to the scope `scope` names, as a choice of the
    /// menu its title opens carries it, until the core is opened again; one
    /// naming no Remote held narrows it to Everywhere. A Remote or a
    /// Workspace chosen turns the Outlook toward it, so that is where the
    /// next Session begins, and Everywhere turns nothing.
    pub async fn narrow_listing(&self, scope: String) -> Result<(), ActError> {
        let Some(turn) = self.inner.listing.narrow(&scope) else {
            return Ok(());
        };
        let inner = self.inner.clone();
        self.run(async move {
            inner
                .change_record_shown(move |record| record.turn(&turn.remote, turn.workspace))
                .await
        })
        .await?
    }

    /// Narrows the listing to the scope `scope` names, as Swift kept it from
    /// a listing in scene state and hands it back after a cold start, and
    /// turns nothing. A scope that is gone narrows it to Everywhere: one
    /// naming a Remote not held, or a Workspace its Remote no longer offers
    /// once it has said what it offers; until then it stands as handed back.
    pub fn restore_listing(&self, scope: String) {
        self.inner.listing.restore(&scope);
    }

    /// Searches the listing by Title for `text`, until the core is opened
    /// again; an empty one searches for nothing.
    pub fn search_listing(&self, text: String) {
        self.inner.listing.search(&text);
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
    /// suspended.
    pub fn left_front(&self) {
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

    /// Reads `invite` for the confirmation screen, dialling nothing.
    pub fn read_invite(&self, invite: String) -> InviteReading {
        pairing::read(&self.inner, &invite)
    }

    /// Pairs with the Server `invite` is from, naming it `remote_name` and
    /// this phone `device_name`, and turns the Outlook toward it; where this
    /// phone holds that Server already, the new Pairing replaces the old.
    /// Keeps dialling for a few seconds while no way answers, and keeps
    /// nothing where it fails.
    pub async fn confirm_pairing(
        &self,
        invite: String,
        remote_name: String,
        device_name: String,
    ) -> PairingOutcome {
        let inner = self.inner.clone();
        let under_way = self.inner.acts.begin();
        let confirmed = self.run(async move {
            let _under_way = under_way;
            pairing::confirm(&inner, &invite, &remote_name, &device_name).await
        });
        confirmed
            .await
            .unwrap_or_else(|error| PairingOutcome::Failed {
                message: error.to_string(),
                again: TryInstead::TryAgain,
            })
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
            drafts: drafts::Drafts::default(),
            acts: acts::Acts::default(),
            trees,
            landing_note: Arc::new(watch::Sender::new(None)),
            workspace_query: watch::Sender::new(String::new()),
            timings,
        });
        for name in inner.remotes.names() {
            let inner = inner.clone();
            runtime.spawn(async move { inner.remotes.ask_ways(&name).await });
        }
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

    /// What the core hands each open Session for its acts.
    fn asked(&self) -> session::Asked {
        let pacing = self.pacing();
        session::Asked {
            drafts: self.inner.drafts.clone(),
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
            landing_note: self.inner.landing_note.clone(),
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

    /// The Landing where the Outlook stands now.
    fn landing(&self) -> Landing {
        let standing = self.remotes.standing();
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
            self.landing_note.borrow().clone(),
        )
    }

    /// Changes the thin Client's own file by `change`, keeping it before
    /// "This device" and the Outlook are told: where it cannot be kept,
    /// nothing changes.
    async fn change_record(
        &self,
        change: impl FnOnce(&mut DeviceRecord) + Send + 'static,
    ) -> Result<(), ActError> {
        self.changing_record(change, false).await
    }

    /// Changes the thin Client's own file by `change`, telling "This device"
    /// and the Outlook whether or not it could be kept, as what the change
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

/// Hears whether any act is under way, at once and each time that changes.
#[uniffi::export(with_foreign)]
pub trait NoticeListener: Send + Sync {
    fn changed(&self, notice: Option<Notice>);
}

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
