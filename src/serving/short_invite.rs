//! Delivering an Invite by a short code over a joined stream (ADR-0065).
//!
//! The code is about 40 bits, which nothing derived from it may be searched
//! against offline, so all it does is enter a SPAKE2 exchange. Who speaks when,
//! `pA` and `pB` being SPAKE2's messages, `R` the redeeming Server and `S` the
//! Serving one:
//!
//! ```text
//! R → S  "suru-short-invite", version, R's identity key, pA     no secret
//! S → R  refused at once (no code live, ...), and it ends       no guess was made
//! S → R  pB                                                     no secret
//! R → S  MAC(R's key, transcript)                               R's proof
//! S → R  refused, and it ends                                   the guess was wrong
//! S → R  MAC(S's key, transcript), AEAD(Invite)                 S's proof, the Invite
//! ```
//!
//! SPAKE2's messages are uniformly random whatever the code is, so they tell
//! nobody anything of it. What follows them does depend on the code, so what it
//! reveals is this:
//!
//! - **A passive Relay** sees all of it, and to test a guess against a MAC it
//!   would need the shared point, which is Diffie-Hellman on what it saw:
//!   nothing here can be searched offline.
//! - **A party who guessed wrong as the redeemer** is sent `pB` and then a
//!   refusal: not one byte derived from the key before it has proven the code.
//!   It learns its one guess was wrong, and the code is spent by then.
//! - **A party who guessed wrong standing in for the Serving Server** does get
//!   the redeemer's MAC. It can test that MAC against the one guess it put into
//!   `pB`, and against no other without solving Diffie-Hellman, so it has one
//!   guess for each attempt its victim makes, and the redeemer makes one for
//!   each thing its reader does.
//!
//! The Serving side sends the Invite only to a party that has proven the code,
//! and the redeemer believes it only once the Serving side has proven it too.
//!
//! The roles are fixed: the redeemer is SPAKE2's side A and the Serving Server
//! its side B. Each side's message says which it is and a message from one's
//! own side is refused, and each side's MAC is keyed apart, so nothing sent can
//! be reflected back as if the other had said it.
//!
//! Both Servers' identity keys are SPAKE2's identities, which it hashes each on
//! its own, so no two pairs of keys read alike, and they enter every key below
//! it with the Relay's address. So a Relay that lies about a key, carries the
//! exchange on to another Relay, or passes through what either side said
//! altered, gets a failed exchange. Neither Server believes the other's key on
//! the strength of this: the redeemer's is only claimed, and the Pairing's TLS
//! has each prove its own afterwards, as it does for any Invite.
//!
//! A code is spent as an exchange takes it, once its opening message has been
//! read whole and before anything is answered, and it is taken from where it is
//! kept there, under its lock: so two at once get one guess between them, the
//! one that came first goes on undisturbed, and a party that hangs up or falls
//! silent having sent its message has had its guess. One that says less than
//! that has made none, and spends nothing.
//!
//! So at most one exchange does any cryptography for each code, and nothing
//! limits how many may come through: a party that has yet to finish its opening
//! holds nothing another needs. It holds a place among the connections the
//! Serving side's acceptor is finishing, as any connection saying nothing does,
//! for its handshake's bound to say its first byte and the reading bound to say
//! the rest, and that shared room is what bounds them (ADR-0065 says what filling
//! it costs).

use std::{
    sync::{Arc, Mutex as StdMutex, atomic::AtomicU64, atomic::Ordering},
    task::{Context as TaskContext, Poll},
};

use curve25519_dalek::edwards::CompressedEdwardsY;

use ring::{
    aead::{self, Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey},
    digest, hkdf, hmac,
    rand::{SecureRandom as _, SystemRandom},
};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf},
    sync::watch,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

use super::{ByteStream, PairingFailure, internal_pairing_failure};
use crate::protocol::{
    IssuedShortCode, SessionErrorCode, ShortCode, ShortCodeStanding, ShortCodeState, Way,
};

/// How a redeeming Server says it is about to speak the exchange rather than
/// TLS. A TLS record's first byte says what kind it is, and a ClientHello's
/// says handshake, `0x16`, so one cannot open as this does.
const PREAMBLE: &[u8; 17] = b"suru-short-invite";
/// The exchange this Server speaks, said right after the preamble so another
/// can be told from it and refused without spending anything.
const VERSION: u8 = 1;

/// How a `SubjectPublicKeyInfo` of each kind of identity key opens, and how
/// long the whole of it is: all that varies is the point at its end, so what
/// is claimed for a key has no room for anything else.
const KEY_FORMS: [(&[u8], usize); 3] = [
    // ECDSA on P-256, an uncompressed point, as a Server makes its key.
    (
        &[
            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06,
            0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
        ],
        91,
    ),
    // ECDSA on P-384, and Ed25519, which a Relay also takes and a key kept in
    // a file may be.
    (
        &[
            0x30, 0x76, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06,
            0x05, 0x2b, 0x81, 0x04, 0x00, 0x22, 0x03, 0x62, 0x00, 0x04,
        ],
        120,
    ),
    (
        &[
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ],
        44,
    ),
];
/// The longest of [`KEY_FORMS`].
const MAX_KEY: usize = {
    let (mut longest, mut form) = (0, 0);
    while form < KEY_FORMS.len() {
        if KEY_FORMS[form].1 > longest {
            longest = KEY_FORMS[form].1;
        }
        form += 1;
    }
    longest
};
/// How long a SPAKE2 message is: which side it comes from, and a point.
const SHARE: usize = 33;
/// What opens a SPAKE2 message from each side.
const REDEEMER_SIDE: u8 = b'A';
const SERVING_SIDE: u8 = b'B';
/// How long a MAC is.
const PROOF: usize = 32;
/// The longest Invite a code delivers, which is far past any an Invite's ways
/// come to: one that long is issued with no code.
const MAX_INVITE: usize = 32 * 1024;
/// The longest anything said in the exchange is.
const MAX_FRAME: usize = 1 + PROOF + MAX_INVITE + AEAD_TAG;
const AEAD_TAG: usize = 16;

/// What opens each thing the Serving side says.
const SAYS_SHARE: u8 = 1;
const SAYS_DELIVERY: u8 = 2;
const SAYS_FAILED: u8 = 3;
const SAYS_REFUSED: u8 = 4;

/// What each key below the exchange is derived for. Distinct, so the two
/// sides' proofs can never be taken for one another.
const TRANSCRIPT_LABEL: &[u8] = b"suru short invite v1";
const REDEEMER_PROOF_LABEL: &[u8] = b"suru short invite v1 redeemer proof";
const SERVING_PROOF_LABEL: &[u8] = b"suru short invite v1 serving proof";
const INVITE_LABEL: &[u8] = b"suru short invite v1 invite";

/// Why a Serving Server turned an exchange down at once, with no guess made:
/// so a redeemer may try another way.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RefusedAtOnce {
    /// No code is live for this Relay way.
    NoCode,
    /// It speaks no such version of the exchange.
    Version,
    /// What it was told was not what the exchange says.
    Malformed,
}

impl RefusedAtOnce {
    fn byte(self) -> u8 {
        match self {
            Self::NoCode => 1,
            Self::Version => 2,
            Self::Malformed => 3,
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::NoCode),
            2 => Some(Self::Version),
            3 => Some(Self::Malformed),
            _ => None,
        }
    }
}

/// Why an attempt ended with its one guess used, or with no way to tell it was
/// not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Dead {
    /// One side proved the other had another code, or spoke for other
    /// parties, or for another Relay.
    Failed,
    /// It never finished.
    Unfinished,
}

/// What came of one attempt by a redeeming Server.
pub(super) enum Attempt {
    Delivered(Zeroizing<String>),
    Refused(RefusedAtOnce),
    Dead(Dead),
}

/// Who an exchange is between and over what, as each side knows that itself.
/// Keys are `SubjectPublicKeyInfo`, as a Relay and a Pairing name them, and a
/// Relay's address is the one way it is written: both Servers name it alike,
/// since neither proves anything to a Relay naming itself otherwise.
#[derive(Clone, Copy)]
pub(super) struct Parties<'a> {
    pub(super) way: &'a str,
    pub(super) redeeming: &'a [u8],
    pub(super) serving: &'a [u8],
}

/// What the exchange leaves both sides with, each key for one thing alone.
struct Keys {
    transcript: [u8; 32],
    redeemer_proof: hmac::Key,
    serving_proof: hmac::Key,
    invite: Zeroizing<[u8; 32]>,
}

impl Keys {
    /// The keys for what SPAKE2 gave as `shared`, over `parties` and what the
    /// two sides said. SPAKE2's output already covers the code, both
    /// identities and both messages; the transcript adds the Relay and the
    /// version of the exchange, and is what the proofs are of.
    fn derive(
        shared: &[u8],
        parties: Parties<'_>,
        redeemer_share: &[u8],
        serving_share: &[u8],
    ) -> Self {
        let transcript = transcript(parties, redeemer_share, serving_share);
        let secret = hkdf::Salt::new(hkdf::HKDF_SHA256, &transcript).extract(shared);
        let expanded = |label: &[u8]| {
            let mut key = Zeroizing::new([0_u8; 32]);
            secret
                .expand(&[label], hkdf::HKDF_SHA256)
                .and_then(|okm| okm.fill(&mut key[..]))
                .expect("thirty-two bytes is within what HKDF expands to");
            key
        };
        Self {
            transcript,
            redeemer_proof: hmac::Key::new(hmac::HMAC_SHA256, &expanded(REDEEMER_PROOF_LABEL)[..]),
            serving_proof: hmac::Key::new(hmac::HMAC_SHA256, &expanded(SERVING_PROOF_LABEL)[..]),
            invite: expanded(INVITE_LABEL),
        }
    }

    fn sealing(&self) -> LessSafeKey {
        LessSafeKey::new(
            UnboundKey::new(&CHACHA20_POLY1305, &self.invite[..])
                .expect("the key is as long as the cipher takes"),
        )
    }

    /// `invite` sealed to whoever holds these keys. The key seals this one
    /// thing and is never used again, so the nonce need say nothing.
    fn seal(&self, invite: &str) -> Vec<u8> {
        let mut sealed = invite.as_bytes().to_vec();
        self.sealing()
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key([0; aead::NONCE_LEN]),
                Aad::from(&self.transcript),
                &mut sealed,
            )
            .expect("the Invite is within what the cipher seals");
        sealed
    }

    fn open(&self, sealed: &[u8]) -> Option<Zeroizing<String>> {
        let mut opened = Zeroizing::new(sealed.to_vec());
        let length = self
            .sealing()
            .open_in_place(
                Nonce::assume_unique_for_key([0; aead::NONCE_LEN]),
                Aad::from(&self.transcript),
                &mut opened,
            )
            .ok()?
            .len();
        opened.truncate(length);
        Some(Zeroizing::new(String::from_utf8(opened.to_vec()).ok()?))
    }
}

/// The hash of everything both sides said and are, each part length-prefixed
/// so no two read alike. It holds nothing of the code.
fn transcript(parties: Parties<'_>, redeemer_share: &[u8], serving_share: &[u8]) -> [u8; 32] {
    let mut hash = digest::Context::new(&digest::SHA256);
    for part in [
        TRANSCRIPT_LABEL,
        &[VERSION],
        parties.way.as_bytes(),
        parties.redeeming,
        parties.serving,
        redeemer_share,
        serving_share,
    ] {
        let length = u32::try_from(part.len()).expect("no part is anywhere near that long");
        hash.update(&length.to_be_bytes());
        hash.update(part);
    }
    hash.finish()
        .as_ref()
        .try_into()
        .expect("SHA-256 is thirty-two bytes")
}

/// Writes `body` as one thing said.
async fn say<S: AsyncWrite + Unpin>(stream: &mut S, body: &[u8]) -> std::io::Result<()> {
    let length = u16::try_from(body.len())
        .map_err(|_| std::io::Error::other("what is said in the exchange is short"))?;
    let mut framed = Vec::with_capacity(2 + body.len());
    framed.extend_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(body);
    stream.write_all(&framed).await?;
    stream.flush().await
}

/// Reads one thing said, no longer than `most`.
async fn hear<S: AsyncRead + Unpin>(stream: &mut S, most: usize) -> std::io::Result<Vec<u8>> {
    let mut length = [0_u8; 2];
    stream.read_exact(&mut length).await?;
    let length = usize::from(u16::from_be_bytes(length));
    if length > most {
        return Err(std::io::Error::other(
            "what is said in the exchange is short",
        ));
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

/// The redeemer's opening message: its identity key and its SPAKE2 message.
fn hello(key: &[u8], share: &[u8]) -> Vec<u8> {
    let mut hello = Vec::with_capacity(PREAMBLE.len() + 1 + 2 + 2 + key.len() + share.len());
    hello.extend_from_slice(PREAMBLE);
    hello.push(VERSION);
    let length = u16::try_from(2 + key.len() + share.len()).expect("keys are bounded");
    hello.extend_from_slice(&length.to_be_bytes());
    let key_length = u16::try_from(key.len()).expect("keys are bounded");
    hello.extend_from_slice(&key_length.to_be_bytes());
    hello.extend_from_slice(key);
    hello.extend_from_slice(share);
    hello
}

/// Reads what the opening message says after the preamble and version: a key
/// of a kind a Server holds, and a SPAKE2 message's length. Nothing else fits,
/// so nothing else can travel there, and whose key it is the Pairing's TLS has
/// its holder prove later.
fn heard_hello(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let (length, rest) = body.split_first_chunk::<2>()?;
    let length = usize::from(u16::from_be_bytes(*length));
    if rest.len() != length + SHARE {
        return None;
    }
    let (key, share) = rest.split_at(length);
    is_identity_key(key).then_some((key, share))
}

/// Whether `key` is written as an identity key of a kind a Server holds.
fn is_identity_key(key: &[u8]) -> bool {
    KEY_FORMS
        .iter()
        .any(|(opening, length)| key.len() == *length && key.starts_with(opening))
}

/// Whether `share` is a SPAKE2 message `side` could have sent. `spake2` itself
/// checks only that its point decompresses, and it has had no audit, so this
/// holds a peer's message to what no honest one ever differs from: one point
/// written the one way, in the group SPAKE2 runs in.
fn is_sound_share(share: &[u8], side: u8) -> bool {
    let Some((&said_side, point)) = share.split_first() else {
        return false;
    };
    let Ok(written) = <[u8; 32]>::try_from(point) else {
        return false;
    };
    let compressed = CompressedEdwardsY(written);
    said_side == side
        && compressed.decompress().is_some_and(|point| {
            point.compress() == compressed && !point.is_small_order() && point.is_torsion_free()
        })
}

/// One attempt by a redeeming Server, over a stream just joined to the Serving
/// Server `parties` names: it speaks first, and never again over any other
/// stream unless it hears it was refused at once. Nothing it does can make a
/// second guess at the code: it proves what it holds at most once, and only
/// after the other side has answered as the party `parties` names would.
pub(super) async fn redeem<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    code: &ShortCode,
    parties: Parties<'_>,
    within: Duration,
) -> Attempt {
    redeem_by(stream, code, parties, within, Spake2::start_a).await
}

async fn redeem_by<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    code: &ShortCode,
    parties: Parties<'_>,
    within: Duration,
    start: impl FnOnce(&Password, &Identity, &Identity) -> (Spake2<Ed25519Group>, Vec<u8>),
) -> Attempt {
    tokio::time::timeout(within, redeeming(stream, code, parties, start))
        .await
        .unwrap_or(Attempt::Dead(Dead::Unfinished))
}

async fn redeeming<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    code: &ShortCode,
    parties: Parties<'_>,
    start: impl FnOnce(&Password, &Identity, &Identity) -> (Spake2<Ed25519Group>, Vec<u8>),
) -> Attempt {
    let (spake, share) = start(
        &Password::new(code.canonical()),
        &Identity::new(parties.redeeming),
        &Identity::new(parties.serving),
    );
    if stream
        .write_all(&hello(parties.redeeming, &share))
        .await
        .is_err()
        || stream.flush().await.is_err()
    {
        return Attempt::Dead(Dead::Unfinished);
    }
    // Its first answer is a share or a refusal, and nothing longer is read.
    let Ok(answer) = hear(stream, 1 + SHARE).await else {
        return Attempt::Dead(Dead::Unfinished);
    };
    let serving_share = match answer.split_first() {
        Some((&SAYS_SHARE, serving_share)) => serving_share,
        Some((&SAYS_REFUSED, [why])) => {
            return match RefusedAtOnce::from_byte(*why) {
                Some(refused) => Attempt::Refused(refused),
                None => Attempt::Dead(Dead::Failed),
            };
        }
        _ => return Attempt::Dead(Dead::Failed),
    };
    if !is_sound_share(serving_share, SERVING_SIDE) {
        return Attempt::Dead(Dead::Failed);
    }
    let Ok(shared) = spake.finish(serving_share).map(Zeroizing::new) else {
        return Attempt::Dead(Dead::Failed);
    };
    let keys = Keys::derive(&shared, parties, &share, serving_share);
    let proof = hmac::sign(&keys.redeemer_proof, &keys.transcript);
    if say(stream, proof.as_ref()).await.is_err() {
        return Attempt::Dead(Dead::Unfinished);
    }
    let Ok(answer) = hear(stream, MAX_FRAME).await else {
        return Attempt::Dead(Dead::Unfinished);
    };
    let Some((&SAYS_DELIVERY, delivery)) = answer.split_first() else {
        return Attempt::Dead(Dead::Failed);
    };
    let Some((proof, sealed)) = delivery.split_at_checked(PROOF) else {
        return Attempt::Dead(Dead::Failed);
    };
    if hmac::verify(&keys.serving_proof, &keys.transcript, proof).is_err() {
        return Attempt::Dead(Dead::Failed);
    }
    match keys.open(sealed) {
        Some(invite) => Attempt::Delivered(invite),
        None => Attempt::Dead(Dead::Failed),
    }
}

/// The short code of the Invite that stands, and what it delivers, gone with
/// that Invite.
struct ShortCodeEntry {
    /// The enrollment token of the Invite it delivers.
    token: [u8; 32],
    expires_at: Instant,
    /// `None` from when an exchange takes it: nothing takes it twice.
    code: Option<ShortCode>,
    serial: u64,
    /// The addresses of the Relays the Invite offers, the only ones the
    /// exchange is run over.
    relays: Vec<String>,
    invite: Zeroizing<String>,
}

impl ShortCodeEntry {
    /// Whether its Invite is yet to expire.
    fn stands(&self) -> bool {
        Instant::now() < self.expires_at
    }
}

/// The Serving side's short codes: the one of the Invite that stands, how it
/// stands, and the door for the exchange that delivers that Invite.
#[derive(Clone)]
pub(super) struct ShortCodes {
    current: Arc<StdMutex<Option<ShortCodeEntry>>>,
    standing: Arc<watch::Sender<ShortCodeStanding>>,
    last_serial: Arc<AtomicU64>,
    reading_timeout: Duration,
    timeout: Duration,
}

impl ShortCodes {
    pub(super) fn new(reading_timeout: Duration, timeout: Duration) -> Self {
        Self {
            current: Arc::default(),
            standing: Arc::new(watch::Sender::new(ShortCodeStanding::default())),
            // Going on from wherever a run before this one got to, so a
            // Client that heard of a code then never takes one now for it.
            last_serial: Arc::new(AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |since| since.as_secs()),
            )),
            reading_timeout,
            timeout,
        }
    }

    /// The same door, an opening read within `reading_timeout`, and each step
    /// of the rest of an exchange within `timeout`.
    pub(super) fn within(self, reading_timeout: Duration, timeout: Duration) -> Self {
        Self {
            reading_timeout,
            timeout,
            ..self
        }
    }

    /// How long an exchange may take a step of, on either side of it.
    pub(super) fn timeout(&self) -> Duration {
        self.timeout
    }

    /// How the latest code stands, moving on as that changes.
    pub(super) fn standing(&self) -> watch::Receiver<ShortCodeStanding> {
        self.standing.subscribe()
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Option<ShortCodeEntry>> {
        self.current
            .lock()
            .expect("short code lock is not poisoned")
    }

    /// Takes that an Invite was issued, whose token is `token`, which offers
    /// `ways` and expires at `expires_at`: a fresh code for it, where it offers
    /// a Relay, since the exchange needs a joined connection, and either way
    /// the end of the code before it. Nothing changes where no code can be had.
    pub(super) fn issued(
        &self,
        token: [u8; 32],
        expires_at: Instant,
        invite: &str,
        ways: &[Way],
    ) -> std::io::Result<Option<IssuedShortCode>> {
        let relays = ways
            .iter()
            .filter_map(|way| match way {
                Way::Relay(relay) => Some(relay.clone()),
                Way::Direct(_) => None,
            })
            .collect::<Vec<_>>();
        if relays.is_empty() || invite.len() > MAX_INVITE {
            *self.current() = None;
            self.says_ended(ShortCodeState::Ended);
            return Ok(None);
        }
        let (code, issued) = self.fresh_code()?;
        *self.current() = Some(ShortCodeEntry {
            token,
            expires_at,
            code: Some(code),
            serial: issued.serial,
            relays,
            invite: Zeroizing::new(invite.to_owned()),
        });
        self.says_live(issued.serial);
        self.watch_expiry(token, expires_at);
        Ok(Some(issued))
    }

    /// Takes that the Invite whose token is `token` was redeemed, which spends
    /// its code too, however it stood.
    pub(super) fn redeemed(&self, token: &[u8; 32]) {
        let mut current = self.current();
        if current.as_ref().is_some_and(|entry| entry.token == *token) {
            *current = None;
            self.says_ended(ShortCodeState::Spent);
        }
    }

    /// Takes that every Invite issued has been discarded, and every code with
    /// it.
    pub(super) fn discarded(&self) {
        *self.current() = None;
        self.says_ended(ShortCodeState::Ended);
    }

    fn fresh_code(&self) -> std::io::Result<(ShortCode, IssuedShortCode)> {
        let mut random = Zeroizing::new([0_u8; 8]);
        SystemRandom::new()
            .fill(&mut random[..])
            .map_err(|_| std::io::Error::other("the system's randomness did not answer"))?;
        let code = ShortCode::from_random(&random);
        let serial = self.last_serial.fetch_add(1, Ordering::AcqRel) + 1;
        Ok((code.clone(), IssuedShortCode { code, serial }))
    }

    /// Says `serial` is the code now live.
    fn says_live(&self, serial: u64) {
        self.standing.send_replace(ShortCodeStanding {
            serial,
            state: ShortCodeState::Live,
        });
    }

    /// Says the Invite the latest code came with no longer stands, however that
    /// code stood: what an attempt did to it is news only while the Invite is
    /// there to get a new code for. Before any code, there is nothing to say.
    fn says_ended(&self, state: ShortCodeState) {
        self.standing.send_if_modified(|standing| {
            let news = standing.serial > 0 && standing.state != state;
            if news {
                standing.state = state;
            }
            news
        });
    }

    /// A fresh code for the Invite that stands, in place of whatever became of
    /// the last, leaving that Invite as it is. A code goes for one guess, so
    /// each one costs a reader's asking for it, and however many they ask for
    /// the Invite's lifetime does not move. None is had while the last is being
    /// tried: that exchange could still deliver the Invite, and it ends within
    /// its bound.
    pub(super) fn replace(&self) -> Result<IssuedShortCode, PairingFailure> {
        let mut current = self.current();
        let Some(entry) = current.as_mut().filter(|entry| entry.stands()) else {
            return Err(PairingFailure::new(
                SessionErrorCode::ShortCodeNotLive,
                "no Invite standing has a code: the last has ended, or offers no Relay; issue a \
                 new one with /serve",
            ));
        };
        if self.standing.borrow().state == ShortCodeState::UnderWay {
            return Err(PairingFailure::new(
                SessionErrorCode::ShortCodeBeingTried,
                "this code is being tried now; a new one can be had once that ends",
            ));
        }
        let (code, fresh) = self
            .fresh_code()
            .map_err(|error| internal_pairing_failure(anyhow::Error::new(error)))?;
        entry.code = Some(code);
        entry.serial = fresh.serial;
        self.says_live(fresh.serial);
        Ok(fresh)
    }

    /// Whether a code is live to be tried over the Relay at `way`, spending
    /// nothing.
    fn live_at(&self, way: &str) -> bool {
        self.current().as_ref().is_some_and(|entry| {
            entry.stands() && entry.code.is_some() && entry.relays.iter().any(|at| at == way)
        })
    }

    /// Takes the code live over the Relay at `way` for one exchange. This is
    /// where the guess is spent: from here nothing else can take it, and it
    /// stays used however the exchange ends.
    fn take(&self, way: &str) -> Option<Taken> {
        let mut current = self.current();
        let entry = current.as_mut().filter(|entry| entry.stands())?;
        if !entry.relays.iter().any(|at| at == way) {
            return None;
        }
        let code = entry.code.take()?;
        let (token, serial) = (entry.token, entry.serial);
        self.standing.send_replace(ShortCodeStanding {
            serial,
            state: ShortCodeState::UnderWay,
        });
        Some(Taken {
            codes: self.clone(),
            code,
            token,
            serial,
        })
    }

    /// Serves one connection a Relay carried, its first byte read and found to
    /// open the exchange, closing it however that ends.
    pub(super) async fn answer(&self, stream: Box<dyn ByteStream>, way: &str, serving: &[u8]) {
        self.answer_by(stream, way, serving, Spake2::start_b).await;
    }

    async fn answer_by(
        &self,
        mut stream: Box<dyn ByteStream>,
        way: &str,
        serving: &[u8],
        start: impl FnOnce(&Password, &Identity, &Identity) -> (Spake2<Ed25519Group>, Vec<u8>),
    ) {
        self.answering(&mut stream, way, serving, start).await;
        let _ = tokio::time::timeout(self.reading_timeout, stream.shutdown()).await;
    }

    /// With no code live a connection is refused at its first byte, whatever
    /// else it would have said, with no cryptography done and nothing held
    /// open. With one live it has the reading bound to say the rest of its
    /// opening, spending nothing if it does not. Past that each step is bounded
    /// on its own: dropped there, what took the code counts as its guess.
    async fn answering(
        &self,
        stream: &mut Box<dyn ByteStream>,
        way: &str,
        serving: &[u8],
        start: impl FnOnce(&Password, &Identity, &Identity) -> (Spake2<Ed25519Group>, Vec<u8>),
    ) {
        if !self.live_at(way) {
            self.refuse(stream, RefusedAtOnce::NoCode).await;
            return;
        }
        let Ok(heard) = tokio::time::timeout(self.reading_timeout, read_opening(stream)).await
        else {
            tracing::debug!("a short Invite exchange said too little in time");
            return;
        };
        let body = match heard {
            Ok(Heard::Opening(body)) => body,
            Ok(Heard::Refused(why)) => {
                self.refuse(stream, why).await;
                return;
            }
            Ok(Heard::Nothing) | Err(_) => return,
        };
        let Some((redeeming, redeemer_share)) = heard_hello(&body) else {
            self.refuse(stream, RefusedAtOnce::Malformed).await;
            return;
        };
        let Some(taken) = self.take(way) else {
            self.refuse(stream, RefusedAtOnce::NoCode).await;
            return;
        };
        // What is taken stays used: from here a share no honest redeemer would
        // send counts as a guess that failed, as one proving a wrong code does.
        let parties = Parties {
            way,
            redeeming,
            serving,
        };
        let (spake, share) = start(
            &Password::new(taken.code.canonical()),
            &Identity::new(parties.redeeming),
            &Identity::new(parties.serving),
        );
        let shared = is_sound_share(redeemer_share, REDEEMER_SIDE)
            .then(|| spake.finish(redeemer_share).ok())
            .flatten()
            .map(Zeroizing::new);
        let Some(shared) = shared else {
            taken.ended(ShortCodeState::WrongCodeTried);
            self.say_within(stream, &[SAYS_FAILED]).await;
            return;
        };
        let keys = Keys::derive(&shared, parties, redeemer_share, &share);
        let mut said = Vec::with_capacity(1 + share.len());
        said.push(SAYS_SHARE);
        said.extend_from_slice(&share);
        if !self.say_within(stream, &said).await {
            return;
        }
        let Ok(Ok(proof)) = tokio::time::timeout(self.timeout, hear(stream, PROOF)).await else {
            tracing::debug!("a short Invite exchange was closed unfinished");
            return;
        };
        if hmac::verify(&keys.redeemer_proof, &keys.transcript, &proof).is_err() {
            taken.ended(ShortCodeState::WrongCodeTried);
            self.say_within(stream, &[SAYS_FAILED]).await;
            return;
        }
        let Some(invite) = taken.invite() else {
            taken.ended(ShortCodeState::Ended);
            self.refuse(stream, RefusedAtOnce::NoCode).await;
            return;
        };
        let proof = hmac::sign(&keys.serving_proof, &keys.transcript);
        let mut delivery = Zeroizing::new(Vec::with_capacity(1 + PROOF + invite.len() + AEAD_TAG));
        delivery.push(SAYS_DELIVERY);
        delivery.extend_from_slice(proof.as_ref());
        delivery.extend_from_slice(&keys.seal(&invite));
        if self.say_within(stream, &delivery).await {
            taken.ended(ShortCodeState::Delivered);
        }
    }

    /// Writes `body` as one thing said, answering whether that was done within
    /// its bound.
    async fn say_within(&self, stream: &mut Box<dyn ByteStream>, body: &[u8]) -> bool {
        matches!(
            tokio::time::timeout(self.timeout, say(stream, body)).await,
            Ok(Ok(()))
        )
    }

    /// Tells `stream` why it is being let go, for no longer than a reader of
    /// an opening is given: it may not be reading.
    async fn refuse(&self, stream: &mut Box<dyn ByteStream>, why: RefusedAtOnce) {
        let refusal = [SAYS_REFUSED, why.byte()];
        let _ = tokio::time::timeout(self.reading_timeout, say(stream, &refusal)).await;
    }

    /// Ends the code of the Invite whose token is `token` where that Invite
    /// still stands as it comes to `expires_at`, so whoever is shown it hears
    /// of it then.
    fn watch_expiry(&self, token: [u8; 32], expires_at: Instant) {
        let codes = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(expires_at).await;
            let mut current = codes.current();
            if current.as_ref().is_some_and(|entry| entry.token == token) {
                *current = None;
                codes.says_ended(ShortCodeState::Ended);
            }
        });
    }
}

/// What the rest of an opening said, its first byte read before it.
enum Heard {
    /// The body of an opening message, in the version this Server speaks.
    Opening(Vec<u8>),
    Refused(RefusedAtOnce),
    /// Not the exchange after all.
    Nothing,
}

async fn read_opening(stream: &mut Box<dyn ByteStream>) -> std::io::Result<Heard> {
    let mut opening = [0_u8; PREAMBLE.len()];
    stream.read_exact(&mut opening).await?;
    let (rest, version) = opening.split_at(PREAMBLE.len() - 1);
    if rest != &PREAMBLE[1..] {
        return Ok(Heard::Nothing);
    }
    if version != [VERSION] {
        return Ok(Heard::Refused(RefusedAtOnce::Version));
    }
    Ok(Heard::Opening(hear(stream, 2 + MAX_KEY + SHARE).await?))
}

/// A code one exchange has taken, and so spent. Dropped without having been
/// ended, as an exchange that is hung up on or closed by its bound drops it,
/// it says so.
struct Taken {
    codes: ShortCodes,
    code: ShortCode,
    token: [u8; 32],
    serial: u64,
}

impl Taken {
    /// The Invite to deliver, where it can still be redeemed.
    fn invite(&self) -> Option<Zeroizing<String>> {
        self.codes
            .current()
            .as_ref()
            .filter(|entry| entry.token == self.token && entry.stands())
            .map(|entry| entry.invite.clone())
    }

    fn says(&self, state: ShortCodeState) {
        self.codes.standing.send_if_modified(|standing| {
            let ends_it =
                standing.serial == self.serial && standing.state == ShortCodeState::UnderWay;
            if ends_it {
                standing.state = state;
            }
            ends_it
        });
    }

    /// Says how the exchange ended.
    fn ended(self, state: ShortCodeState) {
        self.says(state);
    }
}

impl Drop for Taken {
    /// Says nothing where it was already said how the exchange ended.
    fn drop(&mut self) {
        self.says(ShortCodeState::LeftUnfinished);
    }
}

/// What a connection a Relay carried turned out to open with.
pub(super) enum Opening {
    /// TLS, or at any rate not the exchange, which the TLS handshake judges.
    Tls(Box<dyn ByteStream>),
    /// The exchange.
    Exchange(Box<dyn ByteStream>),
    /// Nothing before `by`, or nothing ever.
    Nothing,
}

/// Reads as much of what a connection a Relay carried opens with as tells the
/// exchange from TLS, one byte, nothing later than `by`, and loses none of what
/// TLS would read.
pub(super) async fn opening(mut stream: Box<dyn ByteStream>, by: Instant) -> Opening {
    let mut first = [0_u8];
    match tokio::time::timeout_at(by, stream.read(&mut first)).await {
        Ok(Ok(1)) if first[0] == PREAMBLE[0] => Opening::Exchange(stream),
        Ok(Ok(1)) => Opening::Tls(Box::new(Replayed {
            first: Some(first[0]),
            stream,
        })),
        _ => Opening::Nothing,
    }
}

/// A connection with the byte read off it to tell what it opens with put back.
struct Replayed {
    first: Option<u8>,
    stream: Box<dyn ByteStream>,
}

impl AsyncRead for Replayed {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if buffer.remaining() > 0
            && let Some(first) = self.first.take()
        {
            buffer.put_slice(&[first]);
            return Poll::Ready(Ok(()));
        }
        std::pin::Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for Replayed {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.stream).poll_write(context, bytes)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_shutdown(context)
    }

    fn poll_write_vectored(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffers: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.stream).poll_write_vectored(context, buffers)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use curve25519_dalek::{constants::EIGHT_TORSION, edwards::EdwardsPoint};
    use rand_core::{CryptoRng, RngCore};
    use rcgen::{KeyPair, PublicKeyData};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};

    use super::*;

    const WAY: &str = "https://relay.example.com";
    const OTHER_WAY: &str = "https://other.example.com";
    const INVITE: &str = "suru-v1-the-long-string";
    const BOUND: Duration = Duration::from_secs(30);
    const TOKEN: [u8; 32] = [7; 32];
    const SERVING: &[u8] = b"the Serving Server's key";
    /// As a Server makes its key.
    static REDEEMING: LazyLock<Vec<u8>> =
        LazyLock::new(|| KeyPair::generate().unwrap().subject_public_key_info());
    static ANOTHER_KEY: LazyLock<Vec<u8>> =
        LazyLock::new(|| KeyPair::generate().unwrap().subject_public_key_info());

    fn parties() -> Parties<'static> {
        Parties {
            way: WAY,
            redeeming: &REDEEMING,
            serving: SERVING,
        }
    }

    /// A door with an Invite offering [`WAY`] standing, and that Invite's code,
    /// its opening read within `reading` and each step after within `timeout`.
    fn door_within(reading: Duration, timeout: Duration) -> (ShortCodes, ShortCode) {
        let codes = ShortCodes::new(reading, timeout);
        let code = issue(&codes);
        (codes, code)
    }

    fn door() -> (ShortCodes, ShortCode) {
        door_within(BOUND, BOUND)
    }

    /// Issues an Invite offering [`WAY`], as the controller does.
    fn issue(codes: &ShortCodes) -> ShortCode {
        codes
            .issued(
                TOKEN,
                Instant::now() + Duration::from_secs(600),
                INVITE,
                &[Way::Relay(WAY.to_owned())],
            )
            .unwrap()
            .expect("an Invite offering a Relay has a code")
            .code
    }

    fn state(codes: &ShortCodes) -> ShortCodeState {
        codes.standing().borrow().state
    }

    /// The Serving side's door opened on one end of a stream, as a connection
    /// a Relay carried that opens with the exchange, with [`WAY`] named.
    fn answering(
        codes: &ShortCodes,
        way: &'static str,
    ) -> (DuplexStream, tokio::task::JoinHandle<()>) {
        let (near, far) = duplex(64 * 1024);
        let codes = codes.clone();
        let answering = tokio::spawn(async move {
            let by = Instant::now() + Duration::from_secs(30);
            match opening(Box::new(far), by).await {
                Opening::Exchange(stream) => codes.answer(stream, way, SERVING).await,
                Opening::Tls(_) | Opening::Nothing => {}
            }
        });
        (near, answering)
    }

    async fn delivered_by(codes: &ShortCodes, code: &ShortCode, parties: Parties<'_>) -> Attempt {
        let (mut near, answering) = answering(codes, WAY);
        let attempt = redeem(&mut near, code, parties, BOUND).await;
        answering.await.unwrap();
        attempt
    }

    fn an_other_code(code: &ShortCode) -> ShortCode {
        let other = if code.canonical() == "22222222" {
            "33333333"
        } else {
            "22222222"
        };
        ShortCode::parse(other).unwrap()
    }

    /// Reads one thing the Serving side said as raw bytes.
    async fn heard_said(stream: &mut DuplexStream) -> Vec<u8> {
        hear(stream, MAX_FRAME)
            .await
            .expect("the side says something")
    }

    fn opening_message(
        code: &ShortCode,
        parties: Parties<'_>,
    ) -> (Spake2<Ed25519Group>, Vec<u8>, Vec<u8>) {
        let (spake, share) = Spake2::<Ed25519Group>::start_a(
            &Password::new(code.canonical()),
            &Identity::new(parties.redeeming),
            &Identity::new(parties.serving),
        );
        let said = hello(parties.redeeming, &share);
        (spake, share, said)
    }

    #[tokio::test]
    async fn the_right_code_delivers_the_invite_as_it_was_issued() {
        let (codes, code) = door();
        let Attempt::Delivered(invite) = delivered_by(&codes, &code, parties()).await else {
            panic!("the right code delivers the Invite");
        };
        assert_eq!(invite.as_str(), INVITE);
        assert_eq!(state(&codes), ShortCodeState::Delivered);
        assert!(!codes.live_at(WAY), "a code delivers once");
    }

    #[tokio::test]
    async fn a_wrong_code_delivers_nothing_ends_the_code_and_is_said_to_be_one() {
        let (codes, code) = door();
        let attempt = delivered_by(&codes, &an_other_code(&code), parties()).await;
        assert!(matches!(attempt, Attempt::Dead(Dead::Failed)));
        assert_eq!(state(&codes), ShortCodeState::WrongCodeTried);
        assert!(state(&codes).invite_stands(), "the Invite stands");
        let again = delivered_by(&codes, &code, parties()).await;
        assert!(
            matches!(again, Attempt::Refused(RefusedAtOnce::NoCode)),
            "the right code after a wrong one finds it gone"
        );
    }

    /// The trap: a wrong guesser gets nothing it could test a second guess
    /// against, since nothing it is sent is derived from the key.
    #[tokio::test]
    async fn a_party_guessing_as_the_redeemer_is_sent_nothing_derived_from_the_key() {
        let (codes, code) = door();
        let (mut near, answering) = answering(&codes, WAY);
        let (spake, share, said) = opening_message(&an_other_code(&code), parties());
        near.write_all(&said).await.unwrap();
        let first = heard_said(&mut near).await;
        assert_eq!(first[0], SAYS_SHARE);
        assert_eq!(first.len(), 1 + SHARE, "SPAKE2's message and nothing else");
        let shared = spake.finish(&first[1..]).unwrap();
        let keys = Keys::derive(&shared, parties(), &share, &first[1..]);
        say(
            &mut near,
            hmac::sign(&keys.redeemer_proof, &keys.transcript).as_ref(),
        )
        .await
        .unwrap();
        assert_eq!(heard_said(&mut near).await, [SAYS_FAILED]);
        let mut rest = Vec::new();
        near.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "and it ends there");
        answering.await.unwrap();
    }

    /// Two exchanges under one code: they differ only by what they would have
    /// to guess, so nothing one was sent matches the other.
    #[tokio::test]
    async fn spake2s_messages_are_fresh_each_time_and_hold_nothing_fixed_by_the_code() {
        let code = ShortCode::parse("K7QF-2MXP").unwrap();
        let (_, first, _) = opening_message(&code, parties());
        let (_, second, _) = opening_message(&code, parties());
        assert_ne!(first, second);
        for canonical in [code.canonical().as_bytes(), code.grouped().as_bytes()] {
            assert!(!first.windows(canonical.len()).any(|at| at == canonical));
        }
    }

    #[tokio::test]
    async fn a_redeemer_refuses_its_own_message_reflected_back_and_proves_nothing() {
        let code = ShortCode::parse("K7QF-2MXP").unwrap();
        let (mut near, mut far) = duplex(64 * 1024);
        let reflecting = tokio::spawn(async move {
            let mut said = vec![0_u8; PREAMBLE.len() + 1];
            far.read_exact(&mut said).await.unwrap();
            let body = hear(&mut far, MAX_FRAME).await.unwrap();
            let (_, share) = heard_hello(&body).unwrap();
            let mut back = vec![SAYS_SHARE];
            back.extend_from_slice(share);
            say(&mut far, &back).await.unwrap();
            let mut rest = Vec::new();
            far.read_to_end(&mut rest).await.unwrap();
            rest
        });
        let attempt = redeem(&mut near, &code, parties(), BOUND).await;
        assert!(matches!(attempt, Attempt::Dead(Dead::Failed)));
        drop(near);
        assert!(
            reflecting.await.unwrap().is_empty(),
            "it says nothing further, its proof least of all"
        );
    }

    /// With the side byte rewritten SPAKE2 takes the message, and then the
    /// proof a redeemer sends is no proof that opens the Invite.
    #[tokio::test]
    async fn a_redeemer_refuses_its_own_proof_reflected_back() {
        let code = ShortCode::parse("K7QF-2MXP").unwrap();
        let (mut near, mut far) = duplex(64 * 1024);
        let reflecting = tokio::spawn(async move {
            let mut said = vec![0_u8; PREAMBLE.len() + 1];
            far.read_exact(&mut said).await.unwrap();
            let body = hear(&mut far, MAX_FRAME).await.unwrap();
            let (_, share) = heard_hello(&body).unwrap();
            let mut back = vec![SAYS_SHARE];
            back.extend_from_slice(share);
            back[1] = b'B';
            say(&mut far, &back).await.unwrap();
            let proof = hear(&mut far, PROOF).await.unwrap();
            let mut delivery = vec![SAYS_DELIVERY];
            delivery.extend_from_slice(&proof);
            delivery.extend_from_slice(&[0; AEAD_TAG]);
            say(&mut far, &delivery).await.unwrap();
        });
        let attempt = redeem(&mut near, &code, parties(), BOUND).await;
        assert!(matches!(attempt, Attempt::Dead(Dead::Failed)));
        reflecting.await.unwrap();
    }

    #[tokio::test]
    async fn the_serving_side_refuses_a_message_from_its_own_side() {
        let (codes, code) = door();
        let (mut near, answering) = answering(&codes, WAY);
        let (_, share) = Spake2::<Ed25519Group>::start_b(
            &Password::new(code.canonical()),
            &Identity::new(&REDEEMING),
            &Identity::new(SERVING),
        );
        near.write_all(&hello(&REDEEMING, &share)).await.unwrap();
        assert_eq!(heard_said(&mut near).await, [SAYS_FAILED]);
        answering.await.unwrap();
        assert_eq!(state(&codes), ShortCodeState::WrongCodeTried);
    }

    #[tokio::test]
    async fn the_exchange_binds_both_servers_keys_and_the_relay() {
        for (named, why) in [
            (
                Parties {
                    serving: b"a key the Relay substituted",
                    ..parties()
                },
                "the Serving Server the redeemer believes in",
            ),
            (
                Parties {
                    way: OTHER_WAY,
                    ..parties()
                },
                "the Relay the redeemer joined at",
            ),
        ] {
            let (codes, code) = door();
            let attempt = delivered_by(&codes, &code, named).await;
            assert!(matches!(attempt, Attempt::Dead(Dead::Failed)), "{why}");
            assert_eq!(state(&codes), ShortCodeState::WrongCodeTried, "{why}");
        }
    }

    /// A party passing on what the redeemer said with its key swapped for
    /// another: the Serving side uses the key it is told, and the redeemer the
    /// one it holds.
    #[tokio::test]
    async fn a_redeemers_key_swapped_on_the_way_fails_the_exchange() {
        let (codes, code) = door();
        let (mut near, answering) = answering(&codes, WAY);
        let (spake, share, _) = opening_message(&code, parties());
        near.write_all(&hello(&ANOTHER_KEY, &share)).await.unwrap();
        let first = heard_said(&mut near).await;
        let shared = spake.finish(&first[1..]).unwrap();
        let keys = Keys::derive(&shared, parties(), &share, &first[1..]);
        say(
            &mut near,
            hmac::sign(&keys.redeemer_proof, &keys.transcript).as_ref(),
        )
        .await
        .unwrap();
        assert_eq!(heard_said(&mut near).await, [SAYS_FAILED]);
        answering.await.unwrap();
    }

    #[tokio::test]
    async fn an_exchange_saying_it_speaks_the_exchange_with_no_code_live_is_refused_at_once() {
        let codes = ShortCodes::new(BOUND, BOUND);
        let (mut near, answering) = answering(&codes, WAY);
        let mut said = PREAMBLE.to_vec();
        said.push(VERSION);
        near.write_all(&said).await.unwrap();
        // It has said nothing the exchange could use, and is answered anyway.
        assert_eq!(
            heard_said(&mut near).await,
            [SAYS_REFUSED, RefusedAtOnce::NoCode.byte()]
        );
        answering.await.unwrap();
    }

    #[tokio::test]
    async fn a_code_is_tried_only_over_a_relay_its_invite_offers() {
        let (codes, code) = door();
        let (mut near, answering) = answering(&codes, OTHER_WAY);
        let attempt = redeem(
            &mut near,
            &code,
            Parties {
                way: OTHER_WAY,
                ..parties()
            },
            BOUND,
        )
        .await;
        answering.await.unwrap();
        assert!(matches!(attempt, Attempt::Refused(RefusedAtOnce::NoCode)));
        assert_eq!(state(&codes), ShortCodeState::Live, "nothing was spent");
    }

    #[tokio::test]
    async fn a_connection_saying_too_little_to_make_a_guess_leaves_the_code_live() {
        // Only what falls silent waits this out; the rest hang up.
        let (codes, _) = door_within(Duration::from_millis(20), BOUND);
        for said in [
            &PREAMBLE[..1],
            &PREAMBLE[..],
            &[PREAMBLE.as_slice(), &[VERSION]].concat(),
            &[PREAMBLE.as_slice(), &[VERSION], &[0, 40, 0, 2, 9]].concat(),
        ] {
            // Hung up on, and left to fall silent, each.
            let (mut near, answered) = answering(&codes, WAY);
            near.write_all(said).await.unwrap();
            drop(near);
            answered.await.unwrap();
            let (mut near, answered) = answering(&codes, WAY);
            near.write_all(said).await.unwrap();
            answered.await.unwrap();
            assert_eq!(state(&codes), ShortCodeState::Live, "{said:?}");
        }
    }

    #[tokio::test]
    async fn a_malformed_opening_message_spends_nothing() {
        let (codes, _) = door();
        let (mut near, answered) = answering(&codes, WAY);
        let mut said = PREAMBLE.to_vec();
        said.push(VERSION);
        said.extend_from_slice(&[0, 3, 0, 1, 9]);
        near.write_all(&said).await.unwrap();
        assert_eq!(
            heard_said(&mut near).await,
            [SAYS_REFUSED, RefusedAtOnce::Malformed.byte()]
        );
        answered.await.unwrap();
        assert_eq!(state(&codes), ShortCodeState::Live);
    }

    #[tokio::test]
    async fn another_version_of_the_exchange_is_refused_and_spends_nothing() {
        let (codes, _) = door();
        let (mut near, answered) = answering(&codes, WAY);
        let mut said = PREAMBLE.to_vec();
        said.push(VERSION + 1);
        near.write_all(&said).await.unwrap();
        assert_eq!(
            heard_said(&mut near).await,
            [SAYS_REFUSED, RefusedAtOnce::Version.byte()]
        );
        answered.await.unwrap();
        assert_eq!(state(&codes), ShortCodeState::Live);
    }

    /// Hanging up having sent its message, or falling silent, is a used guess:
    /// otherwise a party could learn something for free. The bound on what
    /// follows the Serving side's answer starts once that answer is out, so a
    /// short one here races nothing but the silence it is for.
    #[tokio::test]
    async fn a_party_that_stops_having_sent_its_message_has_had_its_guess() {
        for hang_up in [true, false] {
            let bound = if hang_up {
                BOUND
            } else {
                Duration::from_millis(20)
            };
            let (codes, code) = door_within(BOUND, bound);
            let (mut near, answered) = answering(&codes, WAY);
            let (_, _, said) = opening_message(&code, parties());
            near.write_all(&said).await.unwrap();
            let share = heard_said(&mut near).await;
            assert_eq!(share[0], SAYS_SHARE);
            if hang_up {
                drop(near);
                answered.await.unwrap();
            } else {
                // Closed by the bound, with `near` still open.
                answered.await.unwrap();
                drop(near);
            }
            assert_eq!(state(&codes), ShortCodeState::LeftUnfinished);
            assert!(state(&codes).invite_stands(), "the Invite stands");
            assert!(!codes.live_at(WAY));
        }
    }

    #[tokio::test]
    async fn a_second_exchange_under_way_gets_no_guess_and_the_first_goes_on_to_finish() {
        let (codes, code) = door();
        let (mut first, first_answered) = answering(&codes, WAY);
        let (spake, share, said) = opening_message(&code, parties());
        first.write_all(&said).await.unwrap();
        let answer = heard_said(&mut first).await;
        assert_eq!(state(&codes), ShortCodeState::UnderWay);

        let second = delivered_by(&codes, &code, parties()).await;
        assert!(matches!(second, Attempt::Refused(RefusedAtOnce::NoCode)));
        assert_eq!(state(&codes), ShortCodeState::UnderWay, "undisturbed");

        let shared = spake.finish(&answer[1..]).unwrap();
        let keys = Keys::derive(&shared, parties(), &share, &answer[1..]);
        say(
            &mut first,
            hmac::sign(&keys.redeemer_proof, &keys.transcript).as_ref(),
        )
        .await
        .unwrap();
        let delivery = heard_said(&mut first).await;
        assert_eq!(delivery[0], SAYS_DELIVERY);
        let (proof, sealed) = delivery[1..].split_at(PROOF);
        hmac::verify(&keys.serving_proof, &keys.transcript, proof).expect("it proves itself");
        assert_eq!(keys.open(sealed).unwrap().as_str(), INVITE);
        first_answered.await.unwrap();
        assert_eq!(state(&codes), ShortCodeState::Delivered);
    }

    /// A party that has yet to finish its opening holds nothing another needs:
    /// however many say one byte and stop, the holder of the right code gets
    /// through, and nothing is told of them because nothing happened.
    #[tokio::test]
    async fn parties_saying_one_byte_and_stopping_keep_no_one_out() {
        let (codes, code) = door();
        let mut stalled = Vec::new();
        for _ in 0..32 {
            let (mut near, answered) = answering(&codes, WAY);
            near.write_all(&PREAMBLE[..1]).await.unwrap();
            stalled.push((near, answered));
        }
        let Attempt::Delivered(invite) = delivered_by(&codes, &code, parties()).await else {
            panic!("the right code gets through");
        };
        assert_eq!(invite.as_str(), INVITE);
        assert_eq!(state(&codes), ShortCodeState::Delivered);
    }

    /// With no code live it is refused at its first byte, so what would have
    /// held the door open has nothing to wait for.
    #[tokio::test]
    async fn a_party_saying_one_byte_with_no_code_live_is_refused_there_and_then() {
        let codes = ShortCodes::new(BOUND, BOUND);
        let (mut near, answered) = answering(&codes, WAY);
        near.write_all(&PREAMBLE[..1]).await.unwrap();
        assert_eq!(
            heard_said(&mut near).await,
            [SAYS_REFUSED, RefusedAtOnce::NoCode.byte()]
        );
        answered.await.unwrap();
    }

    #[tokio::test]
    async fn an_opening_not_finished_within_its_own_bound_is_let_go_with_the_code_live() {
        let (codes, _) = door_within(Duration::from_millis(20), BOUND);
        let (mut near, answered) = answering(&codes, WAY);
        near.write_all(&PREAMBLE[..1]).await.unwrap();
        answered.await.unwrap();
        let mut rest = Vec::new();
        near.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "it is told nothing");
        assert_eq!(state(&codes), ShortCodeState::Live);
    }

    #[tokio::test]
    async fn a_redeemer_gives_up_within_its_bound_saying_it_did_not_finish() {
        let code = ShortCode::parse("K7QF-2MXP").unwrap();
        let (mut near, _far) = duplex(64 * 1024);
        let attempt = redeem(&mut near, &code, parties(), Duration::from_millis(20)).await;
        assert!(matches!(attempt, Attempt::Dead(Dead::Unfinished)));
    }

    #[tokio::test]
    async fn an_invite_ended_under_a_live_exchange_is_not_delivered() {
        let (codes, code) = door();
        let (mut near, answered) = answering(&codes, WAY);
        let (spake, share, said) = opening_message(&code, parties());
        near.write_all(&said).await.unwrap();
        let answer = heard_said(&mut near).await;
        {
            codes.discarded();
        }
        let shared = spake.finish(&answer[1..]).unwrap();
        let keys = Keys::derive(&shared, parties(), &share, &answer[1..]);
        say(
            &mut near,
            hmac::sign(&keys.redeemer_proof, &keys.transcript).as_ref(),
        )
        .await
        .unwrap();
        assert_eq!(
            heard_said(&mut near).await,
            [SAYS_REFUSED, RefusedAtOnce::NoCode.byte()]
        );
        answered.await.unwrap();
    }

    #[tokio::test]
    async fn an_invite_of_direct_ways_alone_has_no_code() {
        let codes = ShortCodes::new(BOUND, BOUND);
        let direct = Way::Direct("192.0.2.1:7777".parse().unwrap());
        let expires_at = Instant::now() + BOUND;
        let issued = |ways: &[Way]| codes.issued(TOKEN, expires_at, INVITE, ways).unwrap();
        assert!(issued(std::slice::from_ref(&direct)).is_none());
        assert_eq!(codes.standing().borrow().serial, 0, "nothing to say yet");
        assert!(issued(&[direct, Way::Relay(WAY.to_owned())]).is_some());
    }

    /// A burn is worth saying while the Invite stands, since a new code can
    /// still be had for it. After that every Client attaching would be told of
    /// something it could do nothing about, for as long as the Server runs.
    #[tokio::test]
    async fn a_burn_stops_being_said_once_its_invite_has_ended() {
        let direct = Way::Direct("192.0.2.1:7777".parse().unwrap());
        for ended in 0..3 {
            let (codes, code) = door();
            let attempt = delivered_by(&codes, &an_other_code(&code), parties()).await;
            assert!(matches!(attempt, Attempt::Dead(Dead::Failed)));
            assert_eq!(state(&codes), ShortCodeState::WrongCodeTried);
            let expected = match ended {
                0 => {
                    codes.discarded();
                    ShortCodeState::Ended
                }
                1 => {
                    let issued = codes
                        .issued(
                            [8; 32],
                            Instant::now() + BOUND,
                            INVITE,
                            std::slice::from_ref(&direct),
                        )
                        .unwrap();
                    assert!(issued.is_none());
                    ShortCodeState::Ended
                }
                _ => {
                    codes.redeemed(&TOKEN);
                    ShortCodeState::Spent
                }
            };
            assert_eq!(state(&codes), expected);
            assert!(!state(&codes).burned());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_invite_expiring_ends_its_code_however_it_stood() {
        let codes = ShortCodes::new(BOUND, BOUND);
        let expires_at = Instant::now() + Duration::from_secs(60);
        codes
            .issued(TOKEN, expires_at, INVITE, &[Way::Relay(WAY.to_owned())])
            .unwrap();
        assert_eq!(state(&codes), ShortCodeState::Live);
        let mut told = codes.standing();
        told.wait_for(|standing| standing.state == ShortCodeState::Ended)
            .await
            .unwrap();
        assert!(!codes.live_at(WAY));
    }

    #[tokio::test]
    async fn no_new_code_is_had_while_the_last_is_being_tried() {
        let (codes, code) = door();
        let (mut near, _answered) = answering(&codes, WAY);
        let (_, _, said) = opening_message(&code, parties());
        near.write_all(&said).await.unwrap();
        heard_said(&mut near).await;
        assert_eq!(state(&codes), ShortCodeState::UnderWay);
        let Err(refused) = codes.replace() else {
            panic!("that exchange could still deliver");
        };
        assert_eq!(refused.code, SessionErrorCode::ShortCodeBeingTried);
        assert_eq!(state(&codes), ShortCodeState::UnderWay);
    }

    #[test]
    fn a_redeemers_key_is_of_a_kind_a_server_holds_and_nothing_else() {
        for algorithm in [
            &rcgen::PKCS_ECDSA_P256_SHA256,
            &rcgen::PKCS_ECDSA_P384_SHA384,
            &rcgen::PKCS_ED25519,
        ] {
            let key = KeyPair::generate_for(algorithm)
                .unwrap()
                .subject_public_key_info();
            assert!(is_identity_key(&key), "{key:?}");
            assert!(!is_identity_key(&key[..key.len() - 1]));
            let mut longer = key.clone();
            longer.push(0);
            assert!(!is_identity_key(&longer));
            let mut altered = key.clone();
            altered[4] ^= 1;
            assert!(!is_identity_key(&altered));
        }
        assert!(!is_identity_key(
            b"a free-form field anything could travel in"
        ));
    }

    /// A key field that is no key is refused before it is taken for anything,
    /// so nothing can travel there, and nothing is spent by that.
    #[tokio::test]
    async fn an_opening_whose_key_is_no_key_spends_nothing() {
        let (codes, code) = door();
        let (_, share, _) = opening_message(&code, parties());
        let mut altered = REDEEMING.clone();
        altered[4] ^= 1;
        for key in [b"not a key".to_vec(), altered] {
            let (mut near, answered) = answering(&codes, WAY);
            near.write_all(&hello(&key, &share)).await.unwrap();
            assert_eq!(
                heard_said(&mut near).await,
                [SAYS_REFUSED, RefusedAtOnce::Malformed.byte()]
            );
            answered.await.unwrap();
            assert_eq!(state(&codes), ShortCodeState::Live);
        }
        // Longer than any key is longer than an opening is read.
        let (mut near, answered) = answering(&codes, WAY);
        near.write_all(&hello(&[9; MAX_KEY + 32], &share))
            .await
            .unwrap();
        answered.await.unwrap();
        let mut rest = Vec::new();
        near.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
        assert_eq!(state(&codes), ShortCodeState::Live);
    }

    /// Every way a share can be other than one point written the one way, in
    /// the group SPAKE2 runs in, from the side it should come from.
    fn unsound_shares(sound: &[u8], side: u8) -> Vec<(&'static str, Vec<u8>)> {
        let share = |point: [u8; 32]| [&[side][..], &point[..]].concat();
        let honest = CompressedEdwardsY(sound[1..].try_into().unwrap())
            .decompress()
            .unwrap();
        let mut identity = [0_u8; 32];
        identity[0] = 1;
        // 2²⁵⁵ - 19 + 2: a `y` written past the field's modulus, which reads as 2.
        let unreduced = |y: u8| {
            let mut written = [0xff_u8; 32];
            written[0] = 0xed + y;
            written[31] = 0x7f;
            written
        };
        let on_curve = (2..18)
            .find(|y| {
                let mut reduced = [0_u8; 32];
                reduced[0] = *y;
                CompressedEdwardsY(reduced)
                    .decompress()
                    .is_some_and(|point| !point.is_small_order() && point.is_torsion_free())
            })
            .or_else(|| {
                (2..18).find(|y| {
                    let mut reduced = [0_u8; 32];
                    reduced[0] = *y;
                    CompressedEdwardsY(reduced).decompress().is_some()
                })
            })
            .expect("some small y is on the curve");
        let torsioned: EdwardsPoint = honest + EIGHT_TORSION[1];
        vec![
            ("the identity", share(identity)),
            (
                "of order two",
                share(EIGHT_TORSION[4].compress().to_bytes()),
            ),
            (
                "of order eight",
                share(EIGHT_TORSION[1].compress().to_bytes()),
            ),
            ("a `y` past the modulus", share(unreduced(on_curve))),
            (
                "outside the prime-order group",
                share(torsioned.compress().to_bytes()),
            ),
            ("no point at all", share([0xff; 32])),
            ("from the other side", {
                let mut other = sound.to_vec();
                other[0] = if side == REDEEMER_SIDE {
                    SERVING_SIDE
                } else {
                    REDEEMER_SIDE
                };
                other
            }),
            ("too short", sound[..32].to_vec()),
        ]
    }

    #[test]
    fn an_honest_share_of_either_side_is_sound_however_it_comes_out() {
        for _ in 0..16 {
            let code = ShortCode::parse("K7QF-2MXP").unwrap();
            let (_, redeemers, _) = opening_message(&code, parties());
            assert!(is_sound_share(&redeemers, REDEEMER_SIDE));
            let (_, serving) = Spake2::<Ed25519Group>::start_b(
                &Password::new(code.canonical()),
                &Identity::new(&REDEEMING),
                &Identity::new(SERVING),
            );
            assert!(is_sound_share(&serving, SERVING_SIDE));
        }
    }

    /// The opening was read whole, so the code is spent, as any failed guess
    /// spends it.
    #[tokio::test]
    async fn the_serving_side_refuses_an_unsound_share_and_counts_it_as_a_failed_guess() {
        let (_, code) = door();
        let (_, sound, _) = opening_message(&code, parties());
        for (why, share) in unsound_shares(&sound, REDEEMER_SIDE) {
            if share.len() != SHARE {
                continue;
            }
            let (codes, _) = door();
            let (mut near, answered) = answering(&codes, WAY);
            near.write_all(&hello(&REDEEMING, &share)).await.unwrap();
            assert_eq!(heard_said(&mut near).await, [SAYS_FAILED], "{why}");
            answered.await.unwrap();
            assert_eq!(state(&codes), ShortCodeState::WrongCodeTried, "{why}");
        }
    }

    /// Nothing derived from the key goes back to what sent one of these.
    #[tokio::test]
    async fn a_redeemer_refuses_an_unsound_share_and_proves_nothing() {
        let code = ShortCode::parse("K7QF-2MXP").unwrap();
        let (_, sound) = Spake2::<Ed25519Group>::start_b(
            &Password::new(code.canonical()),
            &Identity::new(&REDEEMING),
            &Identity::new(SERVING),
        );
        for (why, share) in unsound_shares(&sound, SERVING_SIDE) {
            let (mut near, mut far) = duplex(64 * 1024);
            let standing_in = tokio::spawn(async move {
                let mut opening = vec![0_u8; PREAMBLE.len() + 1];
                far.read_exact(&mut opening).await.unwrap();
                hear(&mut far, MAX_FRAME).await.unwrap();
                let mut answer = vec![SAYS_SHARE];
                answer.extend_from_slice(&share);
                say(&mut far, &answer).await.unwrap();
                let mut rest = Vec::new();
                far.read_to_end(&mut rest).await.unwrap();
                rest
            });
            let attempt = redeem(&mut near, &code, parties(), BOUND).await;
            assert!(matches!(attempt, Attempt::Dead(Dead::Failed)), "{why}");
            drop(near);
            assert!(standing_in.await.unwrap().is_empty(), "{why}");
        }
    }

    #[tokio::test]
    async fn a_first_answer_longer_than_any_there_is_is_not_read() {
        let code = ShortCode::parse("K7QF-2MXP").unwrap();
        let (mut near, mut far) = duplex(64 * 1024);
        let standing_in = tokio::spawn(async move {
            let mut opening = vec![0_u8; PREAMBLE.len() + 1];
            far.read_exact(&mut opening).await.unwrap();
            hear(&mut far, MAX_FRAME).await.unwrap();
            far.write_all(&u16::try_from(1 + SHARE + 1).unwrap().to_be_bytes())
                .await
                .unwrap();
            far
        });
        let attempt = redeem(&mut near, &code, parties(), Duration::from_secs(5)).await;
        assert!(matches!(attempt, Attempt::Dead(Dead::Unfinished)));
        drop(standing_in.await.unwrap());
    }

    /// Randomness held still for a test: nothing else makes two runs of the
    /// exchange comparable, byte for byte.
    struct HeldStill(u64);

    impl RngCore for HeldStill {
        fn next_u32(&mut self) -> u32 {
            self.next_u64() as u32
        }

        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut mixed = self.0;
            mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            mixed ^ (mixed >> 31)
        }

        fn fill_bytes(&mut self, bytes: &mut [u8]) {
            for chunk in bytes.chunks_mut(8) {
                let next = self.next_u64().to_le_bytes();
                chunk.copy_from_slice(&next[..chunk.len()]);
            }
        }

        fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
            self.fill_bytes(bytes);
            Ok(())
        }
    }

    impl CryptoRng for HeldStill {}

    /// A stream noting everything written to it.
    struct Noting {
        stream: DuplexStream,
        written: Arc<StdMutex<Vec<u8>>>,
    }

    impl AsyncRead for Noting {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            context: &mut TaskContext<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.stream).poll_read(context, buffer)
        }
    }

    impl AsyncWrite for Noting {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            context: &mut TaskContext<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let written = std::pin::Pin::new(&mut self.stream).poll_write(context, bytes);
            if let Poll::Ready(Ok(count)) = written {
                self.written
                    .lock()
                    .unwrap()
                    .extend_from_slice(&bytes[..count]);
            }
            written
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            context: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.stream).poll_flush(context)
        }

        fn poll_shutdown(
            mut self: std::pin::Pin<&mut Self>,
            context: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.stream).poll_shutdown(context)
        }
    }

    /// What each side wrote in a whole exchange under `code`, with its
    /// randomness held still.
    async fn run_held_still(code: &ShortCode) -> (Vec<u8>, Vec<u8>) {
        let codes = ShortCodes::new(BOUND, BOUND);
        issue(&codes);
        codes.current().as_mut().unwrap().code = Some(code.clone());
        let (near, far) = duplex(64 * 1024);
        let (redeemer_wrote, serving_wrote) = (Arc::default(), Arc::default());
        let mut near = Noting {
            stream: near,
            written: Arc::clone(&redeemer_wrote),
        };
        let far = Noting {
            stream: far,
            written: Arc::clone(&serving_wrote),
        };
        let answered = tokio::spawn({
            let codes = codes.clone();
            async move {
                let by = Instant::now() + BOUND;
                let Opening::Exchange(stream) = opening(Box::new(far), by).await else {
                    panic!("the exchange opens as the exchange does");
                };
                codes
                    .answer_by(stream, WAY, SERVING, |password, a, b| {
                        Spake2::start_b_with_rng(password, a, b, HeldStill(2))
                    })
                    .await;
            }
        });
        let attempt = redeem_by(&mut near, code, parties(), BOUND, |password, a, b| {
            Spake2::start_a_with_rng(password, a, b, HeldStill(1))
        })
        .await;
        assert!(matches!(attempt, Attempt::Delivered(_)));
        answered.await.unwrap();
        let redeemer_wrote = redeemer_wrote.lock().unwrap().clone();
        let serving_wrote = serving_wrote.lock().unwrap().clone();
        (redeemer_wrote, serving_wrote)
    }

    /// Everything said in an exchange is exactly as long as the ADR says, and
    /// two codes under the same randomness differ only in SPAKE2's points, the
    /// MACs and the sealed Invite: so nothing derived from the code can be
    /// hiding in anything else, however a mutation came to put it there.
    #[tokio::test]
    async fn two_codes_differ_on_the_wire_only_where_the_exchange_says_they_must() {
        let (first_code, second_code) = (
            ShortCode::parse("K7QF-2MXP").unwrap(),
            ShortCode::parse("H4RW-9TNB").unwrap(),
        );
        let (first_r, first_s) = run_held_still(&first_code).await;
        let (second_r, second_s) = run_held_still(&second_code).await;

        // R: the preamble and version, the opening's length and the key's, the
        // key, pA; then its proof, framed.
        let key = REDEEMING.as_slice();
        let opening_body = 2 + key.len() + SHARE;
        let point_at = PREAMBLE.len() + 1 + 2 + 2 + key.len() + 1;
        let proof_at = point_at + 32 + 2;
        assert_eq!(first_r.len(), proof_at + PROOF, "what R says, exactly");
        assert_eq!(second_r.len(), first_r.len());
        assert_eq!(&first_r[..PREAMBLE.len()], PREAMBLE);
        assert_eq!(first_r[PREAMBLE.len()], VERSION);
        assert_eq!(
            &first_r[PREAMBLE.len() + 1..PREAMBLE.len() + 3],
            u16::try_from(opening_body).unwrap().to_be_bytes()
        );
        assert_eq!(
            &first_r[PREAMBLE.len() + 5..point_at - 1],
            key,
            "its own key"
        );
        assert_eq!(first_r[point_at - 1], REDEEMER_SIDE);
        assert_eq!(
            &first_r[proof_at - 2..proof_at],
            u16::try_from(PROOF).unwrap().to_be_bytes()
        );
        assert_eq!(first_r[..point_at], second_r[..point_at], "all but pA");
        assert_eq!(
            first_r[point_at + 32..proof_at],
            second_r[point_at + 32..proof_at],
            "all but pA and the MAC"
        );
        assert_ne!(
            first_r[point_at..point_at + 32],
            second_r[point_at..point_at + 32]
        );
        assert_ne!(first_r[proof_at..], second_r[proof_at..]);

        // S: pB, framed; then its proof and the sealed Invite, framed.
        let share_frame = 2 + 1 + SHARE;
        let delivery_body = 1 + PROOF + INVITE.len() + AEAD_TAG;
        assert_eq!(
            first_s.len(),
            share_frame + 2 + delivery_body,
            "what S says, exactly"
        );
        assert_eq!(second_s.len(), first_s.len());
        let serving_point_at = 2 + 1 + 1;
        assert_eq!(
            first_s[..serving_point_at],
            second_s[..serving_point_at],
            "all but pB"
        );
        assert_eq!(first_s[2], SAYS_SHARE);
        assert_eq!(first_s[3], SERVING_SIDE);
        assert_eq!(
            first_s[share_frame..share_frame + 3],
            second_s[share_frame..share_frame + 3],
            "the delivery's frame"
        );
        assert_eq!(first_s[share_frame + 2], SAYS_DELIVERY);
        assert_ne!(
            first_s[serving_point_at..share_frame],
            second_s[serving_point_at..share_frame]
        );
        assert_ne!(first_s[share_frame + 3..], second_s[share_frame + 3..]);

        // And the same code under the same randomness says the same, so that
        // is everything that varies.
        assert_eq!(run_held_still(&first_code).await, (first_r, first_s));
    }

    #[tokio::test]
    async fn a_new_code_for_the_invite_that_stands_replaces_the_old_and_leaves_the_invite() {
        let (codes, code) = door();
        let wrong = delivered_by(&codes, &an_other_code(&code), parties()).await;
        assert!(matches!(wrong, Attempt::Dead(Dead::Failed)));
        let fresh = codes
            .replace()
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert_eq!(state(&codes), ShortCodeState::Live);
        assert_eq!(codes.standing().borrow().serial, fresh.serial);
        let Attempt::Delivered(invite) = delivered_by(&codes, &fresh.code, parties()).await else {
            panic!("the new code delivers the same Invite");
        };
        assert_eq!(invite.as_str(), INVITE);
    }

    #[tokio::test]
    async fn the_opening_byte_is_read_and_put_back_for_tls() {
        let (mut near, far) = duplex(1024);
        near.write_all(&[0x16, 0x03, 0x01, 0x42]).await.unwrap();
        let by = Instant::now() + BOUND;
        let Opening::Tls(mut stream) = opening(Box::new(far), by).await else {
            panic!("a TLS record is left to TLS");
        };
        let mut read = [0_u8; 4];
        stream.read_exact(&mut read).await.unwrap();
        assert_eq!(read, [0x16, 0x03, 0x01, 0x42]);
        stream.write_all(b"answer").await.unwrap();
        let mut answer = [0_u8; 6];
        near.read_exact(&mut answer).await.unwrap();
        assert_eq!(&answer, b"answer");
    }

    #[tokio::test]
    async fn a_connection_saying_nothing_is_let_go_at_its_bound() {
        let (_near, far) = duplex(1024);
        let by = Instant::now() + Duration::from_millis(10);
        assert!(matches!(opening(Box::new(far), by).await, Opening::Nothing));
        let (near, far) = duplex(1024);
        drop(near);
        assert!(matches!(
            opening(Box::new(far), Instant::now() + BOUND).await,
            Opening::Nothing
        ));
    }

    /// The transcript holds the Relay and both keys each set apart from the
    /// next, so moving a byte from one to another makes another transcript.
    #[test]
    fn what_the_transcript_covers_cannot_be_moved_between_its_parts() {
        let of = |way: &'static str, redeeming: &'static [u8], serving: &'static [u8]| {
            transcript(
                Parties {
                    way,
                    redeeming,
                    serving,
                },
                b"share",
                b"share",
            )
        };
        assert_ne!(of("ab", b"c", b"d"), of("a", b"bc", b"d"));
        assert_ne!(of("a", b"bc", b"d"), of("a", b"b", b"cd"));
    }
}
