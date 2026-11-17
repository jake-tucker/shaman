use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(windows)]
pub(crate) use shaman_private_files::CurrentWindowsUser;
pub(crate) use shaman_private_files::{
    protect_current_user_directory, protect_current_user_file, replace_private_file,
};

use crate::serving::IdentityStoreChoice;

const RUNTIME_FILE: &str = "runtime.json";
const LOCK_FILE: &str = "server.lock";
const LAST_STOP_FILE: &str = "last-stop.json";

#[derive(Clone, Debug)]
struct Channel(String);

impl Channel {
    fn new(channel: impl Into<String>) -> Result<Self> {
        let channel = channel.into();
        let is_safe = !channel.is_empty()
            && channel != "."
            && channel != ".."
            && channel
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !is_safe {
            bail!("channel must contain only letters, numbers, '.', '-', or '_'");
        }
        Ok(Self(channel))
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn resolve_root(&self, base_dir: &Path) -> PathBuf {
        if self.0 == "release" {
            base_dir.to_path_buf()
        } else {
            base_dir.join(&self.0)
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    state_base_dir: PathBuf,
    data_base_dir: PathBuf,
    state_dir: PathBuf,
    data_dir: PathBuf,
    config_dir: Option<PathBuf>,
    channel: Channel,
    /// Whether this process makes the state and data directories where they
    /// are missing, or only uses them as it finds them. See
    /// [`RuntimeConfig::launched_into_existing_dirs`].
    makes_dirs: bool,
    /// The Manual stop the Channel had last seen when this Server was
    /// launched, where its launcher read one. See
    /// [`RuntimeConfig::launched_after`].
    launched_after: Option<LastStop>,
    /// The store a new identity key is kept in whatever the build, where
    /// `SHAMAN_IDENTITY_STORE` names one. See
    /// [`RuntimeConfig::with_identity_store`].
    identity_store: Option<IdentityStoreChoice>,
    /// The home this Server reads `~` from and abbreviates paths to, where it
    /// is given rather than discovered. See [`RuntimeConfig::with_home_dir`].
    home_dir: Option<PathBuf>,
}

impl RuntimeConfig {
    pub fn new(state_base_dir: impl AsRef<Path>, channel: impl Into<String>) -> Result<Self> {
        let channel = Channel::new(channel)?;
        let state_base_dir = state_base_dir.as_ref().to_path_buf();
        let state_dir = channel.resolve_root(&state_base_dir);
        Ok(Self {
            state_base_dir: state_base_dir.clone(),
            data_base_dir: state_base_dir,
            data_dir: state_dir.clone(),
            state_dir,
            config_dir: None,
            channel,
            makes_dirs: true,
            launched_after: None,
            identity_store: None,
            home_dir: None,
        })
    }

    pub fn with_data_dir(mut self, data_base_dir: impl AsRef<Path>) -> Self {
        self.data_base_dir = data_base_dir.as_ref().to_path_buf();
        self.data_dir = self.channel.resolve_root(&self.data_base_dir);
        self
    }

    /// Points the server at the Config Document root. Unlike state and data,
    /// the config root is shared across Channels, so no Channel suffix is
    /// applied. Without this, no Config Documents load and every Setting
    /// keeps its built-in default.
    pub fn with_config_dir(mut self, config_dir: impl AsRef<Path>) -> Self {
        self.config_dir = Some(config_dir.as_ref().to_path_buf());
        self
    }

    /// Configures a Server that a launcher started, having made its state and
    /// data directories just before: such a Server makes neither, and ends
    /// rather than starting where either is missing.
    ///
    /// A directory missing by then was removed since the launch — the
    /// temporary directory of a test that has finished, say, or a user's
    /// state cleared away — and a Server that made it again would run on in
    /// a directory nobody expects it in, and that its launcher may already
    /// have finished with. Nothing else a Server writes makes either
    /// directory, launched or not: what it makes within them it makes only
    /// beneath a root that is still there (`paths::create_dir_beneath`), and
    /// a Server elected in a directory that is removed while it waits for the
    /// channel's lock finds that out once it holds the lock, and ends then
    /// (see `server::election`).
    pub fn launched_into_existing_dirs(mut self) -> Self {
        self.makes_dirs = false;
        self
    }

    /// Whether this process may make the state and data directories, rather
    /// than only using them as it finds them.
    pub(crate) fn makes_dirs(&self) -> bool {
        self.makes_dirs
    }

    /// Configures a Server whose launcher read `last_stop` as the Channel's
    /// last Manual stop just before launching it. A Manual stop is final for
    /// every Server launched before it, so one recorded since — however long
    /// this Server took to reach the election, and however long it then
    /// waited in it — ends this Server once it is elected, rather than
    /// letting it serve the Channel its user just stopped. Without this, a
    /// Server takes the last stop recorded as its launch is asked for, when
    /// `server::spawn` or any of its kin is called.
    pub fn launched_after(mut self, last_stop: LastStop) -> Self {
        self.launched_after = Some(last_stop);
        self
    }

    /// The Manual stop this Server's launcher saw before launching it, if it
    /// was launched having read one.
    pub(crate) fn launched_after_stop(&self) -> Option<LastStop> {
        self.launched_after
    }

    /// Configures a Server to keep a new identity key in `store`, as
    /// `SHAMAN_IDENTITY_STORE` names one, whatever the build. Without this, a
    /// release build keeps it in the platform credential store, and a debug
    /// build, or any build Cargo runs, in an owner-only file in the data
    /// directory.
    pub fn with_identity_store(mut self, store: IdentityStoreChoice) -> Self {
        self.identity_store = Some(store);
        self
    }

    /// The store a new identity key is kept in whatever the build, where one
    /// is named.
    pub(crate) fn identity_store(&self) -> Option<IdentityStoreChoice> {
        self.identity_store
    }

    /// Configures a Server to take `home_dir` as its home: the directory `~`
    /// names in a path its Clients send it, and the one it abbreviates to `~`
    /// in the paths it names Workspaces by. Without this, the Server takes
    /// the platform's home for the user it runs as.
    pub fn with_home_dir(mut self, home_dir: impl AsRef<Path>) -> Self {
        self.home_dir = Some(home_dir.as_ref().to_path_buf());
        self
    }

    /// This Server's home, given or discovered; none where the platform
    /// knows none.
    pub(crate) fn home_dir(&self) -> Option<PathBuf> {
        self.home_dir.clone().or_else(dirs::home_dir)
    }

    pub(crate) fn state_base_dir(&self) -> &Path {
        &self.state_base_dir
    }

    pub(crate) fn data_base_dir(&self) -> &Path {
        &self.data_base_dir
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn config_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref()
    }

    pub fn channel(&self) -> &str {
        self.channel.as_str()
    }

    pub fn descriptor_path(&self) -> PathBuf {
        self.state_dir.join(RUNTIME_FILE)
    }

    pub(crate) fn lock_path(&self) -> PathBuf {
        self.state_dir.join(LOCK_FILE)
    }

    /// Where the Channel's last Manual stop is recorded, beside its election
    /// lock and runtime descriptor.
    pub(crate) fn last_stop_path(&self) -> PathBuf {
        self.state_dir.join(LAST_STOP_FILE)
    }

    /// The Channel's last Manual stop as it is recorded now: none where no
    /// stop has been, and an error where the record cannot be read whole and
    /// decoded. It is only ever replaced whole, so an error is never a record
    /// caught half written.
    pub fn last_stop(&self) -> io::Result<LastStop> {
        LastStop::read(&self.last_stop_path())
    }

    /// Makes the state and data directories readable by the current user
    /// alone, making them first where they are missing — as a launcher does
    /// before it launches a Server, and an in-process Server does as it
    /// starts. A Server [launched into existing
    /// directories](RuntimeConfig::launched_into_existing_dirs) makes
    /// neither, and fails here instead where either is missing.
    pub fn create_private_runtime_dir(&self) -> Result<PathBuf> {
        let runtime_dir = &self.state_dir;
        self.ensure_dir(runtime_dir, "runtime")?;
        protect_current_user_directory(runtime_dir)?;
        self.ensure_dir(&self.data_dir, "data")?;
        protect_current_user_directory(&self.data_dir)?;
        Ok(runtime_dir.to_path_buf())
    }

    fn ensure_dir(&self, dir: &Path, role: &str) -> Result<()> {
        if self.makes_dirs {
            return fs::create_dir_all(dir)
                .with_context(|| format!("create {role} directory {dir:?}"));
        }
        match fs::metadata(dir) {
            Ok(metadata) if metadata.is_dir() => Ok(()),
            Ok(_) => bail!("{role} directory {dir:?} is not a directory"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
                "{role} directory {dir:?} is gone: a launched server uses the directories its \
                 launcher made, and never makes them again"
            ),
            Err(error) => Err(error).with_context(|| format!("inspect {role} directory {dir:?}")),
        }
    }
}

/// The Manual stop last recorded on a Channel — a `shaman server stop`, or
/// the operating system's signal answered as one — naming the Server it
/// stopped, or none where the Channel has never been stopped so.
///
/// A Manual stop is final for every Server launched before it (ADR 0002).
/// Servers that lose an election wait out the winner's hold on the
/// Channel's lock, and one still waiting when the winner is stopped would
/// otherwise take the Channel over the moment it is let go, undoing the stop
/// it was never told of. So a stopping Server records its stop here before
/// it lets the lock go, and a launch carries the last stop it saw: elected,
/// a Server finding a later stop recorded than its launch saw ends instead
/// of serving. Only a Manual stop is recorded. A Server replaced by another
/// build hands the Channel to whichever waiting Server takes it, and one
/// that no longer stands for its Channel speaks for no Channel at all.
///
/// A stop asked for with no Server there to answer it, one having just crashed
/// say, names an id of its own instead (`server::election`). So the id is what
/// tells one stop from another, and need not name a Server that ever ran.
///
/// On a launch's command line it is written `none`, or as the id the stop names.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LastStop(Option<Uuid>);

/// The record of a Manual stop, as it is written to a Channel's state
/// directory.
#[derive(Deserialize, Serialize)]
pub(crate) struct StopRecord {
    pub(crate) instance_id: Uuid,
}

impl LastStop {
    /// The last stop recorded at `path`.
    fn read(path: &Path) -> io::Result<Self> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self(None)),
            Err(error) => return Err(error),
        };
        let record: StopRecord = serde_json::from_reader(io::BufReader::new(file))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(Self(Some(record.instance_id)))
    }

    /// The Server instance this stop stopped, or none where no stop has been
    /// recorded.
    pub fn stopped_instance(&self) -> Option<Uuid> {
        self.0
    }

    /// The stop recorded in `now` that a launch that saw `self` must yield
    /// to: the instance it stopped, where `now` names a stop other than the
    /// one seen. A record gone missing since names nothing — Shaman never
    /// removes one, so it was cleared from outside along with whatever else
    /// was — and is no reason to end a launch.
    pub fn superseded_by(&self, now: LastStop) -> Option<Uuid> {
        now.0.filter(|_| now != *self)
    }
}

impl std::fmt::Display for LastStop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(instance_id) => write!(formatter, "{instance_id}"),
            None => formatter.write_str("none"),
        }
    }
}

impl std::str::FromStr for LastStop {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "none" {
            return Ok(Self(None));
        }
        value.parse().map(|instance_id| Self(Some(instance_id)))
    }
}

/// An HTTP client for this machine's own loopback listeners — a Server's API,
/// its Broker. It never goes through a proxy: the system's HTTP proxy, which
/// a Server honours on its way to a Relay, has no business with loopback.
pub(crate) fn loopback_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("a loopback HTTP client needs nothing that can fail")
}
