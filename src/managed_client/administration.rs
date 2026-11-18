//! A Client's administration of its own Server: its Settings and Onboarding,
//! the Pairings it serves and redeems, and the Relays it keeps. None of it is
//! anything a Peer may ask, so it is never turned toward an Outlook: it is
//! sent to the local Server the descriptor names, with that Server's token.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::watch;

use crate::protocol::{
    AddRelayRequest, InvitePreview, IssueInviteRequest, IssuedInvite, Peer, PreviewInviteRequest,
    RedeemInviteRequest, Relay, RelayListing, RelayLogin, RelayMachines, RelayRemoval,
    RelayServeThroughRequest, Remote, RemoteHealth, RemoteRemoval, RuntimeDescriptor,
    SetRemoteIconRequest, SettingMutation, SettingsSnapshot,
};

use shaman_client::{
    TransportResponse, decode_api_error, decode_api_response, decode_empty_api_response,
};

use super::loopback::response_of;

/// The administration of the local Server the descriptor names.
#[derive(Clone)]
pub(crate) struct Administration {
    http: reqwest::Client,
    descriptor: watch::Receiver<RuntimeDescriptor>,
}

impl Administration {
    pub(crate) fn new(
        http: reqwest::Client,
        descriptor: watch::Receiver<RuntimeDescriptor>,
    ) -> Self {
        Self { http, descriptor }
    }

    pub(crate) async fn mutate_setting(
        &self,
        mutation: SettingMutation,
    ) -> Result<SettingsSnapshot> {
        self.post(&["settings"], &mutation, "Setting mutation command")
            .await
    }

    pub(crate) async fn mark_onboarded(&self) -> Result<()> {
        let response = self
            .send(reqwest::Method::PUT, &["onboarded"], None::<&()>)
            .await
            .context("send the mark that this Server is onboarded")?;
        decode_empty_api_response(response, "Onboarding mark").await
    }

    pub(crate) async fn issue_invite(&self, request: IssueInviteRequest) -> Result<IssuedInvite> {
        self.post(&["pairing", "invites"], &request, "Invite issuance command")
            .await
    }

    pub(crate) async fn prepare_serving(&self) -> Result<crate::protocol::ServingPreparation> {
        self.post_without_body(&["pairing", "serving"], "Serving preparation command")
            .await
    }

    pub(crate) async fn serving_standing(&self) -> Result<crate::protocol::ServingStanding> {
        self.get(&["pairing", "serving"], "Serving standing").await
    }

    pub(crate) async fn offer_invite(
        &self,
        request: crate::protocol::OfferInviteRequest,
    ) -> Result<crate::protocol::InviteOfferAnswer> {
        self.post(
            &["pairing", "invites", "offered"],
            &request,
            "Invite issuance command",
        )
        .await
    }

    pub(crate) async fn new_short_code(&self) -> Result<crate::protocol::IssuedShortCode> {
        self.post_without_body(
            &["pairing", "invites", "short-code"],
            "New short code command",
        )
        .await
    }

    pub(crate) async fn fetch_short_invite(
        &self,
        request: crate::protocol::FetchShortInviteRequest,
    ) -> Result<crate::protocol::DeliveredInvite> {
        self.post(
            &["pairing", "invites", "delivery"],
            &request,
            "Short Invite delivery command",
        )
        .await
    }

    pub(crate) async fn preview_invite(
        &self,
        invite: crate::protocol::InviteText,
    ) -> Result<InvitePreview> {
        self.post(
            &["pairing", "invites", "preview"],
            &PreviewInviteRequest { invite },
            "Invite preview command",
        )
        .await
    }

    pub(crate) async fn redeem_invite(&self, request: RedeemInviteRequest) -> Result<Remote> {
        self.post(
            &["pairing", "remotes"],
            &request,
            "Invite redemption command",
        )
        .await
    }

    pub(crate) async fn list_peers(&self) -> Result<Vec<Peer>> {
        self.get(&["pairing", "peers"], "Peer listing").await
    }

    pub(crate) async fn list_remotes(&self) -> Result<Vec<Remote>> {
        self.get(&["pairing", "remotes"], "Remote listing").await
    }

    pub(crate) async fn probe_remote(&self, name: &str) -> Result<RemoteHealth> {
        self.post_without_body(
            &["pairing", "remotes", name, "health"],
            "Remote probe command",
        )
        .await
    }

    pub(crate) async fn remove_peer(&self, peer_id: &str) -> Result<()> {
        let response = self
            .send(
                reqwest::Method::DELETE,
                &["pairing", "peers", peer_id],
                None::<&()>,
            )
            .await
            .context("send Peer removal")?;
        decode_empty_api_response(response, "Peer removal").await
    }

    pub(crate) async fn remove_remote(&self, name: &str) -> Result<RemoteRemoval> {
        let response = self
            .send(
                reqwest::Method::DELETE,
                &["pairing", "remotes", name],
                None::<&()>,
            )
            .await
            .context("send Remote removal")?;
        decode_api_response(response, "Remote removal").await
    }

    /// Sets the Icon the user chose for the Remote `name` on the Client's own
    /// Server, which keeps it beside the Remote's name.
    pub(crate) async fn set_remote_icon(&self, name: &str, icon: &str) -> Result<()> {
        self.post_answering_nothing(
            &["pairing", "remotes", name, "icon"],
            &SetRemoteIconRequest {
                icon: icon.to_owned(),
            },
            "Remote Icon",
        )
        .await
    }

    pub(crate) async fn list_offers(&self) -> Result<crate::protocol::PairBackOffers> {
        self.get(&["pairing", "offers"], "Offer listing").await
    }

    pub(crate) async fn answer_offer(
        &self,
        id: uuid::Uuid,
        answer: crate::protocol::AnswerOfferRequest,
    ) -> Result<()> {
        self.post_answering_nothing(
            &["pairing", "offers", &id.to_string(), "answer"],
            &answer,
            "Offer answer",
        )
        .await
    }

    pub(crate) async fn withdraw_offer(&self, id: uuid::Uuid) -> Result<()> {
        let response = self
            .send(
                reqwest::Method::DELETE,
                &["pairing", "offers", &id.to_string()],
                None::<&()>,
            )
            .await
            .context("send Offer withdrawal")?;
        decode_empty_api_response(response, "Offer withdrawal").await
    }

    pub(crate) async fn claim_offer_notice(
        &self,
        id: uuid::Uuid,
        claimant: uuid::Uuid,
    ) -> Result<bool> {
        let claim: crate::protocol::OfferNoticeClaimed = self
            .post(
                &["pairing", "offers", &id.to_string(), "notice"],
                &crate::protocol::OfferNoticeClaim { claimant },
                "Offer Notice claim",
            )
            .await?;
        Ok(claim.claimed)
    }

    pub(crate) async fn list_relays(&self) -> Result<RelayListing> {
        self.get(&["relays"], "Relay listing").await
    }

    pub(crate) async fn claim_relay_login_needed_notice(
        &self,
        address: &str,
        lapse: uuid::Uuid,
        claimant: uuid::Uuid,
    ) -> Result<bool> {
        let claim: crate::protocol::RelayLoginNeededNoticeClaim = self
            .post(
                &["relays", address, "login-needed-notice"],
                &crate::protocol::RelayLoginNeededNotice { lapse, claimant },
                "Relay login-needed Notice claim",
            )
            .await?;
        Ok(claim.claimed)
    }

    pub(crate) async fn add_relay(&self, address: String) -> Result<Relay> {
        self.post(&["relays"], &AddRelayRequest { address }, "Relay addition")
            .await
    }

    pub(crate) async fn list_relay_machines(&self, address: &str) -> Result<RelayMachines> {
        self.get(&["relays", address, "machines"], "Relay machine listing")
            .await
    }

    pub(crate) async fn log_in_at_relay(
        &self,
        address: &str,
        afresh: bool,
    ) -> Result<crate::protocol::RelayLoginLead> {
        let response = self
            .send(
                reqwest::Method::PUT,
                &["relays", address, "login"],
                Some(&crate::protocol::RelayLoginRequest { afresh }),
            )
            .await
            .context("ask for a Relay login")?;
        decode_api_response(response, "Relay login").await
    }

    pub(crate) async fn follow_relay_login(&self, address: &str) -> Result<RelayLogin> {
        use eventsource_stream::Eventsource;
        use futures_util::StreamExt;
        use http_body_util::BodyExt;

        let response = self
            .send(
                reqwest::Method::GET,
                &["relays", address, "login"],
                None::<&()>,
            )
            .await
            .context("follow Relay login")?;
        if !response.status().is_success() {
            return Err(decode_api_error(response, "Relay login").await);
        }
        let mut events = response.into_body().into_data_stream().eventsource();
        while let Some(event) = events.next().await {
            let event = event
                .map_err(|error| anyhow!("{error}"))
                .context("read Relay login progress")?;
            if event.event != crate::protocol::RELAY_LOGIN_EVENT {
                continue;
            }
            let login = serde_json::from_str::<RelayLogin>(&event.data)
                .context("decode Relay login progress")?;
            if login.outcome.is_settled() {
                return Ok(login);
            }
        }
        bail!("the Relay login was given up before it ended")
    }

    pub(crate) async fn set_relay_serve_through(
        &self,
        address: &str,
        serve_through: bool,
    ) -> Result<Relay> {
        let response = self
            .send(
                reqwest::Method::PUT,
                &["relays", address, "serve-through"],
                Some(&RelayServeThroughRequest { serve_through }),
            )
            .await
            .context("send Relay Serve-through choice")?;
        decode_api_response(response, "Relay Serve-through choice").await
    }

    pub(crate) async fn remove_relay(&self, address: &str) -> Result<RelayRemoval> {
        let response = self
            .send(reqwest::Method::DELETE, &["relays", address], None::<&()>)
            .await
            .context("send Relay removal")?;
        decode_api_response(response, "Relay removal").await
    }

    /// Sends `method` to the local Server's route beneath `/v1` that
    /// `segments` name, each encoded whole — a Remote's freely editable name
    /// or a Relay's address, slashes and all — carrying `body` as JSON where
    /// there is one.
    async fn send(
        &self,
        method: reqwest::Method,
        segments: &[&str],
        body: Option<&(impl Serialize + ?Sized)>,
    ) -> Result<TransportResponse> {
        let descriptor = self.descriptor.borrow().clone();
        let url = local_url(&descriptor.base_url, segments)?;
        let request = self
            .http
            .request(method, url)
            .bearer_auth(&descriptor.token);
        let request = match body {
            Some(body) => request.json(body),
            None => request,
        };
        Ok(response_of(request.send().await?))
    }

    async fn get<T: DeserializeOwned>(&self, segments: &[&str], operation: &str) -> Result<T> {
        let response = self
            .send(reqwest::Method::GET, segments, None::<&()>)
            .await
            .with_context(|| format!("send {operation}"))?;
        decode_api_response(response, operation).await
    }

    /// Posts `body`; `operation` names what is sent, and what is answered
    /// where it does not end in " command".
    async fn post<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &(impl Serialize + ?Sized),
        operation: &str,
    ) -> Result<T> {
        let response = self
            .send(reqwest::Method::POST, segments, Some(body))
            .await
            .with_context(|| format!("send {operation}"))?;
        decode_api_response(response, answered(operation)).await
    }

    async fn post_answering_nothing(
        &self,
        segments: &[&str],
        body: &(impl Serialize + ?Sized),
        operation: &str,
    ) -> Result<()> {
        let response = self
            .send(reqwest::Method::POST, segments, Some(body))
            .await
            .with_context(|| format!("send {operation}"))?;
        decode_empty_api_response(response, operation).await
    }

    async fn post_without_body<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        operation: &str,
    ) -> Result<T> {
        let response = self
            .send(reqwest::Method::POST, segments, None::<&()>)
            .await
            .with_context(|| format!("send {operation}"))?;
        decode_api_response(response, answered(operation)).await
    }
}

/// What `operation` answers: the operation sent, short of " command".
fn answered(operation: &str) -> &str {
    operation.strip_suffix(" command").unwrap_or(operation)
}

/// The local Server's route beneath `/v1` for `segments`, each one encoded
/// whole.
fn local_url(base_url: &str, segments: &[&str]) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(base_url).context("parse server base URL")?;
    url.path_segments_mut()
        .map_err(|()| anyhow!("server base URL cannot contain path segments"))?
        .clear()
        .push("v1")
        .extend(segments);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{Router, extract::Request, routing::any};
    use tokio::sync::watch;

    use super::{Administration, local_url};
    use crate::protocol::{
        AnswerOfferRequest, FetchShortInviteRequest, InviteRelays, InviteText, IssueInviteRequest,
        OfferInviteRequest, PROTOCOL_VERSION, RedeemInviteRequest, RuntimeDescriptor,
        ServerIdentity, SettingMutation, ShortCode,
    };

    /// Everything a Peer may not ask is administration: each administration
    /// command's route is one the Serving side's route check refuses a Peer,
    /// so none of it belongs among the shared commands.
    #[tokio::test]
    async fn no_administration_command_asks_what_a_peer_may_ask() {
        let asked = Arc::new(Mutex::new(Vec::<(http::Method, http::Uri)>::new()));
        let recorded = asked.clone();
        let app = Router::new().fallback(any(move |request: Request| {
            recorded
                .lock()
                .expect("the record lock")
                .push((request.method().clone(), request.uri().clone()));
            async { http::StatusCode::NO_CONTENT }
        }));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the local Server fixture");
        let address = listener.local_addr().expect("read the fixture address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve the local Server fixture");
        });
        let (_descriptor, descriptor) = watch::channel(RuntimeDescriptor::new(
            format!("http://{address}"),
            "administration-fixture-token".to_owned(),
            ServerIdentity {
                instance_id: uuid::Uuid::new_v4(),
                pid: std::process::id(),
                protocol_version: PROTOCOL_VERSION,
                build_identity: "administration-fixture".to_owned(),
            },
        ));
        let administration =
            Administration::new(crate::runtime::loopback_http_client(), descriptor);
        let id = uuid::Uuid::new_v4();
        let relay = "https://relay.example";
        let invite = || InviteText("invite".to_owned());
        let judge = |name: &str| {
            let asked = std::mem::take(&mut *asked.lock().expect("the record lock"));
            assert!(!asked.is_empty(), "{name} asks nothing");
            for (method, uri) in asked {
                assert!(
                    !crate::serving::a_peer_may_ask(&method, &uri),
                    "{name} asks {method} {uri}, which a Peer may: it is a shared command"
                );
            }
        };

        macro_rules! walk {
            ($($name:literal => $asked:expr),* $(,)?) => {
                $(
                    let _ = $asked;
                    judge($name);
                )*
            };
        }
        walk![
            "mutate_setting" => administration.mutate_setting(
                SettingMutation::ComposerCollapsePastes { value: None },
            ).await,
            "mark_onboarded" => administration.mark_onboarded().await,
            "issue_invite" => {
                administration.issue_invite(IssueInviteRequest { ways: Vec::new() }).await
            },
            "prepare_serving" => administration.prepare_serving().await,
            "serving_standing" => administration.serving_standing().await,
            "offer_invite" => administration.offer_invite(OfferInviteRequest {
                addresses: true,
                relays: InviteRelays::Every,
            }).await,
            "new_short_code" => administration.new_short_code().await,
            "fetch_short_invite" => administration.fetch_short_invite(FetchShortInviteRequest {
                server_key: "server-key".to_owned(),
                relays: vec![relay.to_owned()],
                code: ShortCode::from_random(&[0; 8]),
            }).await,
            "preview_invite" => administration.preview_invite(invite()).await,
            "redeem_invite" => administration.redeem_invite(RedeemInviteRequest {
                invite: invite(),
                name: None,
                ways: Vec::new(),
                pair_back: false,
            }).await,
            "list_peers" => administration.list_peers().await,
            "list_remotes" => administration.list_remotes().await,
            "probe_remote" => administration.probe_remote("lab/east").await,
            "remove_peer" => administration.remove_peer("peer").await,
            "remove_remote" => administration.remove_remote("lab/east").await,
            "set_remote_icon" => administration.set_remote_icon("lab/east", "star").await,
            "list_offers" => administration.list_offers().await,
            "answer_offer" => administration.answer_offer(id, AnswerOfferRequest::Decline).await,
            "withdraw_offer" => administration.withdraw_offer(id).await,
            "claim_offer_notice" => administration.claim_offer_notice(id, id).await,
            "list_relays" => administration.list_relays().await,
            "claim_relay_login_needed_notice" => {
                administration.claim_relay_login_needed_notice(relay, id, id).await
            },
            "add_relay" => administration.add_relay(relay.to_owned()).await,
            "list_relay_machines" => administration.list_relay_machines(relay).await,
            "log_in_at_relay" => administration.log_in_at_relay(relay, false).await,
            "follow_relay_login" => administration.follow_relay_login(relay).await,
            "set_relay_serve_through" => administration.set_relay_serve_through(relay, true).await,
            "remove_relay" => administration.remove_relay(relay).await,
        ];
        server.abort();
    }

    #[test]
    fn a_remotes_freely_editable_name_is_one_path_segment() {
        for (segments, expected) in [
            (
                &["pairing", "remotes", "lab/east?#", "health"][..],
                "http://127.0.0.1:7777/v1/pairing/remotes/lab%2Feast%3F%23/health",
            ),
            (
                &["pairing", "remotes", "lab/east?#"][..],
                "http://127.0.0.1:7777/v1/pairing/remotes/lab%2Feast%3F%23",
            ),
            (
                &["relays", "https://relay.example/a", "login"][..],
                "http://127.0.0.1:7777/v1/relays/https:%2F%2Frelay.example%2Fa/login",
            ),
        ] {
            let url = local_url("http://127.0.0.1:7777", segments).expect("build the route");
            assert_eq!(url.as_str(), expected);
        }
    }
}
