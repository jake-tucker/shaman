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
};

use anyhow::{Context, Result, bail};

use crate::managed_client::ManagedClientConfig;

mod launchd;
mod systemd;
mod task_scheduler;

/// The hidden command a registration runs: the Channel's Keeper.
pub const KEEPER_COMMAND: &str = "__keep-server";

/// What of the environment a Login start carries from the shell it was
/// installed from to the Server it starts, because it decides which Providers
/// that Server finds and what it finds them signed in to: the `PATH` every
/// Provider is found by, the variables that name a Provider's executable over
/// `PATH`, and those that move where a Provider keeps its sign-in. Each names a
/// place and holds no secret, and these are all a registration captures: a
/// file under the user's home is no place for the rest of a shell's
/// environment.
const CARRIED_VARIABLES: [&str; 8] = [
    "PATH",
    "SURU_CODEX_PATH",
    "SURU_COPILOT_PATH",
    "SURU_CLAUDE_PATH",
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "COPILOT_HOME",
    "XDG_CONFIG_HOME",
];

/// Variables a shell may hold that a Server started from it would use, and
/// that a registration will not carry because they hold or may hold a secret.
/// `suru server install` names those it finds set, so a Provider that signs in
/// by one, or a Relay reached through a proxy, does not fail unexplained.
const UNCARRIED_VARIABLES: [&str; 10] = [
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
#[derive(Clone, Debug)]
struct CommandOutcome {
    success: bool,
    /// What it said on its way out, for the user to read.
    said: String,
}

/// The one way anything here runs a service manager's command. An error means
/// the command could not be run at all: the service manager is not there.
trait ServiceManagerCommands: Send + Sync {
    fn run(&self, program: &str, arguments: &[OsString]) -> io::Result<CommandOutcome>;
}

/// Runs the machine's own service manager, or the stand-in `suru server`'s
/// hidden `--service-manager` names in its place.
struct MachineCommands {
    stand_in: Option<PathBuf>,
}

impl ServiceManagerCommands for MachineCommands {
    fn run(&self, program: &str, arguments: &[OsString]) -> io::Result<CommandOutcome> {
        let program = self
            .stand_in
            .as_deref()
            .map_or_else(|| OsString::from(program), |path| path.into());
        let output = Command::new(program)
            .args(arguments)
            .stdin(std::process::Stdio::null())
            .output()?;
        Ok(CommandOutcome {
            success: output.status.success(),
            said: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// Where a registration is kept and whom it is for.
#[derive(Clone, Debug)]
struct Places {
    /// The user's home, which a LaunchAgent lives under and the Keeper runs in.
    home: PathBuf,
    /// The directory systemd reads a user's own units beneath.
    config_home: PathBuf,
    /// The user a Scheduled Task is for, as `DOMAIN\user`, on Windows.
    user: Option<String>,
}

impl Places {
    fn of_this_user() -> Result<Self> {
        let home = dirs::home_dir().context("determine the current user's home directory")?;
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let user = std::env::var("USERNAME")
            .ok()
            .map(|user| match std::env::var("USERDOMAIN") {
                Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
                _ => user,
            });
        Ok(Self {
            home,
            config_home,
            user,
        })
    }
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
                write!(formatter, "Start at login: registered ({location})")
            }
            Self::NotRegistered => formatter.write_str("Start at login: not registered"),
            Self::Unknown(reason) => write!(formatter, "Start at login: could not tell ({reason})"),
        }
    }
}

/// What removing a Login start came to.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Removal {
    Removed(String),
    /// Removed, though the service manager did not answer, for this reason.
    RemovedUnanswered(String, String),
    NothingRegistered,
    /// Nothing is registered and no service manager is there to hold any.
    NoServiceManager(String),
}

/// A Channel's Login start on this machine.
pub struct LoginStart {
    platform: Platform,
    channel: String,
    places: Places,
    commands: Box<dyn ServiceManagerCommands>,
}

impl LoginStart {
    /// `channel`'s Login start for the current user. `service_manager` names a
    /// stand-in to run in place of the machine's service manager, so that a
    /// test of the command line cannot reach the real one.
    pub fn for_this_user(channel: &str, service_manager: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            platform: Platform::current(),
            channel: channel.to_owned(),
            places: Places::of_this_user()?,
            commands: Box::new(MachineCommands {
                stand_in: service_manager,
            }),
        })
    }

    /// Whether this Channel's Login start is registered, as the service
    /// manager's own records have it.
    pub fn standing(&self) -> Standing {
        match self.platform {
            Platform::Launchd => launchd::standing(self),
            Platform::Systemd => systemd::standing(self),
            Platform::TaskScheduler => task_scheduler::standing(self),
        }
    }

    /// Registers `keep` for the next login, over any earlier registration of
    /// this Channel's, and answers where it is kept.
    fn register(&self, keep: &KeepCommand) -> Result<String> {
        match self.platform {
            Platform::Launchd => launchd::register(self, keep),
            Platform::Systemd => systemd::register(self, keep),
            Platform::TaskScheduler => task_scheduler::register(self, keep),
        }
    }

    fn remove(&self) -> Result<Removal> {
        match self.platform {
            Platform::Launchd => launchd::remove(self),
            Platform::Systemd => systemd::remove(self),
            Platform::TaskScheduler => task_scheduler::remove(self),
        }
    }

    fn covers(&self) -> String {
        match self.platform {
            Platform::Launchd => launchd::COVERS.to_owned(),
            Platform::Systemd => systemd::covers(),
            Platform::TaskScheduler => task_scheduler::COVERS.to_owned(),
        }
    }
}

/// The command a registration runs, and every service manager is told it in
/// its own quoting. It carries all a Keeper needs on its command line, rather
/// than in an environment a service manager would have to be told in three
/// ways, so a Channel's registration is whole in itself and a Scheduled Task,
/// which has no way to set an environment, is told it as the others are.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeepCommand {
    program: String,
    arguments: Vec<String>,
}

impl KeepCommand {
    /// The Keeper of `config`'s Channel, run from `executable`, carrying
    /// `carried`. `service_manager` is the stand-in `suru server install` was
    /// given, which its Keeper then asks too.
    fn new(
        config: &ManagedClientConfig,
        executable: &Path,
        carried: &[(String, String)],
        service_manager: Option<&Path>,
    ) -> Result<Self> {
        let runtime = config.runtime();
        let program = spelled(executable)?;
        let mut arguments = vec![
            KEEPER_COMMAND.to_owned(),
            "--executable".to_owned(),
            program.clone(),
            "--state-dir".to_owned(),
            spelled(&std::path::absolute(runtime.state_base_dir())?)?,
            "--data-dir".to_owned(),
            spelled(&std::path::absolute(runtime.data_base_dir())?)?,
            "--channel".to_owned(),
            runtime.channel().to_owned(),
        ];
        if let Some(config_dir) = runtime.config_dir() {
            arguments.push("--config-dir".to_owned());
            arguments.push(spelled(&std::path::absolute(config_dir)?)?);
        }
        if let Some(store) = runtime.identity_store() {
            arguments.push("--identity-store".to_owned());
            arguments.push(store.to_string());
        }
        if let Some(service_manager) = service_manager {
            arguments.push("--service-manager".to_owned());
            arguments.push(spelled(service_manager)?);
        }
        for (name, value) in carried {
            arguments.push("--env".to_owned());
            arguments.push(format!("{name}={value}"));
        }
        // No service manager's format can carry a control character whole, and
        // none belongs in a path a user would keep a Channel at.
        if let Some(argument) = arguments
            .iter()
            .find(|argument| argument.chars().any(char::is_control))
        {
            bail!(
                "cannot register a start at login naming {argument:?}: it holds a control character"
            );
        }
        Ok(Self { program, arguments })
    }

    /// The program and arguments, in order, as a service manager is to run them.
    pub fn words(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.program.as_str()).chain(self.arguments.iter().map(String::as_str))
    }
}

fn spelled(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).with_context(|| {
        format!("cannot register a start at login naming {path:?}: it is not valid Unicode")
    })
}

/// Why a path a registration would name may not outlast an upgrade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PassingPath {
    Build,
    Versioned,
    Temporary,
}

impl PassingPath {
    fn says(self) -> &'static str {
        match self {
            Self::Build => "it is in a build directory",
            Self::Versioned => "it is in a directory named for one version",
            Self::Temporary => "it is in a temporary directory",
        }
    }
}

/// Why `path` may not outlast an upgrade, if there is a reason to think so.
/// Only ever a reason to warn: a path that looks as though it passes may be the
/// one the user means to keep.
fn passing(path: &Path, temporary: &Path) -> Option<PassingPath> {
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
        return Some(PassingPath::Build);
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
        return Some(PassingPath::Temporary);
    }
    let versioned = path.components().any(|component| {
        component.as_os_str().to_str().is_some_and(|name| {
            let digits = name.strip_prefix('v').unwrap_or(name);
            digits.starts_with(|first: char| first.is_ascii_digit()) && digits.contains('.')
        })
    });
    if versioned || is_named(&["Cellar", "Caskroom", "nix"]) {
        return Some(PassingPath::Versioned);
    }
    None
}

/// The path a registration names for the binary at `current`, and why it may
/// not outlast an upgrade where it may not.
///
/// What this process knows itself as may be the far end of a link: on Linux
/// `current_exe` resolves through them, so Homebrew's `bin/suru` reads as a
/// Cellar path naming one version, which the next upgrade removes while the
/// link goes on naming whichever is current. So where `current` looks passing,
/// a `PATH` entry that leads to this same binary and does not is named instead.
fn registered_executable(
    current: &Path,
    path_variable: Option<&OsStr>,
    temporary: &Path,
) -> (PathBuf, Option<PassingPath>) {
    let Some(why) = passing(current, temporary) else {
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
                && passing(candidate, temporary).is_none()
        });
    match lasting {
        Some(lasting) => (lasting, None),
        None => (current.to_path_buf(), Some(why)),
    }
}

/// What of the variables `variable` reads a registration carries.
fn carried_environment(
    variable: impl Fn(&str) -> Option<OsString>,
) -> Result<Vec<(String, String)>> {
    CARRIED_VARIABLES
        .into_iter()
        .filter_map(|name| Some((name, variable(name).filter(|value| !value.is_empty())?)))
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
pub fn install(config: &ManagedClientConfig, service_manager: Option<PathBuf>) -> Result<String> {
    let login_start = LoginStart::for_this_user(config.channel(), service_manager.clone())?;
    let current = std::env::current_exe().context("find current Suru executable")?;
    let path_variable = std::env::var_os("PATH");
    let (executable, passing) =
        registered_executable(&current, path_variable.as_deref(), &std::env::temp_dir());
    let carried = carried_environment(|name| std::env::var_os(name))?;
    let keep = KeepCommand::new(config, &executable, &carried, service_manager.as_deref())?;
    let location = login_start.register(&keep)?;
    Ok(install_report(
        &login_start,
        &location,
        &executable,
        passing,
        &uncarried_variables(|name| std::env::var_os(name)),
    ))
}

fn uncarried_variables(variable: impl Fn(&str) -> Option<OsString>) -> Vec<&'static str> {
    UNCARRIED_VARIABLES
        .into_iter()
        .filter(|name| variable(name).is_some_and(|value| !value.is_empty()))
        .collect()
}

fn install_report(
    login_start: &LoginStart,
    location: &str,
    executable: &Path,
    passing: Option<PassingPath>,
    uncarried: &[&str],
) -> String {
    let channel = &login_start.channel;
    let mut report = format!(
        "Registered a start at login for the `{channel}` Channel's Server: {location}\n\n\
         It comes into force at the next login and changes nothing until then: a Server \
         running now is left running, and `suru server start` starts one where none is.\n\
         At each login a Keeper starts the Server and restarts it if it crashes. A Server \
         stopped by `suru server stop` stays stopped until the next login or `suru server \
         start`.\n\n{}\n\n\
         Providers are found on the PATH this command ran with, which the registration \
         keeps. Run `suru server install` again after it changes.\n",
        login_start.covers()
    );
    if !uncarried.is_empty() {
        report.push_str(&format!(
            "Not carried, because they may hold secrets: {}. A Server started at login does \
             not have them, so a Provider or Relay relying on one needs another way in.\n",
            uncarried.join(", ")
        ));
    }
    if let Some(passing) = passing {
        report.push_str(&format!(
            "\nWarning: this registration names {}, which may not survive an upgrade: {}. \
             Run `suru server install` again from wherever Suru is kept for good.\n",
            executable.display(),
            passing.says()
        ));
    }
    report
}

/// `suru server uninstall`: removes the Login start of `channel`, and answers
/// what to tell the user.
pub fn uninstall(channel: &str, service_manager: Option<PathBuf>) -> Result<String> {
    let login_start = LoginStart::for_this_user(channel, service_manager)?;
    Ok(uninstall_report(channel, &login_start.remove()?))
}

fn uninstall_report(channel: &str, removal: &Removal) -> String {
    const LEFT_ALONE: &str = "A Server running now is left running; `suru server stop` stops it.";
    match removal {
        Removal::Removed(location) => format!(
            "Removed the start at login for the `{channel}` Channel's Server: {location}\n\
             {LEFT_ALONE}\n"
        ),
        Removal::RemovedUnanswered(location, reason) => format!(
            "Removed the start at login for the `{channel}` Channel's Server: {location}\n\
             The service manager did not answer, so it reads this at its next start: {reason}\n\
             {LEFT_ALONE}\n"
        ),
        Removal::NothingRegistered => format!(
            "No start at login is registered for the `{channel}` Channel, so there is nothing \
             to remove.\n"
        ),
        Removal::NoServiceManager(reason) => format!(
            "No start at login is registered for the `{channel}` Channel, and this machine has \
             no service manager to hold one: {reason}\n"
        ),
    }
}

/// A file a registration keeps: readable by its owner alone, and replaced
/// whole, so a registration is never read half written. launchd accepts a
/// LaunchAgent only if nobody else can write to it, and systemd asks no more.
fn keep_file(path: &Path, contents: &[u8]) -> Result<()> {
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
    use std::sync::{Arc, Mutex};

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
    }

    impl RecordingCommands {
        pub(super) fn ran(&self) -> Vec<Vec<String>> {
            self.ran.lock().unwrap().clone()
        }

        pub(super) fn refuse(&self, word: &'static str) {
            self.refusing.lock().unwrap().push(word);
        }

        pub(super) fn go_missing(&self) {
            *self.missing.lock().unwrap() = true;
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
                said: if success {
                    String::new()
                } else {
                    "refused".to_owned()
                },
            })
        }
    }

    /// A Login start kept under `home`, told to `commands`.
    pub(super) fn login_start(
        platform: Platform,
        channel: &str,
        home: &Path,
        commands: &RecordingCommands,
    ) -> LoginStart {
        LoginStart {
            platform,
            channel: channel.to_owned(),
            places: Places {
                home: home.to_path_buf(),
                config_home: home.join(".config"),
                user: Some("HOST\\jo".to_owned()),
            },
            commands: Box::new(commands.clone()),
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
    use super::*;

    fn client_config(channel: &str) -> ManagedClientConfig {
        ManagedClientConfig::new(std::env::temp_dir().join("suru-state"), channel)
            .unwrap()
            .with_data_dir(std::env::temp_dir().join("suru-data"))
            .with_config_dir(std::env::temp_dir().join("suru-config"))
    }

    #[test]
    fn a_keep_command_carries_its_channel_and_directories_on_its_command_line() {
        let carried = [("PATH".to_owned(), "/somewhere/bin".to_owned())];
        let keep = KeepCommand::new(
            &client_config("beta"),
            &std::env::temp_dir().join("suru"),
            &carried,
            None,
        )
        .unwrap();
        let words = keep.words().collect::<Vec<_>>();
        let after =
            |flag: &str| words[words.iter().position(|word| *word == flag).unwrap() + 1].to_owned();
        assert_eq!(words[1], KEEPER_COMMAND);
        assert_eq!(after("--channel"), "beta");
        assert_eq!(after("--executable"), words[0]);
        assert_eq!(
            after("--state-dir"),
            std::env::temp_dir().join("suru-state").to_str().unwrap()
        );
        assert_eq!(
            after("--data-dir"),
            std::env::temp_dir().join("suru-data").to_str().unwrap()
        );
        assert_eq!(
            after("--config-dir"),
            std::env::temp_dir().join("suru-config").to_str().unwrap()
        );
        assert_eq!(after("--env"), "PATH=/somewhere/bin");
    }

    #[test]
    fn a_keep_command_with_a_control_character_is_refused_rather_than_mangled() {
        let error = KeepCommand::new(
            &client_config("beta"),
            &std::env::temp_dir().join("su\nru"),
            &[],
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("control character"), "{error:#}");
    }

    /// The secrets a shell holds go nowhere near a registration: only what
    /// names a place is carried.
    #[test]
    fn only_what_finds_a_provider_is_carried_and_never_a_secret() {
        let shell = |name: &str| {
            let value = match name {
                "PATH" => "/opt/bin",
                "SURU_CLAUDE_PATH" => "/opt/claude",
                "CODEX_HOME" => "/opt/codex-home",
                "COPILOT_HOME" => "",
                "ANTHROPIC_API_KEY" => "secret",
                "AWS_SECRET_ACCESS_KEY" => "secret",
                "HTTPS_PROXY" => "http://me:secret@proxy",
                _ => return None,
            };
            Some(OsString::from(value))
        };
        assert_eq!(
            carried_environment(shell).unwrap(),
            [
                ("PATH".to_owned(), "/opt/bin".to_owned()),
                ("SURU_CLAUDE_PATH".to_owned(), "/opt/claude".to_owned()),
                ("CODEX_HOME".to_owned(), "/opt/codex-home".to_owned()),
            ],
        );
        assert_eq!(
            uncarried_variables(shell),
            ["ANTHROPIC_API_KEY", "HTTPS_PROXY"],
        );
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
            ("/home/jo/suru/target/debug/suru", Some(PassingPath::Build)),
            (
                "/home/jo/suru/target/release/suru",
                Some(PassingPath::Build),
            ),
            (
                "/home/jo/suru/target/x86_64-unknown-linux-gnu/release/suru",
                Some(PassingPath::Build),
            ),
            (
                "/opt/homebrew/Cellar/suru/1.2.3/bin/suru",
                Some(PassingPath::Versioned),
            ),
            (
                "/opt/homebrew/Cellar/suru/HEAD/bin/suru",
                Some(PassingPath::Versioned),
            ),
            (
                "/home/jo/.asdf/installs/suru/v0.4.1/suru",
                Some(PassingPath::Versioned),
            ),
            ("/nix/store/abc-suru/bin/suru", Some(PassingPath::Versioned)),
            ("/scratch/tmp/unpacked/suru", Some(PassingPath::Temporary)),
        ] {
            assert_eq!(passing(&rooted(path), &temporary), expected, "{path}");
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
            passing(&target.join("debug").join("suru"), elsewhere.path()),
            Some(PassingPath::Build)
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
            (binary, Some(PassingPath::Versioned))
        );
    }

    #[test]
    fn the_install_report_says_what_was_done_and_warns_of_what_will_not_last() {
        let home = tempfile::tempdir().unwrap();
        let commands = test_support::RecordingCommands::default();
        let login_start =
            test_support::login_start(Platform::Launchd, "beta", home.path(), &commands);
        let executable = rooted("/home/jo/target/debug/suru");

        let report = install_report(
            &login_start,
            "somewhere",
            &executable,
            Some(PassingPath::Build),
            &["ANTHROPIC_API_KEY"],
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

    /// Every platform's output says what it covers, including that a Mac waits
    /// for its login keychain and what Linux needs to run with no session.
    #[test]
    fn each_platform_says_what_its_registration_covers_and_what_it_does_not() {
        let home = tempfile::tempdir().unwrap();
        let commands = test_support::RecordingCommands::default();
        let covers =
            |platform| test_support::login_start(platform, "beta", home.path(), &commands).covers();
        let mac = covers(Platform::Launchd);
        assert!(mac.contains("login keychain"), "{mac}");
        assert!(mac.contains("ADR-0050"), "{mac}");
        let linux = covers(Platform::Systemd);
        assert!(linux.contains("loginctl enable-linger"), "{linux}");
        let windows = covers(Platform::TaskScheduler);
        assert!(windows.contains("logon"), "{windows}");
    }

    #[test]
    fn the_uninstall_report_says_a_running_server_was_left_alone() {
        let removed = uninstall_report("beta", &Removal::Removed("somewhere".to_owned()));
        assert!(removed.contains("left running"), "{removed}");
        let nothing = uninstall_report("beta", &Removal::NothingRegistered);
        assert!(nothing.contains("nothing to remove"), "{nothing}");
        let no_manager =
            uninstall_report("beta", &Removal::NoServiceManager("no systemd".to_owned()));
        assert!(no_manager.contains("no systemd"), "{no_manager}");
    }
}
