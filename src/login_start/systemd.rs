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

use anyhow::{Context, Result, bail};

use super::{KeepCommand, LoginStart, Removal, Standing, keep_file, remove_file};

const SYSTEMCTL: &str = "systemctl";

pub(super) fn covers() -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "$USER".to_owned());
    format!(
        "\
What this covers on Linux:
- This user's first login, whether at a screen or over SSH.
- Not a machine that has rebooted and that nobody has logged in to, and it stops at the last \
logout. To run with no session at all, run `loginctl enable-linger {user}`, which needs no \
reinstalling.
- A Server whose identity key is in a keyring that nothing has unlocked, as with no desktop \
login, fails Serving where you can see it rather than making a new one (ADR-0050)."
    )
}

fn unit_name(channel: &str) -> String {
    format!("suru-server-{channel}.service")
}

fn units(login_start: &LoginStart) -> std::path::PathBuf {
    login_start.places.config_home.join("systemd").join("user")
}

fn unit_path(login_start: &LoginStart) -> std::path::PathBuf {
    units(login_start).join(unit_name(&login_start.channel))
}

/// The link `systemctl enable` makes for this unit, which is the whole of its
/// being enabled.
fn wants_link(login_start: &LoginStart) -> std::path::PathBuf {
    units(login_start)
        .join("default.target.wants")
        .join(unit_name(&login_start.channel))
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
/// take it down with the Keeper as the unit stops. The Server still goes at
/// logout, when the user instance ends and takes all of its control groups
/// with it, as a SIGTERM it answers as a Manual stop; the next login's Keeper
/// reads that stop as already seen, and starts another.
///
/// Its stdout and stderr go to the journal, which systemd keeps bounded, and the
/// Server's to the state directory, where any launched Server's go.
fn unit(keep: &KeepCommand) -> String {
    let exec_start = keep.words().map(exec_word).collect::<Vec<_>>().join(" ");
    format!(
        "\
[Unit]
Description=Suru Keeper: starts this Channel's Server at login, and restarts it if it crashes

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

pub(super) fn register(login_start: &LoginStart, keep: &KeepCommand) -> Result<String> {
    // Asked first, so a machine with no user instance is told so with nothing
    // left behind.
    let reload = systemctl(login_start, ["daemon-reload"]);
    if let Some(unanswered) = unanswered(&reload) {
        bail!(
            "could not register a start at login: this machine has no systemd user instance to \
             register it with ({unanswered}). Nothing was registered. In a container or under \
             another init system, have what starts things there run `suru server start`."
        );
    }
    let path = unit_path(login_start);
    let was_registered = path.try_exists().unwrap_or(false);
    keep_file(&path, unit(keep).as_bytes())?;
    let enabled = systemctl(login_start, ["daemon-reload"]).and_then(|reloaded| {
        if reloaded.success {
            systemctl(login_start, ["enable", &unit_name(&login_start.channel)])
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
            unit_name(&login_start.channel)
        );
    }
    Ok(path.display().to_string())
}

pub(super) fn remove(login_start: &LoginStart) -> Result<Removal> {
    let path = unit_path(login_start);
    let link = wants_link(login_start);
    let registered =
        path.try_exists().context("inspect the unit")? || link.symlink_metadata().is_ok();
    let manager_unanswered = if registered {
        unanswered(&systemctl(
            login_start,
            ["disable", &unit_name(&login_start.channel)],
        ))
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
    let Some(reason) = manager_unanswered else {
        // Nothing depends on this: an instance that does not answer reads the
        // directory as it next starts.
        let _ = systemctl(login_start, ["daemon-reload"]);
        return Ok(Removal::Removed(path.display().to_string()));
    };
    Ok(Removal::RemovedUnanswered(
        path.display().to_string(),
        reason,
    ))
}

/// Read from the files alone, so a machine with no user instance answering can
/// still be told about, and with no process to spawn each time.
pub(super) fn standing(login_start: &LoginStart) -> Standing {
    let path = unit_path(login_start);
    match (
        path.try_exists(),
        wants_link(login_start).symlink_metadata(),
    ) {
        (Ok(true), Ok(_)) => Standing::Registered(path.display().to_string()),
        (Ok(_), _) => Standing::NotRegistered,
        (Err(error), _) => Standing::Unknown(format!("{}: {error}", path.display())),
    }
}

fn systemctl<const N: usize>(
    login_start: &LoginStart,
    arguments: [&str; N],
) -> std::io::Result<super::CommandOutcome> {
    let arguments = std::iter::once("--user")
        .chain(arguments)
        .map(Into::into)
        .collect::<Vec<_>>();
    login_start.commands.run(SYSTEMCTL, &arguments)
}

/// Why `outcome` is no success, where it is not.
fn unanswered(outcome: &std::io::Result<super::CommandOutcome>) -> Option<String> {
    match outcome {
        Ok(outcome) if outcome.success => None,
        Ok(outcome) => Some(outcome.said.clone()),
        Err(error) => Some(format!("{SYSTEMCTL} could not be run: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        Platform,
        test_support::{RecordingCommands, awkward_keep, login_start},
    };
    use super::*;

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
        assert_eq!(exec_word("/plain/suru"), "\"/plain/suru\"");
        assert_eq!(exec_word("/jo smith/suru"), "\"/jo smith/suru\"");
        assert_eq!(exec_word("/100%/x"), "\"/100%%/x\"");
        assert_eq!(exec_word("/$HOME/x"), "\"/$$HOME/x\"");
        assert_eq!(exec_word("/say \"hi\""), "\"/say \\\"hi\\\"\"");
        assert_eq!(exec_word("/back\\slash"), "\"/back\\\\slash\"");

        let unit = unit(&awkward_keep());
        assert!(
            unit.contains(
                "ExecStart=\"/Users/jo smith/R&D <x>/suru\" \"__keep-server\" \"--state-dir\" \
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
        let beta = login_start(Platform::Systemd, "beta", home.path(), &commands);
        let unit = "suru-server-beta.service";

        register(&beta, &awkward_keep()).unwrap();
        assert!(unit_path(&beta).is_file());
        assert_eq!(
            commands.ran(),
            [
                words(&["systemctl", "--user", "daemon-reload"]),
                words(&["systemctl", "--user", "daemon-reload"]),
                words(&["systemctl", "--user", "enable", unit]),
            ]
        );

        // `--now` is what would start or end something; stop-like verbs too.
        std::fs::create_dir_all(wants_link(&beta).parent().unwrap()).unwrap();
        std::fs::write(wants_link(&beta), "").unwrap();
        assert!(matches!(beta.standing(), Standing::Registered(_)));
        assert!(matches!(remove(&beta).unwrap(), Removal::Removed(_)));
        assert!(!unit_path(&beta).exists());
        assert!(!wants_link(&beta).exists());
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
        let beta = login_start(Platform::Systemd, "beta", home.path(), &commands);

        assert_eq!(remove(&beta).unwrap(), Removal::NothingRegistered);
        assert!(commands.ran().is_empty(), "nothing to tell systemd");

        register(&beta, &awkward_keep()).unwrap();
        let mut moved = awkward_keep();
        moved.program = "/opt/moved/suru".to_owned();
        register(&beta, &moved).unwrap();
        let contents = std::fs::read_to_string(unit_path(&beta)).unwrap();
        assert!(contents.contains("/opt/moved/suru"), "{contents}");
        assert!(!contents.contains("R&D"), "{contents}");

        assert!(matches!(remove(&beta).unwrap(), Removal::Removed(_)));
        assert_eq!(remove(&beta).unwrap(), Removal::NothingRegistered);
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
            let beta = login_start(Platform::Systemd, "beta", home.path(), &commands);
            let error = register(&beta, &awkward_keep()).unwrap_err().to_string();
            assert!(error.contains("no systemd user instance"), "{error}");
            assert!(error.contains("Nothing was registered"), "{error}");
            assert!(!unit_path(&beta).exists());
            assert_eq!(beta.standing(), Standing::NotRegistered);
        }
    }

    #[test]
    fn an_enable_that_fails_leaves_no_unit_of_its_own_behind() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        commands.refuse("enable");
        let beta = login_start(Platform::Systemd, "beta", home.path(), &commands);
        let error = register(&beta, &awkward_keep()).unwrap_err().to_string();
        assert!(error.contains("could not enable"), "{error}");
        assert!(!unit_path(&beta).exists());
    }

    /// Uninstalling where no user instance answers still takes away what is
    /// there, and is no error.
    #[test]
    fn removing_with_no_user_instance_takes_away_what_it_finds_and_says_so() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        let beta = login_start(Platform::Systemd, "beta", home.path(), &commands);
        register(&beta, &awkward_keep()).unwrap();
        std::fs::create_dir_all(wants_link(&beta).parent().unwrap()).unwrap();
        std::fs::write(wants_link(&beta), "").unwrap();

        commands.go_missing();
        assert!(matches!(
            remove(&beta).unwrap(),
            Removal::RemovedUnanswered(_, _)
        ));
        assert!(!unit_path(&beta).exists());
        assert!(!wants_link(&beta).exists());
        assert_eq!(beta.standing(), Standing::NotRegistered);
        assert_eq!(remove(&beta).unwrap(), Removal::NothingRegistered);
    }

    #[test]
    fn a_channels_unit_leaves_another_channels_alone() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        let beta = login_start(Platform::Systemd, "beta", home.path(), &commands);
        let release = login_start(Platform::Systemd, "release", home.path(), &commands);

        register(&beta, &awkward_keep()).unwrap();
        register(&release, &awkward_keep()).unwrap();
        remove(&beta).unwrap();
        assert!(unit_path(&release).is_file());
        assert!(!unit_path(&beta).exists());
        for command in commands.ran() {
            let names_both = command.iter().any(|word| word.contains("beta"))
                && command.iter().any(|word| word.contains("release"));
            assert!(!names_both, "{command:?}");
        }
    }
}
