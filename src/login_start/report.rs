//! What `shaman server install` and `uninstall` tell their user.

use std::path::Path;

use super::{LoginStart, Removal, ShortLived};

/// What `shaman server install` found worth telling its user.
pub(super) struct InstallFindings<'a> {
    pub(super) executable: &'a Path,
    pub(super) short_lived: Option<ShortLived>,
    pub(super) uncarried: Vec<String>,
    pub(super) fleeting_path_entries: Vec<String>,
}

pub(super) fn install_report(
    login_start: &LoginStart,
    location: &str,
    findings: &InstallFindings,
) -> String {
    let channel = &login_start.channel;
    let mut report = format!(
        "Registered a login start for the `{channel}` channel's server: {location}\n\n\
         It comes into force at the next login and changes nothing until then: a server \
         running now is left running, and `shaman server start` starts one where none is.\n\
         At each login a keeper starts the server and restarts it if it crashes. A server \
         stopped by `shaman server stop` stays stopped until the next login or `shaman server \
         start`. So does one ended by a plain `kill`, which a server takes as a stop.\n\n{}\n\n\
         `shaman server status` reads the registration alone, and cannot see it turned off \
         in the service manager itself.\n\n\
         Providers are found on the PATH this command ran with, which the registration \
         keeps. Run `shaman server install` again after it changes.\n",
        login_start.manager.covers()
    );
    if !findings.fleeting_path_entries.is_empty() {
        report.push_str(&format!(
            "These PATH entries may not be there after a reboot, so a provider found only \
             through one will not be found: {}\n",
            findings.fleeting_path_entries.join(", ")
        ));
    }
    if !findings.uncarried.is_empty() {
        report.push_str(&format!(
            "Not carried, because they may hold secrets or belong to this shell: {}. A server \
             started at login does not have them, so a provider or relay relying on one needs \
             another way in.\n",
            findings.uncarried.join(", ")
        ));
    }
    if let Some(short_lived) = findings.short_lived {
        report.push_str(&format!(
            "\nWarning: this registration names {}, which may not survive an upgrade: {}. \
             Run `shaman server install` again from wherever Shaman is kept for good.\n",
            findings.executable.display(),
            short_lived.says()
        ));
    }
    report
}

pub(super) fn uninstall_report(channel: &str, removal: &Removal) -> String {
    const LEFT_ALONE: &str = "A server running now is left running; `shaman server stop` stops it.";
    match removal {
        Removal::Removed { location } => format!(
            "Removed the login start for the `{channel}` channel's server: {location}\n\
             {LEFT_ALONE}\n"
        ),
        Removal::RemovedUnanswered { location, reason } => format!(
            "Removed the login start for the `{channel}` channel's server: {location}\n\
             The service manager did not answer, so it reads this at its next start: {reason}\n\
             {LEFT_ALONE}\n"
        ),
        Removal::NothingRegistered => format!(
            "No login start is registered for the `{channel}` channel, so there is nothing \
             to remove.\n"
        ),
        Removal::NoServiceManager { reason } => format!(
            "No login start is registered for the `{channel}` channel, and this machine has \
             no service manager to hold one: {reason}\n"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        ServiceManager, launchd::LaunchAgent, systemd::UserUnit, task_scheduler::ScheduledTask,
        test_support::RecordingCommands,
    };
    use super::*;
    use std::path::PathBuf;

    fn rooted(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        } else {
            PathBuf::from(path)
        }
    }

    fn login_start() -> LoginStart {
        LoginStart {
            channel: "beta".to_owned(),
            manager: Box::new(LaunchAgent {
                channel: "beta".to_owned(),
                home: rooted("/home/jo"),
            }),
        }
    }

    #[test]
    fn the_install_report_says_what_was_done_and_warns_of_what_will_not_last() {
        let login_start = login_start();
        let executable = rooted("/home/jo/target/debug/shaman");

        let report = install_report(
            &login_start,
            "somewhere",
            &InstallFindings {
                executable: &executable,
                short_lived: Some(ShortLived::Build),
                uncarried: vec!["ANTHROPIC_API_KEY".to_owned()],
                fleeting_path_entries: vec!["/run/user/1000/fnm/bin".to_owned()],
            },
        );
        assert!(
            report.contains("`beta` channel's server: somewhere"),
            "{report}"
        );
        assert!(report.contains("ANTHROPIC_API_KEY"), "{report}");
        assert!(report.contains("/run/user/1000/fnm/bin"), "{report}");
        assert!(report.contains("after a reboot"), "{report}");
        assert!(report.contains("Warning"), "{report}");
        assert!(report.contains("build directory"), "{report}");

        let report = install_report(
            &login_start,
            "somewhere",
            &InstallFindings {
                executable: &executable,
                short_lived: None,
                uncarried: Vec::new(),
                fleeting_path_entries: Vec::new(),
            },
        );
        assert!(!report.contains("Warning"), "{report}");
        assert!(!report.contains("Not carried"), "{report}");
        assert!(!report.contains("after a reboot"), "{report}");
    }

    /// Every platform's output says what it covers, including that a Mac waits
    /// for its login keychain and what Linux needs to run with no session.
    #[test]
    fn each_platform_says_what_its_registration_covers_and_what_it_does_not() {
        let home = rooted("/home/jo");
        let commands = RecordingCommands::default();
        let mac = LaunchAgent {
            channel: "beta".to_owned(),
            home: home.clone(),
        }
        .covers();
        assert!(mac.contains("login keychain"), "{mac}");
        assert!(mac.contains("consent prompt"), "{mac}");
        let linux = UserUnit {
            channel: "beta".to_owned(),
            home: home.clone(),
            config_home: home.clone(),
            commands: commands.shared(),
        }
        .covers();
        assert!(linux.contains("loginctl enable-linger"), "{linux}");
        let windows = ScheduledTask {
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
