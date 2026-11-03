//! Orders Copilot's final on-wire Session events ahead of a harness crash.
//!
//! The SDK reads stdout on its own task and routes each `session.event` through two more tasks
//! before a [`github_copilot_sdk::EventSubscription`] sees it. The process supervisor can observe
//! the child exit while those tasks still hold complete frames that were written before it died.
//! This module keeps a ledger of the last event identifier read for each Session and the last one
//! delivered by the SDK. Once stdout ends, matching those identifiers is the barrier that says every
//! preceding event has reached Suru's own queue.
//!
//! The SDK also stops routing a Session's events the moment the connection closes, dropping those
//! it had read but not yet delivered. So for each Session a checkpoint tracks, the ledger holds the
//! events read and not yet delivered, which are the ones a closed timeline can hand back.

use std::{
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{Arc, Mutex as StdMutex},
    task::{Context, Poll},
};

use github_copilot_sdk::SessionEvent;
use serde::Deserialize;
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::watch,
};

/// Shared progress between the stdout observer and the Session-event drain.
#[derive(Clone)]
pub(super) struct CopilotEventDrain {
    state: Arc<StdMutex<DrainState>>,
    changed: watch::Sender<u64>,
}

#[derive(Default)]
struct DrainState {
    stdout_ended: bool,
    wire_tail: HashMap<String, String>,
    delivered_tail: HashMap<String, String>,
    /// For each Session a checkpoint tracks, how many checkpoints track it and the events read for
    /// it that the SDK has not delivered yet, oldest first.
    undelivered: HashMap<String, (usize, VecDeque<SessionEvent>)>,
}

impl CopilotEventDrain {
    pub(super) fn new() -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            state: Arc::new(StdMutex::new(DrainState::default())),
            changed,
        }
    }

    /// Observes complete JSON-RPC frames while leaving the byte stream unchanged for the SDK.
    pub(super) fn observe<R: AsyncRead + Unpin>(&self, reader: R) -> EventTrackingReader<R> {
        EventTrackingReader {
            reader,
            frames: FrameBuffer::default(),
            drain: self.clone(),
        }
    }

    /// Starts one Session's drain accounting after its SDK subscription already exists.
    pub(super) fn checkpoint(&self, session_id: impl Into<String>) -> EventDrainCheckpoint {
        let session_id = session_id.into();
        let baseline = {
            let mut state = self
                .state
                .lock()
                .expect("Copilot event-drain lock is not poisoned");
            state.undelivered.entry(session_id.clone()).or_default().0 += 1;
            state.wire_tail.get(&session_id).cloned()
        };
        EventDrainCheckpoint {
            tracking: Arc::new(Tracking {
                drain: self.clone(),
                session_id,
            }),
            baseline,
        }
    }

    /// Whether the CLI's stdout has ended, after which no further event can arrive.
    pub(super) fn stdout_ended(&self) -> bool {
        self.state
            .lock()
            .expect("Copilot event-drain lock is not poisoned")
            .stdout_ended
    }

    fn record_wire_event(&self, session_id: String, event: SessionEvent) {
        {
            let mut state = self
                .state
                .lock()
                .expect("Copilot event-drain lock is not poisoned");
            state.wire_tail.insert(session_id.clone(), event.id.clone());
            if let Some((_, undelivered)) = state.undelivered.get_mut(&session_id) {
                undelivered.push_back(event);
            }
        }
        self.notify();
    }

    fn record_delivered_event(&self, session_id: String, event_id: String) {
        {
            let mut state = self
                .state
                .lock()
                .expect("Copilot event-drain lock is not poisoned");
            // Whatever the SDK passed over on the way to this event it never delivers.
            if let Some((_, undelivered)) = state.undelivered.get_mut(&session_id)
                && let Some(position) = undelivered.iter().position(|event| event.id == event_id)
            {
                undelivered.drain(..=position);
            }
            state.delivered_tail.insert(session_id, event_id);
        }
        self.notify();
    }

    fn end_stdout(&self) {
        let changed = {
            let mut state = self
                .state
                .lock()
                .expect("Copilot event-drain lock is not poisoned");
            if state.stdout_ended {
                false
            } else {
                state.stdout_ended = true;
                true
            }
        };
        if changed {
            self.notify();
        }
    }

    fn notify(&self) {
        let next = self.changed.borrow().wrapping_add(1);
        self.changed.send_replace(next);
    }
}

/// One Session's position when Suru subscribed to its SDK timeline.
#[derive(Clone)]
pub(super) struct EventDrainCheckpoint {
    tracking: Arc<Tracking>,
    baseline: Option<String>,
}

/// A Session's events held until delivered for as long as some checkpoint tracks it.
struct Tracking {
    drain: CopilotEventDrain,
    session_id: String,
}

impl Drop for Tracking {
    fn drop(&mut self) {
        let mut state = self
            .drain
            .state
            .lock()
            .expect("Copilot event-drain lock is not poisoned");
        if let Some((tracked, _)) = state.undelivered.get_mut(&self.session_id) {
            *tracked -= 1;
            if *tracked == 0 {
                state.undelivered.remove(&self.session_id);
            }
        }
    }
}

impl EventDrainCheckpoint {
    /// Records an event only after it has entered Suru's unbounded projection queue.
    pub(super) fn delivered(&self, event_id: String) {
        self.tracking
            .drain
            .record_delivered_event(self.tracking.session_id.clone(), event_id);
    }

    /// The events stdout carried that the SDK dropped instead of delivering, oldest first, once
    /// stdout has ended and the timeline has closed on them. Before stdout ends nothing has been
    /// dropped, so there are none.
    pub(super) fn dropped(&self) -> Vec<SessionEvent> {
        let mut state = self
            .tracking
            .drain
            .state
            .lock()
            .expect("Copilot event-drain lock is not poisoned");
        if !state.stdout_ended {
            return Vec::new();
        }
        state
            .undelivered
            .get_mut(&self.tracking.session_id)
            .map(|(_, undelivered)| undelivered.drain(..).collect())
            .unwrap_or_default()
    }

    /// Waits for stdout to end and for its final event after this checkpoint to reach Suru.
    pub(super) async fn wait_until_drained(&self) {
        let session_id = &self.tracking.session_id;
        let mut changed = self.tracking.drain.changed.subscribe();
        loop {
            let drained = {
                let state = self
                    .tracking
                    .drain
                    .state
                    .lock()
                    .expect("Copilot event-drain lock is not poisoned");
                if !state.stdout_ended {
                    false
                } else {
                    let wire_tail = state.wire_tail.get(session_id);
                    wire_tail == self.baseline.as_ref()
                        || wire_tail.is_some_and(|wire_tail| {
                            state.delivered_tail.get(session_id) == Some(wire_tail)
                        })
                }
            };
            if drained {
                return;
            }
            if changed.changed().await.is_err() {
                return;
            }
        }
    }
}

/// An unchanged stdout stream with a sidecar parser for event identifiers.
pub(super) struct EventTrackingReader<R> {
    reader: R,
    frames: FrameBuffer,
    drain: CopilotEventDrain,
}

impl<R: AsyncRead + Unpin> AsyncRead for EventTrackingReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let filled_before = buffer.filled().len();
        let result = Pin::new(&mut self.reader).poll_read(context, buffer);
        match &result {
            Poll::Ready(Ok(())) => {
                let received = &buffer.filled()[filled_before..];
                if received.is_empty() {
                    self.drain.end_stdout();
                } else {
                    let drain = self.drain.clone();
                    self.frames.push(received, &drain);
                }
            }
            Poll::Ready(Err(_)) => self.drain.end_stdout(),
            Poll::Pending => {}
        }
        result
    }
}

impl<R> Drop for EventTrackingReader<R> {
    fn drop(&mut self) {
        // The SDK also drops its reader after a protocol error, without polling the pipe to EOF.
        // That is still the end of what can traverse this connection, so crash settlement must not
        // wait forever for another read.
        self.drain.end_stdout();
    }
}

#[derive(Default)]
struct FrameBuffer {
    bytes: Vec<u8>,
}

impl FrameBuffer {
    fn push(&mut self, bytes: &[u8], drain: &CopilotEventDrain) {
        self.bytes.extend_from_slice(bytes);
        loop {
            let Some((header_end, separator_len)) = header_boundary(&self.bytes) else {
                return;
            };
            let body_start = header_end + separator_len;
            let Some(content_length) = content_length(&self.bytes[..header_end]) else {
                self.bytes.drain(..body_start);
                continue;
            };
            let frame_end = body_start.saturating_add(content_length);
            if self.bytes.len() < frame_end {
                return;
            }
            if let Ok(frame) =
                serde_json::from_slice::<WireMessage>(&self.bytes[body_start..frame_end])
                && frame.method.as_deref() == Some("session.event")
                && let Some(params) = frame.params
                && let Ok(notification) =
                    serde_json::from_value::<github_copilot_sdk::SessionEventNotification>(params)
            {
                drain.record_wire_event(notification.session_id.to_string(), notification.event);
            }
            self.bytes.drain(..frame_end);
        }
    }
}

fn header_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
    let crlf = bytes.windows(4).position(|window| window == b"\r\n\r\n");
    let lf = bytes.windows(2).position(|window| window == b"\n\n");
    match (crlf, lf) {
        (Some(crlf), Some(lf)) if crlf <= lf => Some((crlf, 4)),
        (Some(_), Some(lf)) => Some((lf, 2)),
        (Some(crlf), None) => Some((crlf, 4)),
        (None, Some(lf)) => Some((lf, 2)),
        (None, None) => None,
    }
}

fn content_length(header: &[u8]) -> Option<usize> {
    std::str::from_utf8(header)
        .ok()?
        .lines()
        .filter_map(|line| line.trim_end_matches('\r').split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))?
        .1
        .trim()
        .parse()
        .ok()
}

#[derive(Deserialize)]
struct WireMessage {
    method: Option<String>,
    params: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        time::{Duration, timeout},
    };

    use super::CopilotEventDrain;

    fn event_frame(session: &str, id: &str) -> String {
        let body = format!(
            r#"{{"jsonrpc":"2.0","method":"session.event","params":{{"sessionId":"{session}","event":{{"id":"{id}","timestamp":"2026-01-01T00:00:00Z","parentId":null,"type":"assistant.message_delta","data":{{}}}}}}}}"#
        );
        format!("Content-Length: {}\r\n\r\n{body}", body.len())
    }

    #[tokio::test]
    async fn the_events_a_closed_timeline_dropped_are_those_read_after_the_last_delivered() {
        let drain = CopilotEventDrain::new();
        let checkpoint = drain.checkpoint("session-1");
        let (mut writer, reader) = tokio::io::duplex(4096);
        let mut observed = drain.observe(reader);
        let frames = [
            event_frame("session-1", "first"),
            event_frame("session-2", "elsewhere"),
            event_frame("session-1", "second"),
            event_frame("session-1", "third"),
        ]
        .concat();
        writer
            .write_all(frames.as_bytes())
            .await
            .expect("write events");
        let mut read = vec![0; frames.len()];
        observed
            .read_exact(&mut read)
            .await
            .expect("SDK-side reader reads the events");
        checkpoint.delivered("first".to_owned());
        assert!(
            checkpoint.dropped().is_empty(),
            "nothing is dropped while stdout still runs"
        );
        drop(writer);
        let mut received = Vec::new();
        observed
            .read_to_end(&mut received)
            .await
            .expect("SDK-side reader reaches EOF");

        let dropped: Vec<_> = checkpoint
            .dropped()
            .into_iter()
            .map(|event| event.id)
            .collect();
        assert_eq!(dropped, ["second", "third"]);
        assert!(checkpoint.dropped().is_empty(), "each is handed back once");
    }

    #[tokio::test]
    async fn a_crash_barrier_waits_until_the_final_on_wire_event_reaches_suru() {
        let drain = CopilotEventDrain::new();
        let checkpoint = drain.checkpoint("session-1");
        let (mut writer, reader) = tokio::io::duplex(1024);
        let mut observed = drain.observe(reader);
        let body = r#"{"jsonrpc":"2.0","method":"session.event","params":{"sessionId":"session-1","event":{"id":"last","timestamp":"2026-01-01T00:00:00Z","parentId":null,"type":"assistant.message_delta","data":{}}}}"#;
        let frame = format!("Content-Length: {}\r\n\r\n{body}", body.len());

        writer
            .write_all(frame.as_bytes())
            .await
            .expect("write event");
        drop(writer);
        let mut received = Vec::new();
        observed
            .read_to_end(&mut received)
            .await
            .expect("SDK-side reader reaches EOF");

        assert!(
            timeout(Duration::from_millis(10), checkpoint.wait_until_drained())
                .await
                .is_err(),
            "stdout EOF alone cannot overtake an event still inside the SDK"
        );
        checkpoint.delivered("last".to_owned());
        timeout(Duration::from_millis(10), checkpoint.wait_until_drained())
            .await
            .expect("the final delivered event opens the crash barrier");
    }
}
