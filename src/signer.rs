//! What signs with an identity key, for everything that proves one: the
//! pinned-key TLS of a Pairing, on either side of it, the identity and
//! enrollment certificates that TLS presents, and the proof a Relay asks
//! for. Each takes an [`IdentitySigner`], and none of them holds the key's
//! private bytes, so the key may be kept where it cannot be read at all: a
//! phone's in its Secure Enclave, signing by a call into the app (ADR-0081).
//! A Server wraps the bytes it keeps (ADR-0050) in a [`SoftwareSigner`].

use std::{
    fmt,
    sync::{Arc, Mutex as StdMutex},
};

use anyhow::{Context, Result, anyhow, bail};
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256, PublicKeyData};
use rustls::{
    SignatureAlgorithm, SignatureScheme,
    client::ResolvesClientCert,
    pki_types::{CertificateDer, SubjectPublicKeyInfoDer},
    sign::{CertifiedKey, SingleCertAndKey},
};
use x509_parser::{
    oid_registry::{OID_EC_P256, OID_KEY_TYPE_EC_PUBLIC_KEY},
    prelude::FromDer,
    x509::SubjectPublicKeyInfo,
};

/// Signs with one identity key, wherever that key is kept: the one thing
/// asked of whatever holds it. The key is ECDSA on P-256, and each
/// signature is over SHA-256.
///
/// A signature is asked for in the middle of a TLS handshake, so `sign` is
/// called on an async worker and blocks it until it answers; it is to
/// answer in milliseconds, as a Secure Enclave does.
pub(crate) trait Signer: Send + Sync {
    /// The key, as the DER SubjectPublicKeyInfo of a P-256 public key.
    fn public_key(&self) -> Vec<u8>;

    /// The ECDSA signature by the key of the SHA-256 digest of `message`,
    /// ASN.1 DER-encoded, as TLS 1.3, X.509 and a Relay all read one.
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>>;
}

/// A [`Signer`], taken up: its key checked once to be one every use can
/// prove, and that key's bytes held for the uses to name it by. Cloning it
/// shares the one signer.
#[derive(Clone)]
pub(crate) struct IdentitySigner(Arc<Taken>);

struct Taken {
    signer: Arc<dyn Signer>,
    /// The DER SubjectPublicKeyInfo the signer gave.
    public_key: Vec<u8>,
    /// Where in `public_key` the uncompressed point itself begins.
    point_at: usize,
}

/// The length of an uncompressed P-256 point: its tag and two coordinates.
const P256_POINT_LEN: usize = 65;

impl IdentitySigner {
    /// Takes up `signer`, where its key is one a P-256 signature can be
    /// made with.
    pub(crate) fn new(signer: impl Signer + 'static) -> Result<Self> {
        let public_key = signer.public_key();
        let point_at = p256_point_at(&public_key).context(
            "the identity key is not an ECDSA key on P-256, the only kind Shaman signs with",
        )?;
        Ok(Self(Arc::new(Taken {
            signer: Arc::new(signer),
            public_key,
            point_at,
        })))
    }

    /// The key, as the DER SubjectPublicKeyInfo its Pairings pin.
    pub(crate) fn public_key(&self) -> &[u8] {
        &self.0.public_key
    }

    /// Signs `message` with the key, as [`Signer::sign`] does.
    pub(crate) fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        self.0
            .signer
            .sign(message)
            .context("sign with the identity key")
    }

    /// The DER of a certificate `params` describes, which the key signs
    /// itself, and whose subject key it is.
    pub(crate) fn self_signed(&self, params: CertificateParams) -> Result<Vec<u8>> {
        let key = CertificateKey {
            signer: self,
            refused: StdMutex::default(),
        };
        match params.self_signed(&key) {
            Ok(certificate) => Ok(certificate.der().as_ref().to_vec()),
            Err(error) => match key.refused.into_inner().ok().flatten() {
                Some(refused) => Err(refused),
                None => Err(error.into()),
            },
        }
    }

    /// What a TLS end presents to prove the key: `certificate`, the DER of
    /// a certificate of the key, which the key's signatures stand behind.
    pub(crate) fn tls_identity(&self, certificate: Vec<u8>) -> Result<Arc<CertifiedKey>> {
        let identity = CertifiedKey::new(
            vec![CertificateDer::from(certificate)],
            Arc::new(TlsKey(self.clone())),
        );
        identity
            .keys_match()
            .context("the certificate is not of the identity key")?;
        Ok(Arc::new(identity))
    }

    /// What a TLS client presents to prove the key, whatever a server asks:
    /// `certificate`, as [`Self::tls_identity`] takes it.
    pub(crate) fn tls_client_identity(
        &self,
        certificate: Vec<u8>,
    ) -> Result<Arc<dyn ResolvesClientCert>> {
        Ok(Arc::new(SingleCertAndKey::from(
            self.tls_identity(certificate)?,
        )))
    }

    /// The uncompressed point of the key, as a certificate names its key by.
    fn point(&self) -> &[u8] {
        &self.0.public_key[self.0.point_at..]
    }
}

impl fmt::Debug for IdentitySigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("IdentitySigner")
            .field(&shaman_relay_protocol::fingerprint(self.public_key()))
            .finish()
    }
}

/// Where in `key` its point begins, where `key` is the DER
/// SubjectPublicKeyInfo of an uncompressed P-256 public key and nothing
/// after it.
fn p256_point_at(key: &[u8]) -> Result<usize> {
    let (rest, info) = SubjectPublicKeyInfo::from_der(key)
        .map_err(|_| anyhow!("its public key is not a DER SubjectPublicKeyInfo"))?;
    if !rest.is_empty() {
        bail!("its public key has more after it");
    }
    let curve = info
        .algorithm
        .parameters
        .as_ref()
        .and_then(|parameters| parameters.as_oid().ok());
    if info.algorithm.algorithm != OID_KEY_TYPE_EC_PUBLIC_KEY || curve != Some(OID_EC_P256) {
        bail!("its public key is of another kind");
    }
    let point = &info.subject_public_key.data;
    if point.len() != P256_POINT_LEN || point.first() != Some(&0x04) {
        bail!("its public key is not an uncompressed point");
    }
    Ok(key.len() - point.len())
}

/// The key, as rcgen signs a certificate with it. The reason a signature
/// was refused is kept, as rcgen keeps none.
struct CertificateKey<'a> {
    signer: &'a IdentitySigner,
    refused: StdMutex<Option<anyhow::Error>>,
}

impl PublicKeyData for CertificateKey<'_> {
    fn der_bytes(&self) -> &[u8] {
        self.signer.point()
    }

    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        &PKCS_ECDSA_P256_SHA256
    }
}

impl rcgen::SigningKey for CertificateKey<'_> {
    fn sign(&self, message: &[u8]) -> std::result::Result<Vec<u8>, rcgen::Error> {
        self.signer.sign(message).map_err(|refused| {
            if let Ok(mut kept) = self.refused.lock() {
                *kept = Some(refused);
            }
            rcgen::Error::RemoteKeyError
        })
    }
}

/// The key, as rustls signs a handshake with it.
#[derive(Clone, Debug)]
struct TlsKey(IdentitySigner);

impl rustls::sign::SigningKey for TlsKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn rustls::sign::Signer>> {
        offered
            .contains(&SignatureScheme::ECDSA_NISTP256_SHA256)
            .then(|| Box::new(self.clone()) as Box<dyn rustls::sign::Signer>)
    }

    fn public_key(&self) -> Option<SubjectPublicKeyInfoDer<'_>> {
        Some(SubjectPublicKeyInfoDer::from(self.0.public_key()))
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ECDSA
    }
}

impl rustls::sign::Signer for TlsKey {
    fn sign(&self, message: &[u8]) -> std::result::Result<Vec<u8>, rustls::Error> {
        self.0
            .sign(message)
            .map_err(|refused| rustls::Error::General(format!("{refused:#}")))
    }

    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::ECDSA_NISTP256_SHA256
    }
}

/// Signs with a key whose PKCS#8 bytes are in memory: how a Server signs
/// with the key it keeps in the platform credential store or its key file.
pub(crate) struct SoftwareSigner(KeyPair);

impl SoftwareSigner {
    /// Signs with the P-256 key whose PKCS#8 DER is `key`.
    pub(crate) fn from_pkcs8(key: &[u8]) -> Result<Self> {
        let key = KeyPair::try_from(key).context("read identity key")?;
        if key.algorithm() != &PKCS_ECDSA_P256_SHA256 {
            bail!("the identity key is not an ECDSA key on P-256, the only kind Shaman signs with");
        }
        Ok(Self(key))
    }
}

impl Signer for SoftwareSigner {
    fn public_key(&self) -> Vec<u8> {
        self.0.subject_public_key_info()
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        rcgen::SigningKey::sign(&self.0, message).map_err(|refused| anyhow!("{refused}"))
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A signer no key bytes stand behind in the signer itself: it asks for
    //! each signature by a callback, as a phone's core asks its app.

    use std::{
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
            mpsc,
        },
        thread,
    };

    use anyhow::{Result, anyhow};
    use rcgen::{KeyPair, PublicKeyData};

    use super::Signer;

    /// What answers a [`CallbackSigner`]'s call for a signature.
    type Callback = Box<dyn Fn(&[u8]) -> Result<Vec<u8>> + Send + Sync>;

    /// Signs by calling back to whatever it was made with, knowing nothing
    /// of the key but its public half, and counting what it was asked.
    pub(crate) struct CallbackSigner {
        public_key: Vec<u8>,
        callback: Callback,
        signed: Arc<AtomicU64>,
    }

    impl CallbackSigner {
        /// Signs with `public_key`'s key by calling `callback`.
        pub(crate) fn new(
            public_key: Vec<u8>,
            callback: impl Fn(&[u8]) -> Result<Vec<u8>> + Send + Sync + 'static,
        ) -> Self {
            Self {
                public_key,
                callback: Box::new(callback),
                signed: Arc::default(),
            }
        }

        /// Signs with a new key held by a thread of its own, which the
        /// callback asks for each signature and waits on, as a core's
        /// signer waits on its app.
        pub(crate) fn elsewhere() -> Self {
            let key = KeyPair::generate().expect("generate a key");
            let public_key = key.subject_public_key_info();
            let (asks, asked) = mpsc::channel::<(Vec<u8>, mpsc::Sender<Vec<u8>>)>();
            thread::spawn(move || {
                for (message, answer) in asked {
                    let signature = rcgen::SigningKey::sign(&key, &message).expect("sign");
                    let _ = answer.send(signature);
                }
            });
            Self::new(public_key, move |message| {
                let (answer, answered) = mpsc::channel();
                asks.send((message.to_vec(), answer))
                    .map_err(|_| anyhow!("the key's holder is gone"))?;
                answered
                    .recv()
                    .map_err(|_| anyhow!("the key's holder did not answer"))
            })
        }

        /// What counts the signatures it is asked for.
        pub(crate) fn signed(&self) -> Arc<AtomicU64> {
            self.signed.clone()
        }
    }

    impl Signer for CallbackSigner {
        fn public_key(&self) -> Vec<u8> {
            self.public_key.clone()
        }

        fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
            self.signed.fetch_add(1, Ordering::AcqRel);
            (self.callback)(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use rcgen::{KeyPair, PKCS_ECDSA_P384_SHA384, PKCS_ED25519, PublicKeyData};
    use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
    use rustls::{
        ClientConfig, ServerConfig,
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        pki_types::{ServerName, UnixTime},
        server::danger::{ClientCertVerified, ClientCertVerifier},
    };
    use tokio_rustls::{TlsAcceptor, TlsConnector};
    use x509_parser::prelude::{FromDer, X509Certificate};

    use super::{testing::CallbackSigner, *};

    fn point_of(key: &[u8]) -> Vec<u8> {
        let (_, info) = SubjectPublicKeyInfo::from_der(key).unwrap();
        info.subject_public_key.data.to_vec()
    }

    #[test]
    fn a_software_signer_signs_as_its_key_does_and_names_the_key_as_its_pairings_pin_it() {
        let key = KeyPair::generate().unwrap();
        let signer =
            IdentitySigner::new(SoftwareSigner::from_pkcs8(&key.serialize_der()).unwrap()).unwrap();

        assert_eq!(signer.public_key(), key.subject_public_key_info());
        let signature = signer.sign(b"a message").unwrap();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, point_of(signer.public_key()))
            .verify(b"a message", &signature)
            .expect("the signature is the key's, over SHA-256, in ASN.1 DER");
    }

    #[test]
    fn no_key_but_one_on_p256_is_signed_with() {
        for algorithm in [&PKCS_ECDSA_P384_SHA384, &PKCS_ED25519] {
            let key = KeyPair::generate_for(algorithm).unwrap();
            let refused = SoftwareSigner::from_pkcs8(&key.serialize_der())
                .err()
                .expect("a software signer takes no such key");
            assert!(refused.to_string().contains("P-256"), "{refused:#}");
            let refused =
                IdentitySigner::new(CallbackSigner::new(key.subject_public_key_info(), |_| {
                    unreachable!("nothing is signed")
                }))
                .expect_err("nor does any other");
            assert!(refused.to_string().contains("P-256"), "{refused:#}");
        }
        let mut trailing = KeyPair::generate().unwrap().subject_public_key_info();
        trailing.push(0);
        for public_key in [trailing, b"not a key".to_vec(), Vec::new()] {
            assert!(
                IdentitySigner::new(CallbackSigner::new(public_key, |_| unreachable!())).is_err()
            );
        }
    }

    #[test]
    fn a_certificate_minted_through_a_callback_is_of_its_key_and_signed_by_it() {
        let callback = CallbackSigner::elsewhere();
        let signed = callback.signed();
        let signer = IdentitySigner::new(callback).unwrap();

        let certificate = signer
            .self_signed(CertificateParams::new(vec!["shaman-server".to_owned()]).unwrap())
            .unwrap();

        let (_, parsed) = X509Certificate::from_der(&certificate).unwrap();
        assert_eq!(parsed.public_key().raw, signer.public_key());
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, point_of(signer.public_key()))
            .verify(
                parsed.tbs_certificate.as_ref(),
                &parsed.signature_value.data,
            )
            .expect("the certificate is signed by its own key");
        assert_eq!(signed.load(Ordering::Acquire), 1, "the callback signed it");
    }

    #[test]
    fn a_signer_that_refuses_mints_nothing_and_says_why() {
        let key = KeyPair::generate().unwrap();
        let signer =
            IdentitySigner::new(CallbackSigner::new(key.subject_public_key_info(), |_| {
                Err(anyhow!("the chip will not take the key"))
            }))
            .unwrap();

        let refused = signer
            .self_signed(CertificateParams::new(Vec::new()).unwrap())
            .unwrap_err();
        assert!(
            format!("{refused:#}").contains("the chip will not take the key"),
            "{refused:#}"
        );
    }

    #[test]
    fn a_relay_proof_made_through_a_callback_proves_its_key() {
        let signer = IdentitySigner::new(CallbackSigner::elsewhere()).unwrap();
        let (relay, nonce) = ("https://relay.example.com", [7; 32]);

        let signature = signer
            .sign(&shaman_relay_protocol::proof_message(
                relay,
                &nonce,
                signer.public_key(),
            ))
            .unwrap();

        assert_eq!(
            shaman_relay_protocol::verify_proof(relay, signer.public_key(), &nonce, &signature),
            Ok(())
        );
    }

    #[test]
    fn a_certificate_not_of_the_key_is_presented_by_no_tls_identity() {
        let signer = IdentitySigner::new(CallbackSigner::elsewhere()).unwrap();
        let other = IdentitySigner::new(CallbackSigner::elsewhere()).unwrap();
        let certificate = other
            .self_signed(CertificateParams::new(Vec::new()).unwrap())
            .unwrap();

        assert!(signer.tls_identity(certificate).is_err());
    }

    /// Takes whatever key is presented, and records it: a TLS end that
    /// trusts by pin, without the pin.
    #[derive(Debug)]
    struct AnyKey(Arc<rustls::crypto::CryptoProvider>);

    impl AnyKey {
        fn verify(
            &self,
            message: &[u8],
            certificate: &CertificateDer<'_>,
            signature: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(
                message,
                certificate,
                signature,
                &self.0.signature_verification_algorithms,
            )
        }
    }

    impl ServerCertVerifier for AnyKey {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> std::result::Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _certificate: &CertificateDer<'_>,
            _signature: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            Err(rustls::Error::General("TLS 1.3 alone".to_owned()))
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            certificate: &CertificateDer<'_>,
            signature: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            self.verify(message, certificate, signature)
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    impl ClientCertVerifier for AnyKey {
        fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
            &[]
        }

        fn verify_client_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _now: UnixTime,
        ) -> std::result::Result<ClientCertVerified, rustls::Error> {
            Ok(ClientCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _certificate: &CertificateDer<'_>,
            _signature: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            Err(rustls::Error::General("TLS 1.3 alone".to_owned()))
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            certificate: &CertificateDer<'_>,
            signature: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            self.verify(message, certificate, signature)
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    /// Both ends of a TLS 1.3 handshake prove a key held only behind a
    /// callback, each signature asked of a thread that holds the key, and
    /// each end sees the other's certificate.
    #[tokio::test]
    async fn a_tls_handshake_completes_with_keys_held_behind_callbacks_on_both_ends() {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let (client_signs, server_signs) =
            (CallbackSigner::elsewhere(), CallbackSigner::elsewhere());
        let (client_signed, server_signed) = (client_signs.signed(), server_signs.signed());
        let (client, server) = (
            IdentitySigner::new(client_signs).unwrap(),
            IdentitySigner::new(server_signs).unwrap(),
        );
        let mint = |signer: &IdentitySigner| {
            signer
                .self_signed(CertificateParams::new(vec!["shaman-server".to_owned()]).unwrap())
                .unwrap()
        };
        let (client_certificate, server_certificate) = (mint(&client), mint(&server));

        let accepting = ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_client_cert_verifier(Arc::new(AnyKey(provider.clone())))
            .with_cert_resolver(Arc::new(SingleCertAndKey::from(
                server.tls_identity(server_certificate.clone()).unwrap(),
            )));
        let dialling = ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyKey(provider.clone())))
            .with_client_cert_resolver(
                client
                    .tls_client_identity(client_certificate.clone())
                    .unwrap(),
            );

        let (near, far) = tokio::io::duplex(64 * 1024);
        let (accepted, dialled) = tokio::join!(
            TlsAcceptor::from(Arc::new(accepting)).accept(far),
            TlsConnector::from(Arc::new(dialling))
                .connect(ServerName::try_from("shaman-server").unwrap(), near),
        );
        let (accepted, dialled) = (accepted.unwrap(), dialled.unwrap());

        assert_eq!(
            accepted.get_ref().1.peer_certificates().unwrap()[0].as_ref(),
            client_certificate.as_slice()
        );
        assert_eq!(
            dialled.get_ref().1.peer_certificates().unwrap()[0].as_ref(),
            server_certificate.as_slice()
        );
        assert_eq!(
            client_signed.load(Ordering::Acquire),
            2,
            "a certificate and a handshake"
        );
        assert_eq!(
            server_signed.load(Ordering::Acquire),
            2,
            "a certificate and a handshake"
        );
    }
}
