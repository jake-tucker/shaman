//! How an act reaches its Server, and what Swift is told of acts under way.
//!
//! An act asked while its Remote is being redialed waits and goes when the
//! Remote answers: a request no way carried is asked again, at once when the
//! transport says to try now and otherwise on the readers' backoff, until an
//! act's bound passes. One that may have reached the Remote is never asked
//! again, so a request whose answer was lost, or still to come when the bound
//! passed, fails as one that may have been done.
//!
//! The core tells Swift when acts begin and end, as whether any is under way,
//! so Swift holds iOS background time only between the two.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use shaman_client::{NotReached, RecoveryBackoff, TryNow};
use shaman_protocol::{SessionError, SessionErrorCode};
use tokio::sync::watch;

use crate::ActError;

/// How many acts are under way, and whether any is, which Swift is told.
#[derive(Clone)]
pub(crate) struct Acts {
    under_way: Arc<Mutex<usize>>,
    busy: watch::Sender<bool>,
}

impl Default for Acts {
    fn default() -> Self {
        Self {
            under_way: Arc::default(),
            busy: watch::Sender::new(false),
        }
    }
}

impl Acts {
    /// Whether any act is under way, now and on every change.
    pub fn busy(&self) -> watch::Receiver<bool> {
        self.busy.subscribe()
    }

    /// Marks an act under way until what this answers is dropped.
    pub fn begin(&self) -> UnderWay {
        self.count(|under_way| *under_way += 1);
        UnderWay(self.clone())
    }

    fn count(&self, change: impl FnOnce(&mut usize)) {
        let mut under_way = self
            .under_way
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        change(&mut under_way);
        let busy = *under_way > 0;
        self.busy.send_if_modified(|was| {
            let changed = *was != busy;
            *was = busy;
            changed
        });
    }
}

/// One act under way, until it is dropped.
pub(crate) struct UnderWay(Acts);

impl Drop for UnderWay {
    fn drop(&mut self) {
        self.0
            .count(|under_way| *under_way = under_way.saturating_sub(1));
    }
}

/// How an act's asking of its Server waits: between one undelivered attempt
/// and the next, and in all.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Asking {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// How long it waits on an answer before it fails as Unreachable.
    pub bound: Duration,
}

/// Why an act's request was not answered as asked.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unanswered {
    /// It was not done: the Server refused it, or no way carried it before
    /// the bound passed. Said as it is to be shown.
    NotDone(ActError),
    /// It may have reached the Server: its answer was lost, or had yet to
    /// come when the bound passed.
    MaybeDone,
}

/// What a request's failure says of where it got.
enum Reached {
    /// No way carried it, so it is safe to ask again.
    Undelivered,
    /// It was carried and its answer lost.
    Lost,
    /// The Server answered it, refusing it.
    Answered,
}

fn reached(error: &anyhow::Error) -> Reached {
    if error.downcast_ref::<NotReached>().is_some() {
        return Reached::Undelivered;
    }
    match error
        .downcast_ref::<SessionError>()
        .map(|refusal| refusal.code)
    {
        Some(SessionErrorCode::PairingConnectionFailed) => Reached::Undelivered,
        Some(SessionErrorCode::PairingOutcomeUnknown) => Reached::Lost,
        _ => Reached::Answered,
    }
}

/// What a request said no Server reached, as the banner says it.
const NOT_IN_TIME: &str = "The server did not answer in time";

/// Asks `ask` of a Server, again each time no way carried it, at once where
/// `try_now` says to, until it is answered or `asking`'s bound passes.
pub(crate) async fn deliver<T, F>(
    asking: Asking,
    try_now: &mut TryNow,
    mut ask: impl FnMut() -> F,
) -> Result<T, Unanswered>
where
    F: Future<Output = anyhow::Result<T>>,
{
    let deadline = tokio::time::Instant::now() + asking.bound;
    let mut backoff = RecoveryBackoff::new(asking.initial_backoff, asking.max_backoff);
    loop {
        let error = match tokio::time::timeout_at(deadline, ask()).await {
            Ok(Ok(answered)) => return Ok(answered),
            Ok(Err(error)) => error,
            // It was asked, and may have been carried.
            Err(_) => return Err(Unanswered::MaybeDone),
        };
        match reached(&error) {
            Reached::Undelivered => {
                tracing::debug!("an act went undelivered, and waits to go: {error:#}");
                let retry_in = backoff.next();
                if tokio::time::timeout_at(deadline, try_now.or_after(retry_in))
                    .await
                    .is_err()
                {
                    return Err(Unanswered::NotDone(ActError::Unreachable {
                        message: NOT_IN_TIME.to_owned(),
                    }));
                }
            }
            Reached::Lost => return Err(Unanswered::MaybeDone),
            Reached::Answered => return Err(Unanswered::NotDone(crate::listing::failed(error))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use shaman_client::{
        SessionCommandTimings, SessionCommands, Transport as _, testing::ScriptedTransport,
    };
    use shaman_protocol::SessionId;

    use super::*;

    fn asking(bound: Duration) -> Asking {
        Asking {
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
            bound,
        }
    }

    fn failure(code: SessionErrorCode, message: &str) -> SessionError {
        SessionError {
            code,
            message: message.to_owned(),
            unreachable: None,
        }
    }

    fn commands(transport: &Arc<ScriptedTransport>) -> SessionCommands {
        SessionCommands::new(
            transport.clone(),
            SessionCommandTimings {
                initial_recovery_backoff: Duration::from_millis(1),
                max_recovery_backoff: Duration::from_millis(5),
                attachment_fetch_timeout: Duration::from_secs(5),
            },
        )
    }

    fn interrupted() -> shaman_client::TransportResponse {
        http::Response::new(shaman_client::full_body(Vec::new()))
    }

    #[tokio::test]
    async fn an_act_no_way_carried_waits_and_goes_when_its_server_answers() {
        let transport = Arc::new(ScriptedTransport::new(vec![
            Err(failure(
                SessionErrorCode::PairingConnectionFailed,
                "could not reach Remote at any paired address",
            )),
            Err(shaman_client::testing::unreachable()),
            Ok(interrupted()),
        ]));
        let commands = commands(&transport);
        let mut try_now = transport.try_now();
        let session = SessionId::new();
        let answered = deliver(asking(Duration::from_secs(5)), &mut try_now, || {
            commands.interrupt_session(session)
        })
        .await;
        assert_eq!(answered, Ok(()));
        assert_eq!(
            transport.asked(),
            3,
            "asked again each time it went undelivered"
        );
    }

    #[tokio::test]
    async fn an_act_whose_answer_was_lost_is_never_asked_again() {
        let transport = Arc::new(ScriptedTransport::new(vec![
            Err(failure(
                SessionErrorCode::PairingOutcomeUnknown,
                "Remote API request was delivered, and its answer was lost",
            )),
            Ok(interrupted()),
        ]));
        let commands = commands(&transport);
        let mut try_now = transport.try_now();
        let session = SessionId::new();
        let lost = deliver(asking(Duration::from_secs(5)), &mut try_now, || {
            commands.interrupt_session(session)
        })
        .await;
        assert_eq!(lost, Err(Unanswered::MaybeDone));
        assert_eq!(transport.asked(), 1);
    }

    #[tokio::test]
    async fn an_act_still_unanswered_when_its_bound_passes_may_have_been_done() {
        let stalled = shaman_client::testing::Answering::new(|_| std::future::pending());
        let commands = SessionCommands::new(
            stalled,
            SessionCommandTimings {
                initial_recovery_backoff: Duration::from_millis(1),
                max_recovery_backoff: Duration::from_millis(5),
                attachment_fetch_timeout: Duration::from_secs(5),
            },
        );
        let session = SessionId::new();
        let unanswered = deliver(
            asking(Duration::from_millis(20)),
            &mut TryNow::never(),
            || commands.interrupt_session(session),
        )
        .await;
        assert_eq!(unanswered, Err(Unanswered::MaybeDone));
    }

    #[tokio::test]
    async fn an_act_no_way_carried_by_its_bound_was_not_done_and_says_so() {
        let transport = Arc::new(ScriptedTransport::new(Vec::new()));
        let commands = commands(&transport);
        let session = SessionId::new();
        let unreached = deliver(
            asking(Duration::from_millis(30)),
            &mut TryNow::never(),
            || commands.interrupt_session(session),
        )
        .await;
        assert_eq!(
            unreached,
            Err(Unanswered::NotDone(ActError::Unreachable {
                message: NOT_IN_TIME.to_owned()
            }))
        );
        assert!(transport.asked() > 1, "asked until the bound passed");
    }

    #[tokio::test]
    async fn an_act_its_server_refused_fails_in_its_words_at_once() {
        let refused = failure(SessionErrorCode::SessionNotFound, "Session not found");
        let transport = Arc::new(ScriptedTransport::new(vec![Ok(
            shaman_client::testing::refusal(http::StatusCode::NOT_FOUND, &refused),
        )]));
        let commands = commands(&transport);
        let mut try_now = transport.try_now();
        let session = SessionId::new();
        let gone = deliver(asking(Duration::from_secs(5)), &mut try_now, || {
            commands.interrupt_session(session)
        })
        .await;
        assert_eq!(
            gone,
            Err(Unanswered::NotDone(ActError::Gone {
                message: "Session not found".to_owned()
            }))
        );
        assert_eq!(transport.asked(), 1);
    }

    #[test]
    fn swift_is_told_whether_any_act_is_under_way_only_as_that_changes() {
        let acts = Acts::default();
        let mut busy = acts.busy();
        assert!(!*busy.borrow_and_update());
        let first = acts.begin();
        assert!(busy.has_changed().unwrap());
        assert!(*busy.borrow_and_update());
        let second = acts.begin();
        drop(first);
        assert!(!busy.has_changed().unwrap(), "one is still under way");
        drop(second);
        assert!(!*busy.borrow_and_update());
    }
}
