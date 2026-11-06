//! A Keeper's command line, as a registration writes it and a Keeper reads it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{KEEPER_COMMAND, ServiceManagerStandIn};
use crate::IdentityStoreChoice;

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
}

/// The command a registration runs, and every service manager is told it in
/// its own quoting. It carries all a Keeper needs on its command line, rather
/// than in an environment a service manager would have to be told in three
/// ways, so a Channel's registration is whole in itself and a Scheduled Task,
/// which has no way to set an environment, is told it as the others are.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct KeepCommand {
    pub(super) program: String,
    pub(super) arguments: Vec<String>,
}

impl KeepCommand {
    pub(super) fn new(keeper: &KeeperArguments) -> Result<Self> {
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
    pub(super) fn words(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.program.as_str()).chain(self.arguments.iter().map(String::as_str))
    }
}

fn unicode(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).with_context(|| {
        format!("cannot register a Login start naming {path:?}: it is not valid Unicode")
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

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
}
