//! The Keeper: a Channel's Server kept running for as long as its user is
//! logged in, restarted where it crashed and left stopped where its user
//! stopped it (ADR 0063).
//!
//! A Keeper holds no authority of its own. The Channel's election says whether a
//! Server stands, and its last-stop record whether the last one to stand was
//! stopped, and a Keeper only ever reads both, so it serves no differently from
//! any Client launching: [`keep_server`] sets out as `shaman server start` does,
//! then takes a [`Look`] now and then.
//!
//! - **A Server holds the election.** There is nothing to do, whichever Client
//!   launched it and of whatever build: a Keeper never replaces a Server after
//!   setting out, so it cannot fight the Client that replaced one (ADR 0001).
//! - **Nobody holds it, and a Manual stop has been recorded since the stop the
//!   Keeper follows.** That stop is final (ADR 0002), so it launches nothing. A
//!   stopping Server records its stop before it lets the election go, so a
//!   Keeper that finds the election free has already seen any record that would
//!   tell a stop from a crash. It goes on looking, and takes up a Server someone
//!   starts later, so that one is restarted as well.
//! - **Nobody holds it, and no stop has been recorded.** The Server crashed, and
//!   the Keeper launches another, once it has seen the election free on
//!   consecutive looks: a Client replacing a Server has its successor on the way
//!   within moments of the election going free, and needs no help.
//!
//! Anything that does not answer is no finding, as in an election: a state
//! directory on a home that is not back yet costs a look, and nothing more.
//!
//! The stop a Keeper follows is the one it read as it set out, and it holds that
//! in memory rather than leaving it to a service manager to run it again. A
//! Keeper run afresh takes the Channel's last stop as already seen, which is
//! what a new login should do, and what a service manager's restart must not.
//! That is why no registration asks for one (`login_start`).
//!
//! The Server it launches is launched as any Client launches one: detached, with
//! its stdout and stderr in the state directory. Nothing a service manager does
//! to the Keeper therefore reaches it, and a Keeper ended leaves it running.

use std::time::{Duration, Instant};

use anyhow::Result;
use uuid::Uuid;

use crate::runtime::LastStop;

use super::{ManagedClientConfig, RecoveryBackoff, launcher, lifecycle};

/// How often a Keeper looks by default. A crashed Server is back within two
/// looks, and a Keeper has no cheaper way to hear of a crash that also survives
/// every Client having gone.
const LOOK_INTERVAL: Duration = Duration::from_secs(5);
/// The longest a Keeper leaves between launches of a Server that will not stay
/// up, and how long one must stand to be taken as having stayed up.
const MAX_LAUNCH_BACKOFF: Duration = Duration::from_secs(60);
/// How many consecutive looks must find the election free before a launch.
const FREE_LOOKS_BEFORE_LAUNCH: u32 = 2;

/// How a Keeper paces itself; injectable so a test need not wait out the default.
#[derive(Clone, Copy, Debug)]
pub struct KeeperTimings {
    pub look_interval: Duration,
    pub max_launch_backoff: Duration,
}

impl Default for KeeperTimings {
    fn default() -> Self {
        Self {
            look_interval: LOOK_INTERVAL,
            max_launch_backoff: MAX_LAUNCH_BACKOFF,
        }
    }
}

/// One look at the Channel. Read in this order, which makes the look sound: the
/// last stop first, since a stop recorded after it is read still holds the
/// election while it is asked about, and the descriptor last.
struct Look {
    /// The Channel's last stop, or none where it did not answer.
    last_stop: Option<LastStop>,
    /// Whether a Server holds the election, or none where it did not answer.
    held: Option<bool>,
    /// The Server the runtime descriptor names, where it names one.
    published: Option<Uuid>,
}

impl Look {
    fn take(config: &ManagedClientConfig) -> Self {
        let last_stop = config
            .runtime()
            .last_stop()
            .inspect_err(|error| {
                tracing::warn!("could not read the channel's last manual stop: {error}");
            })
            .ok();
        let held = lifecycle::channel_is_owned(config)
            .inspect_err(|error| {
                tracing::warn!("could not read the channel's election: {error:#}");
            })
            .ok();
        Self {
            last_stop,
            held,
            published: lifecycle::published_instance(config),
        }
    }
}

/// What a look comes to.
#[derive(Debug, Eq, PartialEq)]
enum Finding {
    /// Nothing to do.
    Nothing,
    /// The Server crashed. Launch one following this stop.
    Crashed(LastStop),
}

/// What a Keeper knows between looks.
#[derive(Debug)]
struct Keeping {
    /// The Manual stop this Keeper follows, or none where it could not read one.
    followed: Option<LastStop>,
    /// How many looks in a row have found the election free.
    free_looks: u32,
}

impl Keeping {
    fn new(followed: Option<LastStop>) -> Self {
        Self {
            followed,
            free_looks: 0,
        }
    }

    fn look(&mut self, look: Look) -> Finding {
        let (Some(now), Some(held)) = (look.last_stop, look.held) else {
            return Finding::Nothing;
        };
        let followed = *self.followed.get_or_insert(now);
        if held {
            self.free_looks = 0;
            // A Server other than the one that stop stopped, and which stands
            // now, started after that stop: someone started the Channel again,
            // so that stop is behind this Keeper, and a crash from here is a
            // crash. The stopped Server's own descriptor, seen as it finishes
            // stopping, is no such thing, nor is a look with no descriptor.
            if look
                .published
                .is_some_and(|published| now.stopped_instance() != Some(published))
                && self.followed != Some(now)
            {
                tracing::info!(%now, "a server stands since the stop this keeper followed");
                self.followed = Some(now);
            }
            return Finding::Nothing;
        }
        if followed.superseded_by(now).is_some() {
            self.free_looks = 0;
            return Finding::Nothing;
        }
        self.free_looks += 1;
        if self.free_looks < FREE_LOOKS_BEFORE_LAUNCH {
            return Finding::Nothing;
        }
        self.free_looks = 0;
        Finding::Crashed(followed)
    }
}

/// Keeps `config`'s Channel's Server running until `no_longer_wanted` says so,
/// which it is asked only as a Server comes to be launched: the Keeper of a
/// removed Login start has nothing further to do, and runs out at logout at the
/// latest. Anything short of being sure is for it to answer no, since an answer
/// of yes ends a Keeper for good.
pub async fn keep_server(
    config: &ManagedClientConfig,
    timings: KeeperTimings,
    no_longer_wanted: impl Fn() -> bool,
) -> Result<()> {
    let baseline = launcher::StopBaseline::read(config);
    let startup = tokio::time::Instant::now() + config.startup_timeout;
    // As `shaman server start` does, so a Server of another build still running
    // is replaced here as any Client replaces one. Only here: from now on, what
    // stands is what its user or their Clients have left standing.
    match launcher::ensure_server(config, startup, baseline).await {
        Ok(registration) => tracing::info!(
            instance_id = %registration.health.instance_id,
            "keeping the channel's server"
        ),
        Err(error) => tracing::warn!("the channel's server did not start: {error:#}"),
    }

    let mut keeping = Keeping::new(baseline.followed());
    let mut backoff = RecoveryBackoff::new(timings.look_interval, timings.max_launch_backoff);
    let mut next_launch = Instant::now();
    let mut held_since: Option<Instant> = None;
    loop {
        tokio::time::sleep(timings.look_interval).await;
        let look = Look::take(config);
        let now = Instant::now();
        if look.held == Some(true) {
            let since = *held_since.get_or_insert(now);
            if now.duration_since(since) >= timings.max_launch_backoff {
                backoff.reset();
            }
        } else {
            held_since = None;
        }
        let Finding::Crashed(followed) = keeping.look(look) else {
            continue;
        };
        if now < next_launch {
            continue;
        }
        if no_longer_wanted() {
            tracing::info!("the channel's login start was removed, so its keeper is done");
            return Ok(());
        }
        tracing::warn!("the channel's server is gone with no manual stop recorded: launching one");
        if let Err(error) = launcher::spawn_detached(config, Some(followed)) {
            tracing::warn!("could not launch the channel's server: {error:#}");
        }
        next_launch = now + backoff.next();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOPPED: Uuid = Uuid::from_u128(1);
    const STANDING: Uuid = Uuid::from_u128(2);
    const LATER: Uuid = Uuid::from_u128(3);

    fn stop(instance: Uuid) -> LastStop {
        instance.to_string().parse().unwrap()
    }

    fn none() -> LastStop {
        LastStop::default()
    }

    fn free(last_stop: LastStop) -> Look {
        Look {
            last_stop: Some(last_stop),
            held: Some(false),
            published: None,
        }
    }

    fn held(last_stop: LastStop, published: Uuid) -> Look {
        Look {
            last_stop: Some(last_stop),
            held: Some(true),
            published: Some(published),
        }
    }

    #[test]
    fn a_server_that_stands_needs_nothing() {
        let mut keeping = Keeping::new(Some(none()));
        for _ in 0..5 {
            assert_eq!(keeping.look(held(none(), STANDING)), Finding::Nothing);
        }
    }

    #[test]
    fn a_crash_is_answered_on_the_second_look_to_find_the_election_free() {
        let mut keeping = Keeping::new(Some(none()));
        assert_eq!(keeping.look(held(none(), STANDING)), Finding::Nothing);
        assert_eq!(keeping.look(free(none())), Finding::Nothing);
        assert_eq!(keeping.look(free(none())), Finding::Crashed(none()));
        assert_eq!(
            keeping.look(free(none())),
            Finding::Nothing,
            "a launch gets its own two looks to take the election"
        );
    }

    /// The election free for one look is a Client's replacement on its way.
    #[test]
    fn a_handover_seen_in_passing_is_no_crash() {
        let mut keeping = Keeping::new(Some(none()));
        for _ in 0..5 {
            assert_eq!(keeping.look(free(none())), Finding::Nothing);
            assert_eq!(keeping.look(held(none(), LATER)), Finding::Nothing);
        }
    }

    #[test]
    fn a_login_following_an_old_stop_keeps_a_server_as_ever() {
        let mut keeping = Keeping::new(Some(stop(STOPPED)));
        assert_eq!(keeping.look(free(stop(STOPPED))), Finding::Nothing);
        assert_eq!(
            keeping.look(free(stop(STOPPED))),
            Finding::Crashed(stop(STOPPED))
        );
    }

    #[test]
    fn a_manual_stop_is_final_however_many_looks_find_it() {
        let mut keeping = Keeping::new(Some(none()));
        assert_eq!(keeping.look(held(none(), STOPPED)), Finding::Nothing);
        for _ in 0..5 {
            assert_eq!(keeping.look(free(stop(STOPPED))), Finding::Nothing);
        }
    }

    /// A Server records its stop before it lets the election go, so a look can
    /// land between the two, and must not take the stop as already behind it.
    #[test]
    fn a_stop_seen_while_its_server_still_holds_the_election_is_still_a_stop() {
        let mut keeping = Keeping::new(Some(none()));
        assert_eq!(keeping.look(held(none(), STOPPED)), Finding::Nothing);
        assert_eq!(keeping.look(held(stop(STOPPED), STOPPED)), Finding::Nothing);
        let descriptor_removed = Look {
            last_stop: Some(stop(STOPPED)),
            held: Some(true),
            published: None,
        };
        assert_eq!(keeping.look(descriptor_removed), Finding::Nothing);
        for _ in 0..5 {
            assert_eq!(keeping.look(free(stop(STOPPED))), Finding::Nothing);
        }
    }

    /// `shaman server start` after a stop: that stop is behind the Keeper now, so
    /// the new Server's crash is answered, and its own stop is final.
    #[test]
    fn a_server_started_after_a_stop_is_kept_like_any_other() {
        let mut keeping = Keeping::new(Some(none()));
        assert_eq!(keeping.look(free(stop(STOPPED))), Finding::Nothing);
        assert_eq!(keeping.look(held(stop(STOPPED), LATER)), Finding::Nothing);

        assert_eq!(keeping.look(free(stop(STOPPED))), Finding::Nothing);
        assert_eq!(
            keeping.look(free(stop(STOPPED))),
            Finding::Crashed(stop(STOPPED))
        );

        assert_eq!(keeping.look(held(stop(STOPPED), LATER)), Finding::Nothing);
        for _ in 0..5 {
            assert_eq!(keeping.look(free(stop(LATER))), Finding::Nothing);
        }
    }

    /// Stopped and started again between two looks, so no look found it free.
    #[test]
    fn a_stop_and_start_between_looks_is_followed_all_the_same() {
        let mut keeping = Keeping::new(Some(none()));
        assert_eq!(keeping.look(held(none(), STOPPED)), Finding::Nothing);
        assert_eq!(keeping.look(held(stop(STOPPED), LATER)), Finding::Nothing);
        assert_eq!(keeping.look(free(stop(STOPPED))), Finding::Nothing);
        assert_eq!(
            keeping.look(free(stop(STOPPED))),
            Finding::Crashed(stop(STOPPED))
        );
    }

    #[test]
    fn what_does_not_answer_is_no_finding() {
        let mut keeping = Keeping::new(Some(none()));
        for unanswered in [
            Look {
                last_stop: None,
                held: Some(false),
                published: None,
            },
            Look {
                last_stop: Some(none()),
                held: None,
                published: None,
            },
        ] {
            assert_eq!(keeping.look(unanswered), Finding::Nothing);
        }
        assert_eq!(keeping.free_looks, 0, "it costs a look, and nothing more");
    }

    #[test]
    fn a_keeper_that_could_not_read_a_stop_as_it_set_out_follows_the_next_it_can() {
        let mut keeping = Keeping::new(None);
        assert_eq!(keeping.look(free(stop(STOPPED))), Finding::Nothing);
        assert_eq!(
            keeping.look(free(stop(STOPPED))),
            Finding::Crashed(stop(STOPPED))
        );
    }

    /// A record gone since names nothing, as it does in an election.
    #[test]
    fn a_record_gone_missing_is_no_reason_to_stop_keeping() {
        let mut keeping = Keeping::new(Some(stop(STOPPED)));
        assert_eq!(keeping.look(free(none())), Finding::Nothing);
        assert_eq!(keeping.look(free(none())), Finding::Crashed(stop(STOPPED)));
    }
}
