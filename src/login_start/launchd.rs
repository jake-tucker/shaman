//! A Login start on macOS: a LaunchAgent in the user's own
//! `~/Library/LaunchAgents`.
//!
//! launchd reads that directory at each login at the Mac's screen, so writing
//! the file is the whole of registering and removing it the whole of
//! unregistering, and no `launchctl` is run. That is deliberate:
//!
//! - `launchctl bootstrap` would run the job now, since `RunAtLoad` means run on
//!   loading, which would start a Server its user may have just stopped, and
//!   needs a login at the screen to exist at all, which a Mac reached over SSH
//!   does not have.
//! - `launchctl bootout` would end the job now, and takes what is still in the
//!   job's process group with it, when uninstalling must leave a running Server
//!   alone.
//!
//! A job already loaded from this login stays loaded until it ends, which it
//! does at logout, or sooner: a Keeper finding its registration gone launches
//! nothing further and ends.

use std::path::PathBuf;

use anyhow::Result;

use super::{
    KeepCommand, Removal, ServiceManager, Standing, file_standing, remove_file, write_registration,
    xml_text,
};

const COVERS: &str = "\
What this covers on macOS:
- A login at this Mac's screen, automatic login included.
- Not a Mac that has rebooted and that nobody has logged in to, nor a login over SSH alone: \
those stay unreachable. The server's identity key is in the login keychain, which that login \
unlocks, so a Mac waits for it, and a server that cannot read its key fails serving \
where you can see it rather than making a new one.
- macOS may list this under Login Items & Extensions in System Settings. Turned off there, it \
does not run.
- To macOS's privacy checks, a server started this way may count as its own program rather than \
as part of your terminal. Work in Documents, Desktop or Downloads may raise a consent prompt that \
waits for someone at the screen.";

fn label(channel: &str) -> String {
    format!("ai.shaman.server.{channel}")
}

/// The LaunchAgent that runs `keep` in `working_directory`.
///
/// It has no `KeepAlive`: the Keeper is what restarts a crashed Server, and a
/// Keeper launchd started again after a stop would read that stop as already
/// seen, and start the Server its user had stopped. `AbandonProcessGroup` is
/// belt and braces, the Server having a process group of its own: launchd must
/// take nothing with the Keeper when it ends. No `StandardOutPath` is named, so
/// the Keeper's stdout and stderr go nowhere, its Log being a file of its own
/// that is pruned as every Log is (ADR 0008), and no file grows without bound.
/// No `EnvironmentVariables`: all that is carried is on the command line.
fn plist(channel: &str, keep: &KeepCommand, working_directory: &str) -> String {
    let arguments = keep
        .words()
        .map(|word| format!("    <string>{}</string>\n", xml_text(word)))
        .collect::<String>();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{arguments}  </array>
  <key>WorkingDirectory</key>
  <string>{working_directory}</string>
  <key>RunAtLoad</key>
  <true/>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>AbandonProcessGroup</key>
  <true/>
</dict>
</plist>
"#,
        label = label(channel),
        working_directory = xml_text(working_directory),
    )
}

/// A Channel's LaunchAgent, which needs no command run: the file is all of it.
pub(super) struct LaunchAgent {
    pub(super) channel: String,
    pub(super) home: PathBuf,
}

impl LaunchAgent {
    fn path(&self) -> PathBuf {
        self.home
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{}.plist", label(&self.channel)))
    }
}

impl ServiceManager for LaunchAgent {
    fn register(&self, keep: &KeepCommand) -> Result<String> {
        let path = self.path();
        let contents = plist(&self.channel, keep, &self.home.to_string_lossy());
        write_registration(&path, contents.as_bytes())?;
        Ok(path.display().to_string())
    }

    fn remove(&self) -> Result<Removal> {
        let path = self.path();
        Ok(if remove_file(&path)? {
            Removal::Removed {
                location: path.display().to_string(),
            }
        } else {
            Removal::NothingRegistered
        })
    }

    fn standing(&self) -> Standing {
        file_standing(&self.path(), &self.home)
    }

    fn covers(&self) -> String {
        COVERS.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::awkward_keep;
    use super::*;

    fn agent(channel: &str, home: &std::path::Path) -> LaunchAgent {
        LaunchAgent {
            channel: channel.to_owned(),
            home: home.to_path_buf(),
        }
    }

    #[test]
    fn the_agent_runs_the_keeper_once_at_each_login_at_the_screen() {
        let plist = plist("beta", &awkward_keep(), "/Users/jo smith");
        assert!(
            plist.contains("<string>ai.shaman.server.beta</string>"),
            "{plist}"
        );
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"), "{plist}");
        assert!(plist.contains("<string>Aqua</string>"), "{plist}");
        assert!(
            plist.contains("<key>AbandonProcessGroup</key>\n  <true/>"),
            "{plist}"
        );
        assert!(
            plist.contains("<key>WorkingDirectory</key>\n  <string>/Users/jo smith</string>"),
            "{plist}"
        );
    }

    /// launchd restarting anything would read a Manual stop as already seen,
    /// and its output must go nowhere that grows.
    #[test]
    fn the_agent_asks_launchd_to_restart_nothing_and_keep_no_file() {
        let plist = plist("beta", &awkward_keep(), "/Users/jo");
        for absent in [
            "KeepAlive",
            "StandardOutPath",
            "StandardErrorPath",
            "EnvironmentVariables",
        ] {
            assert!(!plist.contains(absent), "{absent} in {plist}");
        }
    }

    #[test]
    fn an_awkward_path_is_escaped_so_it_reads_back_as_it_was() {
        let plist = plist("beta", &awkward_keep(), "/Users/jo & co");
        assert!(
            plist.contains("<string>/Users/jo smith/R&amp;D &lt;x&gt;/shaman</string>"),
            "{plist}"
        );
        // Neither `%` nor `$` means anything to launchd, so both are left as they are.
        assert!(
            plist.contains("<string>/Users/jo smith/100%/$HOME/\"quoted\"\\state</string>"),
            "{plist}"
        );
        assert!(
            plist.contains("<string>PATH=/a b/bin:/usr/bin</string>"),
            "{plist}"
        );
        assert!(plist.contains("/Users/jo &amp; co"), "{plist}");
    }

    #[test]
    fn doing_either_twice_is_harmless() {
        let home = tempfile::tempdir().unwrap();
        let beta = agent("beta", home.path());
        let path = home
            .path()
            .join("Library")
            .join("LaunchAgents")
            .join("ai.shaman.server.beta.plist");

        assert_eq!(beta.standing(), Standing::NotRegistered);
        assert_eq!(beta.remove().unwrap(), Removal::NothingRegistered);

        beta.register(&awkward_keep()).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains("100%"));
        assert_eq!(
            beta.standing(),
            Standing::Registered(path.display().to_string())
        );

        let mut moved = awkward_keep();
        moved.program = "/opt/moved/shaman".to_owned();
        beta.register(&moved).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert!(second.contains("/opt/moved/shaman"), "{second}");
        assert!(!second.contains("R&amp;D"), "{second}");

        assert_eq!(
            beta.remove().unwrap(),
            Removal::Removed {
                location: path.display().to_string()
            }
        );
        assert_eq!(beta.remove().unwrap(), Removal::NothingRegistered);
        assert_eq!(beta.standing(), Standing::NotRegistered);
    }

    #[test]
    fn a_channels_agent_leaves_another_channels_alone() {
        let home = tempfile::tempdir().unwrap();
        let beta = agent("beta", home.path());
        let release = agent("release", home.path());

        beta.register(&awkward_keep()).unwrap();
        assert_eq!(release.standing(), Standing::NotRegistered);
        release.register(&awkward_keep()).unwrap();
        beta.remove().unwrap();
        assert!(matches!(release.standing(), Standing::Registered(_)));
        assert_eq!(beta.standing(), Standing::NotRegistered);
    }

    #[cfg(unix)]
    #[test]
    fn nobody_else_can_write_to_the_agent_or_launchd_would_refuse_it() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().unwrap();
        let beta = agent("beta", home.path());
        beta.register(&awkward_keep()).unwrap();
        let mode = std::fs::metadata(beta.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }
}
