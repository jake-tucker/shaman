//! A Login start on Linux: a systemd user unit in the user's own
//! `~/.config/systemd/user`.
//!
//! Registering enables the unit and removing disables it, each without `--now`,
//! which would start a Server its user may have just stopped, or end the Keeper
//! running now. A unit file taken away while its Keeper runs leaves that Keeper
//! running, which finds its registration gone the next time it would launch
//! anything, and ends.
//!
//! Where no systemd user instance answers — a container, WSL without systemd,
//! another init system — there is nothing to register with. Installing says so,
//! leaving nothing behind, and removing still takes away what it finds.

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, bail};

use super::{
    CommandOutcome, KeepCommand, Removal, ServiceManager, ServiceManagerCommands, Standing,
    file_standing, remove_file, write_registration,
};

const SYSTEMCTL: &str = "systemctl";

fn covers() -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "$USER".to_owned());
    format!(
        "\
What this covers on Linux:
- This user's first login, whether at a screen or over SSH.
- Not a machine that has rebooted and that nobody has logged in to, and at the last logout the \
server is ended outright, as a crash would. To run with no session at all, run `loginctl enable-linger {user}`, which needs no \
reinstalling.
- A server whose identity key is in a keyring that nothing has unlocked, as with no desktop \
login, fails serving where you can see it rather than making a new one."
    )
}

fn unit_name(channel: &str) -> String {
    format!("shaman-server-{channel}.service")
}

/// `word` as `ExecStart` reads one word back: double quotes, within which `\`
/// and `"` are escaped; `%` begins a specifier and `$` a variable even there,
/// and doubling is how each is written literally.
fn exec_word(word: &str) -> String {
    let escaped = word
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

/// The unit that runs `keep`.
///
/// `Restart` is left at its default, `no`: the Keeper is what restarts a crashed
/// Server, and a Keeper systemd started again after a stop would read that stop
/// as already seen, and start the Server its user had stopped.
///
/// `KillMode=process`, because the Server is in this unit's control group
/// though it is detached from its Keeper: the default, `control-group`, would
/// take it down with the Keeper, whenever the Keeper ended.
///
/// That leaves logout with no linger. The user instance signals the Keeper alone,
/// and the system then ends `user@.service`, killing what is left of its control
/// groups outright. So the Server goes as a crash leaves it, with no Manual stop
/// recorded; SQLite's journal keeps storage whole, its Providers go with it, and
/// the next login's Keeper finds the election free and starts another. There is
/// no clean way to do better here: an `ExecStop=` running `shaman server stop` also
/// runs once the Keeper has ended by itself, which it does on finding its
/// registration gone, and would stop a Server uninstalling must leave alone.
///
/// Its stdout and stderr go to the journal, which systemd keeps bounded, and the
/// Server's to the state directory, where any launched Server's go.
fn unit(keep: &KeepCommand) -> String {
    let exec_start = keep.words().map(exec_word).collect::<Vec<_>>().join(" ");
    format!(
        "\
[Unit]
Description=Shaman keeper: starts this channel's server at login, and restarts it if it crashes

[Service]
Type=simple
ExecStart={exec_start}
WorkingDirectory=%h
KillMode=process

[Install]
WantedBy=default.target
"
    )
}

/// A Channel's systemd user unit.
pub(super) struct UserUnit {
    pub(super) channel: String,
    pub(super) home: PathBuf,
    /// The directory systemd reads a user's own units beneath.
    pub(super) config_home: PathBuf,
    pub(super) commands: Arc<dyn ServiceManagerCommands>,
}

impl UserUnit {
    fn units(&self) -> PathBuf {
        self.config_home.join("systemd").join("user")
    }

    fn unit_path(&self) -> PathBuf {
        self.units().join(unit_name(&self.channel))
    }

    /// The link `systemctl enable` makes for this unit, which is the whole of
    /// its being enabled.
    fn wants_link(&self) -> PathBuf {
        self.units()
            .join("default.target.wants")
            .join(unit_name(&self.channel))
    }

    fn systemctl<const N: usize>(&self, arguments: [&str; N]) -> std::io::Result<CommandOutcome> {
        let arguments = std::iter::once("--user")
            .chain(arguments)
            .map(Into::into)
            .collect::<Vec<_>>();
        self.commands.run(SYSTEMCTL, &arguments)
    }
}

impl ServiceManager for UserUnit {
    fn register(&self, keep: &KeepCommand) -> Result<String> {
        // Asked first, so a machine with no user instance is told so with
        // nothing left behind.
        if let Some(unanswered) = unanswered(&self.systemctl(["daemon-reload"])) {
            bail!(
                "could not register a login start: this machine has no systemd user instance to \
                 register it with ({unanswered}). Nothing was registered. In a container or under \
                 another init system, have what starts things there run `shaman server start`."
            );
        }
        let path = self.unit_path();
        let was_registered = path.try_exists().unwrap_or(false);
        write_registration(&path, unit(keep).as_bytes())?;
        let enabled = self.systemctl(["daemon-reload"]).and_then(|reloaded| {
            if reloaded.success {
                self.systemctl(["enable", &unit_name(&self.channel)])
            } else {
                Ok(reloaded)
            }
        });
        if let Some(unanswered) = unanswered(&enabled) {
            if !was_registered {
                let _ = remove_file(&path);
            }
            bail!(
                "could not enable {}: {unanswered}",
                unit_name(&self.channel)
            );
        }
        Ok(path.display().to_string())
    }

    fn remove(&self) -> Result<Removal> {
        let path = self.unit_path();
        let link = self.wants_link();
        let registered =
            path.try_exists().context("inspect the unit")? || link.symlink_metadata().is_ok();
        let manager_unanswered = if registered {
            unanswered(&self.systemctl(["disable", &unit_name(&self.channel)]))
        } else {
            None
        };
        // With no user instance to answer, the link is taken away as `disable`
        // would have.
        remove_file(&link)?;
        remove_file(&path)?;
        if !registered {
            return Ok(Removal::NothingRegistered);
        }
        let location = path.display().to_string();
        let Some(reason) = manager_unanswered else {
            // Nothing depends on this: an instance that does not answer reads
            // the directory as it next starts.
            let _ = self.systemctl(["daemon-reload"]);
            return Ok(Removal::Removed { location });
        };
        Ok(Removal::RemovedUnanswered { location, reason })
    }

    /// Read from the files alone, so a machine with no user instance answering
    /// can still be told about, and with no process to spawn each time. Only
    /// the link being found gone means the unit is not enabled: any other way of
    /// not reading it says nothing.
    fn standing(&self) -> Standing {
        match file_standing(&self.unit_path(), &self.home) {
            Standing::Registered(location) => match self.wants_link().symlink_metadata() {
                Ok(_) => Standing::Registered(location),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Standing::NotRegistered
                }
                Err(error) => {
                    Standing::Unknown(format!("{}: {error}", self.wants_link().display()))
                }
            },
            other => other,
        }
    }

    fn covers(&self) -> String {
        covers()
    }
}

/// Why `outcome` is no success, where it is not.
fn unanswered(outcome: &std::io::Result<CommandOutcome>) -> Option<String> {
    match outcome {
        Ok(outcome) if outcome.success => None,
        Ok(outcome) => Some(outcome.said.clone()),
        Err(error) => Some(format!("{SYSTEMCTL} could not be run: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{RecordingCommands, awkward_keep};
    use super::*;

    fn user_unit(channel: &str, home: &std::path::Path, commands: &RecordingCommands) -> UserUnit {
        UserUnit {
            channel: channel.to_owned(),
            home: home.to_path_buf(),
            config_home: home.join(".config"),
            commands: commands.shared(),
        }
    }

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_unit_runs_the_keeper_at_login_and_asks_systemd_to_restart_nothing() {
        let unit = unit(&awkward_keep());
        assert!(unit.contains("\nType=simple\n"), "{unit}");
        assert!(unit.contains("\nWantedBy=default.target\n"), "{unit}");
        assert!(unit.contains("\nWorkingDirectory=%h\n"), "{unit}");
        // A Keeper stopped must leave the Server in its control group alone.
        assert!(unit.contains("\nKillMode=process\n"), "{unit}");
        assert!(!unit.contains("Restart="), "{unit}");
        assert!(!unit.contains("Environment"), "{unit}");
    }

    /// systemd reads `%` as a specifier and `$` as a variable even inside
    /// quotes, so a path holding either arrives whole only if each is doubled.
    #[test]
    fn an_awkward_path_is_quoted_so_exec_start_reads_it_back_as_it_was() {
        assert_eq!(exec_word("/plain/shaman"), "\"/plain/shaman\"");
        assert_eq!(exec_word("/jo smith/shaman"), "\"/jo smith/shaman\"");
        assert_eq!(exec_word("/100%/x"), "\"/100%%/x\"");
        assert_eq!(exec_word("/$HOME/x"), "\"/$$HOME/x\"");
        assert_eq!(exec_word("/say \"hi\""), "\"/say \\\"hi\\\"\"");
        assert_eq!(exec_word("/back\\slash"), "\"/back\\\\slash\"");

        let unit = unit(&awkward_keep());
        assert!(
            unit.contains(
                "ExecStart=\"/Users/jo smith/R&D <x>/shaman\" \"__keep-server\" \"--state-dir\" \
                 \"/Users/jo smith/100%%/$$HOME/\\\"quoted\\\"\\\\state\" \"--env\" \
                 \"PATH=/a b/bin:/usr/bin\"\n"
            ),
            "{unit}"
        );
    }

    #[test]
    fn registering_enables_without_starting_and_removing_disables_without_stopping() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        let beta = user_unit("beta", home.path(), &commands);
        let unit = "shaman-server-beta.service";

        beta.register(&awkward_keep()).unwrap();
        assert!(beta.unit_path().is_file());
        assert_eq!(
            commands.ran(),
            [
                words(&["systemctl", "--user", "daemon-reload"]),
                words(&["systemctl", "--user", "daemon-reload"]),
                words(&["systemctl", "--user", "enable", unit]),
            ]
        );

        // `--now` is what would start or end something; stop-like verbs too.
        std::fs::create_dir_all(beta.wants_link().parent().unwrap()).unwrap();
        std::fs::write(beta.wants_link(), "").unwrap();
        assert!(matches!(beta.standing(), Standing::Registered(_)));
        assert!(matches!(beta.remove().unwrap(), Removal::Removed { .. }));
        assert!(!beta.unit_path().exists());
        assert!(!beta.wants_link().exists());
        let ran = commands.ran();
        assert_eq!(ran[3], words(&["systemctl", "--user", "disable", unit]));
        assert_eq!(ran[4], words(&["systemctl", "--user", "daemon-reload"]));
        for command in ran {
            for word in ["--now", "start", "stop", "restart", "kill"] {
                assert!(!command.iter().any(|said| said == word), "{command:?}");
            }
        }
    }

    #[test]
    fn doing_either_twice_is_harmless_and_a_second_install_replaces_the_first() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        let beta = user_unit("beta", home.path(), &commands);

        assert_eq!(beta.remove().unwrap(), Removal::NothingRegistered);
        assert!(commands.ran().is_empty(), "nothing to tell systemd");

        beta.register(&awkward_keep()).unwrap();
        let mut moved = awkward_keep();
        moved.program = "/opt/moved/shaman".to_owned();
        beta.register(&moved).unwrap();
        let contents = std::fs::read_to_string(beta.unit_path()).unwrap();
        assert!(contents.contains("/opt/moved/shaman"), "{contents}");
        assert!(!contents.contains("R&D"), "{contents}");

        assert!(matches!(beta.remove().unwrap(), Removal::Removed { .. }));
        assert_eq!(beta.remove().unwrap(), Removal::NothingRegistered);
    }

    /// A container has no systemd at all, or none for this user.
    #[test]
    fn installing_with_no_user_instance_says_so_and_leaves_nothing_behind() {
        let home = tempfile::tempdir().unwrap();
        for commands in [
            {
                let missing = RecordingCommands::default();
                missing.go_missing();
                missing
            },
            {
                let refusing = RecordingCommands::default();
                refusing.refuse("daemon-reload");
                refusing
            },
        ] {
            let beta = user_unit("beta", home.path(), &commands);
            let error = beta.register(&awkward_keep()).unwrap_err().to_string();
            assert!(error.contains("no systemd user instance"), "{error}");
            assert!(error.contains("Nothing was registered"), "{error}");
            assert!(!beta.unit_path().exists());
            assert_eq!(beta.standing(), Standing::NotRegistered);
        }
    }

    #[test]
    fn an_enable_that_fails_leaves_no_unit_of_its_own_behind() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        commands.refuse("enable");
        let beta = user_unit("beta", home.path(), &commands);
        let error = beta.register(&awkward_keep()).unwrap_err().to_string();
        assert!(error.contains("could not enable"), "{error}");
        assert!(!beta.unit_path().exists());
    }

    /// Uninstalling where no user instance answers still takes away what is
    /// there, and is no error.
    #[test]
    fn removing_with_no_user_instance_takes_away_what_it_finds_and_says_so() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        let beta = user_unit("beta", home.path(), &commands);
        beta.register(&awkward_keep()).unwrap();
        std::fs::create_dir_all(beta.wants_link().parent().unwrap()).unwrap();
        std::fs::write(beta.wants_link(), "").unwrap();

        commands.go_missing();
        assert!(matches!(
            beta.remove().unwrap(),
            Removal::RemovedUnanswered { .. }
        ));
        assert!(!beta.unit_path().exists());
        assert!(!beta.wants_link().exists());
        assert_eq!(beta.standing(), Standing::NotRegistered);
        assert_eq!(beta.remove().unwrap(), Removal::NothingRegistered);
    }

    #[test]
    fn a_channels_unit_leaves_another_channels_alone() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        let beta = user_unit("beta", home.path(), &commands);
        let release = user_unit("release", home.path(), &commands);

        beta.register(&awkward_keep()).unwrap();
        release.register(&awkward_keep()).unwrap();
        beta.remove().unwrap();
        assert!(release.unit_path().is_file());
        assert!(!beta.unit_path().exists());
        for command in commands.ran() {
            let names_both = command.iter().any(|word| word.contains("beta"))
                && command.iter().any(|word| word.contains("release"));
            assert!(!names_both, "{command:?}");
        }
    }
}
