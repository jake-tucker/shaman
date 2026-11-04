//! `move_session`: the Tool through which an Agent changes where a Session
//! works (ADR 0059) — its own, or a brokered Subagent's beneath it — through the
//! operation a Client's `/move` performs. Who is calling comes from the Broker's
//! own knowledge of the call and never from its arguments, and the Server judges
//! how far that caller reaches, so a refusal is in the words the operation gives
//! it and a call is never Approved by anyone: each harness lifts Approvals from
//! the Broker's server by its name (ADR 0035), and the boundary already holds
//! the Agent where it works.
//!
//! The Tool is offered to every Agent alike. A Sidekick is held to the ordinary
//! boundary as any Agent is, which leaves it nothing, since its own Session never
//! Moves and no Session Moves into its Workspace; its wider reach is the
//! Sidekick's form of this Tool.
//!
//! An Agent is Working whenever it calls a Tool, so a Move of its own Session is
//! by nature pending. Its answer is therefore the Agent's instruction as much as
//! its report: what it now has to finish, where it is to stop working, and for a
//! new Worktree, that the commit it was made from has already been read.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::{BrokerTool, BrokerTools, ToolCall, ToolRefusal, takes_only};
use crate::protocol::{MoveDestination, MoveOutcome, SessionId};

pub(super) const DESCRIPTION: &str = "\
Move a Session to another directory: the Session, its Transcript and its \
conversation stay as they are, and it works in the new directory from its next \
Turn. Use it when your work belongs elsewhere, such as in a Worktree of its \
own. Takes \"session_id\", left out for your own Session, or the id of a \
Subagent spawned with spawn_subagent by you or by a Subagent beneath you; \
\"directory\", where it is to work: an absolute path, one relative to where \
that Session works now, or one beginning with ~ for this machine's home; and \
optionally \"new_worktree\": true to Move it instead into a new Worktree of \
\"directory\"'s Repository, which Suru makes as you call, named from the \
Session's Title and started from the commit HEAD stands at now, without any \
uncommitted changes. You reach only so far: the directory must lie in a \
Worktree of the Repository you work in, wherever that Worktree stands, or, \
where you work outside source control, beneath your Workspace's directory, \
judged where the path resolves and whichever Session you Move; only the user \
may take a Session further. Another Agent's Session and any Session of the \
Sidekick Workspace are refused saying so. So is a native Subagent's Session, \
which works inside its parent's Provider process, and where a Provider does not \
tell Suru which Agent is calling, a call from a native Subagent of yours acts \
for your own Session. A directory that does not \
exist is refused saying so too. A Session at work Moves once its Turn and any \
native Subagents it has running settle, your own always among these, so finish \
what you are doing and stop working in the old directory; calling again \
replaces where it is bound, and naming where it works cancels that. Answers \
with JSON of the shape {\"session_id\": \"...\", \"outcome\": \"...\", \
\"directory\": \"...\", \"message\": \"...\"}: the Session, what came of it, \
where it works or is bound, and what that means for you. \"outcome\" is \
\"moved\", \"pending\", \"already_there\" or \"pending_cancelled\".";

/// The JSON Schema of `move_session`'s arguments.
pub(super) fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "session_id": {
                "type": "string",
                "description": "The Session to Move: left out for your own, or the id of a \
                    Subagent spawned with spawn_subagent by you or by a Subagent beneath you.",
            },
            "directory": {
                "type": "string",
                "description": "Where the Session is to work, within a Worktree of the \
                    Repository you work in: an absolute path, one relative to where that Session \
                    works now, or one beginning with ~ for this machine's home.",
            },
            "new_worktree": {
                "type": "boolean",
                "description": "true to Move the Session instead into a new Worktree of the \
                    directory's Repository, which Suru makes as you call.",
            },
        },
        "required": MoveArguments::REQUIRED,
        "additionalProperties": false,
    })
}

impl BrokerTools {
    /// Answers `move_session`: Moves the Session the call names, or the
    /// caller's own, within what the Server finds the caller reaches, and says
    /// what came of it. It runs apart from the call, since a call whose client
    /// has gone is dropped where it stands, and a Move stopped part of the way
    /// would leave a Worktree it made unused, or a Session Moved without its
    /// Provider being told.
    pub(super) async fn move_session(&self, call: ToolCall) -> Result<Value, ToolRefusal> {
        let moving = MoveArguments::read(&call.arguments)?;
        let caller = call.caller.session_id();
        let session_id = moving.session_id.unwrap_or(caller);
        let new_worktree = moving.new_worktree;
        let operations = self.operations.clone();
        let outcome = tokio::spawn(async move {
            operations
                .move_session_for_agent(caller, session_id, moving.destination())
                .await
        })
        .await
        .map_err(|_| ToolRefusal::new("Suru could not finish the Move; nothing more is known."))?
        .map_err(|refusal| ToolRefusal::new(refusal.to_string()))?;
        Ok(
            serde_json::to_value(MoveAnswer::of(&outcome, session_id, caller, new_worktree))
                .expect("a Move's answer always serializes"),
        )
    }
}

/// What `move_session` answers: the Session, what came of the Move, where the
/// Session works or is bound, and what the Agent is to make of that.
#[derive(Debug, Eq, PartialEq, Serialize)]
struct MoveAnswer {
    session_id: SessionId,
    outcome: &'static str,
    directory: PathBuf,
    message: String,
}

impl MoveAnswer {
    /// The answer to `caller` of `outcome`, of a Move of `session_id`, into a
    /// Worktree made for it where `new_worktree`.
    fn of(
        outcome: &MoveOutcome,
        session_id: SessionId,
        caller: SessionId,
        new_worktree: bool,
    ) -> Self {
        let speaker = Speaker::of(session_id, caller);
        let (outcome, directory, message) = match outcome {
            MoveOutcome::Moved { session } => {
                let directory = &session.session.execution_directory.path;
                ("moved", directory, speaker.moved(directory, new_worktree))
            }
            MoveOutcome::AlreadyThere { session } => {
                let directory = &session.session.execution_directory.path;
                let session = speaker.subject();
                (
                    "already_there",
                    directory,
                    format!(
                        "{session} already works in {}, so nothing changed.",
                        directory.display()
                    ),
                )
            }
            MoveOutcome::PendingCancelled { session } => {
                let directory = &session.session.execution_directory.path;
                let session = speaker.subject();
                (
                    "pending_cancelled",
                    directory,
                    format!(
                        "{session} already works in {}, so the Move pending to elsewhere is \
                         cancelled and it stays.",
                        directory.display()
                    ),
                )
            }
            MoveOutcome::Pending { session } => {
                let from = &session.session.execution_directory.path;
                // A pending Move is what the store just left standing.
                let directory = session
                    .session
                    .pending_move
                    .as_ref()
                    .map_or(from, |pending| &pending.destination.path);
                (
                    "pending",
                    directory,
                    speaker.pending(directory, from, new_worktree),
                )
            }
        };
        Self {
            session_id,
            outcome,
            directory: directory.to_owned(),
            message,
        }
    }
}

/// Whose Session an answer speaks of, as the Agent reading it knows that
/// Session: its own, or another by its id.
#[derive(Clone, Copy)]
enum Speaker {
    Own,
    Other(SessionId),
}

impl Speaker {
    fn of(session_id: SessionId, caller: SessionId) -> Self {
        if session_id == caller {
            Self::Own
        } else {
            Self::Other(session_id)
        }
    }

    fn subject(self) -> String {
        match self {
            Self::Own => "Your Session".to_owned(),
            Self::Other(session_id) => format!("Session {session_id}"),
        }
    }

    /// What is said of a Move that stands now, so `directory` is where the
    /// Session works.
    fn moved(self, directory: &Path, new_worktree: bool) -> String {
        let session = self.subject();
        let next = match self {
            Self::Own => "your",
            Self::Other(_) => "its",
        };
        let directory = directory.display();
        if new_worktree {
            format!(
                "{session} now works in the new Worktree Suru made at {directory}, from the \
                 commit HEAD stood at; {next} next Turn works there. Changes left uncommitted \
                 where it worked stayed there."
            )
        } else {
            format!("{session} now works in {directory}; {next} next Turn works there.")
        }
    }

    /// What is said of a Move still waiting on what holds it, from where the
    /// Session works, `from`, to `to`.
    fn pending(self, to: &Path, from: &Path, new_worktree: bool) -> String {
        let (to, from) = (to.display(), from.display());
        let (bound, turn, next) = match (self, new_worktree) {
            (Self::Own, false) => (
                format!("Your Session's Move to {to}"),
                "this Turn and any native Subagents you have running",
                "your",
            ),
            (Self::Own, true) => (
                format!(
                    "Suru made a new Worktree at {to} from the commit HEAD stands at now, and \
                     your Session's Move there"
                ),
                "this Turn and any native Subagents you have running",
                "your",
            ),
            (Self::Other(session_id), false) => (
                format!("Session {session_id}'s Move to {to}"),
                "its Turn and any native Subagents it has running",
                "its",
            ),
            (Self::Other(session_id), true) => (
                format!(
                    "Suru made a new Worktree at {to} from the commit HEAD stands at now, and \
                     Session {session_id}'s Move there"
                ),
                "its Turn and any native Subagents it has running",
                "its",
            ),
        };
        let until = match (self, new_worktree) {
            (Self::Own, false) => format!("Finish what you are doing and stop working in {from}."),
            (Self::Own, true) => format!(
                "Finish what you are doing and stop working in {from}. Commits you make after \
                 this call, and changes you leave uncommitted, stay on the old branch and never \
                 reach the new Worktree: commit first and call move_session again if it needs \
                 them."
            ),
            (Self::Other(_), false) => format!("It keeps working in {from} until then."),
            (Self::Other(_), true) => format!(
                "It keeps working in {from} until then. Commits made there after this call, and \
                 changes left uncommitted, stay on the old branch and never reach the new \
                 Worktree."
            ),
        };
        format!(
            "{bound} is pending: it applies once {turn} settle, and {next} next Turn works there. \
             {until}"
        )
    }
}

/// What `move_session` was called with: the Session it names, where one is
/// named, and where that Session is to work, as written.
#[derive(Debug, Eq, PartialEq)]
struct MoveArguments {
    session_id: Option<SessionId>,
    directory: PathBuf,
    new_worktree: bool,
}

impl MoveArguments {
    /// Everything a call may name.
    const TAKES: [&'static str; 3] = ["session_id", "directory", "new_worktree"];
    /// What a call must name: the Session may be left to the caller's own, and
    /// a new Worktree still says which Repository it is of, as `begin_session`'s
    /// does.
    const REQUIRED: [&'static str; 1] = ["directory"];

    /// The call's arguments, each checked for the shape its schema gives it.
    /// The directory is taken as written, a relative path among them: only the
    /// Session it is read from can say what it names.
    fn read(arguments: &Map<String, Value>) -> Result<Self, ToolRefusal> {
        takes_only(BrokerTool::MoveSession, arguments, &Self::TAKES)?;
        let session_id = match arguments.get("session_id") {
            None | Some(Value::Null) => None,
            Some(named) => Some(serde_json::from_value(named.clone()).map_err(|_| {
                ToolRefusal::new(format!(
                    "move_session's `session_id` must be a Session's id, or left out for your \
                     own; {named} is not one."
                ))
            })?),
        };
        let directory = match arguments.get("directory") {
            Some(Value::String(directory)) if directory.trim().is_empty() => {
                return Err(ToolRefusal::new(
                    "move_session's `directory` is empty; say where the Session is to work.",
                ));
            }
            Some(Value::String(directory)) => PathBuf::from(directory),
            None | Some(Value::Null) => {
                return Err(ToolRefusal::new(
                    "move_session needs `directory`, where the Session is to work or, with \
                     `new_worktree`, a directory of the Repository to make a new Worktree of.",
                ));
            }
            Some(_) => {
                return Err(ToolRefusal::new(
                    "move_session's `directory` must be a path, as a string.",
                ));
            }
        };
        let new_worktree = match arguments.get("new_worktree") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(new_worktree)) => *new_worktree,
            Some(other) => {
                return Err(ToolRefusal::new(format!(
                    "move_session's `new_worktree` must be true or false; {other} is neither."
                )));
            }
        };
        Ok(Self {
            session_id,
            directory,
            new_worktree,
        })
    }

    fn destination(self) -> MoveDestination {
        if self.new_worktree {
            MoveDestination::NewWorktree {
                directory: self.directory,
            }
        } else {
            MoveDestination::Directory {
                path: self.directory,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::BrokerRole;

    fn arguments(value: Value) -> Map<String, Value> {
        let Value::Object(arguments) = value else {
            panic!("arguments are an object");
        };
        arguments
    }

    #[test]
    fn move_arguments_leave_the_session_to_the_caller_and_take_a_directory_as_written() {
        assert_eq!(
            MoveArguments::read(&arguments(json!({ "directory": "../elsewhere" }))),
            Ok(MoveArguments {
                session_id: None,
                directory: PathBuf::from("../elsewhere"),
                new_worktree: false,
            }),
            "a relative path is for the Server to read from the Session it Moves"
        );
        let session_id = SessionId::new();
        let read = MoveArguments::read(&arguments(json!({
            "session_id": session_id,
            "directory": "~/work",
            "new_worktree": true,
        })))
        .expect("every argument");
        assert_eq!(read.session_id, Some(session_id));
        assert_eq!(
            read.destination(),
            MoveDestination::NewWorktree {
                directory: PathBuf::from("~/work"),
            }
        );
        assert_eq!(
            MoveArguments::read(&arguments(json!({
                "session_id": null,
                "directory": "here",
                "new_worktree": null,
            })))
            .map(MoveArguments::destination),
            Ok(MoveDestination::Directory {
                path: PathBuf::from("here"),
            }),
            "what a harness sends as null is left out"
        );
    }

    #[test]
    fn move_arguments_are_refused_in_words_the_agent_can_act_on() {
        for (sent, says) in [
            (
                json!({}),
                "move_session needs `directory`, where the Session is to work or, with \
                 `new_worktree`, a directory of the Repository to make a new Worktree of.",
            ),
            (
                json!({ "new_worktree": true }),
                "move_session needs `directory`, where the Session is to work or, with \
                 `new_worktree`, a directory of the Repository to make a new Worktree of.",
            ),
            (
                json!({ "directory": 7 }),
                "move_session's `directory` must be a path, as a string.",
            ),
            (
                json!({ "directory": " " }),
                "move_session's `directory` is empty; say where the Session is to work.",
            ),
            (
                json!({ "directory": "here", "new_worktree": "yes" }),
                "move_session's `new_worktree` must be true or false; \"yes\" is neither.",
            ),
            (
                json!({ "directory": "here", "session_id": "mine" }),
                "move_session's `session_id` must be a Session's id, or left out for your own; \
                 \"mine\" is not one.",
            ),
            (
                json!({ "directory": "here", "origin": "workstation" }),
                "move_session takes no argument `origin`; it takes `session_id`, `directory`, \
                 `new_worktree`.",
            ),
        ] {
            assert_eq!(
                MoveArguments::read(&arguments(sent.clone())),
                Err(ToolRefusal::new(says)),
                "{sent}"
            );
        }
    }

    #[test]
    fn move_sessions_schema_requires_what_its_description_says_it_takes() {
        let schema = input_schema();
        assert_eq!(schema["required"], json!(["directory"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        for argument in MoveArguments::TAKES {
            assert!(
                schema["properties"].get(argument).is_some()
                    && DESCRIPTION.contains(&format!("\"{argument}\"")),
                "{argument} is in the schema and the description"
            );
        }
        assert_eq!(
            schema["properties"].as_object().map(Map::len),
            Some(MoveArguments::TAKES.len()),
            "and nothing else is"
        );
        for answered in [
            "\"session_id\"",
            "\"outcome\"",
            "\"directory\"",
            "\"message\"",
        ] {
            assert!(DESCRIPTION.contains(answered), "{answered}");
        }
        for outcome in ["moved", "pending", "already_there", "pending_cancelled"] {
            assert!(DESCRIPTION.contains(&format!("\"{outcome}\"")), "{outcome}");
        }
        for boundary in [
            "Worktree of the Repository you work in",
            "Workspace's directory",
        ] {
            assert!(DESCRIPTION.contains(boundary), "{boundary}");
        }
        for refused in ["native Subagent's Session", "Sidekick Workspace"] {
            assert!(DESCRIPTION.contains(refused), "{refused}");
        }
        assert!(
            !DESCRIPTION.contains("Remote"),
            "an Agent other than a Sidekick is told of no Remote"
        );
    }

    /// A Tool every Agent is offered, changing what Suru holds, and recorded as
    /// a Tool Call beside the Move's own row, whose arguments hold nothing a
    /// Transcript may not keep.
    #[test]
    fn move_session_is_every_agents_and_records_as_a_tool_call() {
        let tool = BrokerTool::MoveSession;
        assert_eq!(BrokerTool::named("move_session"), Some(tool));
        assert!(tool.is_offered_to(BrokerRole::Agent));
        assert!(tool.is_offered_to(BrokerRole::Sidekick));
        assert!(!tool.is_read_only());
        assert!(!tool.takes_origin());
        assert!(!tool.is_recorded_by_its_row());
        let sent = json!({ "directory": "here" });
        assert_eq!(tool.recorded_arguments(&sent).as_ref(), &sent);
    }

    /// What a pending Move tells an Agent of a Move of its own Session teaches
    /// it to finish and stop, and of a new Worktree, that what it commits next
    /// is left behind; of a Subagent's, nothing is asked of it.
    #[test]
    fn a_pending_moves_answer_says_what_the_session_is_waiting_for_and_what_to_do() {
        let (to, from) = (Path::new("to"), Path::new("from"));
        let subagent = SessionId::new();
        assert_eq!(
            Speaker::Own.pending(to, from, false),
            "Your Session's Move to to is pending: it applies once this Turn and any native \
             Subagents you have running settle, and your next Turn works there. Finish what you \
             are doing and stop working in from."
        );
        assert_eq!(
            Speaker::Own.pending(to, from, true),
            "Suru made a new Worktree at to from the commit HEAD stands at now, and your \
             Session's Move there is pending: it applies once this Turn and any native Subagents \
             you have running settle, and your next Turn works there. Finish what you are doing \
             and stop working in from. Commits you make after this call, and changes you leave \
             uncommitted, stay on the old branch and never reach the new Worktree: commit first \
             and call move_session again if it needs them."
        );
        assert_eq!(
            Speaker::Other(subagent).pending(to, from, false),
            format!(
                "Session {subagent}'s Move to to is pending: it applies once its Turn and any \
                 native Subagents it has running settle, and its next Turn works there. It keeps \
                 working in from until then."
            )
        );
        assert_eq!(
            Speaker::Other(subagent).pending(to, from, true),
            format!(
                "Suru made a new Worktree at to from the commit HEAD stands at now, and Session \
                 {subagent}'s Move there is pending: it applies once its Turn and any native \
                 Subagents it has running settle, and its next Turn works there. It keeps working \
                 in from until then. Commits made there after this call, and changes left \
                 uncommitted, stay on the old branch and never reach the new Worktree."
            )
        );
    }

    #[test]
    fn a_move_that_stands_now_says_where_the_session_works() {
        let there = Path::new("there");
        let subagent = SessionId::new();
        assert_eq!(
            Speaker::Own.moved(there, false),
            "Your Session now works in there; your next Turn works there."
        );
        assert_eq!(
            Speaker::Other(subagent).moved(there, true),
            format!(
                "Session {subagent} now works in the new Worktree Suru made at there, from the \
                 commit HEAD stood at; its next Turn works there. Changes left uncommitted where \
                 it worked stayed there."
            )
        );
    }
}
