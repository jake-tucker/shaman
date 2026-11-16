//! A thin Client's identity key as something that signs and cannot be read: the signer the app hands in (a Secure
//! Enclave key on a phone, ADR-0081), wrapped so rcgen mints certificates with it, rustls signs the handshake with
//! it, and a Relay's challenge is answered with it. Nothing here ever holds private key bytes.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use rcgen::{CertificateParams, DistinguishedName, DnType, PublicKeyData as _};
use rustls::{SignatureAlgorithm, SignatureScheme, pki_types::CertificateDer, sign::CertifiedKey};

use crate::{Failure, Signer};

/// The identity key, by the signer that holds it.
pub struct RemoteKey {
    signer: Arc<dyn Signer>,
    /// The public key as its X9.63 point, which is what a certificate's SubjectPublicKeyInfo wraps.
    point: Vec<u8>,
    signatures: AtomicU64,
}

impl fmt::Debug for RemoteKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RemoteKey")
    }
}

impl RemoteKey {
    pub fn new(signer: Arc<dyn Signer>) -> Result<Arc<Self>, Failure> {
        let point = signer.public_key();
        if point.len() != 65 || point[0] != 4 {
            return Err(format!(
                "the signer's public key is {} bytes and not an uncompressed P-256 point",
                point.len()
            )
            .into());
        }
        Ok(Arc::new(Self {
            signer,
            point,
            signatures: AtomicU64::new(0),
        }))
    }

    /// How many times the key has signed since the core was made.
    pub fn signatures(&self) -> u64 {
        self.signatures.load(Ordering::Relaxed)
    }

    /// The key as the DER SubjectPublicKeyInfo a Pairing pins and a Relay knows a Login by.
    pub fn spki(&self) -> Vec<u8> {
        self.subject_public_key_info()
    }

    /// An ASN.1 ECDSA signature over SHA-256 of `message`.
    pub fn sign_message(&self, message: &[u8]) -> Result<Vec<u8>, Failure> {
        self.signatures.fetch_add(1, Ordering::Relaxed);
        self.signer
            .sign(message.to_vec())
            .map_err(|error| format!("the signer refused: {error}").into())
    }

    /// The certificate the key presents once paired: self-signed, naming nothing.
    pub fn identity_certificate(&self) -> Result<Vec<u8>, Failure> {
        Ok(CertificateParams::new(Vec::<String>::new())?
            .self_signed(self)?
            .der()
            .to_vec())
    }

    /// The certificate an Invite is redeemed by: the same key, naming the Invite's token.
    pub fn enrollment_certificate(&self, token: &str) -> Result<Vec<u8>, Failure> {
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, format!("shaman-invite-{token}"));
        Ok(params.self_signed(self)?.der().to_vec())
    }
}

impl rcgen::PublicKeyData for RemoteKey {
    fn der_bytes(&self) -> &[u8] {
        &self.point
    }

    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        &rcgen::PKCS_ECDSA_P256_SHA256
    }
}

impl rcgen::SigningKey for RemoteKey {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        self.sign_message(message)
            .map_err(|_| rcgen::Error::RemoteKeyError)
    }
}

/// The key as rustls asks a client's key to sign a handshake.
#[derive(Debug)]
struct HandshakeKey(Arc<RemoteKey>);

impl rustls::sign::SigningKey for HandshakeKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn rustls::sign::Signer>> {
        offered
            .contains(&SignatureScheme::ECDSA_NISTP256_SHA256)
            .then(|| Box::new(HandshakeKey(self.0.clone())) as Box<dyn rustls::sign::Signer>)
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ECDSA
    }
}

impl rustls::sign::Signer for HandshakeKey {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        self.0
            .sign_message(message)
            .map_err(|error| rustls::Error::General(error.to_string()))
    }

    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::ECDSA_NISTP256_SHA256
    }
}

/// `certificate` and the key, as rustls presents a client's.
pub fn certified(key: &Arc<RemoteKey>, certificate: Vec<u8>) -> CertifiedKey {
    CertifiedKey::new(
        vec![CertificateDer::from(certificate)],
        Arc::new(HandshakeKey(key.clone())),
    )
}
