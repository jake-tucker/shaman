//! Starting a Channel's Server when its user logs in (ADR 0063).
//!
//! `shaman server install` registers, for one Channel, a Login start with the
//! machine's own service manager: a LaunchAgent on macOS, a systemd user unit
//! on Linux, a Scheduled Task at logon on Windows. What each is told to run is
//! the same command, the Channel's Keeper (`managed_client::keep_server`), and
//! each is told to run it once, with no restart policy of its own, so nothing a
//! service manager does can undo a Manual stop. The registrations know nothing
//! of Servers beyond that command line; [`KeeperArguments`] is where it turns
//! into one, reading the Channel's directories and launching what it names.
//!
//! Registering and removing touch only what comes into force at the next
//! login: never the Keeper or Server running now. launchd reads a LaunchAgent's
//! file at login by itself, so macOS runs no command at all, and `launchctl
//! bootout`, which would end the running job, never comes up; Linux enables and
//! disables its unit without `--now`; and on Windows the Server is launched
//! detached from the Scheduled Task that started its Keeper, which has yet to be
//! seen to hold against a real Task Scheduler (`docs/validation`).
//!
//! What is generated for each platform is a pure function here, tested on every
//! platform, and what is run is behind [`ServiceManagerCommands`], so no test
//! comes near the real user's launchd, systemd or Task Scheduler.
//!
//! - [`keeper_command`]: a Keeper's command line, a persisted interface.
//! - [`keeping`]: a Keeper getting going under a service manager.
//! - [`service_manager`]: the seam the three platforms go through.
//! - [`launchd`], [`systemd`], [`task_scheduler`]: what each is told.
//! - [`environment`], [`lasting`]: what is carried, and what will not last.
//! - [`report`]: what the user is told.

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};

use crate::managed_client::ManagedClientConfig;

mod environment;
mod keeper_command;
mod keeping;
mod lasting;
mod launchd;
mod report;
mod service_manager;
mod systemd;
mod task_scheduler;

pub use keeper_command::KeeperArguments;
pub use service_manager::ServiceManagerStandIn;

use environment::{carried_environment, uncarried_variables};
use keeper_command::KeepCommand;
use lasting::{ShortLived, fleeting_path_entries, registered_executable};
use report::{InstallFindings, install_report, uninstall_report};
use service_manager::{
    CommandOutcome, MachineCommands, Platform, ServiceManager, ServiceManagerCommands,
    file_standing, remove_file, write_registration, xml_text,
};

/// The hidden command a registration runs: the Channel's Keeper.
pub const KEEPER_COMMAND: &str = "__keep-server";

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

/// `shaman server install`: registers the Login start of `config`'s Channel, and
/// answers what to tell the user.
pub fn install(config: &ManagedClientConfig, stand_in: ServiceManagerStandIn) -> Result<String> {
    let login_start = LoginStart::for_this_user(config.channel(), stand_in.clone())?;
    let current = std::env::current_exe().context("find current Shaman executable")?;
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
        &InstallFindings {
            executable: &executable,
            short_lived,
            uncarried: uncarried_variables(&providers, std::env::vars_os().map(|(name, _)| name)),
            fleeting_path_entries: path_variable.as_deref().map_or_else(Vec::new, |path| {
                fleeting_path_entries(path, &std::env::temp_dir())
            }),
        },
    ))
}

/// `shaman server uninstall`: removes the Login start of `channel`, and answers
/// what to tell the user.
pub fn uninstall(channel: &str, stand_in: ServiceManagerStandIn) -> Result<String> {
    let login_start = LoginStart::for_this_user(channel, stand_in)?;
    Ok(uninstall_report(channel, &login_start.manager.remove()?))
}

#[cfg(test)]
mod test_support {
    use std::{ffi::OsString, io, sync::Mutex};

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
            program: "/Users/jo smith/R&D <x>/shaman".to_owned(),
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
