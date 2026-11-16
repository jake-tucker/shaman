//! What a thin Client would do with its identity key, after `src/serving/identity.rs`, `enrollment_certificate`
//! in `src/serving.rs` and the proof a Relay asks for in `src/relays.rs`: mint the key and its certificate, mint the
//! certificate that names an Invite's token, and prove the key to a Relay.

use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PublicKeyData as _};
use shaman_relay_protocol::{Bytes, SPOKEN, ServerMessage, fingerprint, proof_message};

use crate::{Failure, client::Identity};

/// A fresh ECDSA P-256 key and its self-signed certificate.
pub fn new_identity() -> Result<Identity, Failure> {
    let key = KeyPair::generate()?;
    Ok(Identity {
        certificate: CertificateParams::new(Vec::<String>::new())?
            .self_signed(&key)?
            .der()
            .to_vec(),
        private_key: key.serialize_der(),
    })
}

/// The key as the DER SubjectPublicKeyInfo a Pairing pins.
pub fn public_key(identity: &Identity) -> Result<Vec<u8>, Failure> {
    Ok(KeyPair::try_from(identity.private_key.as_slice())?.subject_public_key_info())
}

/// The identity an Invite is redeemed by: the same key under a certificate naming the Invite's token.
pub fn enrollment_identity(identity: &Identity, token: &str) -> Result<Identity, Failure> {
    let key = KeyPair::try_from(identity.private_key.as_slice())?;
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, format!("shaman-invite-{token}"));
    Ok(Identity {
        certificate: params.self_signed(&key)?.der().to_vec(),
        private_key: identity.private_key.clone(),
    })
}

/// The first thing said to a Relay.
pub fn hello(identity: &Identity) -> Result<String, Failure> {
    Ok(serde_json::to_string(&ServerMessage::Hello {
        versions: SPOKEN.to_vec(),
        key: Bytes(public_key(identity)?),
    })?)
}

/// The answer to a Relay's challenge.
pub fn proof(identity: &Identity, relay: &str, nonce: &[u8]) -> Result<String, Failure> {
    let key = KeyPair::try_from(identity.private_key.as_slice())?;
    let message = proof_message(relay, nonce, &key.subject_public_key_info());
    Ok(serde_json::to_string(&ServerMessage::Proof {
        signature: Bytes(rcgen::SigningKey::sign(&key, &message)?),
    })?)
}

/// The machine's hostname, as `machine_hostname` in `src/serving.rs` reads it before shortening it.
pub fn machine_hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .unwrap_or_default()
}

/// `identity [relay address]`
pub fn describe(arguments: &[String]) -> Result<(), Failure> {
    let identity = new_identity()?;
    println!("hostname: {:?}", machine_hostname());
    println!("fingerprint: {}", fingerprint(&public_key(&identity)?));
    println!("hello: {}", hello(&identity)?);
    if let Some(relay) = arguments.first() {
        let enrolling = enrollment_identity(&identity, "token")?;
        println!(
            "enrollment certificate: {} bytes",
            enrolling.certificate.len()
        );
        println!("proof: {}", proof(&identity, relay, &[0; 32])?);
    }
    Ok(())
}
