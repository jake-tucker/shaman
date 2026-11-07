//! Changing a Setting: the one operation through which both the settings
//! panel's change and a Sidekick's `set_setting` edit the Config Document and
//! put what it then says in force, so a Setting a Sidekick changed takes
//! effect and reaches every Client exactly as one the user changed does.
//!
//! A change is whole and one at a time. It runs on a task of the Server's own,
//! so one whose caller stops waiting — a Sidekick's harness hanging up, a
//! Client gone — is still put in force rather than left written to the
//! document while the running Server and its Clients go on without it. And
//! each change holds every later one off from its write through its
//! adoption, its broadcast and the Approval Postures it reconciles, so the
//! settings in force are always those the document last said, never an older
//! change adopted after a newer one.
//!
//! The operation changes whatever Setting it is given. What a Sidekick may
//! change is bounded where its Tool is served, over the schema (ADR 0043),
//! since the user's own panel may change every Setting there is.
//!
//! The document may also change by any other hand: a user's editor, their
//! dotfiles, a company's rollout. This module follows that too (ADR 0067): the
//! Server looks at the document every so often, and what it then says is put in
//! force as one change more, so a value pinned by hand is adopted exactly as
//! one set from the panel is. A look costs a `stat` of each name a document may
//! go by, and reads a document only where [`Looks`] says that could tell
//! anything new. What is read must say the same twice running first, since
//! nothing stops another writer being seen partway through a save.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime},
};

use tokio::sync::{Mutex, watch};

use super::{RegistryStanding, RootDiscovery, SessionOperations};
use crate::server::root_discovery::Ticket;
use crate::{
    protocol::{SettingMutation, SettingsSnapshot},
    provider::{ProviderOrchestrator, ProviderRuntime},
    relays::RelayController,
    serving::ServingController,
    sessions::SessionStore,
    settings::{ConfigDocuments, DocumentStamps, SettingsMutationError},
};

/// The Config Documents this Server alone writes, and everything that runs
/// under the Settings they leave in force and so adopts each change of them.
#[derive(Clone)]
pub(crate) struct SettingsAdoption {
    documents: ConfigDocuments,
    /// The settings in force, pushed to every attached Client as they change.
    snapshot: Arc<watch::Sender<SettingsSnapshot>>,
    /// Every hosted Provider runtime, handed the Server Settings it now runs
    /// under.
    runtimes: Arc<Vec<Arc<dyn ProviderRuntime>>>,
    /// Moved to whatever the Serving Settings now say.
    serving: ServingController,
    /// Holds an entry for each Relay the Settings now pin.
    relays: RelayController,
    /// Where each change runs, whole and after the one before it.
    changes: OneChangeAtATime,
}

impl SettingsAdoption {
    /// Puts a freshly loaded effective-settings view in force: its problems
    /// reach the Log, every hosted Provider runtime takes the Server Settings
    /// it now runs under, the Server holds an entry for each Relay it pins, the
    /// Serving listener moves to what its Settings say, and every attached
    /// client receives the snapshot. Startup and every change adopt a view
    /// this one way. Answers why the listener could not follow, where it could
    /// not, and everything else has followed all the same.
    pub(in crate::server) async fn adopt(&self, snapshot: &SettingsSnapshot) -> anyhow::Result<()> {
        crate::settings::log_diagnostics(&snapshot.diagnostics);
        for runtime in self.runtimes.iter() {
            runtime.apply_settings(&snapshot.settings);
        }
        let (pinned_relays, pinned_in) = crate::settings::relay_pins(snapshot);
        self.relays.adopt_pins(pinned_relays, pinned_in);
        let serving = self.serving.adopt(snapshot.settings.serving.clone()).await;
        self.snapshot.send_replace(snapshot.clone());
        serving
    }

    pub(in crate::server) fn new(
        documents: ConfigDocuments,
        snapshot: Arc<watch::Sender<SettingsSnapshot>>,
        runtimes: Arc<Vec<Arc<dyn ProviderRuntime>>>,
        serving: ServingController,
        relays: RelayController,
    ) -> Self {
        Self {
            documents,
            snapshot,
            runtimes,
            serving,
            relays,
            changes: OneChangeAtATime::default(),
        }
    }
}

/// A change of a Setting that landed: written to the Config Document and in
/// force everywhere it could be put.
pub(crate) struct SettingChanged {
    /// The settings in force now, as every attached Client was told them.
    pub(crate) snapshot: SettingsSnapshot,
    /// Why the Serving listener could not be moved to what the Serving
    /// Settings now say, where it could not. Everything else follows them all
    /// the same, the Approval Postures they make included.
    pub(crate) serving: Option<ServingNotAdopted>,
}

/// Why the Serving listener could not follow the Serving Settings. What went
/// wrong names the address and port it could not take — the values of
/// Settings no Sidekick may read — so it has no `Display` and no `Debug` that
/// shows it: only the user's own surfaces and the Log read [`Self::detail`],
/// and anything answering a Sidekick words the failure itself.
pub(crate) struct ServingNotAdopted(anyhow::Error);

impl ServingNotAdopted {
    /// What went wrong, for the user's own surfaces alone.
    pub(crate) fn detail(&self) -> String {
        self.0.to_string()
    }

    /// The listener's failure as `error` says it, for a test standing in for
    /// one.
    #[cfg(test)]
    pub(crate) fn because(error: anyhow::Error) -> Self {
        Self(error)
    }
}

impl std::fmt::Debug for ServingNotAdopted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ServingNotAdopted(..)")
    }
}

impl SessionOperations {
    /// Applies `mutation` to the winning Config Document and puts the
    /// settings the reloaded document yields in force: every hosted Provider
    /// runtime takes the Server Settings it now runs under, the Serving
    /// listener moves to what its Settings say, every attached Client is
    /// pushed the snapshot, each live Session following a Provider's
    /// Approval Posture Settings takes the posture they now make, and
    /// Workspace Roots that changed begin to be discovered afresh, what was
    /// discovered under a Root taken away dropped with it unless a Session
    /// belongs to it. The change runs whole, after every change asked for
    /// before it has, and on a task of the Server's own, so it completes
    /// whether or not this is awaited to the end. A document that cannot
    /// take the edit changes nothing. Once it has run, and so holds no later
    /// change off, its answer waits on the discovery it began, though never
    /// past the bound any wait on discovery keeps.
    pub(crate) async fn change_setting(
        &self,
        mutation: SettingMutation,
    ) -> Result<SettingChanged, SettingsMutationError> {
        let changed = self
            .put_settings_in_force(move |documents, _| documents.mutate(&mutation).map(Some))
            .await?;
        Ok(changed.expect("a mutation always yields settings to put in force"))
    }

    /// Looks at the Config Documents every `interval` until `shutdown`, putting
    /// in force what they say as they come to say something else.
    pub(crate) fn follow_config_documents(
        &self,
        interval: Duration,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let operations = self.clone();
        tokio::spawn(async move {
            let mut looks = Looks::default();
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => break,
                    () = tokio::time::sleep(interval) => {}
                }
                let adoption = &operations.settings_adoption;
                // Stamped before anything is read, so an edit landing between
                // the two is told by the next look's stamp.
                let documents = adoption.documents.clone();
                let stamps = tokio::task::spawn_blocking(move || documents.stamps()).await;
                // Only the Server stopping leaves a look unfinished.
                let Ok(stamps) = stamps else {
                    break;
                };
                if !looks.call_for_a_read(stamps, SystemTime::now()) {
                    continue;
                }
                let documents = adoption.documents.clone();
                let in_force = adoption.snapshot.borrow().clone();
                let said = {
                    let in_force = in_force.clone();
                    tokio::task::spawn_blocking(move || documents.read_again(&in_force)).await
                };
                let Ok(said) = said else {
                    break;
                };
                if !looks.reads.settled_on_another(said, &in_force) {
                    continue;
                }
                // Read again once no other change is running, whose own write
                // this may have seen before it was adopted.
                if let Err(error) = operations
                    .put_settings_in_force(|documents, in_force| {
                        let said = documents.read_again(&in_force);
                        Ok((said != in_force).then_some(said))
                    })
                    .await
                {
                    tracing::error!("could not follow the Config Documents: {error}");
                }
            }
        });
    }

    /// Puts in force the settings `read` answers, given the Config Documents
    /// and the settings in force now — it answers none where they are to stay.
    /// All of what [`Self::change_setting`] says of a change holds for this.
    async fn put_settings_in_force<R>(
        &self,
        read: R,
    ) -> Result<Option<SettingChanged>, SettingsMutationError>
    where
        R: FnOnce(
                ConfigDocuments,
                SettingsSnapshot,
            ) -> Result<Option<SettingsSnapshot>, SettingsMutationError>
            + Send
            + 'static,
    {
        let adoption = self.settings_adoption.clone();
        let sessions = self.sessions.clone();
        let providers = self.providers.clone();
        let root_discovery = self.root_discovery.clone();
        let registry_standing = self.registry_standing.clone();
        let changed = self
            .settings_adoption
            .changes
            .run(async move {
                adoption
                    .change(
                        read,
                        &sessions,
                        &providers,
                        &root_discovery,
                        &registry_standing,
                    )
                    .await
            })
            .await?;
        let Some((changed, rediscovery)) = changed else {
            return Ok(None);
        };
        if let Some(ticket) = rediscovery {
            self.root_discovery.settled(ticket).await;
        }
        Ok(Some(changed))
    }
}

/// How long after a document was modified its stamp is too soon to trust: at
/// least a tick of the coarsest modification time a document may live under, two
/// seconds on FAT, and a look's interval, so a look falls after any edit made in
/// the tick that stamp speaks for.
const STAMP_SETTLES_AFTER: Duration = Duration::from_secs(5);

/// How many looks may pass without a read, whatever the stamps say, which bounds
/// how long what no stamp tells goes unnoticed: an edit in place of the same
/// length that sets the time back, say, or a file swapped where the platform
/// gives no number to tell it by.
const LOOKS_BETWEEN_READS: u32 = 60;

/// What the Server has seen of the Config Documents in looking at them.
#[derive(Default)]
struct Looks {
    stamps: Option<DocumentStamps>,
    /// How many looks have passed since the last read.
    unread: u32,
    reads: SuccessiveReads,
}

impl Looks {
    /// Takes `stamps`, the latest look, answering whether reading the documents
    /// could say anything new: their stamps changed, or are too recent to trust,
    /// or a read has yet to be agreed with, or it has been long enough.
    fn call_for_a_read(&mut self, stamps: DocumentStamps, now: SystemTime) -> bool {
        self.unread += 1;
        let read = self.stamps.as_ref() != Some(&stamps)
            || stamps.modified_within(STAMP_SETTLES_AFTER, now)
            || self.reads.awaits_another()
            || self.unread >= LOOKS_BETWEEN_READS;
        self.stamps = Some(stamps);
        if read {
            self.unread = 0;
        }
        read
    }
}

/// What the Config Documents were last read to say, for what is read to be
/// believed only once two reads running agree on it.
#[derive(Default)]
struct SuccessiveReads(Option<SettingsSnapshot>);

impl SuccessiveReads {
    /// Whether a read has said something else and no later one has agreed.
    fn awaits_another(&self) -> bool {
        self.0.is_some()
    }

    /// Takes `said`, the latest read, answering whether it is what the read
    /// before it said too, and differs from what is `in_force`.
    fn settled_on_another(&mut self, said: SettingsSnapshot, in_force: &SettingsSnapshot) -> bool {
        if said == *in_force {
            self.0 = None;
            return false;
        }
        if self.0.as_ref() == Some(&said) {
            self.0 = None;
            return true;
        }
        self.0 = Some(said);
        false
    }
}

impl SettingsAdoption {
    /// One change, from what `read` makes of the document through the Approval
    /// Postures it reconciles and the discovery it begins of Workspace Roots
    /// it changed, which it answers beside what changed without waiting on
    /// it; nothing, where `read` answers there is nothing to put in force.
    async fn change<R>(
        &self,
        read: R,
        sessions: &SessionStore,
        providers: &ProviderOrchestrator,
        root_discovery: &RootDiscovery,
        registry_standing: &RegistryStanding,
    ) -> Result<Option<(SettingChanged, Option<Ticket>)>, SettingsMutationError>
    where
        R: FnOnce(
                ConfigDocuments,
                SettingsSnapshot,
            ) -> Result<Option<SettingsSnapshot>, SettingsMutationError>
            + Send
            + 'static,
    {
        // The Roots are held still from before the ones in force are read
        // until those this change puts in force have taken away what they
        // no longer hold, so the step at start never decides on Roots put
        // in force meanwhile.
        let held = registry_standing.hold_roots().await;
        let before = self.snapshot.borrow().clone();
        let roots = before.settings.workspace.roots.clone();
        let named_whole_before =
            crate::settings::workspace_roots_named_whole(&before).map(<[String]>::to_vec);
        // The edit is filesystem work, and the CST handles it parses the
        // document into are not `Send`; both stay on a blocking thread, where
        // the read, the edit, and the write are one scope.
        let documents = self.documents.clone();
        let snapshot = tokio::task::spawn_blocking(move || read(documents, before))
            .await
            .expect("Config Document edit runs to completion")
            .inspect_err(|error| {
                if let SettingsMutationError::Io { .. } = error {
                    tracing::error!("Setting mutation failed: {error}");
                }
            })?;
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        // The snapshot is in force, and every Client told of it, whether or
        // not the Serving listener could follow it, so the postures it makes
        // are reconciled either way.
        let serving = self.adopt(&snapshot).await.err().map(|error| {
            tracing::error!("could not adopt Serving settings: {error:#}");
            ServingNotAdopted(error)
        });
        let changed = sessions.reconcile_approval_postures(&snapshot.settings);
        crate::server::apply_live_posture_updates(providers, changed).await;
        let rediscovery = if snapshot.settings.workspace.roots == roots {
            None
        } else {
            // The Roots taken away are let go first, so no scan of one still
            // running enters anything once what it discovered is dropped.
            let ticket = root_discovery.rediscover();
            // Only Roots the user named whole, before and after alike, say
            // that one was taken away: those a document left that could not
            // be read whole say nothing.
            if let Some(before) = &named_whole_before
                && let Some(after) = crate::settings::workspace_roots_named_whole(&snapshot)
            {
                registry_standing.roots_changed(&held, before, after).await;
            }
            Some(ticket)
        };
        drop(held);
        Ok(Some((SettingChanged { snapshot, serving }, rediscovery)))
    }
}

/// Runs changes of the Settings whole and one at a time, each on a task of the
/// Server's own.
#[derive(Clone, Default)]
struct OneChangeAtATime(Arc<Mutex<()>>);

impl OneChangeAtATime {
    /// Starts `change` at once on a task of its own, which runs it once no
    /// other change is running — every later change waiting until it has
    /// finished — and answers with its outcome. The change runs to the end
    /// whether or not the answer is awaited.
    fn run<T, F>(&self, change: F) -> impl Future<Output = T> + use<T, F>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let one_at_a_time = self.0.clone();
        let task = tokio::spawn(async move {
            let _running = one_at_a_time.lock().await;
            change.await
        });
        async move {
            task.await
                .expect("a change of the Settings runs to completion")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex as Log, time::Duration};

    use tokio::sync::oneshot;

    use super::*;

    /// What is read of the document is believed only once it says the same
    /// twice running, so a save seen half done — a file emptied for a moment —
    /// is never adopted, and what agrees with what is in force asks nothing.
    #[test]
    fn what_is_read_is_believed_only_once_two_reads_running_agree() {
        let in_force = SettingsSnapshot::default();
        let another = |theme: &str| {
            let mut said = SettingsSnapshot::default();
            said.settings.appearance.theme = theme.to_owned();
            said
        };
        let mut reads = SuccessiveReads::default();

        assert!(!reads.settled_on_another(in_force.clone(), &in_force));
        assert!(!reads.settled_on_another(another("blip"), &in_force));
        assert!(!reads.settled_on_another(another("saved"), &in_force));
        assert!(reads.settled_on_another(another("saved"), &in_force));

        assert!(!reads.settled_on_another(another("blip"), &in_force));
        assert!(
            !reads.settled_on_another(in_force.clone(), &in_force),
            "a read agreeing with what is in force forgets what came before it"
        );
        assert!(!reads.settled_on_another(another("blip"), &in_force));
    }

    /// A look reads a document only where that could say anything new: a stamp
    /// long settled and unchanged asks nothing, for up to the bound on looks
    /// between reads.
    #[test]
    fn a_look_reads_only_where_that_could_say_anything_new() {
        let root = tempfile::tempdir().expect("create a config root");
        let document = root.path().join(crate::settings::PRIMARY_CONFIG_FILE);
        let documents = ConfigDocuments::new(Some(root.path()));
        std::fs::write(&document, "{}\n").unwrap();
        let settled = documents.stamps();
        // A clock long past the document's time, so its stamp is settled.
        let later = SystemTime::now() + Duration::from_secs(3600);
        let mut looks = Looks::default();

        assert!(
            looks.call_for_a_read(settled.clone(), later),
            "the first look"
        );
        for _ in 1..LOOKS_BETWEEN_READS {
            assert!(!looks.call_for_a_read(settled.clone(), later));
        }
        assert!(
            looks.call_for_a_read(settled.clone(), later),
            "however settled, it is read every so often"
        );
        assert!(!looks.call_for_a_read(settled.clone(), later));

        assert!(
            looks.call_for_a_read(settled.clone(), SystemTime::now()),
            "a stamp modified a moment ago is too soon to trust"
        );
        assert!(!looks.call_for_a_read(settled.clone(), later));

        looks.reads.settled_on_another(
            {
                let mut said = SettingsSnapshot::default();
                said.settings.appearance.theme = "saved".to_owned();
                said
            },
            &SettingsSnapshot::default(),
        );
        assert!(
            looks.call_for_a_read(settled.clone(), later),
            "a read yet to be agreed with is read again, however settled the stamp"
        );
        looks
            .reads
            .settled_on_another(SettingsSnapshot::default(), &SettingsSnapshot::default());
        assert!(!looks.call_for_a_read(settled.clone(), later));

        std::fs::write(&document, "{  }\n").unwrap();
        assert!(
            looks.call_for_a_read(documents.stamps(), later),
            "a stamp that changed"
        );
    }

    /// `awaited`, failing the test rather than hanging it where a change never
    /// gets as far as it should.
    async fn within<T>(awaited: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(10), awaited)
            .await
            .expect("the change gets this far")
    }

    /// A change that says when it starts, then waits to be released, and
    /// writes down when it begins and ends.
    fn held_change(
        name: &'static str,
        log: &Arc<Log<Vec<String>>>,
    ) -> (
        impl Future<Output = &'static str> + Send + 'static,
        oneshot::Receiver<()>,
        oneshot::Sender<()>,
    ) {
        let (started, started_rx) = oneshot::channel();
        let (release, released) = oneshot::channel::<()>();
        let log = log.clone();
        let change = async move {
            log.lock().expect("log").push(format!("{name} begins"));
            started.send(()).ok();
            released.await.ok();
            log.lock().expect("log").push(format!("{name} ends"));
            name
        };
        (change, started_rx, release)
    }

    /// A caller that stops waiting partway through its change — a harness
    /// hanging up — leaves the change to run to the end, rather than leaving
    /// it written but never put in force.
    #[tokio::test]
    async fn a_change_runs_to_the_end_though_its_caller_stops_waiting() {
        let log = Arc::new(Log::new(Vec::new()));
        let changes = OneChangeAtATime::default();
        let (change, started, release) = held_change("abandoned", &log);
        let answer = changes.run(change);
        within(started)
            .await
            .expect("the change starts though no one awaits it");
        drop(answer);
        release.send(()).expect("the change is still running");

        let (after, after_started, after_release) = held_change("after", &log);
        let after = changes.run(after);
        within(after_started).await.expect("a later change starts");
        after_release.send(()).expect("release the later change");
        assert_eq!(within(after).await, "after");
        assert_eq!(
            *log.lock().expect("log"),
            [
                "abandoned begins",
                "abandoned ends",
                "after begins",
                "after ends"
            ],
            "the abandoned change ran whole before the next began"
        );
    }

    /// A change asked for while another is running waits until that one has
    /// finished whole, so no change is adopted out of the order it was
    /// written in.
    #[tokio::test]
    async fn a_change_waits_until_the_one_running_has_finished_whole() {
        let log = Arc::new(Log::new(Vec::new()));
        let changes = OneChangeAtATime::default();
        let (first, first_started, first_release) = held_change("first", &log);
        let first = changes.run(first);
        within(first_started)
            .await
            .expect("the first change starts");
        let (second, mut second_started, second_release) = held_change("second", &log);
        let second = changes.run(second);
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            second_started.try_recv(),
            Err(oneshot::error::TryRecvError::Empty),
            "the second change waits while the first runs"
        );
        assert_eq!(*log.lock().expect("log"), ["first begins"]);

        first_release.send(()).expect("release the first change");
        assert_eq!(within(first).await, "first");
        within(second_started)
            .await
            .expect("the second change starts once the first ends");
        second_release.send(()).expect("release the second change");
        assert_eq!(within(second).await, "second");
        assert_eq!(
            *log.lock().expect("log"),
            ["first begins", "first ends", "second begins", "second ends"]
        );
    }
}
