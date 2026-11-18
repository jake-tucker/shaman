//! Each change an older side could not survive fails, each passes once the
//! floor rises with it, and a change by addition passes as it is.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use shaman_pairing_wire::{ChangeKind, Verdict, WireSchema, judge};

/// The version the fixtures start from, and their floor.
const BASE: u32 = 117;

/// The wire a fixture speaks: one Prompt admitted, and the Prompt answered.
fn wire<Request: JsonSchema, Answer: JsonSchema>(version: u32, floor: u32) -> Value {
    let mut wire = WireSchema::new(version, floor);
    wire.api("POST /v1/sessions/{session_id}/prompts")
        .body::<Request>()
        .answer::<Answer>();
    wire.document()
}

fn base() -> Value {
    wire::<base::AdmitPrompt, base::Prompt>(BASE, BASE)
}

/// `Request` and `Answer` judged against the base, at the next version.
fn next<Request: JsonSchema, Answer: JsonSchema>() -> Verdict {
    judge(&base(), &wire::<Request, Answer>(BASE + 1, BASE)).unwrap()
}

/// `Request` and `Answer` judged against the base, at the next version, with
/// the floor raised to it.
fn floor_raised<Request: JsonSchema, Answer: JsonSchema>() -> Verdict {
    judge(&base(), &wire::<Request, Answer>(BASE + 1, BASE + 1)).unwrap()
}

fn kinds(verdict: &Verdict) -> Vec<(ChangeKind, String)> {
    verdict
        .changes
        .iter()
        .map(|change| (change.kind, change.at.clone()))
        .collect()
}

#[allow(dead_code)]
mod base {
    use super::*;

    #[derive(Deserialize, JsonSchema)]
    pub struct AdmitPrompt {
        pub text: String,
        #[serde(default)]
        pub attachments: Vec<String>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct Prompt {
        pub id: String,
        pub status: PromptStatus,
        pub activity: Option<Activity>,
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum PromptStatus {
        Queued,
        Delivered,
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum Activity {
        Message {
            text: String,
        },
        ToolCall {
            name: String,
        },
        #[serde(other)]
        Unknown,
    }
}

#[allow(dead_code)]
mod removed {
    use super::*;
    pub use base::AdmitPrompt;

    #[derive(Deserialize, JsonSchema)]
    pub struct Prompt {
        pub id: String,
        pub activity: Option<base::Activity>,
    }
}

#[allow(dead_code)]
mod renamed {
    use super::*;
    pub use base::Prompt;

    #[derive(Deserialize, JsonSchema)]
    pub struct AdmitPrompt {
        pub message: String,
        #[serde(default)]
        pub attachments: Vec<String>,
    }
}

#[allow(dead_code)]
mod retyped {
    use super::*;
    pub use base::AdmitPrompt;

    #[derive(Deserialize, JsonSchema)]
    pub struct Prompt {
        pub id: u64,
        pub status: base::PromptStatus,
        pub activity: Option<base::Activity>,
    }
}

#[allow(dead_code)]
mod newly_required {
    use super::*;
    pub use base::Prompt;

    #[derive(Deserialize, JsonSchema)]
    pub struct AdmitPrompt {
        pub text: String,
        #[serde(default)]
        pub attachments: Vec<String>,
        pub delivery: String,
    }
}

#[allow(dead_code)]
mod dropped_default {
    use super::*;
    pub use base::Prompt;

    #[derive(Deserialize, JsonSchema)]
    pub struct AdmitPrompt {
        pub text: String,
        pub attachments: Vec<String>,
    }
}

#[allow(dead_code)]
mod added {
    use super::*;

    #[derive(Deserialize, JsonSchema)]
    pub struct AdmitPrompt {
        pub text: String,
        #[serde(default)]
        pub attachments: Vec<String>,
        /// A new optional field.
        #[serde(default)]
        pub delivery: Option<String>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct Prompt {
        pub id: String,
        pub status: PromptStatus,
        pub activity: Option<Activity>,
    }

    /// A new kind of status.
    #[derive(Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum PromptStatus {
        Queued,
        Delivered,
        Withdrawn,
    }

    /// A new kind of Activity.
    #[derive(Deserialize, JsonSchema)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum Activity {
        Message {
            text: String,
        },
        ToolCall {
            name: String,
        },
        Compaction {
            tokens: u64,
        },
        #[serde(other)]
        Unknown,
    }
}

#[allow(dead_code)]
mod type_renamed {
    use super::*;
    pub use base::AdmitPrompt;

    /// The same wire under other Rust names.
    #[derive(Deserialize, JsonSchema)]
    pub struct AdmittedPrompt {
        pub id: String,
        pub status: Standing,
        pub activity: Option<base::Activity>,
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Standing {
        Queued,
        Delivered,
    }
}

/// Asserts `verdict` fails by exactly `expected`, and says why in words that
/// name the floor.
fn assert_breaks(verdict: &Verdict, expected: &[(ChangeKind, &str)]) {
    let expected: Vec<_> = expected
        .iter()
        .map(|(kind, at)| (*kind, (*at).to_owned()))
        .collect();
    assert_eq!(kinds(verdict), expected, "{}", verdict.report());
    assert!(!verdict.passes(), "{}", verdict.report());
    assert!(
        verdict.report().contains("raise PROTOCOL_FLOOR"),
        "{}",
        verdict.report()
    );
}

#[test]
fn a_removal_fails() {
    assert_breaks(
        &next::<removed::AdmitPrompt, removed::Prompt>(),
        &[(ChangeKind::Removed, "Prompt.status")],
    );
}

#[test]
fn a_rename_fails() {
    assert_breaks(
        &next::<renamed::AdmitPrompt, renamed::Prompt>(),
        &[(ChangeKind::Renamed, "AdmitPrompt.text")],
    );
}

#[test]
fn a_retype_fails() {
    assert_breaks(
        &next::<retyped::AdmitPrompt, retyped::Prompt>(),
        &[(ChangeKind::Retyped, "Prompt.id")],
    );
}

#[test]
fn a_newly_required_request_field_fails() {
    assert_breaks(
        &next::<newly_required::AdmitPrompt, newly_required::Prompt>(),
        &[(ChangeKind::NewlyRequired, "AdmitPrompt.delivery")],
    );
}

#[test]
fn a_dropped_default_fails() {
    assert_breaks(
        &next::<dropped_default::AdmitPrompt, dropped_default::Prompt>(),
        &[(ChangeKind::DroppedDefault, "AdmitPrompt.attachments")],
    );
}

#[test]
fn each_break_passes_when_the_floor_rises_with_it() {
    for verdict in [
        floor_raised::<removed::AdmitPrompt, removed::Prompt>(),
        floor_raised::<renamed::AdmitPrompt, renamed::Prompt>(),
        floor_raised::<retyped::AdmitPrompt, retyped::Prompt>(),
        floor_raised::<newly_required::AdmitPrompt, newly_required::Prompt>(),
        floor_raised::<dropped_default::AdmitPrompt, dropped_default::Prompt>(),
    ] {
        assert_eq!(verdict.breaks().count(), 1, "{}", verdict.report());
        assert!(verdict.passes(), "{}", verdict.report());
    }
}

#[test]
fn a_break_fails_with_a_floor_raised_short_of_its_version() {
    let current = wire::<removed::AdmitPrompt, removed::Prompt>(BASE + 2, BASE + 1);
    let verdict = judge(&base(), &current).unwrap();
    assert!(!verdict.passes(), "{}", verdict.report());
    assert!(
        verdict.report().contains("raise PROTOCOL_FLOOR to 119"),
        "{}",
        verdict.report()
    );
}

#[test]
fn an_additive_change_passes() {
    let verdict = next::<added::AdmitPrompt, added::Prompt>();
    assert_eq!(
        kinds(&verdict),
        [
            (ChangeKind::Added, "AdmitPrompt.delivery".to_owned()),
            (ChangeKind::Added, "PromptStatus(\"withdrawn\")".to_owned()),
            (
                ChangeKind::Added,
                "Activity(kind=\"compaction\")".to_owned()
            ),
        ],
        "{}",
        verdict.report()
    );
    assert_eq!(verdict.breaks().count(), 0, "{}", verdict.report());
    assert!(verdict.passes(), "{}", verdict.report());
}

#[test]
fn a_change_fails_where_the_version_does_not_move() {
    let current = wire::<added::AdmitPrompt, added::Prompt>(BASE, BASE);
    let verdict = judge(&base(), &current).unwrap();
    assert!(!verdict.passes(), "{}", verdict.report());
    assert!(
        verdict.report().contains("move it past 117"),
        "{}",
        verdict.report()
    );
}

#[test]
fn renaming_a_type_changes_nothing_on_the_wire() {
    let current = wire::<type_renamed::AdmitPrompt, type_renamed::AdmittedPrompt>(BASE, BASE);
    let verdict = judge(&base(), &current).unwrap();
    assert!(verdict.changes.is_empty(), "{}", verdict.report());
    assert!(verdict.passes(), "{}", verdict.report());
}

#[test]
fn a_route_removed_fails_and_one_added_passes() {
    let mut fewer = WireSchema::new(BASE + 1, BASE);
    fewer.api("POST /v1/sessions/{session_id}/interrupt");
    let verdict = judge(&base(), &fewer.document()).unwrap();
    assert!(
        verdict
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::Removed
                && change.at == "POST /v1/sessions/{session_id}/prompts"),
        "{}",
        verdict.report()
    );
    assert!(!verdict.passes(), "{}", verdict.report());

    let mut more = WireSchema::new(BASE + 1, BASE);
    more.api("POST /v1/sessions/{session_id}/prompts")
        .body::<base::AdmitPrompt>()
        .answer::<base::Prompt>();
    more.api("POST /v1/sessions/{session_id}/interrupt");
    let verdict = judge(&base(), &more.document()).unwrap();
    assert_eq!(
        kinds(&verdict),
        [(
            ChangeKind::Added,
            "POST /v1/sessions/{session_id}/interrupt".to_owned()
        )],
        "{}",
        verdict.report()
    );
    assert!(verdict.passes(), "{}", verdict.report());
}

#[test]
fn a_schema_using_a_keyword_the_check_does_not_read_is_refused() {
    let mut current = base();
    current["$defs"]["Prompt"]["allOf"] = serde_json::json!([{ "type": "object" }]);
    let refusal = judge(&base(), &current).unwrap_err();
    assert!(refusal.contains("`allOf`"), "{refusal}");
}

#[test]
fn a_fixed_length_array_lengthened_is_retyped() {
    #[derive(Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct Pair {
        values: [u8; 2],
    }
    #[derive(Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct Triple {
        values: [u8; 3],
    }
    let verdict = judge(
        &wire::<Pair, base::Prompt>(BASE, BASE),
        &wire::<Triple, base::Prompt>(BASE + 1, BASE),
    )
    .unwrap();
    assert_eq!(
        kinds(&verdict),
        [(ChangeKind::Retyped, "Triple.values".to_owned())],
        "{}",
        verdict.report()
    );
}
