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
//! read whole and before anything is answered, and it is taken out of the
//! ledger there, under the ledger's lock: so two at once get one guess between
//! them, the one that came first goes on undisturbed, and a party that hangs up
//! or falls silent having sent its message has had its guess. One that says
//! less than that has made none, and spends nothing.

use std::{
    sync::{Arc, Mutex as StdMutex, atomic::AtomicU64, atomic::Ordering},
    task::{Context as TaskContext, Poll},
};

use ring::{
    aead::{self, Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey},
    digest, hkdf, hmac,
    rand::{SecureRandom as _, SystemRandom},
};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf},
    sync::{Semaphore, watch},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

use super::{ByteStream, InviteLedger, IssuedToken, TokenState};
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

/// How long an identity key may be, as an Invite carrying one may.
const MAX_KEY: usize = 4096;
/// How long a SPAKE2 message is: which side it comes from, and a point.
const SHARE: usize = 33;
/// How long a MAC is.
const PROOF: usize = 32;
/// The longest Invite a code delivers, which is far past any an Invite's ways
/// come to: one that long is issued with no code.
const MAX_INVITE: usize = 32 * 1024;
/// The longest anything said in the exchange is.
const MAX_FRAME: usize = 1 + PROOF + MAX_INVITE + AEAD_TAG;
const AEAD_TAG: usize = 16;

/// How many exchanges may be under way at once, spending a code or not. Past
/// it one is refused at once, so what stalls here takes up no more of the room
/// the Serving side's TLS handshakes share.
const EXCHANGES_AT_ONCE: usize = 4;

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
    /// As many exchanges are under way as it takes at once.
    Busy,
    /// It speaks no such version of the exchange.
    Version,
    /// What it was told was not what the exchange says.
    Malformed,
}

impl RefusedAtOnce {
    fn byte(self) -> u8 {
        match self {
            Self::NoCode => 1,
            Self::Busy => 2,
            Self::Version => 3,
            Self::Malformed => 4,
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::NoCode),
            2 => Some(Self::Busy),
            3 => Some(Self::Version),
            4 => Some(Self::Malformed),
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

/// Reads what the opening message says after the preamble and version.
fn heard_hello(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let (length, rest) = body.split_first_chunk::<2>()?;
    let length = usize::from(u16::from_be_bytes(*length));
    if length == 0 || length > MAX_KEY || rest.len() != length + SHARE {
        return None;
    }
    Some(rest.split_at(length))
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
    tokio::time::timeout(within, redeeming(stream, code, parties))
        .await
        .unwrap_or(Attempt::Dead(Dead::Unfinished))
}

async fn redeeming<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    code: &ShortCode,
    parties: Parties<'_>,
) -> Attempt {
    let (spake, share) = Spake2::<Ed25519Group>::start_a(
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
    let Ok(answer) = hear(stream, MAX_FRAME).await else {
        return Attempt::Dead(Dead::Unfinished);
    };
    let serving_share = match answer.split_first() {
        Some((&SAYS_SHARE, serving_share)) if serving_share.len() == SHARE => serving_share,
        Some((&SAYS_REFUSED, [why])) => {
            return match RefusedAtOnce::from_byte(*why) {
                Some(refused) => Attempt::Refused(refused),
                None => Attempt::Dead(Dead::Failed),
            };
        }
        _ => return Attempt::Dead(Dead::Failed),
    };
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

/// The short code of the Invite that stands, and what it delivers, held in the
/// ledger with that Invite and gone with it.
pub(super) struct ShortCodeEntry {
    /// `None` from when an exchange takes it: nothing takes it twice.
    code: Option<ShortCode>,
    serial: u64,
    /// The addresses of the Relays the Invite offers, the only ones the
    /// exchange is run over.
    relays: Vec<String>,
    invite: Zeroizing<String>,
}

/// The Serving side's door for the exchange: its codes, however they stand, and
/// how many exchanges may come through at once.
#[derive(Clone)]
pub(super) struct ShortCodes {
    invites: Arc<StdMutex<InviteLedger>>,
    standing: Arc<watch::Sender<ShortCodeStanding>>,
    last_serial: Arc<AtomicU64>,
    at_once: Arc<Semaphore>,
    timeout: Duration,
}

impl ShortCodes {
    pub(super) fn new(invites: Arc<StdMutex<InviteLedger>>, timeout: Duration) -> Self {
        Self {
            invites,
            standing: Arc::new(watch::Sender::new(ShortCodeStanding::default())),
            last_serial: Arc::default(),
            at_once: Arc::new(Semaphore::new(EXCHANGES_AT_ONCE)),
            timeout,
        }
    }

    /// The same door, with exchanges closed within `timeout`.
    pub(super) fn within(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    /// How long an exchange may take, on either side of it.
    pub(super) fn timeout(&self) -> Duration {
        self.timeout
    }

    /// How the latest code stands, moving on as that changes.
    pub(super) fn standing(&self) -> watch::Receiver<ShortCodeStanding> {
        self.standing.subscribe()
    }

    /// A fresh code for the Invite `invite`, which offers `ways`, where it
    /// offers a Relay: the exchange needs a joined connection, so one offering
    /// direct ways alone has none.
    pub(super) fn mint(
        &self,
        invite: &str,
        ways: &[Way],
    ) -> std::io::Result<Option<(ShortCodeEntry, IssuedShortCode)>> {
        let relays = ways
            .iter()
            .filter_map(|way| match way {
                Way::Relay(relay) => Some(relay.clone()),
                Way::Direct(_) => None,
            })
            .collect::<Vec<_>>();
        if relays.is_empty() || invite.len() > MAX_INVITE {
            return Ok(None);
        }
        let (code, issued) = self.fresh_code()?;
        Ok(Some((
            ShortCodeEntry {
                code: Some(code),
                serial: issued.serial,
                relays,
                invite: Zeroizing::new(invite.to_owned()),
            },
            issued,
        )))
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
    pub(super) fn announce(&self, serial: u64) {
        self.standing.send_replace(ShortCodeStanding {
            serial,
            state: ShortCodeState::Live,
            invite_stands: true,
        });
    }

    /// Says the Invite the latest code came with no longer stands: that code
    /// ended with it, unless an attempt used it up first, which goes on being
    /// said.
    pub(super) fn invite_ended(&self) {
        self.standing.send_if_modified(|standing| {
            let before = standing.clone();
            standing.invite_stands = false;
            if !standing.state.burned() {
                standing.state = ShortCodeState::Ended;
            }
            *standing != before
        });
    }

    /// A fresh code for the Invite that stands, in place of whatever became of
    /// the last, leaving that Invite as it is. A code goes for one guess, so
    /// each one costs a reader's asking for it, and however many they ask for
    /// the Invite's lifetime does not move.
    pub(super) fn replace(&self) -> Result<IssuedShortCode, (SessionErrorCode, &'static str)> {
        let mut invites = self
            .invites
            .lock()
            .expect("Invite ledger lock is not poisoned");
        let Some(issued) = standing_invite(&mut invites) else {
            return Err((
                SessionErrorCode::InviteExpired,
                "the Invite this code came with has ended; issue a new one with /serve",
            ));
        };
        let Some(entry) = issued.short.as_mut() else {
            return Err((
                SessionErrorCode::InvalidInviteWays,
                "this Invite offers no Relay, so it has no code",
            ));
        };
        let (code, fresh) = self.fresh_code().map_err(|_| {
            (
                SessionErrorCode::ServingListenerFailed,
                "the system's randomness did not answer",
            )
        })?;
        entry.code = Some(code);
        entry.serial = fresh.serial;
        self.announce(fresh.serial);
        Ok(fresh)
    }

    /// Whether a code is live to be tried over the Relay at `way`, spending
    /// nothing.
    fn live_at(&self, way: &str) -> bool {
        let mut invites = self
            .invites
            .lock()
            .expect("Invite ledger lock is not poisoned");
        standing_invite(&mut invites)
            .and_then(|issued| issued.short.as_ref())
            .is_some_and(|entry| entry.code.is_some() && entry.relays.iter().any(|at| at == way))
    }

    /// Takes the code live over the Relay at `way` for one exchange. This is
    /// where the guess is spent: from here nothing else can take it, and it
    /// stays used however the exchange ends.
    fn take(&self, way: &str) -> Option<Taken> {
        let mut invites = self
            .invites
            .lock()
            .expect("Invite ledger lock is not poisoned");
        let issued = standing_invite(&mut invites)?;
        let token = issued.token;
        let entry = issued.short.as_mut()?;
        if !entry.relays.iter().any(|at| at == way) {
            return None;
        }
        let code = entry.code.take()?;
        let serial = entry.serial;
        self.standing.send_replace(ShortCodeStanding {
            serial,
            state: ShortCodeState::UnderWay,
            invite_stands: true,
        });
        Some(Taken {
            codes: self.clone(),
            code,
            token,
            serial,
        })
    }

    /// Serves one connection a Relay carried, its first byte read and found to
    /// open the exchange, closing it however that ends. With no code live it
    /// is refused as soon as it has said it speaks the exchange, and no
    /// cryptography is done for it. The whole of it is bounded: it is dropped
    /// there, and any code it took counts as a guess.
    pub(super) async fn answer(&self, mut stream: Box<dyn ByteStream>, way: &str, serving: &[u8]) {
        let Ok(_room) = self.at_once.clone().try_acquire_owned() else {
            refuse(&mut stream, RefusedAtOnce::Busy).await;
            let _ = stream.shutdown().await;
            return;
        };
        let deadline = Instant::now() + self.timeout;
        let answered = tokio::time::timeout_at(deadline, self.answering(&mut stream, way, serving));
        if answered.await.is_err() {
            tracing::debug!("a short Invite exchange was closed unfinished");
        }
        let _ = tokio::time::timeout_at(deadline, stream.shutdown()).await;
    }

    async fn answering(&self, stream: &mut Box<dyn ByteStream>, way: &str, serving: &[u8]) {
        // The first byte of the preamble is read already, and the version follows.
        let mut opening = [0_u8; PREAMBLE.len()];
        if stream.read_exact(&mut opening).await.is_err() {
            return;
        }
        let (rest, version) = opening.split_at(PREAMBLE.len() - 1);
        if rest != &PREAMBLE[1..] {
            return;
        }
        if version != [VERSION] {
            refuse(stream, RefusedAtOnce::Version).await;
            return;
        }
        if !self.live_at(way) {
            refuse(stream, RefusedAtOnce::NoCode).await;
            return;
        }
        let Ok(body) = hear(stream, 2 + MAX_KEY + SHARE).await else {
            return;
        };
        let Some((redeeming, redeemer_share)) = heard_hello(&body) else {
            refuse(stream, RefusedAtOnce::Malformed).await;
            return;
        };
        let Some(taken) = self.take(way) else {
            refuse(stream, RefusedAtOnce::NoCode).await;
            return;
        };
        let parties = Parties {
            way,
            redeeming,
            serving,
        };
        let (spake, share) = Spake2::<Ed25519Group>::start_b(
            &Password::new(taken.code.canonical()),
            &Identity::new(parties.redeeming),
            &Identity::new(parties.serving),
        );
        let Ok(shared) = spake.finish(redeemer_share).map(Zeroizing::new) else {
            taken.ended(ShortCodeState::WrongCodeTried);
            let _ = say(stream, &[SAYS_FAILED]).await;
            return;
        };
        let keys = Keys::derive(&shared, parties, redeemer_share, &share);
        let mut said = Vec::with_capacity(1 + share.len());
        said.push(SAYS_SHARE);
        said.extend_from_slice(&share);
        if say(stream, &said).await.is_err() {
            return;
        }
        let Ok(proof) = hear(stream, PROOF).await else {
            return;
        };
        if hmac::verify(&keys.redeemer_proof, &keys.transcript, &proof).is_err() {
            taken.ended(ShortCodeState::WrongCodeTried);
            let _ = say(stream, &[SAYS_FAILED]).await;
            return;
        }
        let Some(invite) = taken.invite() else {
            taken.ended(ShortCodeState::Ended);
            refuse(stream, RefusedAtOnce::NoCode).await;
            return;
        };
        let proof = hmac::sign(&keys.serving_proof, &keys.transcript);
        let mut delivery = Zeroizing::new(Vec::with_capacity(1 + PROOF + invite.len() + AEAD_TAG));
        delivery.push(SAYS_DELIVERY);
        delivery.extend_from_slice(proof.as_ref());
        delivery.extend_from_slice(&keys.seal(&invite));
        if say(stream, &delivery).await.is_ok() {
            taken.ended(ShortCodeState::Delivered);
        }
    }

    /// Expires the Invite whose token is `token` where it still stands as it
    /// comes to `expires_at`, so whoever is shown its code hears of it then.
    pub(super) fn watch_expiry(&self, token: [u8; 32], expires_at: tokio::time::Instant) {
        let codes = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(expires_at).await;
            let expired = {
                let mut invites = codes
                    .invites
                    .lock()
                    .expect("Invite ledger lock is not poisoned");
                invites
                    .issued
                    .last_mut()
                    .filter(|issued| {
                        issued.token == token && matches!(issued.state, TokenState::Outstanding)
                    })
                    .map(|issued| issued.short.take())
                    .is_some()
            };
            if expired {
                codes.invite_ended();
            }
        });
    }
}

/// The Invite in the ledger that can still be redeemed, if any does.
fn standing_invite(invites: &mut InviteLedger) -> Option<&mut IssuedToken> {
    let now = tokio::time::Instant::now();
    invites
        .issued
        .iter_mut()
        .rev()
        .find(|issued| matches!(issued.state, TokenState::Outstanding) && now < issued.expires_at)
}

async fn refuse<S: AsyncWrite + Unpin>(stream: &mut S, why: RefusedAtOnce) {
    let _ = say(stream, &[SAYS_REFUSED, why.byte()]).await;
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
        let mut invites = self
            .codes
            .invites
            .lock()
            .expect("Invite ledger lock is not poisoned");
        standing_invite(&mut invites)
            .filter(|issued| issued.token == self.token)
            .and_then(|issued| issued.short.as_ref())
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};

    use super::*;

    const WAY: &str = "https://relay.example.com";
    const OTHER_WAY: &str = "https://other.example.com";
    const INVITE: &str = "suru-v1-the-long-string";
    const BOUND: Duration = Duration::from_secs(30);
    const SERVING: &[u8] = b"the Serving Server's key";
    const REDEEMING: &[u8] = b"the redeeming Server's key";

    fn parties() -> Parties<'static> {
        Parties {
            way: WAY,
            redeeming: REDEEMING,
            serving: SERVING,
        }
    }

    /// A door with an Invite offering [`WAY`] standing, and that Invite's code.
    fn door_within(timeout: Duration) -> (ShortCodes, ShortCode) {
        let codes = ShortCodes::new(Arc::default(), timeout);
        let code = issue(&codes);
        (codes, code)
    }

    /// Issues an Invite offering [`WAY`], as the controller does.
    fn issue(codes: &ShortCodes) -> ShortCode {
        let (entry, issued) = codes
            .mint(INVITE, &[Way::Relay(WAY.to_owned())])
            .unwrap()
            .expect("an Invite offering a Relay has a code");
        let mut invites = codes.invites.lock().unwrap();
        invites.issued.push(IssuedToken {
            token: [7; 32],
            expires_at: tokio::time::Instant::now() + Duration::from_secs(600),
            state: TokenState::Outstanding,
            short: Some(entry),
        });
        codes.announce(issued.serial);
        issued.code
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
        let (codes, code) = door_within(BOUND);
        let Attempt::Delivered(invite) = delivered_by(&codes, &code, parties()).await else {
            panic!("the right code delivers the Invite");
        };
        assert_eq!(invite.as_str(), INVITE);
        assert_eq!(state(&codes), ShortCodeState::Delivered);
        assert!(!codes.live_at(WAY), "a code delivers once");
    }

    #[tokio::test]
    async fn a_wrong_code_delivers_nothing_ends_the_code_and_is_said_to_be_one() {
        let (codes, code) = door_within(BOUND);
        let attempt = delivered_by(&codes, &an_other_code(&code), parties()).await;
        assert!(matches!(attempt, Attempt::Dead(Dead::Failed)));
        assert_eq!(state(&codes), ShortCodeState::WrongCodeTried);
        assert!(codes.standing().borrow().invite_stands, "the Invite stands");
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
        let (codes, code) = door_within(BOUND);
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
        let (codes, code) = door_within(BOUND);
        let (mut near, answering) = answering(&codes, WAY);
        let (_, share) = Spake2::<Ed25519Group>::start_b(
            &Password::new(code.canonical()),
            &Identity::new(REDEEMING),
            &Identity::new(SERVING),
        );
        near.write_all(&hello(REDEEMING, &share)).await.unwrap();
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
            let (codes, code) = door_within(BOUND);
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
        let (codes, code) = door_within(BOUND);
        let (mut near, answering) = answering(&codes, WAY);
        let (spake, share, _) = opening_message(&code, parties());
        near.write_all(&hello(b"a key of the Relay's own", &share))
            .await
            .unwrap();
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
        let codes = ShortCodes::new(Arc::default(), BOUND);
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
        let (codes, code) = door_within(BOUND);
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
        let (codes, _) = door_within(Duration::from_millis(50));
        for said in [
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
        let (codes, _) = door_within(BOUND);
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
        let (codes, _) = door_within(BOUND);
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
    /// otherwise a party could learn something for free.
    #[tokio::test]
    async fn a_party_that_stops_having_sent_its_message_has_had_its_guess() {
        for hang_up in [true, false] {
            let (codes, code) = door_within(Duration::from_millis(50));
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
            assert!(codes.standing().borrow().invite_stands, "the Invite stands");
            assert!(!codes.live_at(WAY));
        }
    }

    #[tokio::test]
    async fn a_second_exchange_under_way_gets_no_guess_and_the_first_goes_on_to_finish() {
        let (codes, code) = door_within(BOUND);
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

    #[tokio::test]
    async fn past_the_exchanges_it_takes_at_once_one_is_refused_and_spends_nothing() {
        let (codes, code) = door_within(BOUND);
        let mut stalled = Vec::new();
        for _ in 0..EXCHANGES_AT_ONCE {
            let (mut near, answered) = answering(&codes, WAY);
            near.write_all(&PREAMBLE[..1]).await.unwrap();
            stalled.push((near, answered));
        }
        // Waits until each has come through the door.
        while codes.at_once.available_permits() > 0 {
            tokio::task::yield_now().await;
        }
        let refused = delivered_by(&codes, &code, parties()).await;
        assert!(matches!(refused, Attempt::Refused(RefusedAtOnce::Busy)));
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
        let (codes, code) = door_within(BOUND);
        let (mut near, answered) = answering(&codes, WAY);
        let (spake, share, said) = opening_message(&code, parties());
        near.write_all(&said).await.unwrap();
        let answer = heard_said(&mut near).await;
        {
            let mut invites = codes.invites.lock().unwrap();
            invites.issued[0].state = TokenState::Superseded;
            invites.issued[0].short = None;
            codes.invite_ended();
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

    #[test]
    fn an_invite_of_direct_ways_alone_has_no_code() {
        let codes = ShortCodes::new(Arc::default(), BOUND);
        let direct = Way::Direct("192.0.2.1:7777".parse().unwrap());
        assert!(
            codes
                .mint(INVITE, std::slice::from_ref(&direct))
                .unwrap()
                .is_none()
        );
        assert!(
            codes
                .mint(INVITE, &[direct, Way::Relay(WAY.to_owned())])
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_new_code_for_the_invite_that_stands_replaces_the_old_and_leaves_the_invite() {
        let (codes, code) = door_within(BOUND);
        let wrong = delivered_by(&codes, &an_other_code(&code), parties()).await;
        assert!(matches!(wrong, Attempt::Dead(Dead::Failed)));
        let fresh = codes.replace().map_err(|(_, why)| why).unwrap();
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
