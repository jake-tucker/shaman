//! Whether each Remote answers, as the core reads it: Unreachable once every
//! way to it has failed once, or once it has gone unanswered for a while as
//! it is reached anew, and answering again as soon as anything asked of it
//! is answered. Until then it holds its last reading, so nothing ever says
//! "reconnecting".
//!
//! Every reader and every act of a Remote asks over one transport, which
//! notes what each request found. A Remote is reached anew as the core
//! begins to follow it, as its catalog stream drops, and on the check the
//! core makes when the app comes to the front or the network path changes:
//! then each connection it holds is asked something cheap, what answers is
//! kept and the rest given up, and every reader is told to try now, so it
//! redials at once and its stream resumes by a fresh snapshot.
//!
//! With no network path at all nothing is dialled, and every Remote reads
//! Unreachable until a path is back.

use std::{
    net::IpAddr,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use shaman_client::{PairingTransport, Transport, TransportResponse, TryNow};
use shaman_pairing::{ConnectionsChecked, Redeemer};
use shaman_protocol::{SessionError, SessionErrorCode, Way};
use tokio::{
    runtime::Handle,
    sync::{Notify, watch},
    task::JoinHandle,
};

/// Why a Remote reads Unreachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unreached {
    /// No way to it answered.
    NoWay,
    /// This phone has no network path at all.
    NoNetwork,
}

/// How a Remote reads: answering, as far as anything says, or Unreachable
/// for why.
pub(crate) type Reading = Option<Unreached>;

/// Whether this phone has a network path at all, as Swift last said: it has
/// one until it says otherwise.
#[derive(Clone)]
pub(crate) struct Network(Arc<watch::Sender<bool>>);

impl Default for Network {
    fn default() -> Self {
        Self(Arc::new(watch::Sender::new(true)))
    }
}

impl Network {
    pub fn has_path(&self) -> bool {
        *self.0.borrow()
    }

    pub fn set(&self, has_path: bool) {
        self.0.send_replace(has_path);
    }

    /// Why a Remote nothing answered is Unreachable, as the path stands.
    fn unreached(&self) -> Unreached {
        if self.has_path() {
            Unreached::NoWay
        } else {
            Unreached::NoNetwork
        }
    }
}

/// How one Remote reads, kept up as what is asked of it finds.
#[derive(Clone)]
pub(crate) struct Reach {
    inner: Arc<ReachInner>,
}

struct ReachInner {
    runtime: Handle,
    /// How long the Remote is reached anew without an answer before it
    /// reads Unreachable.
    hold: Duration,
    network: Network,
    reading: watch::Sender<Reading>,
    /// The Remote's being reached anew, while it is.
    settling: Mutex<Option<Settling>>,
    /// How many requests to it were answered, and how many failed by every
    /// way, so trying it again hears the next of either.
    answers: AtomicU64,
    failures: AtomicU64,
    /// Told of each.
    noted: Notify,
}

/// A Remote's being reached anew.
struct Settling {
    /// What ends it as Unreachable once the hold is out.
    timer: JoinHandle<()>,
    /// Whether a dial was abandoned in it, failing whatever waited on that:
    /// then failures say nothing of the ways, and only an answer or the hold
    /// ends it.
    excused: bool,
}

impl Drop for Settling {
    fn drop(&mut self) {
        self.timer.abort();
    }
}

impl Reach {
    /// A Remote not yet asked, which reads as answering, its hold `hold`,
    /// on a phone whose path is `network`.
    pub fn new(runtime: Handle, hold: Duration, network: Network) -> Self {
        Self {
            inner: Arc::new(ReachInner {
                runtime,
                hold,
                network,
                reading: watch::Sender::new(None),
                settling: Mutex::new(None),
                answers: AtomicU64::new(0),
                failures: AtomicU64::new(0),
                noted: Notify::new(),
            }),
        }
    }

    pub fn reading(&self) -> watch::Receiver<Reading> {
        self.inner.reading.subscribe()
    }

    /// Something asked of the Remote was answered.
    pub fn answered(&self) {
        self.inner.answers.fetch_add(1, Ordering::AcqRel);
        self.inner.end(None);
        self.inner.noted.notify_waiters();
    }

    /// Every way to the Remote failed, as `why` says; within a reaching
    /// anew that abandoned a dial, it is counted and changes nothing.
    pub fn unreached(&self, why: Unreached) {
        self.inner.failures.fetch_add(1, Ordering::AcqRel);
        let excused = why == Unreached::NoWay
            && self
                .inner
                .lock()
                .as_ref()
                .is_some_and(|settling| settling.excused);
        if !excused {
            self.inner.end(Some(why));
        }
        self.inner.noted.notify_waiters();
    }

    /// A dial to the Remote was abandoned as it is being reached anew, so
    /// what fails for it until that ends says nothing of the ways.
    pub fn excuse_failures(&self) {
        if let Some(settling) = self.inner.lock().as_mut() {
            settling.excused = true;
        }
    }

    /// How many requests to the Remote have been answered, and how many
    /// failed by every way, so far.
    pub fn noted(&self) -> (u64, u64) {
        (
            self.inner.answers.load(Ordering::Acquire),
            self.inner.failures.load(Ordering::Acquire),
        )
    }

    /// Waits until a request to the Remote is answered after `since` was
    /// noted, answering `true`, or, where `failing` says a failure counts,
    /// fails by every way, or the hold is out, answering `false`.
    pub async fn answers_after(&self, since: (u64, u64), failing: bool) -> bool {
        let waiting = async {
            loop {
                let noted = self.inner.noted.notified();
                let (answers, failures) = self.noted();
                if answers > since.0 {
                    return true;
                }
                if failing && failures > since.1 {
                    return false;
                }
                noted.await;
            }
        };
        tokio::time::timeout(self.inner.hold, waiting)
            .await
            .unwrap_or(false)
    }

    /// The Remote is being reached anew: it holds its reading until it
    /// answers, or every way fails, or the hold is out with neither, when it
    /// reads Unreachable. Reached anew while it is already, it waits on the
    /// one hold.
    pub fn settle(&self) {
        let mut settling = self.inner.lock();
        if settling.is_some() {
            return;
        }
        let inner = Arc::downgrade(&self.inner);
        let hold = self.inner.hold;
        let timer = self.inner.runtime.spawn(async move {
            tokio::time::sleep(hold).await;
            // Ending it aborts this very timer, which has nothing left to
            // wait on.
            if let Some(inner) = Weak::upgrade(&inner) {
                let why = inner.network.unreached();
                inner.end(Some(why));
            }
        });
        *settling = Some(Settling {
            timer,
            excused: false,
        });
    }
}

impl ReachInner {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Settling>> {
        self.settling
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Ends any reaching anew, with the Remote reading as `unreached` says.
    fn end(&self, unreached: Reading) {
        drop(self.lock().take());
        self.reading.send_if_modified(|reading| {
            let changed = *reading != unreached;
            *reading = unreached;
            changed
        });
    }
}

/// The words a request is refused with while this phone has no network.
const NO_NETWORK: &str = "This phone has no network connection";

/// A Remote's transport — its Pairing's — noting what each request it
/// carries finds in the Remote's reading, and dialling nothing while the
/// phone has no network path.
pub(crate) struct Reaching {
    transport: Arc<dyn Transport>,
    reach: Reach,
    network: Network,
}

impl Reaching {
    pub fn new(transport: Arc<dyn Transport>, reach: Reach, network: Network) -> Self {
        Self {
            transport,
            reach,
            network,
        }
    }
}

#[async_trait::async_trait]
impl Transport for Reaching {
    async fn send(&self, request: http::Request<Bytes>) -> Result<TransportResponse, SessionError> {
        if !self.network.has_path() {
            self.reach.unreached(Unreached::NoNetwork);
            return Err(SessionError {
                code: SessionErrorCode::PairingConnectionFailed,
                message: NO_NETWORK.to_owned(),
                unreachable: None,
            });
        }
        let answered = self.transport.send(request).await;
        match &answered {
            Ok(_) => self.reach.answered(),
            Err(failure)
                if matches!(
                    failure.code,
                    SessionErrorCode::PairingConnectionFailed | SessionErrorCode::ServerUnreachable
                ) =>
            {
                self.reach.unreached(self.network.unreached());
            }
            // Refused for its Pairing, or for being busy: either way it was
            // no answer about the ways.
            Err(_) => {}
        }
        answered
    }

    fn try_now(&self) -> TryNow {
        self.transport.try_now()
    }

    /// Itself: a Pairing's transport attaches as itself, and a stream over
    /// it is noted as any request is.
    fn attached(self: Arc<Self>) -> Arc<dyn Transport> {
        self
    }
}

/// Checks the Remote `name`, read over `transport`, as the app comes to the
/// front or the network path changes: it is reached anew, every reader over
/// it told to try now, and each connection it is asked over asked something
/// cheap within `deadline`, what answers kept and the rest given up, so the
/// readers that rode them redial at once; one still being dialled that does
/// not stand within `deadline` is abandoned and dialled afresh. With no
/// network path nothing is asked, and it reads Unreachable. What the check
/// found.
pub(crate) async fn check(
    redeemer: &Redeemer,
    name: &str,
    transport: &PairingTransport,
    reach: &Reach,
    deadline: Duration,
) -> ConnectionsChecked {
    if !reach.inner.network.has_path() {
        reach.unreached(Unreached::NoNetwork);
        return ConnectionsChecked::default();
    }
    reach.settle();
    transport.say_try_now();
    let checked = redeemer.check_connections(name, deadline).await;
    if checked.answered > 0 {
        reach.answered();
    }
    if checked.abandoned > 0 {
        reach.excuse_failures();
    }
    if checked.given_up > 0 || checked.abandoned > 0 {
        transport.say_try_now();
    }
    checked
}

/// Whether every way a Remote offering `ways` is dialled by — a Relay way
/// only where `dialled_relay` says its Relay is — is an address on the local
/// network: then a device refusing this app the local network fails them
/// all, as an unreachable host does.
pub(crate) fn only_local(ways: &[Way], dialled_relay: impl Fn(&str) -> bool) -> bool {
    let mut dialled = ways.iter().filter(|way| match way {
        Way::Direct(_) => true,
        Way::Relay(relay) => dialled_relay(relay),
        Way::Unknown => false,
    });
    let mut any = false;
    let all = dialled.all(|way| {
        any = true;
        matches!(way, Way::Direct(address) if local(address.ip()))
    });
    any && all
}

/// Whether `address` is on a local network: private or link-local, which a
/// device's local network permission governs. Loopback is not, nor is a
/// shared address such as a VPN's.
fn local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_private() || address.is_link_local(),
        IpAddr::V6(address) => match address.to_ipv4_mapped() {
            Some(address) => local(IpAddr::V4(address)),
            None => address.is_unique_local() || address.is_unicast_link_local(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use shaman_client::testing::{ScriptedTransport, unreachable as unreached_error};

    use super::*;

    const HOLD: Duration = Duration::from_secs(3);

    fn get() -> http::Request<Bytes> {
        http::Request::get("/v1/sessions")
            .body(Bytes::new())
            .unwrap()
    }

    #[tokio::test]
    async fn what_each_request_finds_is_noted_in_the_remotes_reading() {
        let refused = SessionError {
            code: SessionErrorCode::PairingAuthenticationFailed,
            message: "unknown peer".to_owned(),
            unreachable: None,
        };
        let undelivered = SessionError {
            code: SessionErrorCode::PairingConnectionFailed,
            message: "could not reach Remote at any paired address".to_owned(),
            unreachable: None,
        };
        let network = Network::default();
        let reach = reach(&network);
        let reaching = Arc::new(Reaching::new(
            Arc::new(ScriptedTransport::new(vec![
                Err(undelivered),
                Err(refused),
                Ok(http::Response::new(shaman_client::full_body(Vec::new()))),
                Err(unreached_error()),
            ])),
            reach.clone(),
            network,
        ));
        assert_eq!(unreached(&reach), None, "answering until asked");

        let _ = reaching.send(get()).await;
        assert_eq!(
            unreached(&reach),
            Some(Unreached::NoWay),
            "no way carried it"
        );
        let _ = reaching.send(get()).await;
        assert_eq!(
            unreached(&reach),
            Some(Unreached::NoWay),
            "a refusal of its Pairing says nothing of the ways"
        );
        let _ = reaching.clone().attached().send(get()).await;
        assert_eq!(
            unreached(&reach),
            None,
            "answered, over the attached transport too"
        );
        let _ = reaching.send(get()).await;
        assert_eq!(unreached(&reach), Some(Unreached::NoWay));
    }

    fn reach(network: &Network) -> Reach {
        Reach::new(Handle::current(), HOLD, network.clone())
    }

    fn unreached(reach: &Reach) -> Option<Unreached> {
        *reach.reading().borrow()
    }

    #[tokio::test(start_paused = true)]
    async fn a_remote_reached_anew_holds_its_reading_until_the_hold_is_out() {
        let network = Network::default();
        let reach = reach(&network);
        reach.settle();
        tokio::time::sleep(HOLD - Duration::from_millis(1)).await;
        assert_eq!(unreached(&reach), None, "it holds what it read");
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert_eq!(unreached(&reach), Some(Unreached::NoWay));
    }

    #[tokio::test(start_paused = true)]
    async fn every_way_failing_reads_unreachable_before_the_hold_is_out() {
        let reach = reach(&Network::default());
        reach.settle();
        reach.unreached(Unreached::NoWay);
        assert_eq!(unreached(&reach), Some(Unreached::NoWay), "at once");
        reach.answered();
        assert_eq!(unreached(&reach), None, "and back once it answers");
    }

    #[tokio::test(start_paused = true)]
    async fn an_answer_within_the_hold_keeps_it_answering_after() {
        let reach = reach(&Network::default());
        reach.settle();
        tokio::time::sleep(Duration::from_millis(100)).await;
        reach.answered();
        tokio::time::sleep(HOLD * 2).await;
        assert_eq!(unreached(&reach), None);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreachable_remote_reached_anew_reads_unreachable_until_it_answers() {
        let reach = reach(&Network::default());
        reach.unreached(Unreached::NoWay);
        reach.settle();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            unreached(&reach),
            Some(Unreached::NoWay),
            "it holds that too"
        );
        reach.answered();
        assert_eq!(unreached(&reach), None);
    }

    #[tokio::test(start_paused = true)]
    async fn trying_again_hears_the_next_answer_or_failure_or_the_hold() {
        let reach = reach(&Network::default());
        let answering = reach.clone();
        let since = reach.noted();
        let (answered, ()) = tokio::join!(reach.answers_after(since, true), async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            answering.answered();
        });
        assert!(answered);

        let failing = reach.clone();
        let since = reach.noted();
        let (answered, ()) = tokio::join!(reach.answers_after(since, true), async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            failing.unreached(Unreached::NoWay);
        });
        assert!(!answered, "a failure where failures count");

        let failing = reach.clone();
        let since = reach.noted();
        let started = tokio::time::Instant::now();
        let (answered, ()) = tokio::join!(reach.answers_after(since, false), async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            failing.unreached(Unreached::NoWay);
        });
        assert!(!answered);
        assert_eq!(
            started.elapsed(),
            HOLD,
            "where failures do not count, the hold is waited out"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn failures_for_a_dial_abandoned_are_excused_until_the_hold_is_out() {
        let reach = reach(&Network::default());
        reach.settle();
        reach.excuse_failures();
        reach.unreached(Unreached::NoWay);
        assert_eq!(unreached(&reach), None, "it holds its last state");
        tokio::time::sleep(HOLD).await;
        tokio::task::yield_now().await;
        assert_eq!(unreached(&reach), Some(Unreached::NoWay));
        reach.answered();
        reach.settle();
        reach.unreached(Unreached::NoWay);
        assert_eq!(
            unreached(&reach),
            Some(Unreached::NoWay),
            "a reaching anew of its own excuses nothing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_hold_out_with_no_network_reads_as_no_network() {
        let network = Network::default();
        let reach = reach(&network);
        network.set(false);
        reach.settle();
        tokio::time::sleep(HOLD * 2).await;
        assert_eq!(unreached(&reach), Some(Unreached::NoNetwork));
    }

    fn direct(address: &str) -> Way {
        Way::Direct(address.parse::<SocketAddr>().unwrap())
    }

    #[test]
    fn a_remote_whose_every_dialled_way_is_on_the_local_network_is_told_apart() {
        let never = |_: &str| false;
        assert!(only_local(
            &[direct("192.168.1.20:7791"), direct("10.0.0.4:7791")],
            never
        ));
        assert!(only_local(&[direct("[fe80::1]:7791")], never));
        assert!(only_local(&[direct("[fd7a:115c::1]:7791")], never));
        assert!(only_local(&[direct("172.16.0.9:7791")], never));
        assert!(only_local(&[direct("169.254.3.1:7791")], never));
        assert!(only_local(
            &[
                direct("192.168.1.20:7791"),
                Way::Relay("https://relay.example.com".to_owned())
            ],
            never
        ));
        assert!(
            !only_local(
                &[
                    direct("192.168.1.20:7791"),
                    Way::Relay("https://relay.example.com".to_owned())
                ],
                |_| true
            ),
            "a Relay way dialled is not on the local network"
        );
        assert!(
            !only_local(
                &[direct("192.168.1.20:7791"), direct("100.64.0.7:7791")],
                never
            ),
            "a VPN's shared address is not"
        );
        assert!(!only_local(&[direct("127.0.0.1:7791")], never));
        assert!(!only_local(&[direct("203.0.113.9:7791")], never));
        assert!(!only_local(&[], never), "no way at all is no local way");
    }
}
