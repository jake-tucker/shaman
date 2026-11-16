//! Reads and writes the wire's largest structures, so everything they hold is linked: what a thin Client lists
//! Sessions by, reads a Transcript from, and says to begin a Session, send a Prompt and answer an Intervention.

use serde::{Serialize, de::DeserializeOwned};

use crate::{
    Failure,
    approval::Decision,
    protocol::{
        AdmitPromptRequest, CreateSessionRequest, PROTOCOL_VERSION, SessionCatalogSnapshot,
        SessionCatalogUpdate, SessionError, SessionSnapshot, SessionUpdate, SubagentTreeSnapshot,
    },
    questionnaire::QuestionnaireSubmission,
    session_projection,
};

pub fn round_trip<T: DeserializeOwned + Serialize>(text: &str) -> Result<String, Failure> {
    Ok(serde_json::to_string(&serde_json::from_str::<T>(text)?)?)
}

/// A Session's snapshot with each of `updates` landed on it in turn, as a Session's stream is followed.
pub fn follow(snapshot: &str, updates: &[&str]) -> Result<String, Failure> {
    let mut snapshot: SessionSnapshot = serde_json::from_str(snapshot)?;
    for update in updates {
        let update: SessionUpdate = serde_json::from_str(update)?;
        session_projection::apply_update(&mut snapshot, &update)?;
    }
    Ok(serde_json::to_string(&snapshot)?)
}

/// `decode <kind> <file>`, where the file holds one JSON value a line.
pub fn decode(arguments: &[String]) -> Result<(), Failure> {
    let [kind, file] = arguments else {
        return Err("decode needs a kind and a file".into());
    };
    let text = std::fs::read_to_string(file)?;
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.first().copied().unwrap_or_default();
    let written = match kind.as_str() {
        "session" => follow(first, &lines[1.min(lines.len())..])?,
        "catalog" => round_trip::<SessionCatalogSnapshot>(first)?,
        "catalog-update" => round_trip::<SessionCatalogUpdate>(first)?,
        "subagents" => round_trip::<SubagentTreeSnapshot>(first)?,
        "create" => round_trip::<CreateSessionRequest>(first)?,
        "prompt" => round_trip::<AdmitPromptRequest>(first)?,
        "decision" => round_trip::<Decision>(first)?,
        "answers" => round_trip::<QuestionnaireSubmission>(first)?,
        "error" => round_trip::<SessionError>(first)?,
        other => return Err(format!("no kind {other}").into()),
    };
    println!("protocol {PROTOCOL_VERSION}: {written}");
    Ok(())
}
