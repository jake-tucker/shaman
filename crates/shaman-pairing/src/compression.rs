//! gzip on a Pairing's wire: what a Serving Server sends a Peer of its own
//! Session API, compressed where the Peer asks by `Accept-Encoding` and
//! answered with `Content-Encoding`, and read back plain on the Peer's side
//! before any reader sees it.
//!
//! Both halves live here, the Serving listener's encoder and the redeemer's
//! decoder, so the two are held to each other by one set of tests. Every
//! JSON answer and every event stream is compressed; Attachment bytes and
//! anything else pass as they are, and so does every request body. A live
//! stream is compressed whole, as one gzip member, and flushed after every
//! piece of it the Session API hands on, keep-alives included, so no event
//! waits in the encoder on a later one; tower-http's encoder skips event
//! streams and never flushes, so this is Shaman's own.
//!
//! It is change by addition (ADR-0078): an older Serving Server ignores the
//! header and answers plain, which is read as it came, and an older Peer
//! never asks, so it is answered plain.

use std::{
    io::Write as _,
    pin::Pin,
    task::{Context, Poll, ready},
};

use bytes::Bytes;
use flate2::{Compression, write::GzDecoder, write::GzEncoder};
use futures_util::Stream;
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};

use crate::BoxError;

/// The one coding a Pairing speaks, as `Accept-Encoding` asks for it and
/// `Content-Encoding` answers it.
pub const GZIP: &str = "gzip";

/// Whether a request carrying `headers` asks for gzip by `Accept-Encoding`:
/// it names `gzip` (or its old spelling `x-gzip`), or `*`, with a weight
/// above zero.
pub fn asks_for_gzip(headers: &HeaderMap) -> bool {
    let mut gzip = None;
    let mut any = None;
    for value in headers.get_all(header::ACCEPT_ENCODING) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for coding in value.split(',') {
            let mut parameters = coding.split(';');
            let name = parameters.next().unwrap_or_default().trim();
            let weighted = parameters
                .filter_map(|parameter| {
                    let (name, weight) = parameter.split_once('=')?;
                    name.trim().eq_ignore_ascii_case("q").then_some(weight)
                })
                .next_back()
                .is_none_or(|weight| {
                    weight
                        .trim()
                        .parse::<f32>()
                        .is_ok_and(|weight| weight > 0.0)
                });
            if name.eq_ignore_ascii_case(GZIP) || name.eq_ignore_ascii_case("x-gzip") {
                gzip = Some(weighted);
            } else if name == "*" {
                any = Some(weighted);
            }
        }
    }
    gzip.or(any).unwrap_or(false)
}

/// How an answer is compressed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compressing {
    /// An answer read whole: compressed as it streams, each piece leaving
    /// the encoder only as compression lets it.
    Whole,
    /// A live stream: flushed after every piece, so what has come is sent
    /// at once.
    Live,
}

/// How the Serving listener compresses the answer to `method`, of `status`
/// with `headers`, for a Peer that asked for gzip: none where it passes as
/// it is — an answer with no body, or one it says is empty, one already
/// encoded, or one that is neither JSON nor an event stream, such as an
/// Attachment's bytes.
pub fn compressing(
    method: &Method,
    status: StatusCode,
    headers: &HeaderMap,
) -> Option<Compressing> {
    if *method == Method::HEAD
        || status.is_informational()
        || status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED
        || headers.contains_key(header::CONTENT_ENCODING)
        || headers
            .get(header::CONTENT_LENGTH)
            .and_then(|length| length.to_str().ok())
            .is_some_and(|length| length.trim() == "0")
    {
        return None;
    }
    let content_type = headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if media_type == "text/event-stream" {
        Some(Compressing::Live)
    } else if media_type == "application/json" || media_type.ends_with("+json") {
        Some(Compressing::Whole)
    } else {
        None
    }
}

/// `body`, the pieces of an answer with `headers`, gzipped as `compressing`
/// says, with `headers` made to say so: it is encoded, its length is no
/// longer what was answered, and what it is depends on what was asked.
pub fn compressed<S>(compressing: Compressing, headers: &mut HeaderMap, body: S) -> Gzipped<S> {
    headers.remove(header::CONTENT_LENGTH);
    headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(GZIP));
    headers.append(
        header::VARY,
        HeaderValue::from_static(header::ACCEPT_ENCODING.as_str()),
    );
    Gzipped {
        body: Box::pin(body),
        encoder: Some(GzEncoder::new(Vec::new(), Compression::default())),
        compressing,
    }
}

/// An answer's pieces as one gzip member; see [`compressed`].
pub struct Gzipped<S> {
    body: Pin<Box<S>>,
    /// The encoder, until the answer ends or fails.
    encoder: Option<GzEncoder<Vec<u8>>>,
    compressing: Compressing,
}

impl<S, E> Stream for Gzipped<S>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let Some(encoder) = this.encoder.as_mut() else {
                return Poll::Ready(None);
            };
            match ready!(this.body.as_mut().poll_next(context)) {
                Some(Ok(piece)) => {
                    // An encoder writing to memory has nothing to fail at.
                    encoder.write_all(&piece).expect("gzip writes into memory");
                    if this.compressing == Compressing::Live {
                        encoder.flush().expect("gzip flushes into memory");
                    }
                    let written = std::mem::take(encoder.get_mut());
                    if !written.is_empty() {
                        return Poll::Ready(Some(Ok(Bytes::from(written))));
                    }
                }
                Some(Err(error)) => {
                    // The member is never finished, so the Peer reads the
                    // answer as broken off, as it is.
                    this.encoder = None;
                    return Poll::Ready(Some(Err(error)));
                }
                None => {
                    let rest = this
                        .encoder
                        .take()
                        .expect("the encoder stands until the answer ends")
                        .finish()
                        .expect("gzip finishes into memory");
                    return Poll::Ready(Some(Ok(Bytes::from(rest))));
                }
            }
        }
    }
}

/// `response` as the Peer's readers are to see it: decoded where the
/// Serving Server answered it gzipped, its headers saying no more of the
/// coding, and as it came where it answered plain, as one that ignores
/// `Accept-Encoding` does.
pub fn decoded<B>(response: http::Response<B>) -> http::Response<Gunzipped<B>> {
    let (mut parts, body) = response.into_parts();
    let gzipped = parts
        .headers
        .get(header::CONTENT_ENCODING)
        .and_then(|coding| coding.to_str().ok())
        .is_some_and(|coding| {
            coding.trim().eq_ignore_ascii_case(GZIP) || coding.trim().eq_ignore_ascii_case("x-gzip")
        });
    let decoder = gzipped.then(|| {
        parts.headers.remove(header::CONTENT_ENCODING);
        parts.headers.remove(header::CONTENT_LENGTH);
        Box::new(Gunzipping {
            decoder: GzDecoder::new(Room(Vec::new())),
            pending: Bytes::new(),
            flushing: false,
            body_ended: false,
            finished: false,
        })
    });
    http::Response::from_parts(
        parts,
        Gunzipped {
            body,
            decoder,
            ended: false,
        },
    )
}

/// The most a decoded answer is handed on in at once. A gzip member may
/// decode to a thousand times its size, so what a piece of it decodes to is
/// handed on in pieces of no more than this, each once the reader asks for
/// it, rather than held whole.
pub const DECODED_PIECE: usize = 64 * 1024;

/// An answer's body as it was before it was gzipped; see [`decoded`].
pub struct Gunzipped<B> {
    body: B,
    /// The decoding, where the answer came gzipped: held apart, since its
    /// state is many times the size of the body it reads.
    decoder: Option<Box<Gunzipping>>,
    /// Whether the decoded body has ended.
    ended: bool,
}

/// Where a gzipped answer's decoding stands.
struct Gunzipping {
    decoder: GzDecoder<Room>,
    /// What came of the answer that the decoder has yet to take.
    pending: Bytes,
    /// Whether what the decoder took is still to be drawn out of it whole,
    /// so a piece flushed to the wire is read as it lands.
    flushing: bool,
    /// Whether the answer's body has ended.
    body_ended: bool,
    /// Whether the member's end has been read and checked.
    finished: bool,
}

/// What the decoder writes into: no more than [`DECODED_PIECE`] at once,
/// refusing the rest as `WouldBlock` until what it holds is handed on. The
/// decoder takes no more of the answer while it is refused.
struct Room(Vec<u8>);

impl std::io::Write for Room {
    fn write(&mut self, decoded: &[u8]) -> std::io::Result<usize> {
        let room = DECODED_PIECE - self.0.len();
        if room == 0 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        let taken = room.min(decoded.len());
        self.0.extend_from_slice(&decoded[..taken]);
        Ok(taken)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Gunzipping {
    /// Takes the decoder a step on with what it holds, answering whether
    /// it has more to do before the answer's body is read again.
    fn step(&mut self) -> std::io::Result<bool> {
        if !self.pending.is_empty() {
            let taken = self.decoder.write(&self.pending)?;
            if taken == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "an answer went on past the end of its gzip member",
                ));
            }
            bytes::Buf::advance(&mut self.pending, taken);
        } else if self.flushing {
            self.decoder.flush()?;
            self.flushing = false;
        } else if self.body_ended && !self.finished {
            // An answer broken off before its member's end is broken, not
            // merely short.
            self.decoder.try_finish()?;
            self.finished = true;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// What has been decoded and not yet handed on.
    fn decoded(&mut self) -> Option<http_body::Frame<Bytes>> {
        let decoded = std::mem::take(&mut self.decoder.get_mut().0);
        (!decoded.is_empty()).then(|| http_body::Frame::data(Bytes::from(decoded)))
    }
}

impl<B> http_body::Body for Gunzipped<B>
where
    B: http_body::Body<Data = Bytes> + Unpin,
    B::Error: Into<BoxError>,
{
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, BoxError>>> {
        let this = self.get_mut();
        let Some(decoding) = this.decoder.as_mut() else {
            return Pin::new(&mut this.body)
                .poll_frame(context)
                .map_err(Into::into);
        };
        loop {
            if this.ended {
                return Poll::Ready(None);
            }
            match decoding.step() {
                Ok(true) => {
                    if decoding.decoder.get_ref().0.len() >= DECODED_PIECE {
                        return Poll::Ready(decoding.decoded().map(Ok));
                    }
                    continue;
                }
                Ok(false) => {}
                // The decoded piece is full: it is handed on, and the
                // decoder taken on once it has been.
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    return Poll::Ready(decoding.decoded().map(Ok));
                }
                Err(error) => {
                    this.ended = true;
                    return Poll::Ready(Some(Err(error.into())));
                }
            }
            // The decoder has done all it can with what came: what it
            // decoded is handed on at once, before the answer is read on.
            if let Some(decoded) = decoding.decoded() {
                return Poll::Ready(Some(Ok(decoded)));
            }
            if decoding.finished {
                this.ended = true;
                return Poll::Ready(None);
            }
            match ready!(Pin::new(&mut this.body).poll_frame(context)) {
                Some(Ok(frame)) => match frame.into_data() {
                    Ok(piece) => {
                        decoding.pending = piece;
                        decoding.flushing = true;
                    }
                    Err(frame) => return Poll::Ready(Some(Ok(frame))),
                },
                Some(Err(error)) => {
                    this.ended = true;
                    return Poll::Ready(Some(Err(error.into())));
                }
                None => decoding.body_ended = true,
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        match self.decoder {
            None => self.body.is_end_stream(),
            Some(_) => self.ended,
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self.decoder {
            None => self.body.size_hint(),
            Some(_) => http_body::SizeHint::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, io::Read as _};

    use futures_util::{StreamExt as _, stream};
    use http_body::Body as _;
    use http_body_util::{BodyExt as _, Full, StreamBody};

    use super::*;

    fn asking(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(
                header::ACCEPT_ENCODING,
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    fn answered(content_type: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(content_type).unwrap(),
        );
        headers
    }

    fn gunzip(bytes: &[u8]) -> Vec<u8> {
        let mut plain = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut plain)
            .expect("one whole gzip member");
        plain
    }

    #[test]
    fn a_request_asks_for_gzip_where_it_names_it_or_any_coding_with_some_weight() {
        for asks in [
            &["gzip"][..],
            &["GZIP"],
            &["x-gzip"],
            &["br, gzip;q=0.5"],
            &["deflate", "gzip"],
            &["*"],
            &["br;q=1, *;q=0.1"],
            &["gzip; q=1.0"],
            &["gzip;Q=0.5"],
        ] {
            assert!(asks_for_gzip(&asking(asks)), "{asks:?} asks for gzip");
        }
        for asks_not in [
            &[][..],
            &["identity"],
            &["br, deflate"],
            &["gzip;q=0"],
            &["gzip;q=0.0, br"],
            &["*;q=0"],
            &["*, gzip;q=0"],
            &["gzipped"],
            &["gzip;Q=0"],
            &["gzip; Q = 0, *"],
        ] {
            assert!(
                !asks_for_gzip(&asking(asks_not)),
                "{asks_not:?} does not ask for gzip"
            );
        }
    }

    #[test]
    fn every_json_answer_and_event_stream_is_compressed_and_nothing_else_is() {
        let get = Method::GET;
        let ok = StatusCode::OK;
        for (content_type, compressing_as) in [
            ("application/json", Some(Compressing::Whole)),
            ("application/json; charset=utf-8", Some(Compressing::Whole)),
            ("Application/JSON", Some(Compressing::Whole)),
            ("application/problem+json", Some(Compressing::Whole)),
            ("text/event-stream", Some(Compressing::Live)),
            ("text/event-stream; charset=utf-8", Some(Compressing::Live)),
            ("image/png", None),
            ("image/jpeg", None),
            ("application/octet-stream", None),
            ("text/plain", None),
        ] {
            assert_eq!(
                compressing(&get, ok, &answered(content_type)),
                compressing_as,
                "{content_type}"
            );
        }
        assert_eq!(
            compressing(&get, ok, &HeaderMap::new()),
            None,
            "an answer saying nothing of what it holds passes as it is"
        );
        let json = answered("application/json");
        assert_eq!(
            compressing(&get, StatusCode::CONFLICT, &json),
            Some(Compressing::Whole),
            "a refusal is a JSON answer too"
        );
        assert_eq!(compressing(&Method::HEAD, ok, &json), None);
        assert_eq!(compressing(&get, StatusCode::NO_CONTENT, &json), None);
        assert_eq!(compressing(&get, StatusCode::NOT_MODIFIED, &json), None);
        let mut empty = json.clone();
        empty.insert(header::CONTENT_LENGTH, HeaderValue::from(0));
        assert_eq!(
            compressing(&get, ok, &empty),
            None,
            "a JSON answer that says it is empty passes as it is"
        );
        empty.insert(header::CONTENT_LENGTH, HeaderValue::from(2));
        assert_eq!(compressing(&get, ok, &empty), Some(Compressing::Whole));
        let mut encoded = json.clone();
        encoded.insert(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
        assert_eq!(compressing(&get, ok, &encoded), None);
    }

    #[tokio::test]
    async fn a_whole_answer_is_one_gzip_member_its_headers_saying_so() {
        let json = br#"{"sessions":[]}"#.repeat(2000);
        let mut headers = answered("application/json");
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(json.len()));
        let pieces = json
            .chunks(4096)
            .map(|piece| Ok::<_, Infallible>(Bytes::copy_from_slice(piece)))
            .collect::<Vec<_>>();

        let body = compressed(Compressing::Whole, &mut headers, stream::iter(pieces))
            .map(|piece| piece.unwrap().to_vec())
            .concat()
            .await;

        assert_eq!(headers[header::CONTENT_ENCODING], GZIP);
        assert_eq!(headers[header::VARY], "accept-encoding");
        assert!(!headers.contains_key(header::CONTENT_LENGTH));
        assert!(body.len() < json.len() / 10, "{} bytes", body.len());
        assert_eq!(gunzip(&body), json);
    }

    /// A live stream's every piece leaves the encoder as it comes, decodable
    /// to the piece itself with nothing after it, keep-alives among them;
    /// what is held back is only the member's end, written as the stream
    /// ends.
    #[tokio::test]
    async fn a_live_stream_is_flushed_after_every_piece_keep_alives_included() {
        let (sending, mut receiving) = tokio::sync::mpsc::unbounded_channel();
        let pieces = stream::poll_fn(move |context| receiving.poll_recv(context))
            .map(Ok::<Bytes, Infallible>);
        let mut headers = answered("text/event-stream");
        let mut gzipped = compressed(Compressing::Live, &mut headers, pieces);
        let mut reader = GzDecoder::new(Vec::new());

        for event in [
            &b"event: snapshot\ndata: {\"session\":{}}\n\n"[..],
            b":\n\n",
            b"event: updated\ndata: {\"changes\":[]}\n\n",
            b":\n\n",
        ] {
            sending.send(Bytes::from_static(event)).unwrap();
            let sent = gzipped.next().await.expect("the stream goes on").unwrap();
            reader.write_all(&sent).unwrap();
            reader.flush().unwrap();
            assert_eq!(
                std::mem::take(reader.get_mut()),
                event,
                "the piece is read whole with nothing after it"
            );
        }
        drop(sending);
        let end = gzipped.next().await.expect("the member ends").unwrap();
        reader.write_all(&end).unwrap();
        reader.try_finish().expect("the member is whole");
        assert!(gzipped.next().await.is_none());
    }

    #[tokio::test]
    async fn a_live_stream_that_fails_is_broken_off_without_its_end() {
        let pieces = stream::iter([Ok(Bytes::from_static(b"data: 1\n\n")), Err("lost")]);
        let mut headers = answered("text/event-stream");
        let gzipped: Vec<_> = compressed(Compressing::Live, &mut headers, pieces)
            .collect()
            .await;

        assert_eq!(gzipped.len(), 2);
        assert_eq!(gzipped[1], Err("lost"));
    }

    fn streamed(
        pieces: Vec<Bytes>,
    ) -> StreamBody<impl Stream<Item = Result<http_body::Frame<Bytes>, BoxError>>> {
        StreamBody::new(stream::iter(
            pieces
                .into_iter()
                .map(|piece| Ok(http_body::Frame::data(piece))),
        ))
    }

    async fn gzipped_whole(plain: &[u8]) -> Vec<u8> {
        let mut headers = answered("application/json");
        compressed(
            Compressing::Whole,
            &mut headers,
            stream::iter([Ok::<_, Infallible>(Bytes::copy_from_slice(plain))]),
        )
        .map(|piece| piece.unwrap().to_vec())
        .concat()
        .await
    }

    #[tokio::test]
    async fn a_gzipped_answer_is_read_plain_however_its_pieces_fall() {
        let json = br#"{"id":"0198b27e","title":"Map the workspace"}"#.repeat(500);
        let gzipped = gzipped_whole(&json).await;
        for piece_length in [1, 7, 512, gzipped.len()] {
            let pieces = gzipped
                .chunks(piece_length)
                .map(Bytes::copy_from_slice)
                .collect();
            let response = decoded(
                http::Response::builder()
                    .header(header::CONTENT_ENCODING, "gzip")
                    .header(header::CONTENT_LENGTH, gzipped.len())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(streamed(pieces))
                    .unwrap(),
            );

            assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
            assert!(!response.headers().contains_key(header::CONTENT_LENGTH));
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(body, json, "in pieces of {piece_length}");
        }
    }

    /// A Serving Server that ignores `Accept-Encoding` answers plain, and is
    /// read as it answered, its length and all.
    #[tokio::test]
    async fn an_answer_that_came_plain_is_read_as_it_came() {
        let response = decoded(
            http::Response::builder()
                .header(header::CONTENT_LENGTH, 2)
                .body(Full::new(Bytes::from_static(b"{}")))
                .unwrap(),
        );

        assert_eq!(response.headers()[header::CONTENT_LENGTH], "2");
        assert_eq!(response.body().size_hint().exact(), Some(2));
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body, "{}");
    }

    #[tokio::test]
    async fn an_empty_answer_that_came_plain_is_read_empty() {
        let response = decoded(
            http::Response::builder()
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_LENGTH, 0)
                .body(Full::new(Bytes::new()))
                .unwrap(),
        );

        assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
        assert!(response.body().is_end_stream());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty());
    }

    /// What decodes to many times its size, as zeros do, is handed on in
    /// pieces no larger than [`DECODED_PIECE`], not decoded whole at once.
    #[tokio::test]
    async fn what_a_piece_decodes_to_is_handed_on_in_bounded_pieces() {
        let zeros = vec![0_u8; 16 * 1024 * 1024];
        let gzipped = gzipped_whole(&zeros).await;
        assert!(gzipped.len() < 64 * 1024, "{} bytes", gzipped.len());
        let mut body = decoded(
            http::Response::builder()
                .header(header::CONTENT_ENCODING, "gzip")
                .body(streamed(vec![Bytes::from(gzipped)]))
                .unwrap(),
        )
        .into_body();

        let mut read = 0;
        while let Some(frame) = body.frame().await {
            let piece = frame.unwrap().into_data().unwrap();
            assert!(piece.len() <= DECODED_PIECE, "a piece of {}", piece.len());
            assert!(piece.iter().all(|byte| *byte == 0));
            read += piece.len();
        }
        assert_eq!(read, zeros.len());
        assert!(body.is_end_stream());
    }

    #[tokio::test]
    async fn an_answer_going_on_past_its_gzip_member_fails_to_read() {
        let mut gzipped = gzipped_whole(br#"{"sessions":[]}"#).await;
        gzipped.extend_from_slice(b"and more");
        let response = decoded(
            http::Response::builder()
                .header(header::CONTENT_ENCODING, "gzip")
                .body(streamed(vec![Bytes::from(gzipped)]))
                .unwrap(),
        );
        assert!(response.into_body().collect().await.is_err());
    }

    #[tokio::test]
    async fn a_gzipped_answer_broken_off_or_garbled_fails_to_read() {
        let gzipped = gzipped_whole(&br#"{"sessions":[]}"#.repeat(100)).await;
        let broken_off = Bytes::copy_from_slice(&gzipped[..gzipped.len() - 4]);
        let garbled = Bytes::from_static(b"not gzip at all");
        for body in [broken_off, garbled] {
            let response = decoded(
                http::Response::builder()
                    .header(header::CONTENT_ENCODING, "gzip")
                    .body(streamed(vec![body]))
                    .unwrap(),
            );
            assert!(response.into_body().collect().await.is_err());
        }
    }
}
