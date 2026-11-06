//! Starting a Channel's Server when its user logs in (ADR 0063).
//!
//! `suru server install` registers, for one Channel, a Login start with the
//! machine's own service manager: a LaunchAgent on macOS, a systemd user unit
//! on Linux, a Scheduled Task at logon on Windows. What each is told to run is
//! the same command, the Channel's Keeper (`managed_client::keep_server`), so
//! nothing below knows anything of Servers; and each is told to run it once,
//! with no restart policy of its own, so nothing a service manager does can
//! undo a Manual stop.
//!
//! Registering and removing touch only what comes into force at the next
//! login: never the Keeper or Server running now. launchd reads a LaunchAgent's
//! file at login by itself, so macOS runs no command at all, and `launchctl
//! bootout`, which would end the running job, never comes up; Linux enables and
//! disables its unit without `--now`; and on Windows the Server is detached
//! from the Scheduled Task that started its Keeper.
//!
//! What is generated for each platform is a pure function here, tested on every
//! platform, and what is run is behind [`ServiceManagerCommands`], so no test
//! comes near the real user's launchd, systemd or Task Scheduler.

use std::{
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};

use crate::{
    IdentityStoreChoice, logging,
    managed_client::{KeeperTimings, ManagedClientConfig, keep_server},
};

mod launchd;
mod systemd;
mod task_scheduler;

/// The hidden command a registration runs: the Channel's Keeper.
pub const KEEPER_COMMAND: &str = "__keep-server";

/// What of the environment a Login start carries for every Provider alike: the
/// `PATH` each is found by, and where any of them may read its configuration
/// from. Each Provider declares the rest of what it needs carried
/// ([`ProviderEnvironment`](crate::provider::ProviderEnvironment)). All of it
/// names a place and holds no secret, and it is all a registration captures: a
/// file under the user's home is no place for the rest of a shell's
/// environment.
const CARRIED_FOR_EVERY_PROVIDER: [&str; 2] = ["PATH", "XDG_CONFIG_HOME"];

/// A stand-in for the machine's service manager, for tests that must not reach
/// the real one. A flag rather than an empty `PATH` because Windows looks in its
/// system directory for a program whatever `PATH` says.
#[derive(Clone, Debug, Default, Eq, PartialEq, clap::Args)]
pub struct ServiceManagerStandIn {
    #[arg(long = "service-manager", hide = true)]
    pub program: Option<PathBuf>,
}

/// A Keeper's command line: every path, and all that its Server's environment
/// is to carry, since a service manager gives a process almost none of what a
/// shell does.
///
/// Every registration on every machine holds this, in a file or Task no later
/// build rewrites, until `suru server install` is run again. So these spellings
/// are a persisted interface: a build renaming one would fail at the next login,
/// silently, while `suru server status` said registered. Accept an old spelling
/// as an alias, and never remove one. The test
/// `a_keepers_flags_are_spelled_as_registrations_have_them` holds them still.
#[derive(Clone, Debug, Eq, PartialEq, clap::Args)]
pub struct KeeperArguments {
    /// The binary each Server is launched from, which is the path the
    /// registration names rather than what this process resolves to, so an
    /// upgrade at that path is what the next Server runs.
    #[arg(long)]
    pub executable: PathBuf,
    #[arg(long = "state-dir")]
    pub state_base_dir: PathBuf,
    #[arg(long = "data-dir")]
    pub data_base_dir: PathBuf,
    #[arg(long = "config-dir")]
    pub config_dir: Option<PathBuf>,
    #[arg(long)]
    pub channel: String,
    #[arg(long)]
    pub identity_store: Option<IdentityStoreChoice>,
    /// A variable each Server is launched with, as `NAME=value`.
    #[arg(long = "env", value_parser = carried_variable)]
    pub carried: Vec<(String, String)>,
    #[command(flatten)]
    pub stand_in: ServiceManagerStandIn,
    /// How often this Keeper looks at its Channel, for tests that cannot wait
    /// out the default.
    #[arg(long, hide = true)]
    pub look_interval_ms: Option<u64>,
}

fn carried_variable(value: &str) -> Result<(String, String), String> {
    value
        .split_once('=')
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .ok_or_else(|| format!("{value:?} is not NAME=value"))
}

impl KeeperArguments {
    /// These arguments as a command line, beginning with the command, that
    /// [`clap`] reads back as they are.
    pub fn words(&self) -> Result<Vec<String>> {
        let mut words = vec![
            KEEPER_COMMAND.to_owned(),
            "--executable".to_owned(),
            unicode(&self.executable)?,
            "--state-dir".to_owned(),
            unicode(&self.state_base_dir)?,
            "--data-dir".to_owned(),
            unicode(&self.data_base_dir)?,
            "--channel".to_owned(),
            self.channel.clone(),
        ];
        if let Some(config_dir) = &self.config_dir {
            words.push("--config-dir".to_owned());
            words.push(unicode(config_dir)?);
        }
        if let Some(store) = self.identity_store {
            words.push("--identity-store".to_owned());
            words.push(store.to_string());
        }
        if let Some(stand_in) = &self.stand_in.program {
            words.push("--service-manager".to_owned());
            words.push(unicode(stand_in)?);
        }
        if let Some(interval_ms) = self.look_interval_ms {
            words.push("--look-interval-ms".to_owned());
            words.push(interval_ms.to_string());
        }
        for (name, value) in &self.carried {
            words.push("--env".to_owned());
            words.push(format!("{name}={value}"));
        }
        Ok(words)
    }

    /// Runs this Keeper, for as long as its Channel's Login start stands.
    pub async fn keep(self) -> Result<()> {
        let_go_of_the_console();
        let mut config = ManagedClientConfig::new(self.state_base_dir, self.channel)?
            .with_data_dir(self.data_base_dir)
            .with_server_executable(self.executable);
        if let Some(config_dir) = self.config_dir {
            config = config.with_config_dir(config_dir);
        }
        if let Some(identity_store) = self.identity_store {
            config = config.with_identity_store(identity_store);
        }
        for (name, value) in self.carried {
            config = config.with_server_env(name, value);
        }
        let mut timings = KeeperTimings::default();
        if let Some(interval_ms) = self.look_interval_ms {
            timings.look_interval = Duration::from_millis(interval_ms);
        }
        // A home that is not back yet at login holds no Log for a while, which
        // is no reason to give up keeping a Server.
        let _log_guard = logging::init(config.runtime(), logging::Role::Keeper)
            .inspect_err(|error| eprintln!("Could not open a Log: {error:#}"))
            .ok();
        // One that cannot say whether it is registered goes on as if it were.
        let login_start = LoginStart::for_this_user(config.channel(), self.stand_in)
            .inspect_err(|error| {
                tracing::warn!("could not read the channel's login start: {error:#}");
            })
            .ok();
        keep_server(&config, timings, || {
            login_start
                .as_ref()
                .is_some_and(|login_start| login_start.standing() == Standing::NotRegistered)
        })
        .await
    }
}

/// A Task at logon comes with a console of its own, showing for this moment,
/// and a Keeper has nothing to say there and launches its Server with none.
///
/// Once the console is freed, the handles this process holds for it are not
/// ones to rely on: whether Windows has closed them, and whether a slot may
/// since have gone to some other file, reports differ. So they are cleared
/// rather than left to find out. A launch's `StandardHandleInheritanceGuard`
/// passes over a null standard handle, every Keeper's child is given standard
/// handles of its own, and Rust's standard streams take a null handle for a sink,
/// which is what a Keeper's stray `eprintln!` wants. Its Log is a file of its
/// own, which holds none of these.
#[cfg(windows)]
fn let_go_of_the_console() {
    use windows_sys::Win32::System::Console::{
        FreeConsole, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };

    // SAFETY: FreeConsole takes nothing, and failing changes nothing; a standard
    // handle may be set to null, and nothing else is running yet to be using one.
    unsafe {
        FreeConsole();
        for standard_handle in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            SetStdHandle(standard_handle, std::ptr::null_mut());
        }
    }
}

#[cfg(not(windows))]
fn let_go_of_the_console() {}

/// The service manager a Login start is registered with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Platform {
    Launchd,
    Systemd,
    TaskScheduler,
}

impl Platform {
    const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Launchd
        } else if cfg!(windows) {
            Self::TaskScheduler
        } else {
            Self::Systemd
        }
    }
}

/// How a command given to a service manager ended.
#[derive(Clone, Debug, Default)]
struct CommandOutcome {
    success: bool,
    /// What it printed, for looking through.
    printed: String,
    /// What it said on its way out, for the user to read.
    said: String,
}

/// The one way anything here runs a service manager's command. An error means
/// the command could not be run at all: the service manager is not there.
trait ServiceManagerCommands: Send + Sync {
    fn run(&self, program: &str, arguments: &[OsString]) -> io::Result<CommandOutcome>;
}

/// Runs the machine's own service manager, or its stand-in.
struct MachineCommands {
    stand_in: ServiceManagerStandIn,
}

impl ServiceManagerCommands for MachineCommands {
    fn run(&self, program: &str, arguments: &[OsString]) -> io::Result<CommandOutcome> {
        let program = self
            .stand_in
            .program
            .as_deref()
            .map_or_else(|| OsString::from(program), |path| path.into());
        let mut command = Command::new(program);
        command.args(arguments).stdin(std::process::Stdio::null());
        // A Keeper has let go of its console, so without this each command it
        // ran as a crash was answered would show a window of its own.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let output = command.output()?;
        Ok(CommandOutcome {
            success: output.status.success(),
            printed: String::from_utf8_lossy(&output.stdout).into_owned(),
            said: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// What a Login start is registered with: one per platform, each holding what
/// that platform's registration needs and no more.
trait ServiceManager: Send + Sync {
    /// Registers `keep` for the next login, over any earlier registration of
    /// this Channel's, and answers where it is kept.
    fn register(&self, keep: &KeepCommand) -> Result<String>;
    fn remove(&self) -> Result<Removal>;
    /// Whether this Channel's Login start is registered, as the service
    /// manager's own records have it. [`Standing::NotRegistered`] is an answer,
    /// and ends a Keeper; anything short of one is [`Standing::Unknown`].
    fn standing(&self) -> Standing;
    /// What a registration covers on this platform, and what it does not.
    fn covers(&self) -> String;
}

/// Whether a Login start is registered for a Channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Standing {
    /// Registered, kept where this says.
    Registered(String),
    NotRegistered,
    /// The service manager could not say, for this reason.
    Unknown(String),
}

impl std::fmt::Display for Standing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registered(location) => {
                write!(formatter, "Login start: registered ({location})")
            }
            Self::NotRegistered => formatter.write_str("Login start: not registered"),
            Self::Unknown(reason) => write!(formatter, "Login start: could not tell ({reason})"),
        }
    }
}

/// How a registration kept as a file stands, for `home`'s user. A file not found
/// says nothing while the home itself is not found, as one on a mount that is
/// not back yet at login is not.
fn file_standing(path: &Path, home: &Path) -> Standing {
    match (path.try_exists(), home.try_exists()) {
        (Ok(true), _) => Standing::Registered(path.display().to_string()),
        (Ok(false), Ok(true)) => Standing::NotRegistered,
        (Ok(false), Ok(false)) => Standing::Unknown(format!(
            "the home directory {} is not there",
            home.display()
        )),
        (Err(error), _) | (_, Err(error)) => {
            Standing::Unknown(format!("{}: {error}", path.display()))
        }
    }
}

/// What removing a Login start came to.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Removal {
    Removed {
        location: String,
    },
    /// Removed, though the service manager did not answer, for this reason.
    RemovedUnanswered {
        location: String,
        reason: String,
    },
    NothingRegistered,
    /// Nothing is registered and no service manager is there to hold any.
    NoServiceManager {
        reason: String,
    },
}

/// A Channel's Login start on this machine.
pub struct LoginStart {
    channel: String,
    manager: Box<dyn ServiceManager>,
}

impl LoginStart {
    /// `channel`'s Login start for the current user.
    pub fn for_this_user(channel: &str, stand_in: ServiceManagerStandIn) -> Result<Self> {
        let channel = channel.to_owned();
        let home = dirs::home_dir().context("determine the current user's home directory")?;
        let commands: Arc<dyn ServiceManagerCommands> = Arc::new(MachineCommands { stand_in });
        let manager: Box<dyn ServiceManager> = match Platform::current() {
            Platform::Launchd => Box::new(launchd::LaunchAgent {
                channel: channel.clone(),
                home,
            }),
            Platform::Systemd => {
                let config_home = std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .unwrap_or_else(|| home.join(".config"));
                Box::new(systemd::UserUnit {
                    channel: channel.clone(),
                    home,
                    config_home,
                    commands,
                })
            }
            Platform::TaskScheduler => {
                let user =
                    std::env::var("USERNAME").context("read the current user from USERNAME")?;
                let user = match std::env::var("USERDOMAIN") {
                    Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
                    _ => user,
                };
                Box::new(task_scheduler::ScheduledTask {
                    channel: channel.clone(),
                    user,
                    home,
                    commands,
                })
            }
        };
        Ok(Self { channel, manager })
    }

    /// Whether this Channel's Login start is registered.
    pub fn standing(&self) -> Standing {
        self.manager.standing()
    }
}

/// The command a registration runs, and every service manager is told it in
/// its own quoting. It carries all a Keeper needs on its command line, rather
/// than in an environment a service manager would have to be told in three
/// ways, so a Channel's registration is whole in itself and a Scheduled Task,
/// which has no way to set an environment, is told it as the others are.
#[derive(Clone, Debug, Eq, PartialEq)]
struct KeepCommand {
    program: String,
    arguments: Vec<String>,
}

impl KeepCommand {
    fn new(keeper: &KeeperArguments) -> Result<Self> {
        let arguments = keeper.words()?;
        // No service manager's format can carry a control character whole, and
        // none belongs in a path a user would keep a Channel at.
        if let Some(argument) = arguments
            .iter()
            .find(|argument| argument.chars().any(char::is_control))
        {
            bail!(
                "cannot register a Login start naming {argument:?}: it holds a control character"
            );
        }
        // The program is the binary a Server is launched from, so what a
        // service manager runs and what a Keeper launches stay one path.
        Ok(Self {
            program: unicode(&keeper.executable)?,
            arguments,
        })
    }

    /// The program and arguments, in order, as a service manager is to run them.
    fn words(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.program.as_str()).chain(self.arguments.iter().map(String::as_str))
    }
}

fn unicode(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).with_context(|| {
        format!("cannot register a Login start naming {path:?}: it is not valid Unicode")
    })
}

/// Why a path a registration would name may not outlast an upgrade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShortLived {
    Build,
    Versioned,
    Temporary,
}

impl ShortLived {
    fn says(self) -> &'static str {
        match self {
            Self::Build => "it is in a build directory",
            Self::Versioned => "it is in a directory named for one version",
            Self::Temporary => "it is in a temporary directory",
        }
    }
}

/// Why `path` may not outlast an upgrade, if there is a reason to think so.
/// Only ever a reason to warn: a path that looks short-lived may be the one the
/// user means to keep.
fn short_lived(path: &Path, temporary: &Path) -> Option<ShortLived> {
    let is_named = |names: &[&str]| {
        path.components()
            .any(|component| names.iter().any(|name| component.as_os_str() == *name))
    };
    let profile_directory = path.parent().filter(|parent| {
        parent
            .file_name()
            .is_some_and(|name| name == "debug" || name == "release")
    });
    // Cargo marks the top of every target directory, however it was named.
    let built = profile_directory.is_some_and(|profile| {
        is_named(&["target"])
            || profile
                .parent()
                .is_some_and(|target| target.join("CACHEDIR.TAG").is_file())
    });
    if built {
        return Some(ShortLived::Build);
    }
    let temporary_spellings = [
        Some(temporary.to_path_buf()),
        crate::paths::canonical(temporary).ok(),
    ];
    if temporary_spellings
        .iter()
        .flatten()
        .any(|temporary| path.starts_with(temporary))
    {
        return Some(ShortLived::Temporary);
    }
    let versioned = path.components().any(|component| {
        component.as_os_str().to_str().is_some_and(|name| {
            let digits = name.strip_prefix('v').unwrap_or(name);
            digits.starts_with(|first: char| first.is_ascii_digit()) && digits.contains('.')
        })
    });
    if versioned || is_named(&["Cellar", "Caskroom", "nix"]) {
        return Some(ShortLived::Versioned);
    }
    None
}

/// The path a registration names for the binary at `current`, and why it may
/// not outlast an upgrade where it may not.
///
/// What this process knows itself as may be the far end of a link: on Linux
/// `current_exe` resolves through them, so Homebrew's `bin/suru` reads as a
/// Cellar path naming one version, which the next upgrade removes while the
/// link goes on naming whichever is current. So where `current` looks
/// short-lived, a `PATH` entry that leads to this same binary and does not is
/// named instead.
fn registered_executable(
    current: &Path,
    path_variable: Option<&OsStr>,
    temporary: &Path,
) -> (PathBuf, Option<ShortLived>) {
    let Some(why) = short_lived(current, temporary) else {
        return (current.to_path_buf(), None);
    };
    let (Some(name), Ok(binary), Some(path_variable)) = (
        current.file_name(),
        crate::paths::canonical(current),
        path_variable,
    ) else {
        return (current.to_path_buf(), Some(why));
    };
    let lasting = std::env::split_paths(path_variable)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(name))
        .find(|candidate| {
            crate::paths::canonical(candidate).is_ok_and(|resolved| resolved == binary)
                && short_lived(candidate, temporary).is_none()
        });
    match lasting {
        Some(lasting) => (lasting, None),
        None => (current.to_path_buf(), Some(why)),
    }
}

/// What of the variables `variable` reads a registration carries, of those every
/// Provider needs and those each of `providers` declares.
fn carried_environment(
    providers: &[crate::provider::ProviderEnvironment],
    variable: impl Fn(&str) -> Option<OsString>,
) -> Result<Vec<(String, String)>> {
    CARRIED_FOR_EVERY_PROVIDER
        .iter()
        .chain(providers.iter().flat_map(|provider| provider.carried))
        .filter_map(|name| Some((*name, variable(name).filter(|value| !value.is_empty())?)))
        .map(|(name, value)| {
            let value = value
                .into_string()
                .map_err(|_| anyhow::anyhow!("cannot carry {name}: it is not valid Unicode"))?;
            Ok((name.to_owned(), value))
        })
        .collect()
}

/// `suru server install`: registers the Login start of `config`'s Channel, and
/// answers what to tell the user.
pub fn install(config: &ManagedClientConfig, stand_in: ServiceManagerStandIn) -> Result<String> {
    let login_start = LoginStart::for_this_user(config.channel(), stand_in.clone())?;
    let current = std::env::current_exe().context("find current Suru executable")?;
    let path_variable = std::env::var_os("PATH");
    let (executable, short_lived) =
        registered_executable(&current, path_variable.as_deref(), &std::env::temp_dir());
    let providers = crate::provider::built_in_environments();
    let runtime = config.runtime();
    let keep = KeepCommand::new(&KeeperArguments {
        executable: executable.clone(),
        state_base_dir: std::path::absolute(runtime.state_base_dir())?,
        data_base_dir: std::path::absolute(runtime.data_base_dir())?,
        config_dir: runtime.config_dir().map(std::path::absolute).transpose()?,
        channel: runtime.channel().to_owned(),
        identity_store: runtime.identity_store(),
        carried: carried_environment(&providers, |name| std::env::var_os(name))?,
        stand_in,
        look_interval_ms: None,
    })?;
    let location = login_start.manager.register(&keep)?;
    Ok(install_report(
        &login_start,
        &location,
        &executable,
        short_lived,
        &uncarried_variables(&providers, std::env::vars_os().map(|(name, _)| name)),
    ))
}

fn uncarried_variables(
    _providers: &[crate::provider::ProviderEnvironment],
    names: impl Iterator<Item = OsString>,
) -> Vec<String> {
    const UNCARRIED: [&str; 10] = [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "COPILOT_GITHUB_TOKEN",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
    ];
    names
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| UNCARRIED.contains(&name.as_str()))
        .collect()
}

fn install_report(
    login_start: &LoginStart,
    location: &str,
    executable: &Path,
    short_lived: Option<ShortLived>,
    uncarried: &[String],
) -> String {
    let channel = &login_start.channel;
    let mut report = format!(
        "Registered a Login start for the `{channel}` Channel's Server: {location}\n\n\
         It comes into force at the next login and changes nothing until then: a Server \
         running now is left running, and `suru server start` starts one where none is.\n\
         At each login a Keeper starts the Server and restarts it if it crashes. A Server \
         stopped by `suru server stop` stays stopped until the next login or `suru server \
         start`.\n\n{}\n\n\
         Providers are found on the PATH this command ran with, which the registration \
         keeps. Run `suru server install` again after it changes.\n",
        login_start.manager.covers()
    );
    if !uncarried.is_empty() {
        report.push_str(&format!(
            "Not carried, because they may hold secrets: {}. A Server started at login does \
             not have them, so a Provider or Relay relying on one needs another way in.\n",
            uncarried.join(", ")
        ));
    }
    if let Some(short_lived) = short_lived {
        report.push_str(&format!(
            "\nWarning: this registration names {}, which may not survive an upgrade: {}. \
             Run `suru server install` again from wherever Suru is kept for good.\n",
            executable.display(),
            short_lived.says()
        ));
    }
    report
}

/// `suru server uninstall`: removes the Login start of `channel`, and answers
/// what to tell the user.
pub fn uninstall(channel: &str, stand_in: ServiceManagerStandIn) -> Result<String> {
    let login_start = LoginStart::for_this_user(channel, stand_in)?;
    Ok(uninstall_report(channel, &login_start.manager.remove()?))
}

fn uninstall_report(channel: &str, removal: &Removal) -> String {
    const LEFT_ALONE: &str = "A Server running now is left running; `suru server stop` stops it.";
    match removal {
        Removal::Removed { location } => format!(
            "Removed the Login start for the `{channel}` Channel's Server: {location}\n\
             {LEFT_ALONE}\n"
        ),
        Removal::RemovedUnanswered { location, reason } => format!(
            "Removed the Login start for the `{channel}` Channel's Server: {location}\n\
             The service manager did not answer, so it reads this at its next start: {reason}\n\
             {LEFT_ALONE}\n"
        ),
        Removal::NothingRegistered => format!(
            "No Login start is registered for the `{channel}` Channel, so there is nothing \
             to remove.\n"
        ),
        Removal::NoServiceManager { reason } => format!(
            "No Login start is registered for the `{channel}` Channel, and this machine has \
             no service manager to hold one: {reason}\n"
        ),
    }
}

/// Writes a file a registration keeps: readable by its owner alone, and replaced
/// whole, so a registration is never read half written. launchd accepts a
/// LaunchAgent only if nobody else can write to it, and systemd asks no more.
fn write_registration(path: &Path, contents: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .with_context(|| format!("{path:?} lies in no directory"))?;
    std::fs::create_dir_all(directory).with_context(|| format!("create {directory:?}"))?;
    crate::runtime::replace_private_file(path, contents)
}

/// Removes `path`, answering whether there was anything to remove.
fn remove_file(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("remove {path:?}")),
    }
}

/// `text` as XML character data, as a plist and a Task definition carry it.
fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod test_support {
    use std::sync::Mutex;

    use super::*;

    /// A service manager that records what it is told and answers as it is set
    /// to. Nothing a test does goes further than this.
    #[derive(Clone, Default)]
    pub(super) struct RecordingCommands {
        pub(super) ran: Arc<Mutex<Vec<Vec<String>>>>,
        /// The service manager is not there at all.
        pub(super) missing: Arc<Mutex<bool>>,
        /// Commands holding any of these words fail.
        pub(super) refusing: Arc<Mutex<Vec<&'static str>>>,
        /// What a command that succeeds prints.
        pub(super) printing: Arc<Mutex<String>>,
    }

    impl RecordingCommands {
        pub(super) fn ran(&self) -> Vec<Vec<String>> {
            self.ran.lock().unwrap().clone()
        }

        pub(super) fn refuse(&self, word: &'static str) {
            self.refusing.lock().unwrap().push(word);
        }

        pub(super) fn print(&self, printed: &str) {
            printed.clone_into(&mut self.printing.lock().unwrap());
        }

        pub(super) fn go_missing(&self) {
            *self.missing.lock().unwrap() = true;
        }

        pub(super) fn shared(&self) -> Arc<dyn ServiceManagerCommands> {
            Arc::new(self.clone())
        }
    }

    impl ServiceManagerCommands for RecordingCommands {
        fn run(&self, program: &str, arguments: &[OsString]) -> io::Result<CommandOutcome> {
            if *self.missing.lock().unwrap() {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no such program"));
            }
            let mut words = vec![program.to_owned()];
            words.extend(
                arguments
                    .iter()
                    .map(|word| word.to_string_lossy().into_owned()),
            );
            let success = !self
                .refusing
                .lock()
                .unwrap()
                .iter()
                .any(|refused| words.iter().any(|word| word == refused));
            self.ran.lock().unwrap().push(words);
            Ok(CommandOutcome {
                success,
                printed: self.printing.lock().unwrap().clone(),
                said: if success {
                    String::new()
                } else {
                    "refused".to_owned()
                },
            })
        }
    }

    /// A Keeper command with the awkward spellings every format must carry.
    pub(super) fn awkward_keep() -> KeepCommand {
        KeepCommand {
            program: "/Users/jo smith/R&D <x>/suru".to_owned(),
            arguments: vec![
                KEEPER_COMMAND.to_owned(),
                "--state-dir".to_owned(),
                "/Users/jo smith/100%/$HOME/\"quoted\"\\state".to_owned(),
                "--env".to_owned(),
                "PATH=/a b/bin:/usr/bin".to_owned(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::provider::ProviderEnvironment;

    #[derive(Parser)]
    struct ReadBack {
        #[command(subcommand)]
        command: ReadBackCommand,
    }

    #[derive(clap::Subcommand)]
    enum ReadBackCommand {
        #[command(name = "__keep-server")]
        Keeper(KeeperArguments),
    }

    fn every_argument() -> KeeperArguments {
        KeeperArguments {
            executable: PathBuf::from("/opt/suru"),
            state_base_dir: PathBuf::from("/state dir"),
            data_base_dir: PathBuf::from("/data"),
            config_dir: Some(PathBuf::from("/config")),
            channel: "beta".to_owned(),
            identity_store: Some(IdentityStoreChoice::File),
            carried: vec![
                ("PATH".to_owned(), "/a=b/bin".to_owned()),
                ("CODEX_HOME".to_owned(), "/codex".to_owned()),
            ],
            stand_in: ServiceManagerStandIn {
                program: Some(PathBuf::from("/stand-in")),
            },
            look_interval_ms: None,
        }
    }

    /// What a Keeper is registered with, the parser reads back as it was.
    #[test]
    fn a_keepers_command_line_reads_back_as_it_was_written() {
        let arguments = every_argument();
        let words = arguments.words().unwrap();
        let ReadBackCommand::Keeper(read) = ReadBack::try_parse_from(
            std::iter::once("suru").chain(words.iter().map(String::as_str)),
        )
        .unwrap()
        .command;
        assert_eq!(read, arguments);
    }

    /// These are on disk in every registration an earlier build made, read by
    /// whatever build is at the path at the next login, and a rename fails there
    /// silently. Don't change this list: accept a new spelling as an alias.
    #[test]
    fn a_keepers_flags_are_spelled_as_registrations_have_them() {
        assert_eq!(
            every_argument().words().unwrap(),
            [
                "__keep-server",
                "--executable",
                "/opt/suru",
                "--state-dir",
                "/state dir",
                "--data-dir",
                "/data",
                "--channel",
                "beta",
                "--config-dir",
                "/config",
                "--identity-store",
                "file",
                "--service-manager",
                "/stand-in",
                "--env",
                "PATH=/a=b/bin",
                "--env",
                "CODEX_HOME=/codex",
            ]
        );
    }

    #[test]
    fn a_registration_runs_the_binary_its_keeper_launches() {
        let keep = KeepCommand::new(&every_argument()).unwrap();
        let words = keep.words().collect::<Vec<_>>();
        assert_eq!(words[0], "/opt/suru");
        assert_eq!(words[1], KEEPER_COMMAND);
        assert_eq!(words[3], "/opt/suru");
    }

    #[test]
    fn a_keep_command_with_a_control_character_is_refused_rather_than_mangled() {
        let mut arguments = every_argument();
        arguments.state_base_dir = PathBuf::from("/su\nru");
        let error = KeepCommand::new(&arguments).unwrap_err();
        assert!(error.to_string().contains("control character"), "{error:#}");
    }

    const CODEX_LIKE: ProviderEnvironment = ProviderEnvironment {
        carried: &["SURU_CODEX_PATH", "CODEX_HOME"],
        uncarried_prefixes: &["OPENAI_"],
    };

    /// The secrets a shell holds go nowhere near a registration: only what
    /// names a place is carried.
    #[test]
    fn only_what_finds_a_provider_is_carried_and_never_a_secret() {
        let shell = |name: &str| {
            let value = match name {
                "PATH" => "/opt/bin",
                "SURU_CODEX_PATH" => "/opt/codex",
                "CODEX_HOME" => "",
                "ANTHROPIC_API_KEY" => "secret",
                "AWS_SECRET_ACCESS_KEY" => "secret",
                "HTTPS_PROXY" => "http://me:secret@proxy",
                _ => return None,
            };
            Some(OsString::from(value))
        };
        assert_eq!(
            carried_environment(&[CODEX_LIKE], shell).unwrap(),
            [
                ("PATH".to_owned(), "/opt/bin".to_owned()),
                ("SURU_CODEX_PATH".to_owned(), "/opt/codex".to_owned()),
            ],
        );
    }

    /// A Provider is carried what it declares: with none declared here, its
    /// variables go no further than the shell.
    #[test]
    fn what_each_provider_declares_is_what_is_carried_for_it() {
        let shell = |name: &str| (name == "SURU_CODEX_PATH").then(|| OsString::from("/codex"));
        assert!(carried_environment(&[], shell).unwrap().is_empty());
        assert_eq!(carried_environment(&[CODEX_LIKE], shell).unwrap().len(), 1);
    }

    fn rooted(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        } else {
            PathBuf::from(path)
        }
    }

    #[test]
    fn paths_that_will_not_outlast_an_upgrade_are_told_from_those_that_will() {
        let temporary = rooted("/scratch/tmp");
        for (path, expected) in [
            ("/home/jo/.local/bin/suru", None),
            ("/usr/local/bin/suru", None),
            ("/opt/homebrew/bin/suru", None),
            ("/home/jo/python3.12/suru", None),
            ("/home/jo/suru/target/debug/suru", Some(ShortLived::Build)),
            ("/home/jo/suru/target/release/suru", Some(ShortLived::Build)),
            (
                "/home/jo/suru/target/x86_64-unknown-linux-gnu/release/suru",
                Some(ShortLived::Build),
            ),
            (
                "/opt/homebrew/Cellar/suru/1.2.3/bin/suru",
                Some(ShortLived::Versioned),
            ),
            (
                "/opt/homebrew/Cellar/suru/HEAD/bin/suru",
                Some(ShortLived::Versioned),
            ),
            (
                "/home/jo/.asdf/installs/suru/v0.4.1/suru",
                Some(ShortLived::Versioned),
            ),
            ("/nix/store/abc-suru/bin/suru", Some(ShortLived::Versioned)),
            ("/scratch/tmp/unpacked/suru", Some(ShortLived::Temporary)),
        ] {
            assert_eq!(short_lived(&rooted(path), &temporary), expected, "{path}");
        }
    }

    /// Cargo's target directory may go by any name, and is known by its mark.
    #[test]
    fn a_target_directory_under_another_name_is_a_build_directory() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let target = root.path().join("cargo-target");
        std::fs::create_dir_all(target.join("debug")).unwrap();
        std::fs::write(target.join("CACHEDIR.TAG"), "").unwrap();
        assert_eq!(
            short_lived(&target.join("debug").join("suru"), elsewhere.path()),
            Some(ShortLived::Build)
        );
    }

    #[test]
    fn a_lasting_path_is_named_as_it_is() {
        let temporary = rooted("/scratch/tmp");
        let current = rooted("/home/jo/.local/bin/suru");
        assert_eq!(
            registered_executable(&current, None, &temporary),
            (current, None)
        );
    }

    /// Homebrew's link is what lasts; the Cellar path it leads to is gone
    /// after the next upgrade. No `PATH` entry leading elsewhere is taken for it.
    #[cfg(unix)]
    #[test]
    fn a_link_on_path_is_named_rather_than_the_version_it_leads_to() {
        let root = tempfile::tempdir().unwrap();
        let root = crate::paths::canonical(root.path()).unwrap();
        let temporary = rooted("/scratch/tmp");
        let cellar = root.join("Cellar").join("suru").join("1.2.3").join("bin");
        let links = root.join("links");
        let other = root.join("other");
        for directory in [&cellar, &links, &other] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let binary = cellar.join("suru");
        std::fs::write(&binary, "").unwrap();
        std::fs::write(other.join("suru"), "another binary").unwrap();
        std::os::unix::fs::symlink(&binary, links.join("suru")).unwrap();

        let path_variable = std::env::join_paths([&other, &links]).unwrap();
        assert_eq!(
            registered_executable(&binary, Some(&path_variable), &temporary),
            (links.join("suru"), None)
        );

        let no_link = std::env::join_paths([&other]).unwrap();
        assert_eq!(
            registered_executable(&binary, Some(&no_link), &temporary),
            (binary, Some(ShortLived::Versioned))
        );
    }

    fn login_start() -> LoginStart {
        LoginStart {
            channel: "beta".to_owned(),
            manager: Box::new(launchd::LaunchAgent {
                channel: "beta".to_owned(),
                home: rooted("/home/jo"),
            }),
        }
    }

    #[test]
    fn the_install_report_says_what_was_done_and_warns_of_what_will_not_last() {
        let login_start = login_start();
        let executable = rooted("/home/jo/target/debug/suru");

        let report = install_report(
            &login_start,
            "somewhere",
            &executable,
            Some(ShortLived::Build),
            &["ANTHROPIC_API_KEY".to_owned()],
        );
        assert!(
            report.contains("`beta` Channel's Server: somewhere"),
            "{report}"
        );
        assert!(report.contains("ANTHROPIC_API_KEY"), "{report}");
        assert!(report.contains("Warning"), "{report}");
        assert!(report.contains("build directory"), "{report}");

        let report = install_report(&login_start, "somewhere", &executable, None, &[]);
        assert!(!report.contains("Warning"), "{report}");
        assert!(!report.contains("Not carried"), "{report}");
    }

    /// A file missing says nothing while the home it would be under is missing
    /// too, as one on a mount that is not back yet is; and a Keeper reading
    /// "not registered" ends.
    #[test]
    fn a_registration_is_not_called_gone_while_the_home_is_not_there() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("registration");
        assert_eq!(file_standing(&path, home.path()), Standing::NotRegistered);
        std::fs::write(&path, "").unwrap();
        assert!(matches!(
            file_standing(&path, home.path()),
            Standing::Registered(_)
        ));
        let gone = home.path().join("not mounted");
        assert!(matches!(
            file_standing(&gone.join("registration"), &gone),
            Standing::Unknown(_)
        ));
    }

    /// Every platform's output says what it covers, including that a Mac waits
    /// for its login keychain and what Linux needs to run with no session.
    #[test]
    fn each_platform_says_what_its_registration_covers_and_what_it_does_not() {
        let home = rooted("/home/jo");
        let commands = test_support::RecordingCommands::default();
        let mac = launchd::LaunchAgent {
            channel: "beta".to_owned(),
            home: home.clone(),
        }
        .covers();
        assert!(mac.contains("login keychain"), "{mac}");
        let linux = systemd::UserUnit {
            channel: "beta".to_owned(),
            home: home.clone(),
            config_home: home.clone(),
            commands: commands.shared(),
        }
        .covers();
        assert!(linux.contains("loginctl enable-linger"), "{linux}");
        let windows = task_scheduler::ScheduledTask {
            channel: "beta".to_owned(),
            user: "HOST\\jo".to_owned(),
            home,
            commands: commands.shared(),
        }
        .covers();
        assert!(windows.contains("logon"), "{windows}");
    }

    #[test]
    fn the_uninstall_report_says_a_running_server_was_left_alone() {
        let removed = uninstall_report(
            "beta",
            &Removal::Removed {
                location: "somewhere".to_owned(),
            },
        );
        assert!(removed.contains("left running"), "{removed}");
        let nothing = uninstall_report("beta", &Removal::NothingRegistered);
        assert!(nothing.contains("nothing to remove"), "{nothing}");
        let no_manager = uninstall_report(
            "beta",
            &Removal::NoServiceManager {
                reason: "no systemd".to_owned(),
            },
        );
        assert!(no_manager.contains("no systemd"), "{no_manager}");
    }
}
