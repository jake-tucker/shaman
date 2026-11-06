//! The seam every platform's registration goes through: what a service manager is
//! asked and how its commands are run, and the file handling and XML two of them
//! share.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result};

use super::{KeepCommand, Removal, Standing};

/// A stand-in for the machine's service manager, for tests that must not reach
/// the real one. A flag rather than an empty `PATH` because Windows looks in its
/// system directory for a program whatever `PATH` says.
#[derive(Clone, Debug, Default, Eq, PartialEq, clap::Args)]
pub struct ServiceManagerStandIn {
    #[arg(long = "service-manager", hide = true)]
    pub program: Option<PathBuf>,
}

/// The service manager a Login start is registered with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Platform {
    Launchd,
    Systemd,
    TaskScheduler,
}

impl Platform {
    pub(super) const fn current() -> Self {
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
pub(super) struct CommandOutcome {
    pub(super) success: bool,
    /// What it printed, for looking through.
    pub(super) printed: String,
    /// What it said on its way out, for the user to read.
    pub(super) said: String,
}

/// The one way anything here runs a service manager's command. An error means
/// the command could not be run at all: the service manager is not there.
pub(super) trait ServiceManagerCommands: Send + Sync {
    fn run(&self, program: &str, arguments: &[OsString]) -> io::Result<CommandOutcome>;
}

/// Runs the machine's own service manager, or its stand-in.
pub(super) struct MachineCommands {
    pub(super) stand_in: ServiceManagerStandIn,
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
pub(super) trait ServiceManager: Send + Sync {
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

/// How a registration kept as a file stands, for `home`'s user. A file not found
/// says nothing while the home itself is not found, as one on a mount that is
/// not back yet at login is not.
pub(super) fn file_standing(path: &Path, home: &Path) -> Standing {
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

/// Writes a file a registration keeps: readable by its owner alone, and replaced
/// whole, so a registration is never read half written. launchd accepts a
/// LaunchAgent only if nobody else can write to it, and systemd asks no more.
pub(super) fn write_registration(path: &Path, contents: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .with_context(|| format!("{path:?} lies in no directory"))?;
    std::fs::create_dir_all(directory).with_context(|| format!("create {directory:?}"))?;
    crate::runtime::replace_private_file(path, contents)
}

/// Removes `path`, answering whether there was anything to remove.
pub(super) fn remove_file(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("remove {path:?}")),
    }
}

/// `text` as XML character data, as a plist and a Task definition carry it.
pub(super) fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
