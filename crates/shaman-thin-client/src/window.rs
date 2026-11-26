//! A window of the app: a Client of its own over the device's core.
//!
//! Each window the app shows asks the device's [`ThinClient`] for one, and
//! holds for itself what a Client holds: its Outlook and the Workspace it
//! stands at, its listing's scope, search and shelf, its Landing and what
//! the Landing says, a Workspace Picker's search, and which notices it has
//! said. Everything else is the device's, one for the phone however many
//! windows it shows: the identity and the name, the Pairings, relay logins
//! and connections, what each Remote says, the front, and every draft, which
//! a window reads and writes through here but never holds.

use std::sync::Arc;

use tokio::sync::watch;

use crate::{
    ActError, Draft, DraftListener, LandingListener, ListingListener, NoticeListener, OpenSession,
    OutlookListener, PairingOutcome, Subscription, ThinClient, TryInstead, WorkspacesListener,
    beginning, frame, listing::ListingView, notices::WindowNotices, pairing,
    remotes::WindowOutlook, session,
};

/// What one window holds for itself, while it lives.
pub(crate) struct Window {
    /// Which window it is, of all the core has handed out.
    pub(crate) id: u64,
    pub(crate) outlook: Arc<WindowOutlook>,
    /// The listing as this window narrows it.
    pub(crate) view: ListingView,
    pub(crate) notices: WindowNotices,
    /// What its Landing says of a Session begun from it and left before its
    /// Server was seen holding it.
    pub(crate) landing_note: Arc<watch::Sender<Option<String>>>,
    /// What its Workspace Picker is searched for, until it is opened again.
    pub(crate) workspace_query: watch::Sender<String>,
    /// Whether it was handed back an Outlook its scene kept, as it was made.
    pub(crate) handed_back: bool,
}

/// One window of the app over the device's core, which it holds open.
#[derive(uniffi::Object)]
pub struct ThinClientWindow {
    device: Arc<ThinClient>,
    window: Arc<Window>,
}

impl ThinClientWindow {
    pub(crate) fn new(device: Arc<ThinClient>, window: Arc<Window>) -> Arc<Self> {
        Arc::new(Self { device, window })
    }
}

#[uniffi::export]
impl ThinClientWindow {
    /// Tells `listener` where this window's Outlook stands now, and again on
    /// every change, until the subscription is cancelled.
    pub fn watch_outlook(&self, listener: Arc<dyn OutlookListener>) -> Arc<Subscription> {
        self.device
            .watch(self.window.outlook.outlook(), move |outlook| {
                listener.changed(outlook);
            })
    }

    /// Tells `listener` what this window's listing shows now, and again on
    /// every change, until the subscription is cancelled.
    pub fn watch_listing(&self, listener: Arc<dyn ListingListener>) -> Arc<Subscription> {
        self.device
            .watch(self.window.view.listing(), move |listing| {
                listener.changed(listing);
            })
    }

    /// Tells `listener` what the core says of its own accord in this window
    /// now, beside what an act's failure says, and again on every change,
    /// until the subscription is cancelled: shown in the banner an act's
    /// failure is, until Swift says, by [`Self::notice_seen`], that it was
    /// seen. Each window says each for itself.
    pub fn watch_notices(&self, listener: Arc<dyn NoticeListener>) -> Arc<Subscription> {
        // Followed, the window stands, and says what was said as none stood.
        self.device.inner.stands(&self.window);
        self.device
            .inner
            .listing
            .notices()
            .stands(&self.window.notices);
        self.device
            .watch(self.window.notices.shown(), move |notice| {
                listener.changed(notice);
            })
    }

    /// Hears that the notice `id` names has been seen in this window, so its
    /// next is shown. This window never says it again.
    pub fn notice_seen(&self, id: u64) {
        self.window.notices.seen(id);
    }

    /// The unsent text of the composer of the Session `session` names, or of
    /// the Landing's under the key it names, asked for when its screen
    /// appears, with the version of it Swift hands back with each change:
    /// the device's, kept from one run to the next, whichever window wrote
    /// it last.
    pub fn draft(&self, session: String) -> Draft {
        self.device
            .inner
            .drafts
            .get_for(&session, Some(self.window.id))
    }

    /// Holds `text`, written over the version `version` of the draft, as the
    /// unsent text of the composer of the Session `session` names. Text the
    /// core gave back since that version stands ahead of it, and the
    /// draft's listeners are told. It is written to the directory a moment
    /// after the last change, and as the app leaves the front.
    pub fn set_draft(&self, session: String, text: String, version: u64) {
        let inner = &self.device.inner;
        // The Landing's is written for the Remote this window's Outlook
        // stands at, and dropped with it.
        let remote = (session == beginning::LANDING_DRAFT)
            .then(|| inner.remotes.standing(&self.window.outlook))
            .flatten()
            .and_then(|(remote, _)| inner.listing.fingerprint(&remote));
        inner
            .drafts
            .set_for(&session, text, version, remote, Some(self.window.id));
    }

    /// Tells `listener` the draft of the composer of the Session `session`
    /// names now, and again each time the core changes it on its own: a
    /// Prompt whose send failed, or that a stop withdrew, given back ahead of
    /// what is written, which each field of the Session, in every window, is
    /// to put ahead of its own text.
    pub fn watch_draft(
        &self,
        session: String,
        listener: Arc<dyn DraftListener>,
    ) -> Arc<Subscription> {
        let window = self.window.id;
        self.device
            .watch(self.device.inner.drafts.changes(&session), move |held| {
                listener.changed(held.draft_for(Some(window)));
            })
    }

    /// Tells `listener` what this window's Landing shows now, where its
    /// Outlook stands, and again on every change, until the subscription is
    /// cancelled.
    pub fn watch_landing(&self, listener: Arc<dyn LandingListener>) -> Arc<Subscription> {
        let inner = self.device.inner.clone();
        let window = self.window.clone();
        let mut outlook = window.outlook.outlook();
        let mut moved = inner.listing.moved();
        let mut noted = window.landing_note.subscribe();
        let task = self.device.runtime().spawn(async move {
            // Held for as long as the Landing is answered, so a Workspace is
            // resolved for it only while it is up.
            let up = inner.listing.landing_up();
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
                let standing = inner.remotes.standing(&window.outlook);
                if standing != asked {
                    if let Some((remote, _)) = &standing
                        && asked.as_ref().is_none_or(|(was, _)| was != remote)
                    {
                        inner.listing.ask_for_agent(remote);
                    }
                    up.aim(standing.as_ref().and_then(|(remote, workspace)| {
                        workspace
                            .as_ref()
                            .map(|workspace| (remote.as_str(), workspace.path.as_path()))
                    }));
                    asked = standing;
                }
                let landing = inner.landing(&window);
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

    /// Tells `listener` what this window's Workspace Picker shows now,
    /// searched for nothing, and again on every change, until the
    /// subscription is cancelled: every Remote's Workspaces, the Outlook's
    /// ticked.
    pub fn watch_workspaces(&self, listener: Arc<dyn WorkspacesListener>) -> Arc<Subscription> {
        let inner = self.device.inner.clone();
        let window = self.window.clone();
        // Each time the picker opens, it opens unsearched, and asks every
        // Remote for its Workspaces afresh.
        window.workspace_query.send_replace(String::new());
        inner.listing.ask_for_workspaces();
        let mut outlook = window.outlook.outlook();
        let mut moved = inner.listing.moved();
        let mut query = window.workspace_query.subscribe();
        let task = self.device.runtime().spawn(async move {
            let mut told = None;
            loop {
                outlook.borrow_and_update();
                moved.borrow_and_update();
                let searched = query.borrow_and_update().clone();
                let standing = inner.remotes.standing(&window.outlook);
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

    /// Searches this window's Workspace Picker by name for `text`, until it
    /// is opened again; an empty one searches for nothing.
    pub fn search_workspaces(&self, text: String) {
        self.window.workspace_query.send_replace(text);
    }

    /// Turns this window's Outlook toward the Workspace a row of the
    /// Workspace Picker hands back, so the next Session begun in it begins
    /// there, leaving the listing's scope where it was. Refused where its
    /// Server cannot read it.
    pub async fn choose_workspace(&self, workspace: String) -> Result<(), ActError> {
        let turn = self.device.inner.listing.choosing(&workspace)?;
        self.turn(turn).await
    }

    /// Begins a Session with the Prompt `text` where this window's Outlook
    /// stands, on its Workspace's main Worktree or, where `new_worktree`, in
    /// a new one its Server prepares first, under an identity the core
    /// chooses, and opens it at once as the Provisional Session for the
    /// Landing's screen to turn into: until its Server answers with the
    /// Session it made, which replaces it in place, or refuses it, which
    /// stands in its conversation. The beginning goes on whatever becomes of
    /// the screen, as an act under way. Refused at once where nothing is
    /// written, where no Workspace is remembered for the Outlook's Remote,
    /// where it is a bare Repository, or where a new Worktree is asked of a
    /// Workspace the Landing offers none in.
    pub fn begin_session(
        &self,
        text: String,
        new_worktree: bool,
    ) -> Result<Arc<OpenSession>, ActError> {
        let inner = &self.device.inner;
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(ActError::Invalid {
                message: "Write something to send".to_owned(),
            });
        }
        let Some((remote, workspace)) = inner.remotes.standing(&self.window.outlook) else {
            return Err(ActError::Gone {
                message: "This phone is paired with no server to begin a session on".to_owned(),
            });
        };
        let Some(workspace) = workspace else {
            let origin_named = inner.remotes.names().len() > 1;
            return Err(ActError::Invalid {
                message: beginning::asks(origin_named.then_some(remote.as_str())),
            });
        };
        let Some(fingerprint) = inner.listing.fingerprint(&remote) else {
            return Err(ActError::Gone {
                message: frame::GONE_REMOTE.to_owned(),
            });
        };
        let session_id = shaman_protocol::SessionId::new();
        let reference = format!("{fingerprint}/{session_id}");
        let origin = inner
            .listing
            .opening(&reference)
            .and_then(|opening| opening.listed.origin);
        let place = origin
            .into_iter()
            .chain([workspace.name.clone()])
            .collect::<Vec<_>>()
            .join(" · ");
        let landing = inner.landing(&self.window);
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
        // Kept in the Landing's draft until its Server answers with the
        // Session, so it is there the next run should the app end first.
        let sent = inner
            .drafts
            .sending(beginning::LANDING_DRAFT, &text, Some(fingerprint.clone()));
        // Its own composer's draft is not dropped by a listing that does not
        // hold it yet.
        inner.drafts.beginning(&reference);
        let provisional =
            beginning::Provisional::new(session_id, workspace.path.clone(), text, place)
                .sent_from_landing(sent);
        let provisional = if new_worktree {
            provisional.in_new_worktree()
        } else {
            provisional
        };
        // A Session begun anew says nothing more of one left before.
        self.window.landing_note.send_replace(None);
        Ok(session::begin(
            self.device.runtime().handle(),
            &inner.listing,
            &reference,
            self.device.pacing(),
            self.device.asked(&self.window),
            provisional,
        ))
    }

    /// Opens the Session `session` names for a screen pushed to read it,
    /// framed at once by what the listing knows of it, and turns this
    /// window's Outlook toward its Origin and Workspace. Closing it cancels
    /// nothing on the Server.
    pub fn open_session(&self, session: String) -> Arc<OpenSession> {
        let inner = &self.device.inner;
        if let Some(opening) = inner.listing.opening(&session) {
            let (remote, workspace) = inner
                .remotes
                .turn_toward(&self.window.outlook, &opening.listed);
            self.keep_turn(remote, workspace);
        }
        session::open(
            self.device.runtime().handle(),
            &inner.listing,
            &session,
            self.device.pacing(),
            self.device.asked(&self.window),
        )
    }

    /// Brings up the next of the settled shelf's rows in this window, while
    /// it lives.
    pub fn show_more_settled(&self) {
        self.window.view.show_more();
    }

    /// Narrows this window's listing to the scope `scope` names, as a choice
    /// of the menu its title opens carries it, while the window lives; one
    /// naming no Remote held narrows it to Everywhere. A Remote or a
    /// Workspace chosen turns this window's Outlook toward it, so that is
    /// where the next Session begun in it begins, and Everywhere turns
    /// nothing.
    pub async fn narrow_listing(&self, scope: String) -> Result<(), ActError> {
        let Some(turn) = self.window.view.narrow(&scope) else {
            return Ok(());
        };
        self.turn(turn).await
    }

    /// Narrows this window's listing to the scope `scope` names, as Swift
    /// kept it from a listing in scene state and hands it back after a cold
    /// start, and turns nothing. A scope that is gone narrows it to
    /// Everywhere: one naming a Remote not held, or a Workspace its Remote no
    /// longer offers once it has said what it offers; until then it stands
    /// as handed back.
    pub fn restore_listing(&self, scope: String) {
        self.window.view.restore(&scope);
    }

    /// What the scene is to keep of where this window's Outlook stands now,
    /// its Remote and the Workspace it stands in there, as a plain string
    /// Swift keeps beside the listing's scope and hands back by
    /// [`ThinClient::window`] after a cold start; none while the phone
    /// holds no Remote. It changes only as the Outlook does.
    pub fn kept_outlook(&self) -> Option<String> {
        self.device.inner.remotes.kept(&self.window.outlook)
    }

    /// Searches this window's listing by Title for `text`, while the window
    /// lives; an empty one searches for nothing.
    pub fn search_listing(&self, text: String) {
        self.window.view.search(&text);
    }

    /// Pairs with the Server `invite` is from, naming it `remote_name` and
    /// this phone `device_name`, and turns this window's Outlook toward it;
    /// where this phone holds that Server already, the new Pairing replaces
    /// the old, and every window remembers of it what it remembered. Keeps
    /// dialling for a few seconds while no way answers, and keeps nothing
    /// where it fails.
    pub async fn confirm_pairing(
        &self,
        invite: String,
        remote_name: String,
        device_name: String,
    ) -> PairingOutcome {
        let inner = self.device.inner.clone();
        let window = self.window.clone();
        let under_way = inner.acts.begin();
        let confirmed = self.device.run(async move {
            let _under_way = under_way;
            pairing::confirm(&inner, &window, &invite, &remote_name, &device_name).await
        });
        confirmed
            .await
            .unwrap_or_else(|error| PairingOutcome::Failed {
                message: error.to_string(),
                again: TryInstead::TryAgain,
            })
    }
}

impl ThinClientWindow {
    /// Turns this window's Outlook as `turn` says, keeping it as the phone's
    /// latest turn.
    async fn turn(&self, turn: crate::listing::Turn) -> Result<(), ActError> {
        let inner = self.device.inner.clone();
        let standing = inner
            .remotes
            .turn(&self.window.outlook, &turn.remote, turn.workspace);
        let remote = turn.remote;
        self.device
            .run(async move {
                inner
                    .change_record_shown(move |record| record.turn(&remote, standing))
                    .await
            })
            .await?
    }

    /// Keeps this window's turn toward the Remote `remote`, standing in
    /// `workspace`, as the phone's latest, in the background.
    fn keep_turn(&self, remote: String, workspace: Option<crate::device::RememberedWorkspace>) {
        let inner = self.device.inner.clone();
        self.device.runtime().spawn(async move {
            let turned = inner
                .change_record_shown(move |record| record.turn(&remote, workspace))
                .await;
            if let Err(error) = turned {
                tracing::warn!("the Outlook was turned and not kept: {error}");
            }
        });
    }
}
