//! Drives the spike's core from a Mac's command line with a key kept in a file, so the core can be tried against
//! a Server without Xcode or a phone. The app drives the same calls with a Secure Enclave key.

use std::{path::PathBuf, sync::Arc, time::Duration};

use rcgen::{KeyPair, PublicKeyData as _};
use shaman_phone_spike::{
    Core, LogSink, LoginListener, PairingRecord, Signer, SignerError, StreamEvent, StreamListener,
};

type Failure = Box<dyn std::error::Error + Send + Sync>;

struct FileKey(KeyPair);

impl Signer for FileKey {
    fn public_key(&self) -> Vec<u8> {
        self.0.der_bytes().to_vec()
    }

    fn sign(&self, message: Vec<u8>) -> Result<Vec<u8>, SignerError> {
        rcgen::SigningKey::sign(&self.0, &message).map_err(|error| SignerError::Failed {
            message: error.to_string(),
        })
    }
}

struct Stderr;

impl LogSink for Stderr {
    fn line(&self, text: String) {
        eprintln!("{text}");
    }
}

impl StreamListener for Stderr {
    fn event(&self, event: StreamEvent) {
        eprintln!(
            "  event {} ({} bytes) {}",
            event.name, event.bytes, event.summary
        );
    }

    fn ended(&self, reason: String) {
        eprintln!("  ended: {reason}");
    }
}

impl LoginListener for Stderr {
    fn started(&self, verification_uri: String, user_code: String, expires_in_seconds: u64) {
        eprintln!("  enter {user_code} at {verification_uri} within {expires_in_seconds}s");
    }
}

fn main() -> std::process::ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let waiting = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime starts");
    match waiting.block_on(run(&arguments)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("failed: {failure}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(arguments: &[String]) -> Result<(), Failure> {
    let [state, command, rest @ ..] = arguments else {
        return Err("usage: spike-cli <state directory> <command> [arguments]".into());
    };
    let state = PathBuf::from(state);
    std::fs::create_dir_all(&state)?;
    let key_file = state.join("key.der");
    let key = match std::fs::read(&key_file) {
        Ok(der) => KeyPair::try_from(der.as_slice())?,
        Err(_) => {
            let key = KeyPair::generate()?;
            std::fs::write(&key_file, key.serialize_der())?;
            key
        }
    };
    let core = Core::new(Arc::new(FileKey(key)), Arc::new(Stderr))?;
    let record_file = state.join("pairing.json");
    let record = || -> Result<PairingRecord, Failure> {
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&record_file)?)?;
        Ok(PairingRecord {
            hostname: saved["hostname"].as_str().unwrap_or_default().to_owned(),
            server_key: serde_json::from_value(saved["server_key"].clone())?,
            fingerprint: saved["fingerprint"].as_str().unwrap_or_default().to_owned(),
            ways: serde_json::from_value(saved["ways"].clone())?,
            protocol_version: serde_json::from_value(saved["protocol_version"].clone())?,
        })
    };
    let way = |record: &PairingRecord, index: &str| -> Result<String, Failure> {
        Ok(record
            .ways
            .get(index.parse::<usize>()?)
            .ok_or("the Pairing has no such way")?
            .clone())
    };
    match (command.as_str(), rest) {
        ("hostname", []) => println!("{}", core.hostname()),
        ("redeem", [invite, name, only]) => {
            let paired = core
                .redeem(invite.clone(), name.clone(), only.clone())
                .await?;
            std::fs::write(
                &record_file,
                serde_json::to_vec_pretty(&serde_json::json!({
                    "hostname": paired.hostname,
                    "server_key": paired.server_key,
                    "fingerprint": paired.fingerprint,
                    "ways": paired.ways,
                    "protocol_version": paired.protocol_version,
                }))?,
            )?;
            println!("paired with {} by {:?}", paired.hostname, paired.ways);
        }
        ("sessions", [index]) => {
            let record = record()?;
            let connection = core.connect(record.clone(), way(&record, index)?).await?;
            for row in connection.list_sessions().await? {
                println!("{} {}", row.id, row.title);
            }
        }
        ("follow", [index, path, seconds]) => {
            let record = record()?;
            let connection = core.connect(record.clone(), way(&record, index)?).await?;
            let handle = connection.follow(path.clone(), Arc::new(Stderr));
            tokio::time::sleep(Duration::from_secs(seconds.parse()?)).await;
            handle.cancel();
        }
        ("relay-standing", [relay]) => println!("{:?}", core.relay_standing(relay.clone()).await?),
        ("relay-login", [relay, name]) => {
            let account = core
                .relay_login(relay.clone(), name.clone(), Arc::new(Stderr))
                .await?;
            println!("logged in as {account}");
        }
        ("relay-forget", [relay]) => core.relay_forget(relay.clone()).await?,
        ("dial", [address]) => println!("{}", core.plain_dial(address.clone()).await),
        _ => return Err("no such command".into()),
    }
    Ok(())
}
