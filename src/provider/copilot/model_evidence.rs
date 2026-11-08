//! Transient evidence of Models actually executing in a Copilot conversation.

use std::collections::HashMap;

use github_copilot_sdk::{
    SessionEvent,
    session_events::{
        AssistantFusionPhaseCompletedData, AssistantFusionPhaseFailedData,
        AssistantFusionPhaseStartedData, AssistantTurnEndData, AssistantTurnStartData,
        ModelCallFinishedData, ModelCallStartData, SessionEventType,
    },
};

use crate::{
    protocol::{ModelAttribution, ModelId, ProviderId},
    provider::ProviderEvent,
};

use super::{COPILOT_PROVIDER_ID, reported};

#[derive(Default)]
struct TurnEvidence {
    model: Option<ModelId>,
    saw_call: bool,
}

struct PhaseEvidence {
    model: Option<ModelId>,
    eligible: bool,
}

/// Each conversation has its own tracker; neither a parent's selection nor
/// another conversation's dispatches establish what this conversation runs.
#[derive(Default)]
pub(super) struct WorkingModels {
    turns: HashMap<String, TurnEvidence>,
    calls: HashMap<String, Vec<Option<ModelId>>>,
    phases: HashMap<(String, String), PhaseEvidence>,
    saw_phase: bool,
    uncertain: bool,
    published: Option<ModelId>,
    own_auto: bool,
}

impl WorkingModels {
    pub(super) fn own_auto(&self) -> bool {
        self.own_auto
    }

    /// Marks a native Subagent's own evidence of Auto. The root instead uses
    /// its explicitly selected Model.
    pub(super) fn set_own_auto(&mut self, auto: bool) -> Option<ProviderEvent> {
        self.own_auto = auto;
        self.publish(auto)
    }

    /// Reads only lifecycle evidence; Auto resolution by itself is not a
    /// dispatch and therefore cannot establish a currently executing Model.
    pub(super) fn read(&mut self, event: &SessionEvent, auto: bool) -> Option<ProviderEvent> {
        match event.parsed_type() {
            SessionEventType::AssistantTurnStart => {
                let Some(start): Option<AssistantTurnStartData> = reported(event) else {
                    return self.uncertain(auto);
                };
                if event.agent_id.is_none() && start.parent_tool_call_id.is_some() {
                    return None;
                }
                self.turns.entry(start.turn_id).or_default().model =
                    concrete(start.model.as_deref());
            }
            SessionEventType::AssistantTurnEnd => {
                let Some(end): Option<AssistantTurnEndData> = reported(event) else {
                    return self.uncertain(auto);
                };
                if event.agent_id.is_none() && end.parent_tool_call_id.is_some() {
                    return None;
                }
                self.turns.remove(&end.turn_id);
                self.calls.remove(&end.turn_id);
            }
            SessionEventType::ModelCallStart => {
                let Some(start): Option<ModelCallStartData> = reported(event) else {
                    return self.uncertain(auto);
                };
                if event.agent_id.is_none() && start.parent_tool_call_id.is_some() {
                    return None;
                }
                self.turns
                    .entry(start.turn_id.clone())
                    .or_default()
                    .saw_call = true;
                self.calls
                    .entry(start.turn_id)
                    .or_default()
                    .push(concrete(start.model.as_deref()));
            }
            SessionEventType::ModelCallFinished => {
                let Some(end): Option<ModelCallFinishedData> = reported(event) else {
                    return self.uncertain(auto);
                };
                self.turns.entry(end.turn_id.clone()).or_default().saw_call = true;
                if let Some(calls) = self.calls.get_mut(&end.turn_id) {
                    calls.pop();
                    // Finish carries no call ID. When dispatches share a Turn
                    // ID, it cannot say which Model remains.
                    if !calls.is_empty() {
                        calls.fill(None);
                    }
                    if calls.is_empty() {
                        self.calls.remove(&end.turn_id);
                    }
                } else {
                    for calls in self.calls.values_mut() {
                        calls.fill(None);
                    }
                }
                // A phase or turn may outlive its dispatch. Do not revive its
                // earlier Model after a call has finished.
                for phase in self.phases.values_mut() {
                    phase.eligible = false;
                }
            }
            SessionEventType::AssistantFusionPhaseStarted => {
                let Some(phase): Option<AssistantFusionPhaseStartedData> = reported(event) else {
                    return self.uncertain(auto);
                };
                self.saw_phase = true;
                self.phases.insert(
                    (phase.fusion_id, phase.phase_id),
                    PhaseEvidence {
                        model: concrete(Some(&phase.model)),
                        eligible: true,
                    },
                );
            }
            SessionEventType::AssistantFusionPhaseCompleted => {
                let Some(phase): Option<AssistantFusionPhaseCompletedData> = reported(event) else {
                    return self.uncertain(auto);
                };
                self.saw_phase = true;
                self.phases.remove(&(phase.fusion_id, phase.phase_id));
                self.discard_unmatched_calls();
            }
            SessionEventType::AssistantFusionPhaseFailed => {
                let Some(phase): Option<AssistantFusionPhaseFailedData> = reported(event) else {
                    return self.uncertain(auto);
                };
                self.saw_phase = true;
                self.phases.remove(&(phase.fusion_id, phase.phase_id));
                self.discard_unmatched_calls();
            }
            SessionEventType::SessionFusionCompleted
            | SessionEventType::SessionFusionRouteFailed => {
                self.saw_phase = true;
                self.phases.clear();
                self.discard_unmatched_calls();
            }
            SessionEventType::Abort | SessionEventType::AgentInterrupted => return self.clear(),
            _ => return None,
        }
        self.publish(auto)
    }

    pub(super) fn clear(&mut self) -> Option<ProviderEvent> {
        self.turns.clear();
        self.calls.clear();
        self.phases.clear();
        self.saw_phase = false;
        self.uncertain = false;
        self.publish(false)
    }

    fn uncertain(&mut self, auto: bool) -> Option<ProviderEvent> {
        self.uncertain = true;
        self.publish(auto)
    }

    fn discard_unmatched_calls(&mut self) {
        for calls in self.calls.values_mut() {
            calls.fill(None);
        }
    }

    fn publish(&mut self, auto: bool) -> Option<ProviderEvent> {
        let current = auto.then(|| self.current()).flatten();
        if self.published == current {
            return None;
        }
        self.published.clone_from(&current);
        Some(ProviderEvent::CurrentModelChanged {
            model: current.map(|actual| ModelAttribution {
                provider: ProviderId::new(COPILOT_PROVIDER_ID),
                selected: ModelId::new("auto"),
                actual,
            }),
        })
    }

    fn current(&self) -> Option<ModelId> {
        if self.uncertain {
            return None;
        }
        let mut candidates: Vec<Option<ModelId>> = self.calls.values().flatten().cloned().collect();
        if !self.phases.is_empty() {
            candidates.extend(
                self.phases
                    .values()
                    .map(|phase| phase.eligible.then(|| phase.model.clone()).flatten()),
            );
        } else if !self.saw_phase {
            candidates.extend(
                self.turns
                    .values()
                    .filter(|turn| !turn.saw_call)
                    .map(|turn| turn.model.clone()),
            );
        }
        let first = candidates.first()?.as_ref()?;
        candidates
            .iter()
            .all(|candidate| candidate.as_ref() == Some(first))
            .then(|| first.clone())
    }
}

fn concrete(model: Option<&str>) -> Option<ModelId> {
    model
        .filter(|model| !model.trim().is_empty() && *model != "auto")
        .map(ModelId::new)
}

#[cfg(test)]
mod tests {
    use github_copilot_sdk::SessionEvent;
    use serde_json::{Value, json};

    use crate::provider::ProviderEvent;

    use super::WorkingModels;

    fn event(kind: &str, data: Value) -> SessionEvent {
        SessionEvent {
            id: "fixture".to_owned(),
            timestamp: "2026-01-01T00:00:00Z".to_owned(),
            parent_id: None,
            ephemeral: None,
            agent_id: None,
            debug_cli_received_at_ms: None,
            debug_ws_forwarded_at_ms: None,
            event_type: kind.to_owned(),
            data,
        }
    }

    fn reading(event: Option<ProviderEvent>) -> Option<Option<String>> {
        event.map(|event| match event {
            ProviderEvent::CurrentModelChanged { model } => {
                model.map(|model| model.actual.to_string())
            }
            _ => panic!("the tracker emits only current-Model changes"),
        })
    }

    fn start(
        models: &mut WorkingModels,
        turn: &str,
        model: Option<&str>,
    ) -> Option<Option<String>> {
        reading(models.read(
            &event(
                "model.call_start",
                json!({ "turnId": turn, "model": model }),
            ),
            true,
        ))
    }

    fn finish(models: &mut WorkingModels, turn: &str) -> Option<Option<String>> {
        reading(models.read(
            &event(
                "model.call_finished",
                json!({
                    "turnId": turn, "dispatchDurationMs": 1,
                    "editClassifierVersion": 1, "outcome": "success",
                }),
            ),
            true,
        ))
    }

    fn phase(models: &mut WorkingModels, phase: &str, model: &str) -> Option<Option<String>> {
        reading(models.read(
            &event(
                "assistant.fusion_phase_started",
                json!({
                    "fusionId": "route", "phaseId": phase, "phaseKind": "primary",
                    "role": "solver", "conversationScope": "root",
                    "pattern": "cascade", "model": model,
                }),
            ),
            true,
        ))
    }

    #[test]
    fn concurrent_models_are_shown_only_when_every_active_call_agrees() {
        let mut models = WorkingModels::default();
        assert_eq!(
            start(&mut models, "first", Some("alpha")),
            Some(Some("alpha".into()))
        );
        assert_eq!(start(&mut models, "second", Some("beta")), Some(None));
        assert_eq!(finish(&mut models, "second"), Some(Some("alpha".into())));
        assert_eq!(start(&mut models, "unknown", None), Some(None));
        assert_eq!(finish(&mut models, "unknown"), Some(Some("alpha".into())));
        assert_eq!(finish(&mut models, "first"), Some(None));
    }

    #[test]
    fn a_finish_without_a_unique_call_identity_never_guesses_which_model_remains() {
        let mut models = WorkingModels::default();
        assert_eq!(
            start(&mut models, "same", Some("alpha")),
            Some(Some("alpha".into()))
        );
        assert_eq!(start(&mut models, "same", Some("beta")), Some(None));
        assert_eq!(finish(&mut models, "same"), None);
        assert_eq!(finish(&mut models, "same"), None);
    }

    #[test]
    fn finished_calls_do_not_reveal_stale_turn_or_fusion_phase_models() {
        let mut models = WorkingModels::default();
        assert_eq!(
            reading(models.read(
                &event(
                    "assistant.turn_start",
                    json!({ "turnId": "t", "model": "alpha" })
                ),
                true,
            )),
            Some(Some("alpha".into()))
        );
        assert_eq!(phase(&mut models, "p1", "alpha"), None);
        assert_eq!(start(&mut models, "t", Some("alpha")), None);
        assert_eq!(finish(&mut models, "t"), Some(None));
        assert_eq!(
            reading(models.read(
                &event(
                    "assistant.turn_end",
                    json!({ "turnId": "t", "model": "alpha" })
                ),
                true,
            )),
            None
        );
    }

    #[test]
    fn conflicting_fusion_phases_and_a_route_completion_are_not_a_single_model() {
        let mut models = WorkingModels::default();
        assert_eq!(
            phase(&mut models, "p1", "alpha"),
            Some(Some("alpha".into()))
        );
        assert_eq!(phase(&mut models, "p2", "beta"), Some(None));
        assert_eq!(start(&mut models, "call", Some("alpha")), None);
        assert_eq!(finish(&mut models, "call"), None);
        assert_eq!(
            reading(models.read(&event("session.fusion_completed", json!({})), true)),
            None
        );
        assert_eq!(finish(&mut models, "call"), None);
        assert_eq!(
            phase(&mut models, "next", "beta"),
            Some(Some("beta".into()))
        );
        assert_eq!(
            reading(models.read(&event("abort", json!({})), true)),
            Some(None)
        );
    }

    #[test]
    fn unreadable_lifecycle_evidence_never_leaves_a_stale_model_on_screen() {
        let mut models = WorkingModels::default();
        assert_eq!(
            start(&mut models, "first", Some("alpha")),
            Some(Some("alpha".into()))
        );
        assert_eq!(
            reading(models.read(&event("model.call_finished", json!({})), true)),
            Some(None)
        );
        assert_eq!(start(&mut models, "second", Some("beta")), None);
        assert_eq!(
            reading(models.read(&event("agent.interrupted", json!({})), true)),
            None
        );
        assert_eq!(
            start(&mut models, "third", Some("gamma")),
            Some(Some("gamma".into()))
        );
    }

    #[test]
    fn a_root_never_claims_a_child_dispatch_and_auto_resolution_is_not_a_call() {
        let mut models = WorkingModels::default();
        assert_eq!(
            reading(models.read(
                &event(
                    "session.auto_mode_resolved",
                    json!({ "chosenModel": "alpha" })
                ),
                true,
            )),
            None
        );
        assert_eq!(
            reading(models.read(
                &event(
                    "model.call_start",
                    json!({ "turnId": "child", "model": "beta", "parentToolCallId": "spawn" }),
                ),
                true,
            )),
            None
        );
        assert_eq!(
            start(&mut models, "root", Some("alpha")),
            Some(Some("alpha".into()))
        );
    }
}
