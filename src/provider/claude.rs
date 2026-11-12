//! Claude Provider runtime over the Claude Code CLI's stream-json protocol.
//!
//! Shaman drives the user's installed Claude Code CLI directly over its stream-json wire — newline-
//! delimited JSON on stdio, with control requests for everything that is not conversation — rather
//! than embedding the Claude Agent SDK (ADR 0010). The integration is layered like the Codex one:
//! [`wire`] holds the serde types Shaman exchanges with the CLI, [`transport`] carries control
//! requests and conversation over a supervised CLI process's stdio, [`projection`] turns the
//! conversation into Provider events, [`catalog`] presents the Models the CLI reports verbatim as
//! Shaman's Model catalog, [`availability`] decides whether the CLI can be driven at all before
//! anything asks it to work, [`session`] runs one Session per long-lived CLI process,
//! [`turn_in_flight`] holds what a Session and its projection must agree on about the Turn in
//! flight, [`broker`] writes the MCP config each Session launch is handed the Broker through,
//! [`errand`] runs Shaman's own one-shot work through the CLI's print mode, and [`runtime`]
//! composes them into the Provider runtime the rest of Shaman uses. What the layers share sits here:
//! how a Claude failure reads, and the launch flags an Agent Selection lowers onto, which a
//! Session's child and an Errand's one-shot carry alike.
//!
//! The Provider is named **Claude**, never "Claude Code": the Agent SDK asks applications not to
//! take the product's name. Text below names the Claude Code CLI only where it factually refers to
//! the binary being driven.
//!
//! A CLI that is not installed or has no signed-in user reaches Model discovery — and so a Turn's
//! Session startup — as its own typed reason rather than as a failure. [`availability`] also asks
//! the CLI for its version: one below the verified 2.1.280 suggestion remains usable with advisory
//! guidance, while a CLI too old to answer the probe is incompatible. A catalog refresh re-runs
//! conditions that need the user to fix something outside Shaman.

mod approval;
mod availability;
mod broker;
mod catalog;
mod compaction;
mod context;
mod errand;
mod message_models;
mod model_usage;
mod projection;
mod prompt_cache;
mod questionnaire;
mod runtime;
mod session;
mod skills;
mod thinking;
mod transport;
mod turn_in_flight;
mod wire;

pub use prompt_cache::ClaudeCacheLifetimes;
pub use runtime::ClaudeRuntime;

use std::ffi::OsString;

use crate::protocol::{AgentSelection, ModelOptionValue, ProviderUnavailability};

use super::{ProviderError, concise_remote_message};

/// What the Log and failures call the Claude harness process.
const CLAUDE_HARNESS_NAME: &str = "Claude Code CLI";

/// What they call the one-shot Claude run an Errand is, which is the same binary in a different
/// mode from the stream-json process above — and, unlike it, one Shaman never speaks a protocol to.
const CLAUDE_ONE_SHOT_NAME: &str = "Claude Code CLI print mode";

/// What Shaman knows this Provider as.
const CLAUDE_PROVIDER_ID: &str = "claude";

/// What the Transcript attributes this Provider's Turns to.
const CLAUDE_AGENT_ID: &str = "claude";

const REASONING_EFFORT_OPTION_ID: &str = "reasoning_effort";

/// The suffix a 1M-context alias or Model carries, naming the context tier it runs at rather than
/// another Model.
const ONE_MILLION_CONTEXT_SUFFIX: &str = "[1m]";

/// `model` without its 1M-context suffix: the Model the CLI names when it reports what ran, which
/// never carries the tier, and the one models.dev prices.
fn without_context_tier(model: &str) -> &str {
    model
        .strip_suffix(ONE_MILLION_CONTEXT_SUFFIX)
        .unwrap_or(model)
}

/// `model` as Claude's own catalog names it: without its 1M-context tier, and without the
/// `-YYYYMMDD` snapshot date a dated wire id carries — the API and the picker's `resolvedModel`
/// name Haiku 4.5 `claude-haiku-4-5-20251001`, where the catalog, and so a result's
/// `canonicalModel`, names it `claude-haiku-4-5`. The CLI recognises the date by the same
/// pattern, `-\d{8}$`. Every surface that names a Claude Model that ran names it this way, so a
/// Model Share, a Message's Actual Model and a catalog row's resolution agree.
fn canonical_model(model: &str) -> &str {
    let model = without_context_tier(model);
    match model.len().checked_sub(9).map(|at| model.split_at(at)) {
        Some((base, date))
            if !base.is_empty()
                && date.starts_with('-')
                && date[1..].bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            base
        }
        _ => model,
    }
}

/// What a Claude failure is called when the CLI itself said nothing usable.
const CLAUDE_FAILURE_FALLBACK: &str = "Claude Provider failed";

/// What the user does outside Shaman about each way Claude can be unavailable. The install page is the
/// Claude Code documentation's own, and the login command is the one `claude auth --help` lists.
fn claude_fix(reason: ProviderUnavailability) -> &'static str {
    match reason {
        ProviderUnavailability::NotInstalled => {
            "install the Claude Code CLI from https://code.claude.com/docs/en/setup"
        }
        ProviderUnavailability::NotSignedIn => "run `claude auth login`",
        ProviderUnavailability::IncompatibleVersion => "update the Claude Code CLI",
    }
}

fn claude_error(message: impl AsRef<str>) -> ProviderError {
    ProviderError::new(concise_remote_message(
        message.as_ref(),
        CLAUDE_FAILURE_FALLBACK,
    ))
}

/// Wraps `error` in the operation that failed, preserving the classification it already carries.
fn claude_error_context(context: &str, error: ProviderError) -> ProviderError {
    let message = concise_remote_message(&format!("{context}: {error}"), CLAUDE_FAILURE_FALLBACK);
    error.reworded(message)
}

/// The user-readable account of a result the CLI ended a loop with badly, told as a failure of
/// `what` — a Turn, an Errand.
///
/// The first user-facing error the CLI reported comes first — `[ede_diagnostic]` entries are
/// CLI-internal telemetry the CLI hides from its own UI — then the result text an errored `success`
/// carries, then the bare subtype when the CLI said nothing more.
fn result_failure_message(what: &str, result: &wire::ResultMessage) -> String {
    let reported = result
        .errors
        .iter()
        .find(|error| !error.starts_with("[ede_diagnostic]"))
        .map(String::as_str)
        .or_else(|| result.result.as_ref().and_then(serde_json::Value::as_str))
        .filter(|reported| !reported.trim().is_empty());
    match reported {
        Some(reported) => concise_remote_message(
            &format!("Claude {what} failed: {reported}"),
            CLAUDE_FAILURE_FALLBACK,
        ),
        None => format!(
            "Claude {what} failed: the Claude Code CLI reported `{}`",
            result.subtype
        ),
    }
}

/// The launch flags an Agent Selection lowers onto. The Model and its reasoning effort are
/// spawn-time flags on the CLI rather than anything it takes over its wire, so every process Shaman
/// launches to do work — a Session's child and an Errand's one-shot alike — carries the Selection
/// this way.
fn selection_args(selection: &AgentSelection) -> Result<Vec<OsString>, ProviderError> {
    debug_assert_eq!(selection.provider.as_str(), CLAUDE_PROVIDER_ID);
    let mut args = vec![
        OsString::from("--model"),
        OsString::from(selection.model.as_str()),
    ];
    let mut effort = None;
    for option in &selection.options {
        let ModelOptionValue::Select { choice } = &option.value else {
            return Err(ProviderError::selection_rejected(format!(
                "Claude does not support toggle Model Option `{}`",
                option.id
            )));
        };
        match option.id.as_str() {
            REASONING_EFFORT_OPTION_ID if effort.is_none() => {
                effort = Some(choice.as_str().to_owned());
            }
            REASONING_EFFORT_OPTION_ID => {
                return Err(ProviderError::selection_rejected(format!(
                    "Claude Model Option `{}` was selected more than once",
                    option.id
                )));
            }
            _ => {
                return Err(ProviderError::selection_rejected(format!(
                    "Claude does not support Model Option `{}`",
                    option.id
                )));
            }
        }
    }
    if let Some(effort) = effort {
        args.push(OsString::from("--effort"));
        args.push(OsString::from(effort));
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::canonical_model;

    #[test]
    fn a_wire_id_is_named_without_its_context_tier_or_snapshot_date() {
        for (wire, canonical) in [
            ("claude-haiku-4-5-20251001", "claude-haiku-4-5"),
            ("claude-opus-4-6[1m]", "claude-opus-4-6"),
            ("claude-sonnet-4-5-20250929[1m]", "claude-sonnet-4-5"),
            ("claude-opus-5-5", "claude-opus-5-5"),
            ("claude-model-2025", "claude-model-2025"),
            ("claude-x-2025100a", "claude-x-2025100a"),
            ("-20251001", "-20251001"),
            ("default", "default"),
        ] {
            assert_eq!(canonical_model(wire), canonical, "{wire}");
        }
    }
}
