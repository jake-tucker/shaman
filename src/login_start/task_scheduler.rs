//! A Login start on Windows: a Scheduled Task at logon, for this user alone.
//!
//! It is registered with `schtasks /Create /XML`, which alone can say all a Task
//! here must: a `/TR` command line holds at most 261 characters, and `/SC
//! ONLOGON` asks for elevation, because with no user named on its trigger it
//! would run at anybody's logon. It is never run on registering, and deleting
//! it ends at most the Keeper it started: the Server is detached from that, as
//! from any process that launches one.
//!
//! A Task's defaults would quietly undo a Keeper, so each is said here: it would
//! be ended after three days, and on going onto battery, and be run at the low
//! priority its Server and Providers would then inherit. It is `InteractiveToken`
//! because any other logon type has no way to read the user's Credential
//! Manager, where the Server's identity key is (ADR-0050). No restart setting is
//! given: the Keeper is what restarts a crashed Server, and a Task run again
//! would take a Manual stop as already seen.

use anyhow::{Context, Result, bail};

use super::{KeepCommand, LoginStart, Removal, Standing, xml_text};

const SCHTASKS: &str = "schtasks";

pub(super) const COVERS: &str = "\
What this covers on Windows:
- A logon by this user.
- Not a machine that has rebooted and that nobody has logged on to: it stays Unreachable until \
someone does. A console window may show for a moment as it starts.
- A Server whose identity key is in Credential Manager reads it as this user. One that cannot \
fails Serving where you can see it rather than making a new one (ADR-0050).";

/// A Task's name is the machine's rather than the user's, so it names both,
/// which keeps one user's Channel from colliding with another's.
fn task_name(channel: &str, user: &str) -> String {
    let user = user.rsplit('\\').next().unwrap_or(user);
    let user = user
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("suru-server-{channel} ({user})")
}

/// `word` as `CommandLineToArgvW` reads one back: in double quotes, a `"` is
/// written `\"`, and any backslashes before a `"` or the closing quote are
/// doubled.
fn argv_word(word: &str) -> String {
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in word.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

/// The Task that runs `keep`, at `user`'s logon alone, in `working_directory`.
/// Its trigger names `user` because a Task any user's logon starts needs
/// elevation; and it is `LeastPrivilege` because that is all it has.
fn task_definition(keep: &KeepCommand, user: &str, working_directory: &str) -> String {
    let mut words = keep.words();
    let command = words.next().unwrap_or_default();
    let arguments = words.map(argv_word).collect::<Vec<_>>().join(" ");
    let user = xml_text(user);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Suru Keeper: starts this Channel's Server at logon, and restarts it if it crashes</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{working_directory}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        command = xml_text(command),
        arguments = xml_text(&arguments),
        working_directory = xml_text(working_directory),
    )
}

/// `definition` as a Task file: UTF-16, as it says it is, whatever the paths in
/// it are written in, and with the mark `schtasks` looks for.
fn task_file(definition: &str) -> Vec<u8> {
    std::iter::once(0xFEFF)
        .chain(definition.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn user(login_start: &LoginStart) -> Result<&str> {
    login_start
        .places
        .user
        .as_deref()
        .context("determine the current Windows user: USERNAME is not set")
}

pub(super) fn register(login_start: &LoginStart, keep: &KeepCommand) -> Result<String> {
    let user = user(login_start)?;
    let name = task_name(&login_start.channel, user);
    let definition = task_definition(keep, user, &login_start.places.home.to_string_lossy());
    let file = tempfile::Builder::new()
        .prefix("suru-login-start-")
        .suffix(".xml")
        .tempfile()
        .context("create the Task's definition")?;
    std::fs::write(file.path(), task_file(&definition)).context("write the Task's definition")?;
    let created = login_start.commands.run(
        SCHTASKS,
        &[
            "/Create".into(),
            "/TN".into(),
            (&name).into(),
            "/XML".into(),
            file.path().into(),
            "/F".into(),
        ],
    );
    match created {
        Ok(outcome) if outcome.success => Ok(format!("Scheduled Task \"{name}\"")),
        Ok(outcome) => bail!("could not register a start at login: {}", outcome.said),
        Err(error) => bail!(
            "could not register a start at login: {SCHTASKS} could not be run ({error}). \
             Nothing was registered."
        ),
    }
}

pub(super) fn remove(login_start: &LoginStart) -> Result<Removal> {
    let name = task_name(&login_start.channel, user(login_start)?);
    match standing(login_start) {
        Standing::NotRegistered => return Ok(Removal::NothingRegistered),
        Standing::Unknown(reason) => return Ok(Removal::NoServiceManager(reason)),
        Standing::Registered(_) => {}
    }
    let deleted = login_start.commands.run(
        SCHTASKS,
        &["/Delete".into(), "/TN".into(), (&name).into(), "/F".into()],
    )?;
    if !deleted.success {
        bail!(
            "could not remove Scheduled Task \"{name}\": {}",
            deleted.said
        );
    }
    Ok(Removal::Removed(format!("Scheduled Task \"{name}\"")))
}

/// Told by the exit status alone, since what `schtasks` says is in the
/// language of the machine.
pub(super) fn standing(login_start: &LoginStart) -> Standing {
    let user = match user(login_start) {
        Ok(user) => user,
        Err(error) => return Standing::Unknown(format!("{error:#}")),
    };
    let name = task_name(&login_start.channel, user);
    let queried = login_start
        .commands
        .run(SCHTASKS, &["/Query".into(), "/TN".into(), (&name).into()]);
    match queried {
        Ok(outcome) if outcome.success => {
            Standing::Registered(format!("Scheduled Task \"{name}\""))
        }
        Ok(_) => Standing::NotRegistered,
        Err(error) => Standing::Unknown(format!("{SCHTASKS} could not be run: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        Platform,
        test_support::{RecordingCommands, awkward_keep, login_start},
    };
    use super::*;

    const USER: &str = r"HOST\jo";

    #[test]
    fn the_task_runs_the_keeper_at_this_users_logon_alone_with_no_elevation() {
        let task = task_definition(&awkward_keep(), USER, r"C:\Users\jo");
        assert!(task.contains("<LogonTrigger>"), "{task}");
        assert_eq!(
            task.matches(r"<UserId>HOST\jo</UserId>").count(),
            2,
            "{task}"
        );
        assert!(
            task.contains("<LogonType>InteractiveToken</LogonType>"),
            "{task}"
        );
        assert!(
            task.contains("<RunLevel>LeastPrivilege</RunLevel>"),
            "{task}"
        );
        assert!(
            task.contains(r"<WorkingDirectory>C:\Users\jo</WorkingDirectory>"),
            "{task}"
        );
    }

    /// Each of these defaults would end the Keeper, or starve its Server.
    #[test]
    fn the_task_overrides_each_default_that_would_undo_a_keeper() {
        let task = task_definition(&awkward_keep(), USER, r"C:\Users\jo");
        for said in [
            "<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>",
            "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>",
            "<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>",
            "<StopOnIdleEnd>false</StopOnIdleEnd>",
            "<Priority>4</Priority>",
            "<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>",
        ] {
            assert!(task.contains(said), "{said} not in {task}");
        }
        assert!(!task.contains("RestartOnFailure"), "{task}");
    }

    #[test]
    fn a_word_is_quoted_so_a_program_reads_it_back_as_it_was() {
        assert_eq!(argv_word("plain"), "\"plain\"");
        assert_eq!(argv_word(r"C:\Program Files\x"), r#""C:\Program Files\x""#);
        // A backslash means anything only just before a quote.
        assert_eq!(argv_word(r"C:\dir\"), r#""C:\dir\\""#);
        assert_eq!(argv_word(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(argv_word(r#"back\"slash"#), r#""back\\\"slash""#);
        assert_eq!(argv_word(""), "\"\"");
    }

    #[test]
    fn an_awkward_path_is_quoted_and_escaped_in_the_definition() {
        let task = task_definition(&awkward_keep(), USER, r"C:\Users\jo & co");
        // The program is its own element, in need of no quoting but XML's.
        assert!(
            task.contains("<Command>/Users/jo smith/R&amp;D &lt;x&gt;/suru</Command>"),
            "{task}"
        );
        assert!(
            task.contains(
                "<Arguments>\"__keep-server\" \"--state-dir\" \
                 \"/Users/jo smith/100%/$HOME/\\\"quoted\\\"\\state\" \"--env\" \
                 \"PATH=/a b/bin:/usr/bin\"</Arguments>"
            ),
            "{task}"
        );
        assert!(task.contains(r"C:\Users\jo &amp; co"), "{task}");
    }

    /// `schtasks` refuses a definition whose bytes are not what its declaration
    /// says, and a path may hold anything.
    #[test]
    fn the_definition_is_utf_16_as_it_declares() {
        let file = task_file("<?xml encoding=\"UTF-16\"?><Task>é</Task>");
        assert_eq!(&file[..2], &[0xFF, 0xFE]);
        let read = file[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect::<Vec<_>>();
        assert_eq!(
            String::from_utf16(&read).unwrap(),
            "<?xml encoding=\"UTF-16\"?><Task>é</Task>"
        );
    }

    #[test]
    fn a_tasks_name_is_kept_apart_by_channel_and_by_user() {
        assert_eq!(task_name("beta", USER), "suru-server-beta (jo)");
        assert_eq!(task_name("release", USER), "suru-server-release (jo)");
        assert_eq!(task_name("beta", r"HOST\sam"), "suru-server-beta (sam)");
        assert_eq!(task_name("beta", "jo smith"), "suru-server-beta (jo_smith)");
    }

    #[test]
    fn registering_creates_over_any_earlier_task_and_never_runs_it() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        let beta = login_start(Platform::TaskScheduler, "beta", home.path(), &commands);

        register(&beta, &awkward_keep()).unwrap();
        register(&beta, &awkward_keep()).unwrap();
        for command in commands.ran() {
            assert_eq!(command[0], "schtasks");
            assert_eq!(command[1], "/Create");
            assert_eq!(&command[2..4], ["/TN", "suru-server-beta (jo)"]);
            assert_eq!(command[4], "/XML");
            assert_eq!(
                command[6], "/F",
                "overwrites, so a second install is harmless"
            );
            assert!(!command.iter().any(|word| word == "/Run"), "{command:?}");
        }
    }

    #[test]
    fn removing_asks_first_so_doing_it_twice_is_harmless() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        let beta = login_start(Platform::TaskScheduler, "beta", home.path(), &commands);

        assert!(matches!(remove(&beta).unwrap(), Removal::Removed(_)));
        let ran = commands.ran();
        assert_eq!(ran[0][1], "/Query");
        assert_eq!(ran[1][1], "/Delete");
        assert_eq!(&ran[1][2..], ["/TN", "suru-server-beta (jo)", "/F"]);
        assert!(!ran.iter().flatten().any(|word| word == "/End"), "{ran:?}");

        commands.refuse("/Query");
        assert_eq!(remove(&beta).unwrap(), Removal::NothingRegistered);
        assert_eq!(
            commands.ran().len(),
            3,
            "nothing is deleted when nothing is there"
        );
    }

    #[test]
    fn with_no_schtasks_nothing_is_claimed_and_nothing_crashes() {
        let home = tempfile::tempdir().unwrap();
        let commands = RecordingCommands::default();
        commands.go_missing();
        let beta = login_start(Platform::TaskScheduler, "beta", home.path(), &commands);

        assert!(matches!(beta.standing(), Standing::Unknown(_)));
        assert!(matches!(
            remove(&beta).unwrap(),
            Removal::NoServiceManager(_)
        ));
        let error = register(&beta, &awkward_keep()).unwrap_err().to_string();
        assert!(error.contains("could not be run"), "{error}");
    }
}
