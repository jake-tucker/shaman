//! The thin Client's core: everything the iOS app is but its screen (ADR-0080).
//!
//! Swift makes one [`ThinClient`] from a directory in the app's container and
//! reaches everything through it. What a screen shows crosses as a finished
//! model, whole on every change, to a listener Swift hands in, which hears the
//! current value at once; an act is an async call that returns once it is
//! done, with nothing but success, what it changed arriving by the listener.
//! Everything the core keeps is in that one directory, which Swift keeps out
//! of every backup.
//!
//! This is the one crate that knows of UniFFI. What it hands Swift carries no
//! compatibility promise: the app and its core are always built together.

use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tokio::{
    runtime::Runtime,
    sync::{Mutex as AsyncMutex, watch},
    task::JoinHandle,
};

mod device;

pub use device::ThisDevice;
use device::{DeviceRecord, Records};

uniffi::setup_scaffolding!();

/// The core of one thin Client, kept in one directory.
#[derive(uniffi::Object)]
pub struct ThinClient {
    /// Every act and every listener runs here, so nothing Swift calls needs
    /// a runtime of its own, and UniFFI's `tokio` feature is not used.
    runtime: Runtime,
    records: Records,
    /// Holds one rename at a time to the end of its write, so the last asked
    /// is the one kept.
    renaming: Arc<AsyncMutex<()>>,
    device: watch::Sender<ThisDevice>,
}

#[uniffi::export]
impl ThinClient {
    /// The core kept in `directory`, made where it is absent.
    #[uniffi::constructor]
    pub fn open(directory: String) -> Result<Arc<Self>, OpenError> {
        let records = Records::open(PathBuf::from(directory)).map_err(OpenError::from)?;
        let record = records.read().map_err(OpenError::from)?;
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
            renaming: Arc::default(),
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
        let records = self.records.clone();
        let renaming = self.renaming.clone();
        let device = self.device.clone();
        self.runtime
            .spawn(async move {
                let _held = renaming.lock().await;
                let record = DeviceRecord { name };
                let written = record.clone();
                let records = records.clone();
                tokio::task::spawn_blocking(move || records.write(&written))
                    .await
                    .map_err(|error| ActError::failed(anyhow::Error::from(error)))?
                    .map_err(ActError::failed)?;
                device.send_replace(ThisDevice::of(&record));
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
    use std::{sync::mpsc, time::Duration};

    use super::*;

    /// How long a test waits for the listener to hear something it must.
    const HEARD_WITHIN: Duration = Duration::from_secs(5);

    fn opened(directory: &std::path::Path) -> Arc<ThinClient> {
        ThinClient::open(directory.to_string_lossy().into_owned()).expect("open the core")
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

    #[test]
    fn a_new_phone_is_first_offered_as_iphone_with_the_protocol_it_speaks() {
        let directory = tempfile::tempdir().unwrap();
        let core = opened(&directory.path().join("core"));
        let (_subscription, heard) = listening(&core);
        let device = heard.recv_timeout(HEARD_WITHIN).expect("heard at once");
        assert_eq!(device.name, "iPhone");
        assert_eq!(device.protocol_version, shaman_protocol::PROTOCOL_VERSION);
        assert!(
            device.name_note.contains("paired from now on"),
            "{}",
            device.name_note
        );
    }

    #[test]
    fn a_rename_is_heard_and_kept_from_one_run_to_the_next() {
        let directory = tempfile::tempdir().unwrap();
        let core = opened(directory.path());
        let (_subscription, heard) = listening(&core);
        assert_eq!(heard.recv_timeout(HEARD_WITHIN).unwrap().name, "iPhone");

        act(core.rename_device("  Pocket phone ".to_owned())).expect("renamed");
        assert_eq!(
            heard.recv_timeout(HEARD_WITHIN).unwrap().name,
            "Pocket phone",
            "kept as the Serving side would keep it"
        );
        drop(core);

        let reopened = opened(directory.path());
        let (_subscription, heard) = listening(&reopened);
        assert_eq!(
            heard.recv_timeout(HEARD_WITHIN).unwrap().name,
            "Pocket phone"
        );
    }

    #[test]
    fn a_name_the_serving_side_would_not_keep_is_refused_in_its_words_and_changes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let core = opened(directory.path());
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
        let reopened = opened(directory.path());
        let (_subscription, heard) = listening(&reopened);
        assert_eq!(heard.recv_timeout(HEARD_WITHIN).unwrap().name, "iPhone");
    }

    #[test]
    fn a_cancelled_listener_hears_no_more() {
        let directory = tempfile::tempdir().unwrap();
        let core = opened(directory.path());
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
        let Err(OpenError::Failed { message }) =
            ThinClient::open(directory.path().to_string_lossy().into_owned())
        else {
            panic!("opened over records it cannot read");
        };
        assert!(
            message.starts_with("This phone's records could not be read"),
            "{message}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }
}
