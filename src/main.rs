use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use suru::{
    IdentityStoreChoice, LastStop, logging,
    login_start::{self, LoginStart},
    managed_client::{
        KeeperTimings, ManagedClient, ManagedClientConfig, ServerStatus, keep_server,
        server_status, start_server, stop_server,
    },
    server::{self, ServerConfig},
    tui,
};

// Keep the marker in allocated storage so stripping debug information preserves
// it. Taking its address in main also prevents linker garbage collection.
#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__suru"))]
#[cfg_attr(not(target_os = "macos"), unsafe(link_section = ".suru"))]
static BUILD_ID: [u8; 32] = suru_build_id::generate!();

#[derive(Debug, Parser)]
#[command(name = "suru", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Server {
        #[command(subcommand)]
        command: ServerCommand,
    },
    #[command(name = "__server", hide = true)]
    InternalServer {
        #[arg(long = "state-dir")]
        state_base_dir: PathBuf,
        #[arg(long = "data-dir")]
        data_base_dir: PathBuf,
        #[arg(long = "config-dir")]
        config_dir: Option<PathBuf>,
        #[arg(long)]
        channel: String,
        /// How often the server looks to see that it still stands for its
        /// channel, for tests that cannot wait out the default.
        #[arg(long, hide = true)]
        state_dir_check_interval_ms: Option<u64>,
        /// The channel's last manual stop as the launcher read it before
        /// launching this server: `none`, or the stopped instance's id. A
        /// stop recorded since ends this server once it is elected.
        #[arg(long, hide = true)]
        last_stop: Option<LastStop>,
        /// The store a new identity key is kept in whatever the build, as
        /// the launcher read `SURU_IDENTITY_STORE`: `system` or `file`.
        #[arg(long, hide = true)]
        identity_store: Option<IdentityStoreChoice>,
        /// How long this server waits for the channel's election lock to come
        /// free, for tests that hold a server in its election.
        #[arg(long, hide = true)]
        election_handoff_ms: Option<u64>,
        /// How long this server's stop may take to let go gracefully of all
        /// it can abandon before it is cut short, for tests that hold a stop
        /// up and must not see it cut short meanwhile.
        #[arg(long, hide = true)]
        shutdown_deadline_ms: Option<u64>,
        /// How long past its deadline this server's stop may still take over
        /// the steps it never skips, for tests that see a stop held to its
        /// cutoff without waiting out the default.
        #[arg(long, hide = true)]
        shutdown_overrun_ms: Option<u64>,
        /// How long past its stop's deadline and overrun, or past its Server
        /// stopping, this process may run before its cutoff ends it, for
        /// tests that see it cut off without waiting out the default.
        #[arg(long, hide = true)]
        shutdown_cutoff_margin_ms: Option<u64>,
    },
    /// A Channel's Keeper, as a Login start runs it: every path and all that
    /// its Server's environment is to carry on its command line, since a
    /// service manager gives a process almost none of what a shell does.
    #[command(name = "__keep-server", hide = true)]
    Keeper {
        /// The binary each Server is launched from, which is the path the
        /// registration names rather than what this process resolves to, so
        /// an upgrade at that path is what the next Server runs.
        #[arg(long)]
        executable: PathBuf,
        #[arg(long = "state-dir")]
        state_base_dir: PathBuf,
        #[arg(long = "data-dir")]
        data_base_dir: PathBuf,
        #[arg(long = "config-dir")]
        config_dir: Option<PathBuf>,
        #[arg(long)]
        channel: String,
        #[arg(long)]
        identity_store: Option<IdentityStoreChoice>,
        /// A variable each Server is launched with, as `NAME=value`.
        #[arg(long = "env", value_parser = carried_variable)]
        carried: Vec<(String, String)>,
        /// A stand-in for the machine's service manager, for tests that must
        /// not reach the real one.
        #[arg(long, hide = true)]
        service_manager: Option<PathBuf>,
        /// How often this Keeper looks at its Channel, for tests that cannot
        /// wait out the default.
        #[arg(long, hide = true)]
        look_interval_ms: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
enum ServerCommand {
    Start {
        #[arg(long, hide = true)]
        startup_timeout_ms: Option<u64>,
        #[arg(long, hide = true)]
        election_handoff_ms: Option<u64>,
    },
    Status {
        /// A stand-in for the machine's service manager, for tests that must
        /// not reach the real one.
        #[arg(long, hide = true)]
        service_manager: Option<PathBuf>,
    },
    Stop {
        #[arg(long, hide = true)]
        stop_timeout_ms: Option<u64>,
        #[arg(long, hide = true)]
        health_check_timeout_ms: Option<u64>,
    },
    /// Start this Channel's Server whenever this user logs in.
    Install {
        /// A stand-in for the machine's service manager, for tests that must
        /// not reach the real one.
        #[arg(long, hide = true)]
        service_manager: Option<PathBuf>,
    },
    /// Stop starting this Channel's Server at login. A Server running now is left running.
    Uninstall {
        /// A stand-in for the machine's service manager, for tests that must
        /// not reach the real one.
        #[arg(long, hide = true)]
        service_manager: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    std::hint::black_box(&BUILD_ID);
    match Cli::parse().command {
        Some(CliCommand::Server {
            command:
                ServerCommand::Start {
                    startup_timeout_ms,
                    election_handoff_ms,
                },
        }) => {
            let mut config = default_client_config()?;
            if let Some(timeout_ms) = startup_timeout_ms {
                config = config.with_startup_timeout(std::time::Duration::from_millis(timeout_ms));
            }
            if let Some(handoff_ms) = election_handoff_ms {
                config = config.with_election_handoff(std::time::Duration::from_millis(handoff_ms));
            }
            let health = start_server(&config).await?;
            println!(
                "Suru server ready (pid {}, instance {})",
                health.identity.pid, health.identity.instance_id
            );
            Ok(())
        }
        Some(CliCommand::Server {
            command: ServerCommand::Status { service_manager },
        }) => {
            let config = default_client_config()?;
            let status = server_status(&config).await?;
            let login_start = match LoginStart::for_this_user(config.channel(), service_manager) {
                Ok(login_start) => login_start.standing(),
                Err(error) => login_start::Standing::Unknown(format!("{error:#}")),
            };
            if matches!(status, ServerStatus::Ready(_)) {
                println!("{status}");
                println!("{login_start}");
                Ok(())
            } else {
                println!("{login_start}");
                anyhow::bail!(status)
            }
        }
        Some(CliCommand::Server {
            command:
                ServerCommand::Stop {
                    stop_timeout_ms,
                    health_check_timeout_ms,
                },
        }) => {
            let mut config = default_client_config()?;
            if let Some(timeout_ms) = stop_timeout_ms {
                config = config.with_stop_timeout(std::time::Duration::from_millis(timeout_ms));
            }
            if let Some(timeout_ms) = health_check_timeout_ms {
                config =
                    config.with_health_check_timeout(std::time::Duration::from_millis(timeout_ms));
            }
            let health = stop_server(&config).await?;
            println!(
                "Suru server stopped (pid {}, instance {})",
                health.identity.pid, health.identity.instance_id
            );
            Ok(())
        }
        Some(CliCommand::Server {
            command: ServerCommand::Install { service_manager },
        }) => {
            print!(
                "{}",
                login_start::install(&default_client_config()?, service_manager)?
            );
            Ok(())
        }
        Some(CliCommand::Server {
            command: ServerCommand::Uninstall { service_manager },
        }) => {
            print!(
                "{}",
                login_start::uninstall(default_client_config()?.channel(), service_manager)?
            );
            Ok(())
        }
        Some(CliCommand::Keeper {
            executable,
            state_base_dir,
            data_base_dir,
            config_dir,
            channel,
            identity_store,
            carried,
            service_manager,
            look_interval_ms,
        }) => {
            // A Task at logon comes with a console of its own, showing for
            // this moment; the Server is launched with none.
            #[cfg(windows)]
            // SAFETY: FreeConsole takes nothing, and failing changes nothing.
            unsafe {
                windows_sys::Win32::System::Console::FreeConsole();
            }
            let mut config = ManagedClientConfig::new(state_base_dir, channel)?
                .with_data_dir(data_base_dir)
                .with_server_executable(executable);
            if let Some(config_dir) = config_dir {
                config = config.with_config_dir(config_dir);
            }
            if let Some(identity_store) = identity_store {
                config = config.with_identity_store(identity_store);
            }
            for (name, value) in carried {
                config = config.with_server_env(name, value);
            }
            let mut timings = KeeperTimings::default();
            if let Some(interval_ms) = look_interval_ms {
                timings.look_interval = std::time::Duration::from_millis(interval_ms);
            }
            // A home that is not back yet at login holds no Log for a while,
            // which is no reason to give up keeping a Server.
            let _log_guard = logging::init(config.runtime(), logging::Role::Keeper)
                .inspect_err(|error| eprintln!("Could not open a Log: {error:#}"))
                .ok();
            let login_start = LoginStart::for_this_user(config.channel(), service_manager)?;
            keep_server(&config, timings, || login_start.standing()).await
        }
        Some(CliCommand::InternalServer {
            state_base_dir,
            data_base_dir,
            config_dir,
            channel,
            state_dir_check_interval_ms,
            last_stop,
            identity_store,
            election_handoff_ms,
            shutdown_deadline_ms,
            shutdown_overrun_ms,
            shutdown_cutoff_margin_ms,
        }) => {
            // The launcher made the state and data directories just before
            // launching this server, so a server that finds either missing was
            // launched into a directory removed since, and never makes it again.
            let mut config = ServerConfig::new(state_base_dir, channel)?
                .with_data_dir(data_base_dir)
                .launched_into_existing_dirs();
            if let Some(config_dir) = config_dir {
                config = config.with_config_dir(config_dir);
            }
            if let Some(last_stop) = last_stop {
                config = config.launched_after(last_stop);
            }
            if let Some(identity_store) = identity_store {
                config = config.with_identity_store(identity_store);
            }
            let log_guard = logging::init(&config, logging::Role::Server)
                .context("initialize server logging")?;
            // Before anything is spawned: every Provider process and Git
            // command holds descriptors of the Server's.
            suru::open_files::raise_limit();
            let signals = server::ShutdownSignals::listen()?;
            let mut timings = server::ServerTimings::default();
            if let Some(interval_ms) = state_dir_check_interval_ms {
                timings = timings
                    .with_state_dir_check_interval(std::time::Duration::from_millis(interval_ms));
            }
            if let Some(handoff_ms) = election_handoff_ms {
                timings.election_handoff = std::time::Duration::from_millis(handoff_ms);
            }
            if let Some(deadline_ms) = shutdown_deadline_ms {
                timings.shutdown_deadline = std::time::Duration::from_millis(deadline_ms);
            }
            if let Some(overrun_ms) = shutdown_overrun_ms {
                timings.shutdown_overrun = std::time::Duration::from_millis(overrun_ms);
            }
            if let Some(margin_ms) = shutdown_cutoff_margin_ms {
                timings.shutdown_cutoff_margin = std::time::Duration::from_millis(margin_ms);
            }
            // Started after the signals are listened for, and before the
            // Server is spawned: a signal from here on holds the process to
            // its cutoff, whatever becomes of the runtime.
            let cutoff = server::ProcessCutoff::start(&timings)
                .context("start keeping the server's cutoff")?;
            let server = server::spawn_with_timings(config, timings).await?;
            server.on_stopping({
                let cutoff = cutoff.clone();
                move |began| cutoff.stop_began(began)
            });
            let stopped = server.run_until_signalled(signals).await;
            // What is left — reporting how the stop went, to a stderr a
            // managed client points into the state directory, and flushing
            // the Log there — has the cutoff's margin, so a directory that
            // does not answer cannot keep a stopped Server's process alive.
            cutoff.server_stopped();
            if let Err(error) = &stopped {
                eprintln!("Error: {error:?}");
            }
            // The process ends here rather than returning through the
            // runtime, whose shutdown waits out every blocking task still
            // running: one waiting on a state directory that does not answer
            // would keep a stopped Server's process alive for as long. The
            // Log is flushed first.
            drop(log_guard);
            std::process::exit(i32::from(stopped.is_err()))
        }
        None => {
            let config = default_client_config()?;
            let _log_guard = logging::init(config.runtime(), logging::Role::Client)
                .context("initialize client logging")?;
            let client = ManagedClient::connect(config)
                .await
                .context("prepare the managed Suru server connection")?;
            tui::run(client).await
        }
    }
}

fn carried_variable(value: &str) -> Result<(String, String), String> {
    value
        .split_once('=')
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .ok_or_else(|| format!("{value:?} is not NAME=value"))
}

fn default_client_config() -> Result<ManagedClientConfig> {
    let state_base_dir = match std::env::var_os("SURU_STATE_DIR") {
        Some(path) => PathBuf::from(path),
        None => dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .context("determine the current user's state directory")?
            .join("suru"),
    };
    let data_base_dir = match std::env::var_os("SURU_DATA_DIR") {
        Some(path) => PathBuf::from(path),
        None => dirs::data_local_dir()
            .context("determine the current user's data directory")?
            .join("suru"),
    };
    let config_dir = suru::settings::resolve_config_root(
        std::env::var_os("SURU_CONFIG_DIR").as_deref(),
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        dirs::home_dir().as_deref(),
    );
    let channel = std::env::var("SURU_CHANNEL").unwrap_or_else(|_| {
        if cfg!(debug_assertions) {
            "debug".to_owned()
        } else {
            "release".to_owned()
        }
    });
    let identity_store = match std::env::var("SURU_IDENTITY_STORE") {
        Ok(store) => Some(
            store
                .parse::<IdentityStoreChoice>()
                .context("read SURU_IDENTITY_STORE")?,
        ),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error).context("read SURU_IDENTITY_STORE"),
    };
    let mut config =
        ManagedClientConfig::new(state_base_dir, channel)?.with_data_dir(data_base_dir);
    if let Some(config_dir) = config_dir {
        config = config.with_config_dir(config_dir);
    }
    if let Some(identity_store) = identity_store {
        config = config.with_identity_store(identity_store);
    }
    Ok(config)
}
