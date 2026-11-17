//! Classification shared by every stream crossing a Remote proxy.

use crate::protocol::{RemoteStatus, SessionError, SessionErrorCode, UnreachableReason};

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
}

pub(super) async fn classify(
    response: reqwest::Result<reqwest::Response>,
) -> Result<reqwest::Response, RemoteConnectionFailure> {
    let response = response.map_err(|_| RemoteConnectionFailure::Transient(None))?;
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let error = response.json::<SessionError>().await.ok();
    match error {
        Some(error) if error.code == SessionErrorCode::PairingAuthenticationFailed => {
            Err(RemoteConnectionFailure::Terminal {
                status: RemoteStatus::Revoked,
                message: "Remote revoked this pairing".to_owned(),
            })
        }
        Some(error) if error.code == SessionErrorCode::PairingProtocolMismatch => {
            Err(RemoteConnectionFailure::Terminal {
                status: RemoteStatus::ProtocolMismatch,
                message: error.message,
            })
        }
        Some(error)
            if error.code == SessionErrorCode::PairingConnectionFailed
                || status.is_server_error() =>
        {
            Err(RemoteConnectionFailure::Transient(error.unreachable))
        }
        None if status.is_server_error() => Err(RemoteConnectionFailure::Transient(None)),
        Some(error) if error.code == SessionErrorCode::SessionNotFound => {
            Err(RemoteConnectionFailure::Missing(error.message))
        }
        Some(error) => Err(RemoteConnectionFailure::Rejected(error.message)),
        None => Err(RemoteConnectionFailure::Rejected(format!(
            "Remote proxy rejected the request with {status}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a Remote proxy answers with `status` and `body`.
    fn answered(status: u16, body: serde_json::Value) -> reqwest::Result<reqwest::Response> {
        let mut response = axum::http::Response::new(body.to_string());
        *response.status_mut() = axum::http::StatusCode::from_u16(status).expect("a status");
        Ok(reqwest::Response::from(response))
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
