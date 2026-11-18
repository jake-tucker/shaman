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
    #[schemars(extend("x-unknown-kinds" = "tolerated"))]
    #[serde(rename_all = "snake_case")]
    pub enum PromptStatus {
        Queued,
        Delivered,
        #[serde(other)]
        Unknown,
    }

    #[derive(Deserialize, JsonSchema)]
    #[schemars(extend("x-unknown-kinds" = "tolerated"))]
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
    #[schemars(extend("x-unknown-kinds" = "tolerated"))]
    #[serde(rename_all = "snake_case")]
    pub enum PromptStatus {
        Queued,
        Delivered,
        Withdrawn,
        #[serde(other)]
        Unknown,
    }

    /// A new kind of Activity.
    #[derive(Deserialize, JsonSchema)]
    #[schemars(extend("x-unknown-kinds" = "tolerated"))]
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
    #[schemars(extend("x-unknown-kinds" = "tolerated"))]
    #[serde(rename_all = "snake_case")]
    pub enum Standing {
        Queued,
        Delivered,
        #[serde(other)]
        Unknown,
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

/// `old` and `new` answered by one route, judged at the next version.
fn answered<Old: JsonSchema, New: JsonSchema>() -> Verdict {
    let answering = |version| {
        let mut wire = WireSchema::new(version, BASE);
        wire.api("GET /v1/enrollment").answer::<Old>();
        wire
    };
    let mut current = WireSchema::new(BASE + 1, BASE);
    current.api("GET /v1/enrollment").answer::<New>();
    judge(&answering(BASE).document(), &current.document()).unwrap()
}

#[allow(dead_code)]
mod writer_leaves_out {
    use super::*;

    #[derive(Deserialize, schemars::JsonSchema, serde::Serialize)]
    pub struct Before {
        pub hostname: String,
    }

    /// The reader now does without it, and the writer leaves it out where it
    /// is empty, which an older reader that requires it cannot read.
    #[derive(Deserialize, schemars::JsonSchema, serde::Serialize)]
    pub struct After {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub hostname: String,
    }
}

#[test]
fn a_required_answer_field_a_writer_may_now_leave_out_fails() {
    let verdict = answered::<writer_leaves_out::Before, writer_leaves_out::After>();
    assert_breaks(&verdict, &[(ChangeKind::LeftOut, "After.hostname")]);
}

#[allow(dead_code)]
mod closed {
    use super::*;

    #[derive(Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct AdmitPrompt {
        pub text: String,
        #[serde(default)]
        pub attachments: Vec<String>,
    }
}

#[test]
fn refusing_unknown_fields_fails() {
    assert_breaks(
        &next::<closed::AdmitPrompt, base::Prompt>(),
        &[(ChangeKind::Retyped, "AdmitPrompt")],
    );
}

#[allow(dead_code)]
mod skipped_default {
    use super::*;

    fn yes() -> bool {
        true
    }

    #[derive(Deserialize, schemars::JsonSchema, serde::Serialize)]
    pub struct Before {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub new_worktree: bool,
    }

    /// A writer still leaves out `false`, which a reader now takes for
    /// `true`.
    #[derive(Deserialize, schemars::JsonSchema, serde::Serialize)]
    pub struct After {
        #[serde(default = "yes", skip_serializing_if = "std::ops::Not::not")]
        pub new_worktree: bool,
    }

    /// Only the writer changed: it leaves out the `false` a reader takes
    /// for a field left out anyway.
    #[derive(Deserialize, schemars::JsonSchema, serde::Serialize)]
    pub struct Skipping {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub new_worktree: bool,
    }

    #[derive(Deserialize, schemars::JsonSchema, serde::Serialize)]
    pub struct Writing {
        #[serde(default)]
        pub new_worktree: bool,
    }
}

#[test]
fn flipping_a_default_a_writer_skips_fails() {
    let verdict = answered::<skipped_default::Before, skipped_default::After>();
    assert_breaks(
        &verdict,
        &[(ChangeKind::DroppedDefault, "After.new_worktree")],
    );
}

#[test]
fn skipping_the_default_a_reader_takes_anyway_passes() {
    let verdict = answered::<skipped_default::Writing, skipped_default::Skipping>();
    assert_eq!(verdict.breaks().count(), 0, "{}", verdict.report());
}

#[allow(dead_code)]
mod newtype_variants {
    use super::*;

    #[derive(Deserialize, JsonSchema)]
    pub struct Words {
        pub text: String,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct Picture {
        pub url: String,
    }

    pub mod before {
        use super::*;

        #[derive(Deserialize, JsonSchema)]
        #[schemars(extend("x-unknown-kinds" = "tolerated"))]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum Item {
            Text(Words),
            Image(Picture),
        }
    }

    pub mod after {
        use super::*;

        /// `text` is now `words`: the same body under another tag.
        #[derive(Deserialize, JsonSchema)]
        #[schemars(extend("x-unknown-kinds" = "tolerated"))]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum Item {
            Words(Words),
            Image(Picture),
        }
    }
}

#[test]
fn renaming_a_variant_that_holds_a_struct_beside_its_tag_fails() {
    let verdict = answered::<newtype_variants::before::Item, newtype_variants::after::Item>();
    assert_breaks(&verdict, &[(ChangeKind::Renamed, "Item(kind=\"text\")")]);
}

#[allow(dead_code)]
mod flattened {
    use super::*;

    pub mod before {
        use super::*;

        #[derive(Deserialize, JsonSchema)]
        pub struct Holder {
            pub id: String,
            #[serde(flatten)]
            pub held: Held,
        }

        #[derive(Deserialize, JsonSchema)]
        #[schemars(extend("x-unknown-kinds" = "tolerated"))]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum Held {
            File { path: String },
            Link { url: String },
        }
    }

    pub mod after {
        use super::*;

        #[derive(Deserialize, JsonSchema)]
        pub struct Holder {
            pub id: String,
            #[serde(flatten)]
            pub held: Held,
        }

        /// A file's path is now a number.
        #[derive(Deserialize, JsonSchema)]
        #[schemars(extend("x-unknown-kinds" = "tolerated"))]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum Held {
            File { path: u64 },
            Link { url: String },
        }
    }
}

#[test]
fn retyping_a_field_of_an_enum_a_struct_flattens_fails() {
    let verdict = answered::<flattened::before::Holder, flattened::after::Holder>();
    assert_breaks(
        &verdict,
        &[(ChangeKind::Retyped, "Holder(kind=\"file\").path")],
    );
}

#[allow(dead_code)]
mod intolerant {
    use super::*;

    #[derive(Deserialize, JsonSchema)]
    pub struct Prompt {
        pub status: Status,
    }

    /// No fallback, and no mark of one.
    #[derive(Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Status {
        Queued,
        Delivered,
    }
}

#[test]
fn an_answer_carrying_an_enum_with_no_fallback_fails() {
    let verdict = judge(
        &base(),
        &wire::<base::AdmitPrompt, intolerant::Prompt>(BASE + 1, BASE + 1),
    )
    .unwrap();
    assert_eq!(verdict.intolerant, ["Status"], "{}", verdict.report());
    assert!(!verdict.passes(), "{}", verdict.report());
}

#[test]
fn a_request_carrying_an_enum_with_no_fallback_passes() {
    let mut asking = WireSchema::new(BASE, BASE);
    asking.api("POST /v1/prompts").body::<intolerant::Prompt>();
    let asking = asking.document();
    let verdict = judge(&asking, &asking).unwrap();
    assert!(verdict.passes(), "{}", verdict.report());
}

#[test]
fn a_route_removed_or_added_is_reached_by_itself() {
    let mut more = WireSchema::new(BASE + 1, BASE);
    more.api("POST /v1/sessions/{session_id}/interrupt");
    let verdict = judge(&base(), &more.document()).unwrap();
    for change in &verdict.changes {
        assert_eq!(change.via, change.at, "{}", verdict.report());
    }
}
