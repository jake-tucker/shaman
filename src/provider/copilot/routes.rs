//! Copilot Fusion's Routes: what the projection reads of one, and what it must remember to show a
//! Route's work once (ADR 0062, docs/validation/copilot-fusion-routes.md).
//!
//! On a Routing Tier a Turn holds one or more Routes, each named by its `fusionId`. A phase's
//! Messages and Tools arrive staged, as ephemeral events attributed to the phase, and show as they
//! arrive, because a Route can run for minutes. Its commit then names the phase that won and
//! re-sends every staged item of that phase, once, in order, under the `messageId` or `toolCallId`
//! it had. So the first copy of an item stands and any later one adds nothing: a staged row is never
//! reopened, reset or doubled by its committed twin, whichever reaches Suru first, and a Tool still
//! running when its twin completes settles on that.
//!
//! Nothing retracts a losing phase. The commit's `sourcePhaseId` is the only word on it, and it marks
//! the Messages of the Route's other phases Superseded. Its Tools and Reasoning stay as they are,
//! since that work happened. A Route that ends without a commit marks nothing: an interrupt, a
//! failure or a crash leaves a row saying Copilot discarded its conversation, since a staged phase
//! is in no history the Model sees again, however much of it the reader watched.
//!
//! `turnId` restarts at each phase, so nothing here keys on it, and `session.fusion_completed` closes
//! a Route rather than a Turn: one Turn can hold two, when a steer waits out an accepting judge. A
//! failed Route's fallback runs live, with deltas, no commit and no completion, so its Turn simply
//! ends at idle.

use std::collections::{HashMap, HashSet};

use github_copilot_sdk::{
    SessionEvent,
    session_events::{
        AssistantFusionPhaseCompletedData, AssistantFusionPhaseFailedData,
        AssistantFusionPhaseStartedData, FusionAttribution, FusionPhaseKind, FusionPhaseStatus,
        SessionEventType, SessionFusionCompletedData, SessionFusionResolvedData,
        SessionFusionRouteFailedData,
    },
};
use serde::Deserialize;

use super::projection::reported;
use crate::{
    protocol::{ModelId, RouteAction, RouteDecision, RouteStep, RouteVerdict},
    provider::{ProviderEvent, ProviderMessageId},
};

/// Which Provider a discarded conversation is told of as discarded by.
const DISCARDING_PROVIDER: &str = "Copilot";

/// Copilot's name for a Route handing its work to another Model, which this SDK types no data for.
const HANDOFF: &str = "session.fusion_handoff";

/// Copilot's name for a Route's commit beginning, which this SDK types no data for.
const COMMIT_STARTED: &str = "session.fusion_commit_started";

/// `session.fusion_handoff`, as captured on CLI 1.0.105.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Handoff {
    fusion_id: String,
    target_model: String,
}

/// `session.fusion_commit_started`, as captured on CLI 1.0.105.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitStarted {
    fusion_id: String,
    source_phase_id: String,
}

/// What a review phase's `content` holds: a judge's `{score, rationale}`, or a critic's
/// `{assessment, feedback, defect}`, whose `verdict` Copilot leaves null. Observed rather than
/// contracted, so a review Suru cannot read still stands, with no account of itself.
#[derive(Default, Deserialize)]
struct Review {
    rationale: Option<String>,
    assessment: Option<String>,
    feedback: Option<String>,
}

/// How far one Route has come.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Its phases stage their work, which only a commit keeps.
    Staging,
    /// Its routing failed, and a fallback runs live: nothing is staged, so nothing is lost.
    Live,
    /// A commit named its winner, and whatever else comes of it is settled.
    Committed,
}

/// Everything shown of one Route, for telling a committed copy from new work.
struct Route {
    /// Which stretch of the loop it ran in, since a stretch can be cut off and leave it behind.
    stretch: u64,
    stage: Stage,
    /// Whether a phase began, without which nothing was done to discard.
    worked: bool,
    /// Whether a row has already said what failed, which a degraded completion need not repeat.
    failure_told: bool,
    /// The Messages this Route has carried, in the order they arrived.
    messages: Vec<RoutedMessage>,
    /// The tool executions this Route has opened, settled or not.
    tools: HashSet<String>,
}

impl Route {
    fn new(stretch: u64, stage: Stage) -> Self {
        Self {
            stretch,
            stage,
            worked: false,
            failure_told: false,
            messages: Vec::new(),
            tools: HashSet::new(),
        }
    }
}

/// One Message a Route carried, and whether a commit of another phase passes it over.
struct RoutedMessage {
    message_id: String,
    /// The phase it was written in, where it stands in the main Transcript with something in it. A
    /// Message with nothing in it never reached the Transcript, and a Subagent's stands in the
    /// Subagent's own, so neither is the commit's to mark.
    supersedable_in: Option<String>,
}

/// Which conversation a staged item belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Conversation {
    Main,
    Subagent,
}

/// The Routes the running stretches of one Copilot Session's loop hold, by `fusionId`.
#[derive(Default)]
pub(super) struct Routes {
    /// Copilot is deciding a Route, which has no `fusionId` to file it under yet.
    routing: bool,
    routes: HashMap<String, Route>,
}

/// The attribution tying an ordinary event to the Route and phase that produced it, if any did.
pub(super) fn attribution(event: &SessionEvent) -> Option<FusionAttribution> {
    event
        .data
        .get("fusion")
        .filter(|fusion| !fusion.is_null())
        .and_then(|fusion| serde_json::from_value(fusion.clone()).ok())
}

/// Whether `event` reports on a Route itself, rather than on the work it runs.
pub(super) fn is_lifecycle(event: &SessionEvent) -> bool {
    matches!(
        event.parsed_type(),
        SessionEventType::SessionFusionRouteStarted
            | SessionEventType::SessionFusionRouteFailed
            | SessionEventType::SessionFusionResolved
            | SessionEventType::SessionFusionCompleted
            | SessionEventType::AssistantFusionPhaseStarted
            | SessionEventType::AssistantFusionPhaseCompleted
            | SessionEventType::AssistantFusionPhaseFailed
    ) || matches!(event.event_type.as_str(), HANDOFF | COMMIT_STARTED)
}

/// Whether `event` begins a Route, which is the only lifecycle report that opens a stretch to stand
/// in. Any other finds its Route where the stretch that began it left it, or lands nowhere.
pub(super) fn begins_route(event: &SessionEvent) -> bool {
    matches!(
        event.parsed_type(),
        SessionEventType::SessionFusionRouteFailed | SessionEventType::SessionFusionResolved
    )
}

impl Routes {
    /// Whether a Route is at work, deciding or running, which only an abort stops.
    pub(super) fn is_running(&self) -> bool {
        self.routing
            || self
                .routes
                .values()
                .any(|route| route.stage != Stage::Committed)
    }

    /// Reads one report on a Route in stretch `stretch`, answering what it says in the Transcript.
    pub(super) fn read(&mut self, event: &SessionEvent, stretch: u64) -> Vec<ProviderEvent> {
        match event.event_type.as_str() {
            HANDOFF => {
                return reported::<Handoff>(event)
                    .filter(|handoff| self.routes.contains_key(&handoff.fusion_id))
                    .map(|handoff| {
                        decided(RouteDecision::Handoff {
                            to: ModelId::new(handoff.target_model),
                        })
                    })
                    .into_iter()
                    .collect();
            }
            COMMIT_STARTED => {
                return reported::<CommitStarted>(event)
                    .map_or_else(Vec::new, |commit| self.committed(&commit));
            }
            _ => {}
        }
        match event.parsed_type() {
            SessionEventType::SessionFusionRouteStarted => {
                self.routing = true;
                Vec::new()
            }
            SessionEventType::SessionFusionResolved => {
                self.routing = false;
                reported::<SessionFusionResolvedData>(event).map_or_else(Vec::new, |resolved| {
                    self.routes.insert(
                        resolved.fusion_id.clone(),
                        Route::new(stretch, Stage::Staging),
                    );
                    vec![decided(RouteDecision::Chosen {
                        steps: planned_steps(&resolved),
                    })]
                })
            }
            SessionEventType::SessionFusionRouteFailed => {
                self.routing = false;
                reported::<SessionFusionRouteFailedData>(event).map_or_else(Vec::new, |failed| {
                    self.routes.insert(
                        failed.fusion_id.unwrap_or(failed.attempt_id),
                        Route::new(stretch, Stage::Live),
                    );
                    vec![decided(RouteDecision::Fallback {
                        failed: None,
                        stand_in: Some(ModelId::new(failed.fallback_model)),
                        reason: failed.error_message,
                    })]
                })
            }
            SessionEventType::AssistantFusionPhaseStarted => {
                if let Some(route) = reported::<AssistantFusionPhaseStartedData>(event)
                    .and_then(|started| self.routes.get_mut(&started.fusion_id))
                {
                    route.worked = true;
                }
                Vec::new()
            }
            SessionEventType::AssistantFusionPhaseCompleted => {
                reported::<AssistantFusionPhaseCompletedData>(event)
                    .filter(|completed| self.routes.contains_key(&completed.fusion_id))
                    .and_then(|completed| verdict(&completed))
                    .map(decided)
                    .into_iter()
                    .collect()
            }
            SessionEventType::AssistantFusionPhaseFailed => {
                reported::<AssistantFusionPhaseFailedData>(event)
                    // An interrupt cancels the phase it stops, and the idle that follows says what
                    // became of the Route.
                    .filter(|failed| failed.status != FusionPhaseStatus::Cancelled)
                    .and_then(|failed| {
                        self.routes.get_mut(&failed.fusion_id)?.failure_told = true;
                        Some(decided(RouteDecision::Fallback {
                            failed: Some(ModelId::new(failed.model)),
                            stand_in: None,
                            reason: failed.error_message.or(Some(failed.reason)),
                        }))
                    })
                    .into_iter()
                    .collect()
            }
            SessionEventType::SessionFusionCompleted => {
                reported::<SessionFusionCompletedData>(event)
                    .and_then(|completed| {
                        let route = self.routes.get_mut(&completed.fusion_id)?;
                        route.stage = Stage::Committed;
                        let reason = completed.degraded_reason.filter(|_| !route.failure_told)?;
                        Some(decided(RouteDecision::Fallback {
                            failed: None,
                            stand_in: completed.final_source_model.map(ModelId::new),
                            reason: Some(reason),
                        }))
                    })
                    .into_iter()
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// A Route's commit names its winner, and the Messages of its other phases are passed over.
    fn committed(&mut self, commit: &CommitStarted) -> Vec<ProviderEvent> {
        let Some(route) = self.routes.get_mut(&commit.fusion_id) else {
            return Vec::new();
        };
        route.stage = Stage::Committed;
        route
            .messages
            .iter()
            .filter(|message| {
                message
                    .supersedable_in
                    .as_ref()
                    .is_some_and(|phase| *phase != commit.source_phase_id)
            })
            .map(|message| ProviderEvent::AgentMessageSuperseded {
                message: ProviderMessageId::new(message.message_id.clone()),
            })
            .collect()
    }

    /// Whether any Route has already carried `message_id`, for an event of it that names no Route.
    pub(super) fn holds_message(&self, message_id: &str) -> bool {
        self.routes
            .values()
            .any(|route| route.holds_message(message_id))
    }

    /// Takes a Message `fusion` attributes to its Route, answering whether it is new: its first
    /// copy, which stands, rather than a committed twin. `shows` says the Message has something in
    /// it for the Transcript to show.
    pub(super) fn admit_message(
        &mut self,
        fusion: &FusionAttribution,
        message_id: &str,
        shows: bool,
        conversation: Conversation,
        stretch: u64,
    ) -> bool {
        let route = self.route_of(fusion, stretch);
        if route.holds_message(message_id) {
            return false;
        }
        route.messages.push(RoutedMessage {
            message_id: message_id.to_owned(),
            supersedable_in: (shows && conversation == Conversation::Main)
                .then(|| fusion.source_phase_id.clone().or(fusion.phase_id.clone()))
                .flatten(),
        });
        true
    }

    /// Takes the start of a tool execution `fusion` attributes to its Route, answering whether it
    /// is new rather than a committed twin of one already opened.
    pub(super) fn admit_tool(
        &mut self,
        fusion: &FusionAttribution,
        tool_call_id: &str,
        stretch: u64,
    ) -> bool {
        self.route_of(fusion, stretch)
            .tools
            .insert(tool_call_id.to_owned())
    }

    /// The Route `fusion` names, whose phase is at work. One Suru never saw begin, as when it could
    /// not read its beginning, is known from here on as a staging Route like any other.
    fn route_of(&mut self, fusion: &FusionAttribution, stretch: u64) -> &mut Route {
        let route = self
            .routes
            .entry(fusion.fusion_id.clone())
            .or_insert_with(|| Route::new(stretch, Stage::Staging));
        route.worked = true;
        route
    }

    /// Ends every Route with the stretch `stretch` settling, answering a row for each still staging
    /// whose conversation is gone, which `discarded` says the way the stretch ended means. One that
    /// ended any other way ended unexplained, with its rows left as they stand. Routes of a stretch
    /// cut off before this one go with it.
    pub(super) fn settle(&mut self, stretch: u64, discarded: bool) -> Vec<ProviderEvent> {
        self.routing = false;
        let mut lost = Vec::new();
        for route in std::mem::take(&mut self.routes).into_values() {
            let cut_short =
                route.stretch == stretch && route.stage == Stage::Staging && route.worked;
            if cut_short && discarded {
                lost.push(decided(RouteDecision::Discarded {
                    by: DISCARDING_PROVIDER.to_owned(),
                }));
            } else if cut_short {
                tracing::warn!("a Copilot Route ended with neither a commit nor an interruption");
            }
        }
        lost
    }

    /// Forgets the Routes of stretches before `stretch`, one of which has just idled unowed.
    pub(super) fn forget_before(&mut self, stretch: u64) {
        self.routes.retain(|_, route| route.stretch == stretch);
    }
}

impl Route {
    fn holds_message(&self, message_id: &str) -> bool {
        self.messages
            .iter()
            .any(|message| message.message_id == message_id)
    }
}

fn decided(decision: RouteDecision) -> ProviderEvent {
    ProviderEvent::RouteDecided { decision }
}

/// The plan a resolved Route set out on: each phase Copilot planned, by the Model it named for it.
/// Copilot names a Critique's reviser nowhere but as the Model that drafted it, and an older plan
/// may name no phases, in which case its primary Model is all that is known of it.
fn planned_steps(resolved: &SessionFusionResolvedData) -> Vec<RouteStep> {
    let primary = || Some(resolved.primary_model.clone());
    let reviewer = || {
        resolved
            .judge_model
            .clone()
            .or_else(|| resolved.secondary_model.clone())
    };
    let steps: Vec<RouteStep> = resolved
        .phase_plan
        .iter()
        .flatten()
        .filter_map(|step| {
            let (model, action) = match step.kind {
                FusionPhaseKind::Primary | FusionPhaseKind::Draft | FusionPhaseKind::FollowUp => {
                    (primary(), RouteAction::Works)
                }
                FusionPhaseKind::Judge | FusionPhaseKind::Critic => {
                    (reviewer(), RouteAction::Reviews)
                }
                FusionPhaseKind::Repair | FusionPhaseKind::Revision => (
                    resolved.repair_model.clone().or_else(primary),
                    RouteAction::Revises,
                ),
                _ => return None,
            };
            Some(RouteStep {
                model: ModelId::new(model?),
                action,
            })
        })
        .collect();
    if steps.is_empty() {
        return vec![RouteStep {
            model: ModelId::new(resolved.primary_model.clone()),
            action: RouteAction::Works,
        }];
    }
    steps
}

/// The verdict a finished phase reached, if it was a review. A judge states its own; a critic's is
/// `assessment` inside its content.
fn verdict(completed: &AssistantFusionPhaseCompletedData) -> Option<RouteDecision> {
    if !matches!(
        completed.phase_kind,
        FusionPhaseKind::Judge | FusionPhaseKind::Critic
    ) {
        return None;
    }
    let review: Review = serde_json::from_str(&completed.content).unwrap_or_default();
    let verdict = match completed
        .verdict
        .as_deref()
        .or(review.assessment.as_deref())
    {
        Some("accept" | "approve") => RouteVerdict::Accepted,
        Some("reject" | "revise") => RouteVerdict::Rejected,
        _ => RouteVerdict::Unread,
    };
    Some(RouteDecision::Verdict {
        reviewer: ModelId::new(completed.model.clone()),
        verdict,
        detail: review.rationale.or(review.feedback),
    })
}
