//! The experiment assignment a Copilot Session opens with to be offered Copilot's Fusion routing
//! tiers, which the `provider.copilot.fusionTiers` Setting governs (ADR 0060).
//!
//! This assignment is hand-written. Copilot's experimentation service did not issue it, and Suru
//! never calls that service: a Session driven over the SDK receives no assignments from the CLI,
//! and these parameters did not come back to any request identifying itself as Suru. The SDK takes
//! assignments from its integrator, so Suru states the two it needs and nothing else
//! (docs/validation/copilot-dynamic-auto-tiers.md).

use std::collections::HashMap;

use github_copilot_sdk::{CopilotExpAssignmentResponse, ExpConfigEntry, ExpFlagValue};

/// The Config the CLI reads these parameters from.
const CONFIG_ID: &str = "default";

/// Lists the routing tiers in `session.model.list` and lets a Session select one. Alone it routes
/// on Fusion's older plan: planVersion 1, with no policy.
const DYNAMIC_AUTO_TIERS: &str = "copilot_cli_dynamic_auto_tiers";

/// Routes on the plan the CLI's own issued assignments route on: planVersion 2, with its policy.
/// A one-word Prompt on `hydrafusion` cost about 36 times as much without it.
const HYDRAFUSION_PLAN_V2: &str = "copilot_cli_hydrafusion_plan_v2";

/// The assignment, with no Features or Flights to report and no context to forward, because no
/// experiment assigned it. Other experiments stay as they are for an SDK Session: unassigned.
pub(super) fn fusion_tier_assignment() -> CopilotExpAssignmentResponse {
    let parameters = [DYNAMIC_AUTO_TIERS, HYDRAFUSION_PLAN_V2]
        .map(|parameter| (parameter.to_owned(), ExpFlagValue::Bool(true)));
    CopilotExpAssignmentResponse {
        configs: vec![ExpConfigEntry {
            id: CONFIG_ID.to_owned(),
            parameters: HashMap::from(parameters),
        }],
        ..CopilotExpAssignmentResponse::default()
    }
}
