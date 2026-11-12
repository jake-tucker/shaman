//! A Login start on Windows: a Scheduled Task at logon, for this user alone.
//!
//! It is registered with `schtasks /Create /XML`, which alone can say all a Task
//! here must: a `/TR` command line holds at most 261 characters, and `/SC
//! ONLOGON` asks for elevation, because with no user named on its trigger it
//! would run at anybody's logon. It is never run on registering. Deleting it
//! should end at most the Keeper it started, the Server being launched detached
//! from that as from any process that launches one; whether Task Scheduler holds
//! to that as a Task is ended or deleted has yet to be seen on a real one
//! (`docs/validation/0608-login-start.md`).
//!
//! A Task's defaults would quietly undo a Keeper, so each is said here: it would
//! be ended after three days, and on going onto battery, and be run at the low
//! priority its Server and Providers would then inherit. It is `InteractiveToken`
//! because any other logon type has no way to read the user's Credential
//! Manager, where the Server's identity key is (ADR-0050). No restart setting is
//! given: the Keeper is what restarts a crashed Server, and a Task run again
//! would take a Manual stop as already seen.

use std::{io, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, bail};

use super::{KeepCommand, Removal, ServiceManager, ServiceManagerCommands, Standing, xml_text};

const SCHTASKS: &str = "schtasks";

const COVERS: &str = "\
What this covers on Windows:
- A logon by this user.
- Not a machine that has rebooted and that nobody has logged on to: it stays unreachable until \
someone does. A console window may show for a moment as it starts.
- A server whose identity key is in Credential Manager reads it as this user. One that cannot \
fails serving where you can see it rather than making a new one.";

/// A Task's name is the machine's rather than the user's, so it names both, the
/// user's domain included, which keeps one user's Channel from colliding with
/// another's. Its trigger goes on naming `DOMAIN\\user` as Windows does: only the
/// name of the Task is spelled otherwise, a `\\` there meaning a folder.
///
/// So a machine renamed goes by another name here, and its Task is left behind
/// under the old one, which `status` and `uninstall` no longer find.
fn task_name(channel: &str, user: &str) -> String {
    let user = match user.split_once('\\') {
        Some((domain, user)) => format!("{user}@{domain}"),
        None => user.to_owned(),
    };
    let user = user
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '@') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("shaman-server-{channel} ({user})")
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
    <Description>Shaman keeper: starts this channel's server at logon, and restarts it if it crashes</Description>
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

/// A Channel's Scheduled Task, for one user.
pub(super) struct ScheduledTask {
    pub(super) channel: String,
    /// The user it is for, as `DOMAIN\user`.
    pub(super) user: String,
    pub(super) home: PathBuf,
    pub(super) commands: Arc<dyn ServiceManagerCommands>,
}

/// What Task Scheduler answered about a Task.
enum Queried {
    Found,
    Gone,
    /// `schtasks` ran and did not answer, for this reason.
    Unanswered(String),
    /// `schtasks` could not be run at all, for this reason.
    NotThere(String),
}

impl ScheduledTask {
    fn name(&self) -> String {
        task_name(&self.channel, &self.user)
    }

    fn location(&self) -> String {
        format!("Scheduled Task \"{}\"", self.name())
    }

    fn query(&self, arguments: &[&str]) -> io::Result<super::CommandOutcome> {
        let arguments = arguments.iter().map(Into::into).collect::<Vec<_>>();
        self.commands.run(SCHTASKS, &arguments)
    }

    /// Whether the Task is there. `schtasks` says the same of a Task it cannot
    /// find as of any other trouble, in the language of the machine and with the
    /// one exit status, so a query for the Task failing says nothing alone: it
    /// is gone only where a listing of every Task, which is what shows Task
    /// Scheduler answering at all, comes back without it.
    fn queried(&self) -> Queried {
        let name = self.name();
        match self.query(&["/Query", "/TN", &name]) {
            Ok(outcome) if outcome.success => return Queried::Found,
            Ok(_) => {}
            Err(error) => {
                return Queried::NotThere(format!("{SCHTASKS} could not be run: {error}"));
            }
        }
        match self.query(&["/Query", "/FO", "CSV", "/NH"]) {
            Ok(listing) if listing.success => {
                if listing.printed.contains(&format!("\"\\{name}\"")) {
                    Queried::Found
                } else {
                    Queried::Gone
                }
            }
            Ok(listing) => {
                Queried::Unanswered(format!("{SCHTASKS} did not answer: {}", listing.said))
            }
            Err(error) => Queried::NotThere(format!("{SCHTASKS} could not be run: {error}")),
        }
    }
}

impl ServiceManager for ScheduledTask {
    fn register(&self, keep: &KeepCommand) -> Result<String> {
        let name = self.name();
        let definition = task_definition(keep, &self.user, &self.home.to_string_lossy());
        let file = tempfile::Builder::new()
            .prefix("shaman-login-start-")
            .suffix(".xml")
            .tempfile()
            .context("create the Task's definition")?;
        std::fs::write(file.path(), task_file(&definition))
            .context("write the Task's definition")?;
        let created = self.commands.run(
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
            Ok(outcome) if outcome.success => Ok(self.location()),
            Ok(outcome) => bail!("could not register a login start: {}", outcome.said),
            Err(error) => bail!(
                "could not register a login start: {SCHTASKS} could not be run ({error}). \
                 Nothing was registered."
            ),
        }
    }

    fn remove(&self) -> Result<Removal> {
        match self.queried() {
            Queried::Gone => return Ok(Removal::NothingRegistered),
            Queried::NotThere(reason) => return Ok(Removal::NoServiceManager { reason }),
            Queried::Unanswered(reason) => {
                bail!("could not tell whether a login start is registered: {reason}")
            }
            Queried::Found => {}
        }
        let name = self.name();
        let deleted = self.query(&["/Delete", "/TN", &name, "/F"])?;
        if !deleted.success {
            bail!(
                "could not remove Scheduled Task \"{name}\": {}",
                deleted.said
            );
        }
        Ok(Removal::Removed {
            location: self.location(),
        })
    }

    fn standing(&self) -> Standing {
        match self.queried() {
            Queried::Found => Standing::Registered(self.location()),
            Queried::Gone => Standing::NotRegistered,
            Queried::Unanswered(reason) | Queried::NotThere(reason) => Standing::Unknown(reason),
        }
    }

    fn covers(&self) -> String {
        COVERS.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{RecordingCommands, awkward_keep};
    use super::*;

    fn scheduled_task(channel: &str, commands: &RecordingCommands) -> ScheduledTask {
        ScheduledTask {
            channel: channel.to_owned(),
            user: USER.to_owned(),
            home: PathBuf::from(r"C:\Users\jo"),
            commands: commands.shared(),
        }
    }

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
            task.contains("<Command>/Users/jo smith/R&amp;D &lt;x&gt;/shaman</Command>"),
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
    fn a_tasks_name_is_kept_apart_by_channel_by_user_and_by_domain() {
        assert_eq!(task_name("beta", USER), "shaman-server-beta (jo@HOST)");
        assert_eq!(
            task_name("release", USER),
            "shaman-server-release (jo@HOST)"
        );
        assert_eq!(
            task_name("beta", r"HOST\sam"),
            "shaman-server-beta (sam@HOST)"
        );
        assert_eq!(
            task_name("beta", r"OTHER\jo"),
            "shaman-server-beta (jo@OTHER)"
        );
        assert_eq!(
            task_name("beta", r"HOST\jo smith"),
            "shaman-server-beta (jo_smith@HOST)"
        );
        assert_eq!(task_name("beta", "jo"), "shaman-server-beta (jo)");
    }

    /// A Task's name carries its domain another way; what it runs as does not.
    #[test]
    fn a_tasks_trigger_goes_on_naming_the_user_as_windows_does() {
        let task = task_definition(&awkward_keep(), r"OTHER\jo", r"C:\Users\jo");
        assert_eq!(
            task.matches(r"<UserId>OTHER\jo</UserId>").count(),
            2,
            "{task}"
        );
    }

    #[test]
    fn registering_creates_over_any_earlier_task_and_never_runs_it() {
        let commands = RecordingCommands::default();
        let beta = scheduled_task("beta", &commands);

        beta.register(&awkward_keep()).unwrap();
        beta.register(&awkward_keep()).unwrap();
        for command in commands.ran() {
            assert_eq!(command[0], "schtasks");
            assert_eq!(command[1], "/Create");
            assert_eq!(&command[2..4], ["/TN", "shaman-server-beta (jo@HOST)"]);
            assert_eq!(command[4], "/XML");
            assert_eq!(
                command[6], "/F",
                "overwrites, so a second install is harmless"
            );
            assert!(!command.iter().any(|word| word == "/Run"), "{command:?}");
        }
    }

    #[test]
    fn removing_finds_the_task_first_so_doing_it_twice_is_harmless() {
        let commands = RecordingCommands::default();
        let beta = scheduled_task("beta", &commands);

        assert!(matches!(beta.remove().unwrap(), Removal::Removed { .. }));
        let ran = commands.ran();
        assert_eq!(ran[0][1], "/Query");
        assert_eq!(ran[1][1], "/Delete");
        assert_eq!(&ran[1][2..], ["/TN", "shaman-server-beta (jo@HOST)", "/F"]);
        assert!(!ran.iter().flatten().any(|word| word == "/End"), "{ran:?}");

        commands.refuse("/TN");
        assert_eq!(beta.remove().unwrap(), Removal::NothingRegistered);
        assert!(
            !commands.ran()[2..]
                .iter()
                .flatten()
                .any(|word| word == "/Delete"),
            "nothing is deleted when nothing is there"
        );
    }

    /// `schtasks` fails alike for a Task that is not there and for any trouble,
    /// and a Keeper reading "not registered" ends for good: so only a listing
    /// that came back without the Task means it is gone.
    #[test]
    fn a_query_failing_alone_does_not_make_a_task_gone() {
        let commands = RecordingCommands::default();
        let beta = scheduled_task("beta", &commands);

        commands.refuse("/TN");
        assert_eq!(beta.standing(), Standing::NotRegistered);

        commands.print("\"\\other task\",\"N/A\",\"Ready\"\r\n");
        assert_eq!(beta.standing(), Standing::NotRegistered);

        commands.print("\"\\shaman-server-beta (jo@HOST)\",\"N/A\",\"Running\"\r\n");
        assert!(matches!(beta.standing(), Standing::Registered(_)));

        commands.refuse("/FO");
        assert!(matches!(beta.standing(), Standing::Unknown(_)));
        let error = beta.remove().unwrap_err().to_string();
        assert!(error.contains("could not tell"), "{error}");
    }

    #[test]
    fn with_no_schtasks_nothing_is_claimed_and_nothing_crashes() {
        let commands = RecordingCommands::default();
        commands.go_missing();
        let beta = scheduled_task("beta", &commands);

        assert!(matches!(beta.standing(), Standing::Unknown(_)));
        assert!(matches!(
            beta.remove().unwrap(),
            Removal::NoServiceManager { .. }
        ));
        let error = beta.register(&awkward_keep()).unwrap_err().to_string();
        assert!(error.contains("could not be run"), "{error}");
    }
}
