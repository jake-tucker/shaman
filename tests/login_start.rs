//! `suru server install`, `uninstall` and `status`, and the Keeper a Login start
//! runs, through the real binary and against real Servers.
//!
//! Nothing here reaches the real user's launchd, systemd or Task Scheduler. Each
//! command runs with its home pointed at a temporary directory, and with
//! `--service-manager` naming a stand-in in place of `systemctl` or `schtasks`:
//! Windows looks in its system directory for a program whatever `PATH` says, so
//! an empty `PATH` would not be enough. launchd is never run at all. What is
//! generated for each platform is tested on every platform beside the generator,
//! in `src/login_start`.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use suru::{
    login_start::{KeeperArguments, ServiceManagerStandIn},
    managed_client::{ManagedClient, ManagedEvent},
    protocol::{RuntimeDescriptor, ShutdownReason},
};
use tokio::{
    process::{Child, Command},
    time::{Duration, timeout},
};
use uuid::Uuid;

use crate::support::{
    PROGRESS_DEADLINE, detached_servers::DetachedServers, read_runtime_descriptor,
};

/// How often a Keeper under test looks at its Channel.
const LOOK_INTERVAL_MS: u64 = 20;

/// Enough looks for a Keeper to have done anything it was going to: a launch
/// takes two looks to come, and a launched Server moments to take the election,
/// which any Server launched against the Channel would then show.
const LONG_ENOUGH_TO_HAVE_ACTED: Duration = Duration::from_millis(40 * LOOK_INTERVAL_MS);

/// A machine for a test: an isolated home and a stand-in service manager beside
/// the state directory every Server is launched against.
struct Machine {
    // Declared first, so what it launched is gone before the home is.
    servers: DetachedServers,
    home: tempfile::TempDir,
    stand_in: PathBuf,
}

impl Machine {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("create an isolated home");
        let stand_in = write_stand_in_service_manager(home.path());
        Self {
            servers: DetachedServers::new(),
            home,
            stand_in,
        }
    }

    /// What `XDG_CONFIG_HOME` is for this machine's processes.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn config_home(&self) -> PathBuf {
        self.home.path().join(".config")
    }

    /// A process isolated to this machine's home, with all a program needs on
    /// the platform it runs on to run at all, and nothing a shell would add.
    fn sparse(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        crate::support::detached_servers::isolated_command(program, self.home.path())
    }

    /// `suru server <subcommand>` for `channel`, as a user's shell runs it.
    fn server_cli(&self, channel: &str, subcommand: &[&str]) -> Command {
        crate::cli_integration::server_cli_with_home(
            &self.servers,
            channel,
            subcommand[0],
            self.home.path(),
            &self.stand_in,
        )
    }

    async fn run(&self, channel: &str, subcommand: &[&str]) -> std::process::Output {
        timeout(
            PROGRESS_DEADLINE * 2,
            self.server_cli(channel, subcommand).output(),
        )
        .await
        .unwrap_or_else(|_| panic!("`suru server {subcommand:?}` did not settle"))
        .expect("run the server CLI")
    }

    /// Runs `suru server <subcommand>` and answers what it printed, failing the
    /// test should it fail.
    async fn succeed(&self, channel: &str, subcommand: &[&str]) -> String {
        let output = self.run(channel, subcommand).await;
        assert!(
            output.status.success(),
            "`suru server {subcommand:?}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("the CLI prints UTF-8")
    }

    /// What `suru server status` says of the Login start, the Server's own
    /// standing being none of this.
    async fn login_start_standing(&self, channel: &str) -> String {
        let output = self.run(channel, &["status"]).await;
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .find(|line| line.starts_with("Login start:"))
            .unwrap_or_else(|| panic!("status says nothing of a start at login: {output:?}"))
            .to_owned()
    }

    /// The Keeper of `channel`, run as a registration runs it: with nothing of a
    /// shell's environment but what it carries on its command line.
    fn keeper(&self, channel: &str, executable: &Path) -> Child {
        self.keeper_looking_every(channel, executable, LOOK_INTERVAL_MS)
    }

    fn keeper_looking_every(
        &self,
        channel: &str,
        executable: &Path,
        look_interval_ms: u64,
    ) -> Child {
        let arguments = KeeperArguments {
            executable: executable.to_path_buf(),
            state_base_dir: self.servers.state_dir().to_path_buf(),
            data_base_dir: self.servers.state_dir().to_path_buf(),
            config_dir: Some(self.servers.state_dir().to_path_buf()),
            channel: channel.to_owned(),
            // As every process a test runs here keeps it: no Server of its may
            // depend on a desktop session, or touch the real keyring.
            identity_store: Some(suru::IdentityStoreChoice::File),
            carried: self
                .servers
                .isolated_environment()
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.display().to_string()))
                .collect(),
            stand_in: ServiceManagerStandIn {
                program: Some(self.stand_in.clone()),
            },
            look_interval_ms: Some(look_interval_ms),
        };
        let mut command = self.sparse(env!("CARGO_BIN_EXE_suru"));
        command
            .args(arguments.words().expect("spell the Keeper's command line"))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().expect("run the Keeper")
    }

    fn descriptor_path(&self, channel: &str) -> PathBuf {
        self.servers.state_dir().join(channel).join("runtime.json")
    }

    fn last_stop_path(&self, channel: &str) -> PathBuf {
        self.servers
            .state_dir()
            .join(channel)
            .join("last-stop.json")
    }

    /// Waits for a Server other than `other` to publish itself, and to have
    /// answered as ready: the descriptor is published before that.
    async fn server_other_than(&self, channel: &str, other: Option<Uuid>) -> RuntimeDescriptor {
        let path = self.descriptor_path(channel);
        timeout(PROGRESS_DEADLINE, async {
            loop {
                if let Ok(contents) = std::fs::read(&path)
                    && let Ok(descriptor) = serde_json::from_slice::<RuntimeDescriptor>(&contents)
                    && Some(descriptor.instance_id) != other
                    && self.responds(&descriptor).await
                {
                    return descriptor;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no Server other than {other:?} came up"))
    }

    async fn responds(&self, descriptor: &RuntimeDescriptor) -> bool {
        crate::support::loopback_client()
            .get(format!("{}/health", descriptor.base_url))
            .bearer_auth(&descriptor.token)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    /// Waits until a Keeper's Log says it has seen a Server stand since the stop
    /// it followed.
    async fn keeper_has_seen_a_server_since_the_stop(&self, channel: &str) {
        let log_dir = self.servers.state_dir().join(channel).join("log");
        let says_so = || {
            std::fs::read_dir(&log_dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().contains("-keeper-"))
                .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
                .any(|log| log.contains("a server stands since the stop"))
        };
        timeout(PROGRESS_DEADLINE, async {
            while !says_so() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the Keeper sees the Server that was started");
    }

    /// Kills the Server `descriptor` names outright, and waits until it is gone,
    /// as a crash would leave things.
    async fn crash(&self, descriptor: &RuntimeDescriptor) {
        let pid = sysinfo::Pid::from_u32(descriptor.pid);
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        assert!(
            system.process(pid).expect("find the Server").kill(),
            "crash the Server"
        );
        timeout(PROGRESS_DEADLINE, async {
            while self.responds(descriptor).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the crashed Server stops answering");
    }
}

/// A stand-in for what `suru server` tells a service manager, which remembers a
/// registration the way the real one would: as the link `systemctl enable`
/// makes, which is what a Linux `status` reads, or as a Task `schtasks /Query`
/// would answer for.
fn write_stand_in_service_manager(home: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let path = home.join("stand-in-service-manager");
        std::fs::write(
            &path,
            r#"#!/bin/sh
PATH=/usr/bin:/bin
units="$XDG_CONFIG_HOME/systemd/user"
case "$2" in
  enable) mkdir -p "$units/default.target.wants" && ln -sf "../$3" "$units/default.target.wants/$3" ;;
esac
exit 0
"#,
        )
        .expect("write the stand-in");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("make the stand-in runnable");
        path
    }
    // One definition per Task, named for it, as `/TN` names it: `%~3` for each
    // of `/Create`, `/Query` and `/Delete`. A listing shows nothing, which is only
    // ever asked for once a query has found no such Task.
    #[cfg(windows)]
    {
        let path = home.join("stand-in-service-manager.cmd");
        let definition = home.join(format!("{WINDOWS_TASK_PREFIX}%~3.xml"));
        std::fs::write(
            &path,
            format!(
                "@echo off\r\n\
                 if /i \"%~1\"==\"/Create\" goto create\r\n\
                 if /i \"%~1\"==\"/Delete\" goto delete\r\n\
                 if /i \"%~2\"==\"/FO\" exit /b 0\r\n\
                 if exist \"{definition}\" exit /b 0\r\n\
                 exit /b 1\r\n\
                 :create\r\n\
                 copy /y \"%~5\" \"{definition}\" >nul\r\n\
                 exit /b 0\r\n\
                 :delete\r\n\
                 del \"{definition}\"\r\n\
                 exit /b 0\r\n",
                definition = definition.display()
            ),
        )
        .expect("write the stand-in");
        path
    }
}

/// How the Windows stand-in names what it was given to keep.
#[cfg(windows)]
const WINDOWS_TASK_PREFIX: &str = "stand-in-task-";

/// What the Task the stand-in was given runs, read back as Windows would: UTF-16,
/// XML, then a command line as a program splits it.
#[cfg(windows)]
fn registered_words(home: &Path, channel: &str) -> Vec<String> {
    let prefix = format!("{WINDOWS_TASK_PREFIX}suru-server-{channel} (");
    let definition = std::fs::read_dir(home)
        .expect("read the home")
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .expect("the Task was given to the stand-in");
    let bytes = std::fs::read(definition.path()).expect("read the Task");
    let units = bytes[2..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect::<Vec<_>>();
    let task = String::from_utf16(&units).expect("the Task is UTF-16");
    let element = |name: &str| {
        let (_, rest) = task
            .split_once(&format!("<{name}>"))
            .unwrap_or_else(|| panic!("the Task names its {name}"));
        rest.split_once(&format!("</{name}>"))
            .expect("the element ends")
            .0
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
    };
    let mut words = vec![element("Command")];
    let arguments = element("Arguments");
    let mut characters = arguments.chars().peekable();
    while let Some(opening) = characters.next() {
        assert_eq!(opening, '"', "every word is quoted: {arguments}");
        let mut word = String::new();
        let mut backslashes = 0;
        loop {
            match characters.next().expect("a word ends") {
                '\\' => backslashes += 1,
                '"' if backslashes % 2 == 1 => {
                    word.push_str(&"\\".repeat(backslashes / 2));
                    word.push('"');
                    backslashes = 0;
                }
                '"' => {
                    word.push_str(&"\\".repeat(backslashes / 2));
                    break;
                }
                other => {
                    word.push_str(&"\\".repeat(backslashes));
                    word.push(other);
                    backslashes = 0;
                }
            }
        }
        words.push(word);
        characters.next();
    }
    words
}

/// The program and arguments the registration of `channel` under `home` names,
/// read back as the service manager would read them.
#[cfg(target_os = "macos")]
fn registered_words(home: &Path, channel: &str) -> Vec<String> {
    let plist = std::fs::read_to_string(
        home.join("Library/LaunchAgents")
            .join(format!("ai.suru.server.{channel}.plist")),
    )
    .expect("read the LaunchAgent");
    let (_, arguments) = plist
        .split_once("<key>ProgramArguments</key>")
        .expect("the agent names a program");
    let (arguments, _) = arguments.split_once("</array>").expect("arguments end");
    arguments
        .split("<string>")
        .skip(1)
        .map(|word| {
            word.split_once("</string>")
                .expect("a word ends")
                .0
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&")
        })
        .collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn registered_words(home: &Path, channel: &str) -> Vec<String> {
    let unit = std::fs::read_to_string(
        home.join(".config/systemd/user")
            .join(format!("suru-server-{channel}.service")),
    )
    .expect("read the unit");
    let exec_start = unit
        .lines()
        .find_map(|line| line.strip_prefix("ExecStart="))
        .expect("the unit names a program");
    let mut words = Vec::new();
    let mut characters = exec_start.chars();
    while let Some(opening) = characters.next() {
        assert_eq!(opening, '"', "every word is quoted: {exec_start}");
        let mut word = String::new();
        loop {
            match characters.next().expect("a word ends") {
                '"' => break,
                '\\' => word.push(characters.next().expect("an escape ends")),
                '%' | '$' => word.push(characters.next().expect("a doubling ends")),
                other => word.push(other),
            }
        }
        words.push(word);
        characters.next();
    }
    words
}

/// A Claude CLI that is there and says nothing useful, so finding it can be told
/// from finding nothing, with no real Claude and no sign-in concerned.
#[cfg(unix)]
fn write_scripted_claude(bin: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(bin).expect("make the directory Claude is installed in");
    let claude = bin.join("claude");
    std::fs::write(&claude, "#!/bin/sh\necho '0.0.1 (Claude Code)'\n").expect("write the CLI");
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o700))
        .expect("make the CLI runnable");
}

/// On Windows a Provider is found on `PATH` only as an `.exe`, so a batch file will
/// not do: the stand-in for a CLI is a small program of the system's own, under
/// the name Claude's goes by.
#[cfg(windows)]
fn write_scripted_claude(bin: &Path) {
    std::fs::create_dir_all(bin).expect("make the directory Claude is installed in");
    std::fs::copy(
        windows_system_directory().join("hostname.exe"),
        bin.join("claude.exe"),
    )
    .expect("copy a program to stand in for the CLI");
}

#[cfg(windows)]
fn windows_system_directory() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").expect("Windows names its directory"))
        .join("System32")
}

/// The `PATH` of a shell that has `bin` on it.
fn shell_path(bin: &Path) -> std::ffi::OsString {
    #[cfg(unix)]
    let rest = [PathBuf::from("/usr/bin"), PathBuf::from("/bin")];
    #[cfg(windows)]
    let rest = [windows_system_directory()];
    std::env::join_paths(std::iter::once(bin.to_path_buf()).chain(rest))
        .expect("join the shell's PATH")
}

#[tokio::test]
async fn installing_and_uninstalling_are_each_harmless_twice_and_status_follows() {
    let machine = Machine::new();
    let channel = "login-start-round-trip";
    assert_eq!(
        machine.login_start_standing(channel).await,
        "Login start: not registered"
    );
    let nothing = machine.succeed(channel, &["uninstall"]).await;
    assert!(nothing.contains("nothing to remove"), "{nothing}");

    let installed = machine.succeed(channel, &["install"]).await;
    assert!(
        installed.contains(&format!("`{channel}` Channel's Server")),
        "{installed}"
    );
    assert!(installed.contains("What this covers on "), "{installed}");
    #[cfg(target_os = "macos")]
    {
        assert!(installed.contains("login keychain"), "{installed}");
        assert!(
            machine
                .home
                .path()
                .join("Library/LaunchAgents")
                .join(format!("ai.suru.server.{channel}.plist"))
                .is_file()
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        assert!(installed.contains("enable-linger"), "{installed}");
        assert!(
            machine
                .config_home()
                .join("systemd/user")
                .join(format!("suru-server-{channel}.service"))
                .is_file()
        );
    }
    let standing = machine.login_start_standing(channel).await;
    assert!(
        standing.starts_with("Login start: registered"),
        "{standing}"
    );

    machine.succeed(channel, &["install"]).await;
    let standing = machine.login_start_standing(channel).await;
    assert!(
        standing.starts_with("Login start: registered"),
        "{standing}"
    );

    let removed = machine.succeed(channel, &["uninstall"]).await;
    assert!(removed.contains("left running"), "{removed}");
    assert_eq!(
        machine.login_start_standing(channel).await,
        "Login start: not registered"
    );
    let nothing = machine.succeed(channel, &["uninstall"]).await;
    assert!(nothing.contains("nothing to remove"), "{nothing}");
}

/// The binary under test is in Cargo's target directory, which is exactly what
/// install is to warn of.
#[tokio::test]
async fn installing_from_a_build_directory_warns_that_it_will_not_survive_an_upgrade() {
    let machine = Machine::new();
    let installed = machine.succeed("login-start-warns", &["install"]).await;
    assert!(installed.contains("Warning"), "{installed}");
    assert!(installed.contains("build directory"), "{installed}");
    // As the running binary names itself, which on Linux has every link in its
    // path resolved, and a target directory may be reached through one.
    let executable = Path::new(env!("CARGO_BIN_EXE_suru"));
    let resolved = std::fs::canonicalize(executable).expect("resolve the binary's path");
    let resolved = resolved.to_string_lossy();
    assert!(
        installed.contains(&*executable.to_string_lossy())
            || installed.contains(&*resolved)
            // Windows resolves a path into its form with no length limit.
            || installed.contains(resolved.trim_start_matches(r"\\?\")),
        "{installed}"
    );
}

#[tokio::test]
async fn a_channels_registration_leaves_another_channels_alone() {
    let machine = Machine::new();
    let (beta, gamma) = ("login-start-beta", "login-start-gamma");

    machine.succeed(beta, &["install"]).await;
    assert_eq!(
        machine.login_start_standing(gamma).await,
        "Login start: not registered"
    );

    machine.succeed(gamma, &["install"]).await;
    machine.succeed(beta, &["uninstall"]).await;
    let standing = machine.login_start_standing(gamma).await;
    assert!(
        standing.starts_with("Login start: registered"),
        "{standing}"
    );
    assert!(
        registered_words(machine.home.path(), gamma)
            .iter()
            .any(|word| word == gamma),
        "the registration names its own Channel"
    );
}

#[tokio::test]
async fn neither_installing_nor_uninstalling_starts_or_stops_a_server() {
    let machine = Machine::new();
    let channel = "login-start-leaves-alone";

    machine.succeed(channel, &["install"]).await;
    machine.succeed(channel, &["uninstall"]).await;
    assert!(
        !machine.descriptor_path(channel).exists(),
        "installing started a Server"
    );

    machine.succeed(channel, &["start"]).await;
    let running = machine.server_other_than(channel, None).await;
    machine.succeed(channel, &["install"]).await;
    machine.succeed(channel, &["uninstall"]).await;
    assert!(machine.responds(&running).await, "a running Server stopped");
    assert_eq!(
        read_runtime_descriptor(machine.descriptor_path(channel)).instance_id,
        running.instance_id
    );
}

/// What a Keeper does at each login: a stop before it is one it has seen.
#[tokio::test]
async fn a_keeper_starts_the_server_though_an_earlier_login_had_stopped_it() {
    let machine = Machine::new();
    let channel = "keeper-login";
    machine.succeed(channel, &["start"]).await;
    machine.succeed(channel, &["stop"]).await;
    assert!(machine.last_stop_path(channel).exists());

    machine.succeed(channel, &["install"]).await;
    let _keeper = machine.keeper(channel, Path::new(env!("CARGO_BIN_EXE_suru")));
    machine.server_other_than(channel, None).await;
}

#[tokio::test]
async fn a_keeper_restarts_a_crashed_server() {
    let machine = Machine::new();
    let channel = "keeper-crash";
    machine.succeed(channel, &["install"]).await;
    let _keeper = machine.keeper(channel, Path::new(env!("CARGO_BIN_EXE_suru")));

    let first = machine.server_other_than(channel, None).await;
    machine.crash(&first).await;
    let second = machine
        .server_other_than(channel, Some(first.instance_id))
        .await;
    machine.crash(&second).await;
    machine
        .server_other_than(channel, Some(second.instance_id))
        .await;
}

/// The whole of what a Manual stop must do with a Keeper running: it stays
/// stopped, a start brings it back, and a crash of that one is answered.
#[tokio::test]
async fn a_stopped_server_stays_stopped_until_it_is_started_and_is_then_kept() {
    let machine = Machine::new();
    let channel = "keeper-stop";
    machine.succeed(channel, &["install"]).await;
    let mut keeper = machine.keeper(channel, Path::new(env!("CARGO_BIN_EXE_suru")));
    let first = machine.server_other_than(channel, None).await;

    machine.succeed(channel, &["stop"]).await;
    tokio::time::sleep(LONG_ENOUGH_TO_HAVE_ACTED).await;
    assert!(
        !machine.descriptor_path(channel).exists(),
        "a Keeper undid a Manual stop"
    );
    assert_eq!(machine.servers.servers_running(), 0);
    assert!(
        keeper.try_wait().expect("look at the Keeper").is_none(),
        "the Keeper goes on looking, for the Server someone starts next"
    );

    machine.succeed(channel, &["start"]).await;
    let second = machine
        .server_other_than(channel, Some(first.instance_id))
        .await;
    // It must have seen the new Server stand, or the crash reads as the stop.
    machine
        .keeper_has_seen_a_server_since_the_stop(channel)
        .await;
    machine.crash(&second).await;
    let third = machine
        .server_other_than(channel, Some(second.instance_id))
        .await;

    machine.succeed(channel, &["stop"]).await;
    tokio::time::sleep(LONG_ENOUGH_TO_HAVE_ACTED).await;
    assert!(
        !machine.responds(&third).await && !machine.descriptor_path(channel).exists(),
        "a Keeper undid the second Manual stop"
    );
}

/// A Keeper setting out beside a Client doing the same: the election yields
/// one, and the one that lost is no crash to the Keeper.
#[tokio::test]
async fn a_keeper_and_a_client_starting_together_leave_one_server() {
    let machine = Machine::new();
    let channel = "keeper-election";
    machine.succeed(channel, &["install"]).await;
    let _keeper = machine.keeper(channel, Path::new(env!("CARGO_BIN_EXE_suru")));
    machine.succeed(channel, &["start"]).await;

    let standing = machine.server_other_than(channel, None).await;
    tokio::time::sleep(LONG_ENOUGH_TO_HAVE_ACTED).await;
    assert_eq!(
        read_runtime_descriptor(machine.descriptor_path(channel)).instance_id,
        standing.instance_id,
        "the Server that won was disturbed"
    );
    timeout(PROGRESS_DEADLINE, async {
        while machine.servers.servers_running() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("only the Server that won goes on running");
}

/// Ending a Keeper is all that uninstalling or a service manager can do to it,
/// and that reaches no Server.
#[tokio::test]
async fn a_server_outlives_its_keeper() {
    let machine = Machine::new();
    let channel = "keeper-ended";
    machine.succeed(channel, &["install"]).await;
    let mut keeper = machine.keeper(channel, Path::new(env!("CARGO_BIN_EXE_suru")));
    let standing = machine.server_other_than(channel, None).await;

    keeper.kill().await.expect("end the Keeper");
    keeper.wait().await.expect("collect the Keeper");
    tokio::time::sleep(LONG_ENOUGH_TO_HAVE_ACTED).await;
    assert!(machine.responds(&standing).await, "the Server went with it");
}

/// A Keeper running as its registration is removed stops restarting anything,
/// and ends, rather than running on past what its user asked.
#[tokio::test]
async fn a_keeper_whose_registration_is_gone_restarts_nothing_and_ends() {
    let machine = Machine::new();
    let channel = "keeper-uninstalled";
    machine.succeed(channel, &["install"]).await;
    let mut keeper = machine.keeper(channel, Path::new(env!("CARGO_BIN_EXE_suru")));
    let standing = machine.server_other_than(channel, None).await;

    machine.succeed(channel, &["uninstall"]).await;
    assert!(
        machine.responds(&standing).await,
        "uninstalling reached the Server"
    );
    machine.crash(&standing).await;
    let exit = timeout(PROGRESS_DEADLINE, keeper.wait())
        .await
        .expect("the Keeper ends")
        .expect("collect the Keeper");
    assert!(exit.success(), "{exit:?}");
    assert_eq!(machine.servers.servers_running(), 0);
}

/// The path a registration names now holds another build, as it does after an
/// upgrade or once a Client has replaced the Server with its own. A Keeper that
/// compared builds as it looked would replace the Server, and fight whichever
/// Client had put it there.
#[cfg(unix)]
#[tokio::test]
async fn a_keeper_leaves_a_server_of_another_build_to_the_clients_that_replace_them() {
    let machine = Machine::new();
    let channel = "keeper-other-build";
    machine.succeed(channel, &["install"]).await;
    let executable = machine.home.path().join("suru-at-a-lasting-path");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_suru"), &executable).expect("link the binary");
    let _keeper = machine.keeper(channel, &executable);
    let standing = machine.server_other_than(channel, None).await;

    std::fs::remove_file(&executable).expect("take the binary away");
    std::fs::write(&executable, "another build").expect("upgrade at the path");
    tokio::time::sleep(LONG_ENOUGH_TO_HAVE_ACTED).await;
    assert!(machine.responds(&standing).await, "the Server was replaced");
    assert_eq!(
        read_runtime_descriptor(machine.descriptor_path(channel)).instance_id,
        standing.instance_id,
    );
}

/// Run the way each service manager runs it — with a sparse environment and only
/// what the registration names — a Server finds the Provider a Server started
/// from the shell finds, while what the shell held that should go no further
/// does not reach the registration.
#[tokio::test]
async fn a_server_started_by_its_registration_finds_the_providers_a_shells_does() {
    let machine = Machine::new();
    let bin = machine.home.path().join("a place only the shell knows");
    write_scripted_claude(&bin);
    let empty = machine.home.path().join("nothing here");
    std::fs::create_dir(&empty).expect("make an empty directory");
    let shell_path = shell_path(&bin);
    let (from_shell, from_registration, unassisted) = (
        "provider-from-shell",
        "provider-from-registration",
        "provider-unassisted",
    );

    let mut started = machine.server_cli(from_shell, &["start"]);
    started
        .env("PATH", &shell_path)
        .env_remove("SURU_CLAUDE_PATH");
    let output = started.output().await.expect("start from the shell");
    assert!(output.status.success(), "{output:?}");

    let mut installed = machine.server_cli(from_registration, &["install"]);
    installed
        .env("PATH", &shell_path)
        .env_remove("SURU_CLAUDE_PATH")
        .env("ANTHROPIC_API_KEY", "a-secret-of-the-shells")
        .env("AWS_SECRET_ACCESS_KEY", "another-secret-of-the-shells");
    let output = installed.output().await.expect("install from the shell");
    assert!(output.status.success(), "{output:?}");
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        report.contains("ANTHROPIC_API_KEY") && !report.contains("a-secret-of-the-shells"),
        "{report}"
    );

    let words = registered_words(machine.home.path(), from_registration);
    assert!(
        !words.iter().any(|word| word.contains("secret")),
        "a secret reached the registration: {words:?}"
    );
    let mut registered = machine.sparse(&words[0]);
    registered
        .args(&words[1..])
        .env("PATH", &empty)
        .arg("--look-interval-ms")
        .arg(LOOK_INTERVAL_MS.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _keeper = registered.spawn().expect("run what is registered");

    // With what a service manager gives and nothing carried, no Provider is there.
    let mut unassisted_keeper = machine.sparse(env!("CARGO_BIN_EXE_suru"));
    let nothing_carried = format!("PATH={}", empty.display());
    unassisted_keeper
        .args(words[1..].iter().map(|word| {
            if word == from_registration {
                unassisted
            } else if word.starts_with("PATH=") {
                &nothing_carried
            } else {
                word
            }
        }))
        .env("PATH", &empty)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _unassisted_keeper = unassisted_keeper.spawn().expect("run the Keeper");

    let claude_status = |channel: &'static str| {
        let machine = &machine;
        async move {
            machine.server_other_than(channel, None).await;
            let client = suru::managed_client::ManagedClient::connect(
                suru::managed_client::ManagedClientConfig::new(
                    machine.servers.state_dir(),
                    channel,
                )
                .expect("configure a client")
                .with_data_dir(machine.servers.state_dir())
                .with_server_executable(env!("CARGO_BIN_EXE_suru")),
            )
            .await
            .expect("attach a client");
            let catalog = client.refresh_models().await.expect("read the catalog");
            catalog
                .providers
                .into_iter()
                .find(|provider| provider.provider == suru::protocol::ProviderId::new("claude"))
                .expect("the catalog lists Claude")
                .status
        }
    };
    let (shell, registration, unaided) = tokio::join!(
        claude_status(from_shell),
        claude_status(from_registration),
        claude_status(unassisted),
    );
    assert_eq!(registration, shell, "the two Servers disagree about Claude");
    assert!(
        !matches!(
            shell,
            suru::protocol::ProviderCatalogStatus::Unavailable {
                reason: suru::protocol::ProviderUnavailability::NotInstalled,
                ..
            }
        ),
        "the shell's Server should find the CLI on its PATH: {shell:?}"
    );
    assert!(
        matches!(
            unaided,
            suru::protocol::ProviderCatalogStatus::Unavailable {
                reason: suru::protocol::ProviderUnavailability::NotInstalled,
                ..
            }
        ),
        "with nothing carried no Provider should be found: {unaided:?}"
    );
}

/// `kill -9`, then `suru server stop` at once: the Server is gone but its
/// descriptor is not, and the stop is still the user's word. It leaves a record,
/// with the stale descriptor gone, and exits as a stop that took.
#[tokio::test]
async fn a_stop_after_a_crash_is_recorded_and_says_none_was_running() {
    let machine = Machine::new();
    let channel = "stop-after-crash";
    machine.succeed(channel, &["start"]).await;
    let crashed = machine.server_other_than(channel, None).await;
    machine.crash(&crashed).await;
    assert!(machine.descriptor_path(channel).exists());
    assert!(!machine.last_stop_path(channel).exists());

    let said = machine.succeed(channel, &["stop"]).await;
    assert!(said.contains("No Suru server was running"), "{said}");
    assert!(machine.last_stop_path(channel).exists());
    assert!(
        !machine.descriptor_path(channel).exists(),
        "a stopped Channel names no Server"
    );

    machine.succeed(channel, &["start"]).await;
    machine.server_other_than(channel, None).await;
}

/// What the stop is for: a Keeper that would have answered the crash does not.
/// Whichever of the two gets there first, the stop stands: recorded with no
/// Server there, or stopping the one the Keeper just launched.
#[tokio::test]
async fn a_stop_racing_a_keeper_answering_a_crash_stands() {
    let machine = Machine::new();
    let channel = "stop-racing-keeper";
    machine.succeed(channel, &["install"]).await;
    // Slow enough that the stop usually lands in the gap the review found, and
    // the outcome is the same if it does not.
    let mut keeper =
        machine.keeper_looking_every(channel, Path::new(env!("CARGO_BIN_EXE_suru")), 100);
    let crashed = machine.server_other_than(channel, None).await;

    machine.crash(&crashed).await;
    machine.succeed(channel, &["stop"]).await;

    tokio::time::sleep(LONG_ENOUGH_TO_HAVE_ACTED * 2).await;
    assert_eq!(machine.servers.servers_running(), 0, "a stop was undone");
    assert!(
        keeper.try_wait().expect("look at the Keeper").is_none(),
        "the Keeper goes on looking, for the Server someone starts next"
    );

    machine.succeed(channel, &["start"]).await;
    machine
        .server_other_than(channel, Some(crashed.instance_id))
        .await;
}

/// A Server launched before such a stop and still on its way to the election is
/// held to it, as to any Manual stop, which is what settles the race with a
/// launch under way.
#[tokio::test]
async fn a_server_launched_before_a_stop_with_no_server_there_stands_down() {
    let machine = Machine::new();
    let channel = "stop-overtaking-launch";
    machine.succeed(channel, &["start"]).await;
    let crashed = machine.server_other_than(channel, None).await;
    let followed = std::fs::read_to_string(machine.last_stop_path(channel)).ok();
    assert_eq!(followed, None);
    machine.crash(&crashed).await;
    machine.succeed(channel, &["stop"]).await;

    let launched = machine
        .sparse(env!("CARGO_BIN_EXE_suru"))
        .arg("__server")
        .arg("--state-dir")
        .arg(machine.servers.state_dir())
        .arg("--data-dir")
        .arg(machine.servers.state_dir())
        .arg("--channel")
        .arg(channel)
        .arg("--last-stop")
        .arg("none")
        .envs(machine.servers.isolated_environment())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let status = timeout(PROGRESS_DEADLINE, launched)
        .await
        .expect("the Server stands down at once")
        .expect("run the Server");
    assert!(!status.success());
    assert!(!machine.descriptor_path(channel).exists());
}

/// A client recovering from the crash hears the stop as it hears any.
#[tokio::test]
async fn a_client_recovering_from_a_crash_leaves_on_a_stop() {
    let machine = Machine::new();
    let channel = "stop-recovering-client";
    let mut client = ManagedClient::connect(
        machine
            .servers
            .client_config(channel)
            .with_recovery_backoff(Duration::from_millis(100), Duration::from_millis(200)),
    )
    .await
    .expect("attach a client");
    let crashed = machine.server_other_than(channel, None).await;

    machine.crash(&crashed).await;
    machine.succeed(channel, &["stop"]).await;

    timeout(PROGRESS_DEADLINE, async {
        loop {
            match client.next().await {
                Some(ManagedEvent::ServerShutdown(shutdown)) => {
                    assert_eq!(shutdown.reason, ShutdownReason::Manual);
                    return;
                }
                Some(_) => {}
                None => panic!("the client ended without hearing the stop"),
            }
        }
    })
    .await
    .expect("the recovering client hears the stop");
}

/// A Server that does not answer yet holds the election is no crash: nothing
/// can be recorded past the lock it holds, and it may wake. That stays as it was.
#[cfg(unix)]
#[tokio::test]
async fn a_stop_of_a_hung_server_says_so_and_records_nothing() {
    let machine = Machine::new();
    let channel = "stop-hung";
    machine.succeed(channel, &["start"]).await;
    let hung = machine.server_other_than(channel, None).await;
    let signal = |signal| {
        // SAFETY: `kill` takes plain integers, and the pid is the Server's own.
        assert_eq!(unsafe { libc::kill(hung.pid as libc::pid_t, signal) }, 0);
    };
    signal(libc::SIGSTOP);

    let stopped = suru::managed_client::stop_server(
        &machine
            .servers
            .client_config(channel)
            .with_health_check_timeout(Duration::from_millis(100))
            .with_stop_timeout(Duration::from_millis(300)),
    )
    .await;
    signal(libc::SIGCONT);

    let error = stopped.expect_err("a hung Server is not stopped");
    assert!(error.to_string().contains("cannot stop"), "{error:#}");
    assert!(!machine.last_stop_path(channel).exists());
    assert!(machine.responds(&hung).await, "the Server woke as it was");
}

/// A Server that dies each time it is launched never comes to publish anything,
/// and `suru server stop` still ends the loop.
#[cfg(unix)]
#[tokio::test]
async fn a_stop_ends_a_crash_loop() {
    use std::os::unix::fs::PermissionsExt;

    let machine = Machine::new();
    let channel = "stop-crash-loop";
    machine.succeed(channel, &["install"]).await;
    let launches = machine.home.path().join("launches");
    let crashing = machine.home.path().join("crashing-server");
    std::fs::write(
        &crashing,
        format!(
            "#!/bin/sh\necho launched >> '{}'\nexit 1\n",
            launches.display()
        ),
    )
    .expect("write a Server that crashes");
    std::fs::set_permissions(&crashing, std::fs::Permissions::from_mode(0o700))
        .expect("make it runnable");
    let launch_count = || {
        std::fs::read_to_string(&launches)
            .map(|launched| launched.lines().count())
            .unwrap_or(0)
    };
    let _keeper = machine.keeper(channel, &crashing);
    timeout(PROGRESS_DEADLINE, async {
        while launch_count() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the Keeper goes on launching");

    let said = machine.succeed(channel, &["stop"]).await;
    assert!(said.contains("No Suru server was running"), "{said}");
    // One launch may have been under way as the stop landed.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let settled = launch_count();
    tokio::time::sleep(LONG_ENOUGH_TO_HAVE_ACTED * 2).await;
    assert_eq!(
        launch_count(),
        settled,
        "the stop did not end the crash loop"
    );
}
