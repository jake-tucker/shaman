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
    fmt,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use shaman_pairing::IdentityMaterial;
use tokio::{
    runtime::Runtime,
    sync::{Mutex as AsyncMutex, watch},
    task::JoinHandle,
};

mod device;
mod identity;

pub use device::ThisDevice;
use device::{DeviceRecord, Records};
pub use identity::{KeySigner, SignError};

uniffi::setup_scaffolding!();

/// The core of one thin Client, kept in one directory.
#[derive(uniffi::Object)]
pub struct ThinClient {
    /// Every act and every listener runs here, so nothing Swift calls needs
    /// a runtime of its own, and UniFFI's `tokio` feature is not used.
    runtime: Runtime,
    records: Records,
    /// What the thin Client's own file holds, held to the end of each change
    /// to it, so one change never undoes another and the last asked is kept.
    record: Arc<AsyncMutex<DeviceRecord>>,
    identity: IdentityMaterial,
    device: watch::Sender<ThisDevice>,
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
        let signer = identity::taken_up(signer).map_err(OpenError::key)?;
        let records = Records::open(PathBuf::from(directory)).map_err(OpenError::from)?;
        let identity::Reconciled { record, identity } =
            identity::reconcile(&records, signer).map_err(OpenError::from)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("shaman-core")
            .enable_all()
            .build()
            .map_err(|error| OpenError::from(anyhow::Error::from(error)))?;
        let (device, _) = watch::channel(ThisDevice::of(&record));
        Ok(Arc::new(Self {
            runtime,
            records,
            record: Arc::new(AsyncMutex::new(record)),
            identity,
            device,
        }))
    }

    /// Tells `listener` what "This device" shows now, and again on every
    /// change, until the subscription is cancelled.
    pub fn watch_device(&self, listener: Arc<dyn DeviceListener>) -> Arc<Subscription> {
        let mut device = self.device.subscribe();
        let task = self.runtime.spawn(async move {
            loop {
                let current = device.borrow_and_update().clone();
                listener.changed(current);
                if device.changed().await.is_err() {
                    break;
                }
            }
        });
        Subscription::of(task)
    }

    /// Renames this device to `name`, as the Serving side would keep it,
    /// refusing a name it would not, with why.
    pub async fn rename_device(&self, name: String) -> Result<(), ActError> {
        let name = shaman_pairing::shown_peer_name(&name).map_err(|refusal| ActError::Invalid {
            message: refusal.to_string(),
        })?;
        self.change_record(move |record| record.name = name).await
    }

    /// Says no more that this phone must pair again.
    pub async fn dismiss_pair_again(&self) -> Result<(), ActError> {
        self.change_record(|record| record.pair_again = false).await
    }
}

impl ThinClient {
    /// What proves this phone to the Servers it pairs with: what signs with
    /// its key, and the certificate of the key the core keeps.
    pub fn identity(&self) -> &IdentityMaterial {
        &self.identity
    }

    /// Changes the thin Client's own file by `change`, keeping it before
    /// "This device" is told.
    async fn change_record(
        &self,
        change: impl FnOnce(&mut DeviceRecord) + Send + 'static,
    ) -> Result<(), ActError> {
        let records = self.records.clone();
        let record = self.record.clone();
        let device = self.device.clone();
        self.runtime
            .spawn(async move {
                let mut held = record.lock().await;
                let mut changed = held.clone();
                change(&mut changed);
                let written = changed.clone();
                tokio::task::spawn_blocking(move || records.write(&written))
                    .await
                    .map_err(|error| ActError::failed(anyhow::Error::from(error)))?
                    .map_err(ActError::failed)?;
                device.send_replace(ThisDevice::of(&changed));
                *held = changed;
                Ok(())
            })
            .await
            .map_err(|error| ActError::failed(anyhow::Error::from(error)))?
    }
}

/// Hears what "This device" shows, at once and on every change.
#[uniffi::export(with_foreign)]
pub trait DeviceListener: Send + Sync {
    fn changed(&self, device: ThisDevice);
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
#[derive(Debug, uniffi::Error)]
pub enum OpenError {
    Failed { message: String },
}

impl OpenError {
    /// The key the app handed in is not one this phone can be proved by.
    fn key(error: anyhow::Error) -> Self {
        Self::Failed {
            message: format!("This phone's identity key could not be used: {error:#}"),
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
        let Self::Failed { message } = self;
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
    fn a_key_that_will_not_sign_opens_nothing_and_discards_nothing() {
        let directory = tempfile::tempdir().unwrap();
        drop(opened(directory.path(), &TestKey::new()));
        keep_records(directory.path());
        let before = std::fs::read(directory.path().join(device::RECORD_FILE)).unwrap();

        let Err(OpenError::Failed { message }) = open(directory.path(), &TestKey::refusing())
        else {
            panic!("opened over a key that will not sign");
        };
        assert!(
            message.contains("the chip will not take the key"),
            "{message}"
        );
        assert_eq!(
            std::fs::read(directory.path().join(device::RECORD_FILE)).unwrap(),
            before
        );
        assert!(directory.path().join(shaman_pairing::REMOTES_FILE).exists());
        assert!(directory.path().join("drafts.json").exists());
    }

    #[test]
    fn a_key_of_another_kind_is_said_so() {
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
        let Err(OpenError::Failed { message }) = ThinClient::open(
            directory.path().to_string_lossy().into_owned(),
            Arc::new(NotAKey),
        ) else {
            panic!("opened over no key");
        };
        assert!(
            message.starts_with("This phone's identity key could not be used"),
            "{message}"
        );
        assert!(message.contains("P-256"), "{message}");
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
    fn records_it_cannot_read_are_said_so_and_not_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(device::RECORD_FILE);
        std::fs::write(&path, b"{ not json").unwrap();
        let Err(OpenError::Failed { message }) = open(directory.path(), &TestKey::new()) else {
            panic!("opened over records it cannot read");
        };
        assert!(
            message.starts_with("This phone's records could not be read"),
            "{message}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }
}
