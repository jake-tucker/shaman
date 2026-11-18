//! This phone's identity key: what signs with it, which the app hands in, and
//! the reconciliation, at every start, of that key with the records kept
//! beside it (ADR-0081).
//!
//! The key never crosses. On an iPhone it is a Secure Enclave key whose blob
//! the app keeps in the Keychain, and the core asks the app for each
//! signature through a [`KeySigner`]. The records are of one key: what each
//! Remote pins and each Relay knows this phone by. So records found beside
//! another key than theirs are discarded, the name alone staying, and the
//! phone is told once that it must pair again. Records found with no key are
//! no concern of the core's: the app reuses whatever key the Keychain still
//! holds, and the core takes it up as the key of the records it begins.

use std::{fmt, path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use shaman_pairing::{IdentityMaterial, relay::RELAYS_FILE, signer::IdentitySigner};

use crate::{
    OpenError,
    device::{DeviceRecord, RECORD_FILE, Records},
};

/// Signs with this phone's identity key, wherever the app keeps it: the
/// signer, written in Swift, that the core takes up.
///
/// The key is ECDSA on P-256 and every signature is over SHA-256. A
/// signature is asked for in the middle of a TLS handshake, so `sign` is
/// called on one of the core's workers and blocks it until it answers, as a
/// Secure Enclave does in milliseconds.
#[uniffi::export(with_foreign)]
pub trait KeySigner: Send + Sync {
    /// The key, as the DER SubjectPublicKeyInfo of a P-256 public key.
    fn public_key(&self) -> Vec<u8>;

    /// The ECDSA signature by the key of the SHA-256 digest of `message`,
    /// ASN.1 DER-encoded.
    fn sign(&self, message: Vec<u8>) -> Result<Vec<u8>, SignError>;
}

/// Why the key would not sign, in the words of whatever holds it.
#[derive(Debug, PartialEq, Eq, uniffi::Error)]
pub enum SignError {
    Refused { message: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for SignError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Refused {
            message: error.reason,
        }
    }
}

impl fmt::Display for SignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Refused { message } = self;
        formatter.write_str(message)
    }
}

impl std::error::Error for SignError {}

/// A [`KeySigner`] as the shared pairing and Relay code signs through one.
struct Crossed(Arc<dyn KeySigner>);

impl shaman_pairing::signer::Signer for Crossed {
    fn public_key(&self) -> Vec<u8> {
        self.0.public_key()
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        self.0
            .sign(message.to_vec())
            .map_err(|refused| anyhow!(refused))
    }
}

/// `signer`, taken up as everything that proves this phone signs.
pub(crate) fn taken_up(signer: Arc<dyn KeySigner>) -> Result<IdentitySigner> {
    IdentitySigner::new(Crossed(signer))
}

/// The key the records are of, as the thin Client's own file keeps it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct KeptKey {
    /// The key's fingerprint, as a Serving Server knows this phone by it.
    pub fingerprint: String,
    /// The phone's identity certificate, which the key signed once and
    /// every redial presents, as base64 DER.
    certificate: String,
}

impl KeptKey {
    fn of(identity: &IdentityMaterial) -> Self {
        Self {
            fingerprint: shaman_pairing::fingerprint(identity.public_key()),
            certificate: STANDARD.encode(&identity.certificate),
        }
    }

    /// What proves `signer`'s key, by the certificate kept for it, where
    /// that certificate is still one of the key.
    fn identity(&self, signer: &IdentitySigner) -> Option<IdentityMaterial> {
        let certificate = STANDARD.decode(&self.certificate).ok()?;
        signer.tls_identity(certificate.clone()).ok()?;
        Some(IdentityMaterial {
            signer: signer.clone(),
            certificate,
        })
    }
}

/// The records and the key, reconciled.
pub(crate) struct Reconciled {
    pub record: DeviceRecord,
    pub identity: IdentityMaterial,
}

/// Reconciles `signer`'s key with what `records` holds, so that everything
/// kept from here on is of that key: records of another are discarded, the
/// name alone staying, and the phone is told it must pair again where they
/// held a Remote or a Relay entry. The certificate is minted only where none
/// of this key is kept, so a start signs once at most, and a key that will
/// not sign discards nothing.
pub(crate) fn reconcile(
    records: &Records,
    signer: IdentitySigner,
) -> Result<Reconciled, OpenError> {
    reconcile_discarding(records, signer, discard)
}

/// [`reconcile`], each record of another key going by `discard`, which a
/// test may end partway, as the app may be ended.
fn reconcile_discarding(
    records: &Records,
    signer: IdentitySigner,
    discard: impl Fn(&Path) -> Result<()>,
) -> Result<Reconciled, OpenError> {
    let mut record = records.read()?;
    let fingerprint = shaman_pairing::fingerprint(signer.public_key());
    let theirs = record
        .key
        .as_ref()
        .is_some_and(|kept| kept.fingerprint == fingerprint);
    if theirs && let Some(identity) = record.key.as_ref().and_then(|kept| kept.identity(&signer)) {
        return Ok(Reconciled { record, identity });
    }

    let identity = IdentityMaterial::minted(signer).map_err(|error| OpenError::unsigned(&error))?;
    if !theirs {
        let others = records.others()?;
        if !others.is_empty() {
            record = DeviceRecord {
                pair_again: record.pair_again || others.iter().any(|other| held_anything(other)),
                ..DeviceRecord::named(record.name)
            };
            // Kept before anything goes, so a start ended midway still says
            // it the next time.
            records.write(&record).map_err(OpenError::unkept)?;
            for other in &others {
                discard(other).map_err(OpenError::unkept)?;
            }
        }
    }
    record.key = Some(KeptKey::of(&identity));
    records.write(&record).map_err(OpenError::unkept)?;
    Ok(Reconciled { record, identity })
}

/// Whether the record at `path` held a Remote or a Relay entry, which this
/// phone must pair again to have back. One that cannot be read is taken to
/// have.
fn held_anything(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name != shaman_pairing::REMOTES_FILE && name != RELAYS_FILE {
        return false;
    }
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<serde_json::Value>>(&bytes).ok())
        .is_none_or(|entries| !entries.is_empty())
}

fn discard(path: &Path) -> Result<()> {
    let removed = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    removed.with_context(|| format!("discard {}, kept for another key", path.display()))
}

impl Records {
    /// Everything in the directory but the thin Client's own file.
    fn others(&self) -> Result<Vec<std::path::PathBuf>> {
        let entries = std::fs::read_dir(self.directory())
            .with_context(|| format!("read the directory {}", self.directory().display()))?;
        let mut others = Vec::new();
        for entry in entries {
            let entry = entry.context("read the directory")?;
            if entry.file_name() != RECORD_FILE {
                others.push(entry.path());
            }
        }
        Ok(others)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::bail;
    use shaman_pairing::signer::testing::CallbackSigner;

    use super::*;

    fn new_key() -> IdentitySigner {
        IdentitySigner::new(CallbackSigner::elsewhere()).unwrap()
    }

    #[test]
    fn a_start_ended_while_discarding_still_says_this_phone_must_pair_again() {
        let directory = tempfile::tempdir().unwrap();
        let records = Records::open(directory.path().to_owned()).unwrap();
        reconcile(&records, new_key()).unwrap();
        let remotes = directory.path().join(shaman_pairing::REMOTES_FILE);
        std::fs::write(&remotes, br#"[{"name":"laptop"}]"#).unwrap();
        std::fs::write(directory.path().join("drafts.json"), b"{}").unwrap();

        // The app is ended once the Remote is gone, before the start is
        // done, so nothing left says a Remote was held.
        let new = new_key();
        let ended = reconcile_discarding(&records, new.clone(), |path| {
            std::fs::remove_file(path)?;
            if path.ends_with(shaman_pairing::REMOTES_FILE) {
                bail!("the app was ended");
            }
            Ok(())
        });
        assert!(
            matches!(ended, Err(OpenError::Failed { .. })),
            "ended partway"
        );
        assert!(!remotes.exists());

        let started = reconcile(&records, new).expect("the next start");
        assert!(started.record.pair_again, "still says it");
        assert_eq!(records.others().unwrap(), Vec::<std::path::PathBuf>::new());
    }
}
