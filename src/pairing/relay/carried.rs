//! The joins a Relay carries: each the byte stream a Pairing's pinned-key TLS
//! runs over, ended as whatever holds the Relay client stops.

use std::{
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context as TaskContext, Poll, ready},
};

use futures_util::{Sink, StreamExt, task::AtomicWaker};
use shaman_relay_protocol::MAX_MESSAGE_LEN;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{self, Message},
};

/// The joins Relays carry for this Server, each ended as the Server stops.
#[derive(Default)]
pub(super) struct Carrying {
    carried: StdMutex<Carried>,
}

#[derive(Default)]
struct Carried {
    stopped: bool,
    stops: Vec<Weak<Stop>>,
}

impl Carrying {
    /// What ends one more join as the Server stops — at once, where it
    /// already has.
    pub(super) fn stop(&self) -> Arc<Stop> {
        let stop = Arc::new(Stop::default());
        let mut carried = self
            .carried
            .lock()
            .expect("carried join lock is not poisoned");
        if carried.stopped {
            stop.stop();
        } else {
            carried.stops.retain(|stop| stop.strong_count() > 0);
            carried.stops.push(Arc::downgrade(&stop));
        }
        stop
    }

    /// Ends every join carried, and every one carried from now on.
    pub(super) fn stop_all(&self) {
        let stops = {
            let mut carried = self
                .carried
                .lock()
                .expect("carried join lock is not poisoned");
            carried.stopped = true;
            std::mem::take(&mut carried.stops)
        };
        for stop in stops.iter().filter_map(Weak::upgrade) {
            stop.stop();
        }
    }
}

/// Says one join a Relay carries is to end, waking whatever reads or writes
/// it.
#[derive(Default)]
pub(super) struct Stop {
    stopped: AtomicBool,
    waker: AtomicWaker,
}

impl Stop {
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.waker.wake();
    }

    fn poll(&self, context: &TaskContext<'_>) -> bool {
        self.waker.register(context.waker());
        self.stopped.load(Ordering::Acquire)
    }
}

/// A join a Relay carries, as the byte stream the Pairing's pinned-key TLS
/// runs over: what is written goes to the Relay in binary frames of at most
/// [`MAX_MESSAGE_LEN`] bytes, and the bytes of each binary frame the Relay
/// carries back are read as they come, no more than one frame held unread at
/// once. The Relay closing the join or saying anything else ends it, and so
/// does the Server stopping.
///
/// Whether the Relay, and the Server it joins this one to, still answer is
/// not judged here: what a Relay echoes of a join shows only that it read
/// this far of what this Server sent, which waits on its passing on all
/// before, however healthy it is. The two Servers judge it end to end, inside
/// their pinned-key TLS, as [`crate::pairing::Keepalive`] says — until
/// that is under way, their TLS handshake and then HTTP/2's start are each
/// bounded by the handshake timeout — and either drops the join, whatever it
/// was still sending, once it has stalled past that keepalive's verdict.
pub(crate) struct CarriedStream<Socket = reqwest::Upgraded> {
    socket: WebSocketStream<Socket>,
    /// What is left unread of the latest frame the Relay carried.
    unread: tungstenite::Bytes,
    /// Whether the Relay has closed the join.
    ended: bool,
    stop: Arc<Stop>,
}

impl<Socket: AsyncRead + AsyncWrite + Unpin> CarriedStream<Socket> {
    /// The join `socket` carries, ended by `stop`.
    pub(super) fn new(socket: WebSocketStream<Socket>, stop: Arc<Stop>) -> Self {
        Self {
            socket,
            unread: tungstenite::Bytes::new(),
            ended: false,
            stop,
        }
    }

    /// Takes in what the Relay has carried until bytes are held unread:
    /// `true` where they are, `false` once the join has ended, and pending
    /// where nothing more has come yet. No more than one frame is held at
    /// once.
    fn poll_take_in(&mut self, context: &mut TaskContext<'_>) -> Poll<std::io::Result<bool>> {
        loop {
            if !self.unread.is_empty() {
                return Poll::Ready(Ok(true));
            }
            if self.ended {
                return Poll::Ready(Ok(false));
            }
            match ready!(self.socket.poll_next_unpin(context)) {
                Some(Ok(Message::Binary(bytes))) => self.unread = bytes,
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                Some(Ok(Message::Close(_))) | None => self.ended = true,
                Some(Ok(Message::Text(_))) => {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "the Relay said something that is no part of the join it carries",
                    )));
                }
                Some(Err(error)) => return Poll::Ready(Err(std::io::Error::other(error))),
            }
        }
    }

    /// An error where the Server is stopping, which ends every join.
    fn stopping(&self, context: &TaskContext<'_>) -> std::io::Result<()> {
        if self.stop.poll(context) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "the Server is stopping",
            ));
        }
        Ok(())
    }
}

impl<Socket: AsyncRead + AsyncWrite + Unpin> AsyncRead for CarriedStream<Socket> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        this.stopping(context)?;
        if ready!(this.poll_take_in(context))? {
            let length = this.unread.len().min(buffer.remaining());
            let read = this.unread.split_to(length);
            buffer.put_slice(&read);
        }
        Poll::Ready(Ok(()))
    }
}

impl<Socket: AsyncRead + AsyncWrite + Unpin> AsyncWrite for CarriedStream<Socket> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if buffer.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let this = &mut *self;
        this.stopping(context)?;
        ready!(Pin::new(&mut this.socket).poll_ready(context)).map_err(std::io::Error::other)?;
        let length = buffer.len().min(MAX_MESSAGE_LEN);
        let frame = Message::Binary(tungstenite::Bytes::copy_from_slice(&buffer[..length]));
        Pin::new(&mut this.socket)
            .start_send(frame)
            .map_err(std::io::Error::other)?;
        Poll::Ready(Ok(length))
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        this.stopping(context)?;
        Pin::new(&mut this.socket)
            .poll_flush(context)
            .map_err(std::io::Error::other)
    }

    /// Closes the join, waiting on none once the Server stops.
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.stop.poll(context) {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.socket)
            .poll_close(context)
            .map_err(std::io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_tungstenite::tungstenite::protocol::Role;

    use super::*;

    /// A join over one end of a pipe, ended by `stop`; and the Relay's end.
    async fn carried_pair(
        stop: Arc<Stop>,
    ) -> (
        CarriedStream<tokio::io::DuplexStream>,
        WebSocketStream<tokio::io::DuplexStream>,
    ) {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        (
            CarriedStream::new(
                WebSocketStream::from_raw_socket(ours, Role::Client, None).await,
                stop,
            ),
            WebSocketStream::from_raw_socket(theirs, Role::Server, None).await,
        )
    }

    /// The error reading `carried` ends in, once it ends within a second.
    async fn ending(carried: &mut CarriedStream<tokio::io::DuplexStream>) -> std::io::Error {
        use tokio::io::AsyncReadExt as _;
        let read = tokio::time::timeout(Duration::from_secs(1), async {
            let mut buffer = [0_u8; 64];
            loop {
                if let Err(error) = carried.read(&mut buffer).await {
                    return error;
                }
            }
        });
        read.await.expect("the join ends in time")
    }

    /// What `inner` carries, taken in no more than a kilobyte at a time and
    /// a while apart.
    struct Slowly<S> {
        inner: S,
        next: Pin<Box<tokio::time::Sleep>>,
    }

    impl<S: AsyncRead + Unpin> AsyncRead for Slowly<S> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            ready!(self.next.as_mut().poll(context));
            let mut chunk = [0_u8; 1024];
            let room = chunk.len().min(buffer.remaining());
            let mut chunk = ReadBuf::new(&mut chunk[..room]);
            ready!(Pin::new(&mut self.inner).poll_read(context, &mut chunk))?;
            buffer.put_slice(chunk.filled());
            let next = tokio::time::Instant::now() + Duration::from_millis(5);
            self.next.as_mut().reset(next);
            Poll::Ready(Ok(()))
        }
    }

    impl<S: AsyncWrite + Unpin> AsyncWrite for Slowly<S> {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
            buffer: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.inner).poll_write(context, buffer)
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(context)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(context)
        }
    }

    /// A Relay that takes in what this Server sends slowly but steadily,
    /// each frame taking it a while, ends no transfer: how long a frame
    /// takes to go is no sign the Relay has stopped answering. What the
    /// Relay takes in is counted as the bytes the frames carry, not those
    /// framing them.
    #[tokio::test]
    async fn a_relay_taking_in_slowly_but_steadily_ends_no_transfer() {
        use tokio::io::AsyncWriteExt as _;
        const SENT: usize = 128 * 1024;
        let (ours, theirs) = tokio::io::duplex(1024);
        let mut carried = CarriedStream::new(
            WebSocketStream::from_raw_socket(ours, Role::Client, None).await,
            Arc::default(),
        );
        let taking_in = tokio::spawn(async move {
            let slowly = Slowly {
                inner: theirs,
                next: Box::pin(tokio::time::sleep(Duration::ZERO)),
            };
            let mut relay = WebSocketStream::from_raw_socket(slowly, Role::Server, None).await;
            let mut taken = 0;
            while taken < SENT {
                match relay.next().await {
                    Some(Ok(Message::Binary(carried))) => taken += carried.len(),
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
            (relay, taken)
        });

        let sent = tokio::time::timeout(Duration::from_secs(10), async {
            carried.write_all(&vec![7; SENT]).await?;
            carried.flush().await
        })
        .await
        .expect("the transfer finishes in time");
        sent.expect("the transfer goes on however slowly the Relay takes it in");
        let (_relay, taken) = taking_in.await.unwrap();
        assert_eq!(taken, SENT, "the Relay took in all that was sent");
    }

    #[tokio::test]
    async fn every_join_carried_ends_as_the_server_stops_and_any_carried_after() {
        let carrying = Carrying::default();
        let (mut carried, _relay) = carried_pair(carrying.stop()).await;
        carrying.stop_all();
        assert_eq!(
            ending(&mut carried).await.kind(),
            std::io::ErrorKind::ConnectionAborted
        );
        let (mut late, _relay) = carried_pair(carrying.stop()).await;
        assert_eq!(
            ending(&mut late).await.kind(),
            std::io::ErrorKind::ConnectionAborted
        );
    }
}
