//! Classification shared by every stream a reader opens over a transport.

use crate::protocol::{RemoteStatus, SessionError, SessionErrorCode, UnreachableReason};

use super::transport::{self, TransportResponse};

pub(super) enum RemoteConnectionFailure {
    /// The Remote could not be reached just now, and is tried again later;
    /// where its user can do something about it, why.
    Transient(Option<UnreachableReason>),
    Terminal {
        status: RemoteStatus,
        message: String,
    },
    Rejected(String),
    /// The Server holds no Session the request named — never did, or no
    /// longer does. A subscription that has already heard from that Session
    /// reads it as the Session having been deleted.
    Missing(String),
    /// The Server the transport was bound to has been replaced, so the
    /// reading ends without complaint, for its reader to open afresh.
    Replaced,
}

/// What came of a request a reader sent: the answer to read on, or what
/// failed, whether the transport could not reach the Server or the Server
/// refused.
pub(super) async fn classify(
    response: Result<TransportResponse, SessionError>,
) -> Result<TransportResponse, RemoteConnectionFailure> {
    let response = match response {
        Ok(response) => response,
        // Not reaching the Server is, like a Server failing, worth trying
        // again, unless the transport says why it never will be.
        Err(error) => return Err(failure_of(error, true)),
    };
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    match transport::read_json::<SessionError>(response).await {
        Some(error) => Err(failure_of(error, status.is_server_error())),
        None if status.is_server_error() => Err(RemoteConnectionFailure::Transient(None)),
        None => Err(RemoteConnectionFailure::Rejected(format!(
            "Server rejected the request with {status}"
        ))),
    }
}

/// What `error` failed a request for, `server_error` saying whether it is a
/// failure to be tried again whatever its code.
fn failure_of(error: SessionError, server_error: bool) -> RemoteConnectionFailure {
    match error.code {
        SessionErrorCode::PairingAuthenticationFailed => RemoteConnectionFailure::Terminal {
            status: RemoteStatus::Revoked,
            message: "Remote revoked this pairing".to_owned(),
        },
        SessionErrorCode::PairingProtocolMismatch => RemoteConnectionFailure::Terminal {
            status: RemoteStatus::ProtocolMismatch,
            message: error.message,
        },
        SessionErrorCode::ServerReplaced => RemoteConnectionFailure::Replaced,
        SessionErrorCode::PairingConnectionFailed | SessionErrorCode::ServerUnreachable => {
            RemoteConnectionFailure::Transient(error.unreachable)
        }
        _ if server_error => RemoteConnectionFailure::Transient(error.unreachable),
        SessionErrorCode::SessionNotFound => RemoteConnectionFailure::Missing(error.message),
        _ => RemoteConnectionFailure::Rejected(error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a Remote proxy answers with `status` and `body`.
    fn answered(status: u16, body: serde_json::Value) -> Result<TransportResponse, SessionError> {
        let mut response = http::Response::new(transport::full_body(body.to_string()));
        *response.status_mut() = http::StatusCode::from_u16(status).expect("a status");
        Ok(response)
    }

    #[tokio::test]
    async fn a_refusal_under_a_code_a_newer_server_added_is_rejected_in_its_own_words() {
        let refused = classify(answered(
            409,
            serde_json::json!({
                "code": "a_code_added_since_this_build",
                "message": "The Remote is mid-upgrade",
            }),
        ))
        .await;

        assert!(
            matches!(
                refused,
                Err(RemoteConnectionFailure::Rejected(ref message))
                    if message == "The Remote is mid-upgrade"
            ),
            "the message beside the code is what is passed on"
        );
    }

    #[tokio::test]
    async fn a_failure_to_reach_for_a_reason_a_newer_server_added_is_still_tried_again() {
        let failed = classify(answered(
            409,
            serde_json::json!({
                "code": "pairing_connection_failed",
                "message": "The Remote could not be reached",
                "unreachable": { "reason": "a_reason_added_since_this_build" },
            }),
        ))
        .await;

        assert!(
            matches!(failed, Err(RemoteConnectionFailure::Transient(None))),
            "the reason this build does not know reads as none, and the failure as one to reach"
        );
    }
}
