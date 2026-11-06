//! A Keeper getting going, as a service manager runs it: what it reads from its
//! command line, the console a Scheduled Task gives it, its Log, and the one
//! question it asks of its Login start.

use std::time::Duration;

use anyhow::Result;

use super::{KeeperArguments, LoginStart, Standing};
use crate::{
    logging,
    managed_client::{KeeperTimings, ManagedClientConfig, keep_server},
};

impl KeeperArguments {
    /// Runs this Keeper, for as long as its Channel's Login start stands.
    pub async fn keep(self) -> Result<()> {
        let_go_of_the_console();
        let mut config = ManagedClientConfig::new(self.state_base_dir, self.channel)?
            .with_data_dir(self.data_base_dir)
            .with_server_executable(self.executable);
        if let Some(config_dir) = self.config_dir {
            config = config.with_config_dir(config_dir);
        }
        if let Some(identity_store) = self.identity_store {
            config = config.with_identity_store(identity_store);
        }
        for (name, value) in self.carried {
            config = config.with_server_env(name, value);
        }
        let mut timings = KeeperTimings::default();
        if let Some(interval_ms) = self.look_interval_ms {
            timings.look_interval = Duration::from_millis(interval_ms);
        }
        // A home that is not back yet at login holds no Log for a while, which
        // is no reason to give up keeping a Server.
        let _log_guard = logging::init(config.runtime(), logging::Role::Keeper)
            .inspect_err(|error| eprintln!("Could not open a Log: {error:#}"))
            .ok();
        // One that cannot say whether it is registered goes on as if it were.
        let login_start = LoginStart::for_this_user(config.channel(), self.stand_in)
            .inspect_err(|error| {
                tracing::warn!("could not read the channel's login start: {error:#}");
            })
            .ok();
        keep_server(&config, timings, || {
            login_start
                .as_ref()
                .is_some_and(|login_start| login_start.standing() == Standing::NotRegistered)
        })
        .await
    }
}

/// A Task at logon comes with a console of its own, showing for this moment,
/// and a Keeper has nothing to say there and launches its Server with none.
///
/// Once the console is freed, the handles this process holds for it are not
/// ones to rely on: whether Windows has closed them, and whether a slot may
/// since have gone to some other file, reports differ. So they are cleared
/// rather than left to find out. A launch's `StandardHandleInheritanceGuard`
/// passes over a null standard handle, every Keeper's child is given standard
/// handles of its own, and Rust's standard streams take a null handle for a sink,
/// which is what a Keeper's stray `eprintln!` wants. Its Log is a file of its
/// own, which holds none of these.
#[cfg(windows)]
fn let_go_of_the_console() {
    use windows_sys::Win32::System::Console::{
        FreeConsole, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };

    // SAFETY: FreeConsole takes nothing, and failing changes nothing; a standard
    // handle may be set to null, and nothing else is running yet to be using one.
    unsafe {
        FreeConsole();
        for standard_handle in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            SetStdHandle(standard_handle, std::ptr::null_mut());
        }
    }
}

#[cfg(not(windows))]
fn let_go_of_the_console() {}
