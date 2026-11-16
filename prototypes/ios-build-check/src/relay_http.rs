//! How a Relay is reached, after `relay_http_client` and `websocket` in `src/relays.rs`: an HTTP client that takes
//! the system's proxy and trusts what the operating system's trust store trusts, and a WebSocket opened by
//! upgrading a request it sends.

use std::sync::Arc;

use reqwest::{StatusCode, header};
use shaman_relay_protocol::{ENDPOINT_PATH, MAX_MESSAGE_LEN};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        self,
        protocol::{Role, WebSocketConfig},
    },
};

use crate::Failure;

/// The client, and whether the operating system's trust store could be read for it.
pub fn relay_http_client(proxied: bool) -> (reqwest::Client, Result<(), String>) {
    use rustls_platform_verifier::BuilderVerifierExt as _;

    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring speaks every TLS version rustls deems safe");
    let (mut tls, trust) = match builder.clone().with_platform_verifier() {
        Ok(verified) => (verified.with_no_client_auth(), Ok(())),
        Err(error) => (
            builder
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth(),
            Err(error.to_string()),
        ),
    };
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .tcp_keepalive(std::time::Duration::from_secs(15))
        .tcp_keepalive_interval(std::time::Duration::from_secs(15))
        .tcp_keepalive_retries(3);
    let client = if proxied { client } else { client.no_proxy() }
        .build()
        .expect("a Relay HTTP client over rustls needs nothing that can fail");
    (client, trust)
}

pub async fn websocket(
    client: &reqwest::Client,
    address: &str,
) -> Result<WebSocketStream<reqwest::Upgraded>, Failure> {
    let key = tungstenite::handshake::client::generate_key();
    let response = client
        .get(format!("{address}{ENDPOINT_PATH}"))
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_VERSION, "13")
        .header(header::SEC_WEBSOCKET_KEY, &key)
        .send()
        .await?;
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        return Err(format!("the address answered HTTP {}", response.status()).into());
    }
    let accepted = response
        .headers()
        .get(header::SEC_WEBSOCKET_ACCEPT)
        .is_some_and(|accept| {
            accept.as_bytes()
                == tungstenite::handshake::derive_accept_key(key.as_bytes()).as_bytes()
        });
    if !accepted {
        return Err("the address did not open a WebSocket".into());
    }
    let upgraded = response.upgrade().await?;
    let limits = WebSocketConfig::default()
        .write_buffer_size(0)
        .max_write_buffer_size(2 * MAX_MESSAGE_LEN)
        .max_message_size(Some(MAX_MESSAGE_LEN))
        .max_frame_size(Some(MAX_MESSAGE_LEN));
    Ok(WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(limits)).await)
}

/// `relay <address>`
pub async fn dial(arguments: &[String]) -> Result<(), Failure> {
    let [address] = arguments else {
        return Err("relay needs an address".into());
    };
    let (client, trust) = relay_http_client(true);
    println!("trust store: {trust:?}");
    let mut socket = websocket(&client, address).await?;
    let (text, bytes) = crate::client::converse(&mut socket, "{}".to_owned(), vec![0; 16]).await?;
    println!("websocket: {text} and {} bytes", bytes.len());
    Ok(())
}
