//! `move_session`: the Tool through which an Agent changes where a Session
//! works (ADR 0059) — its own, or a brokered Subagent's beneath it — through the
//! operation a Client's `/move` performs. Who is calling comes from the Broker's
//! own knowledge of the call and never from its arguments, and the Server judges
//! how far that caller reaches, so a refusal is in the words the operation gives
//! it and a call is never Approved by anyone: each harness lifts Approvals from
//! the Broker's server by its name (ADR 0035), and the Tool goes no further than
//! where the Agent works.
//!
//! The Tool is offered to every Agent, in the form that answers the caller. A
//! Sidekick's reaches any Session it may act on, on its own Server or on a
//! Remote its `origin` names, and any directory there: at a Remote the very text
//! given is carried through the Pairing, for the Remote to read and judge as it
//! does a Client's. It names its own author, as its other acts do, and takes
//! `session_id` always, since its own Session never Moves and could only be
//! refused. A Sidekick's Subagent is no Sidekick, and is offered the ordinary
//! form.
//!
//! An Agent is Working whenever it calls a Tool, so a Move of its own Session is
//! by nature pending. Its answer is therefore the Agent's instruction as much as
//! its report: that ending its Turn is what lets the Move apply, that Shaman then
//! begins its next Turn in the destination for it to carry on in, and for a new
//! Worktree, that the commit it was made from has already been read. The
//! description says as much before any call, since an Agent weighing a Move
//! mid-task reads only that.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::session_acts::origin_of_a_session_read_named;
use super::{BrokerTool, BrokerTools, ToolCall, ToolRefusal, origins, takes_only};
use crate::broker::BrokerRole;
use crate::protocol::{MoveDestination, MoveOutcome, SessionId};

pub(super) const DESCRIPTION: &str = "\
Move a Session to another directory: it keeps its conversation and works there \
from its next Turn. Use it when your work belongs elsewhere, such as in a \
Worktree of its own. Takes \"session_id\", left out for your own Session, or a \
Subagent spawned with spawn_subagent by you or beneath you; \"directory\", an \
absolute path, one relative to where that Session works, or one beginning with \
~; and optionally \"new_worktree\": true to Move it into a new Worktree of that \
directory's Repository, made now from the commit HEAD stands at, without \
uncommitted changes. The directory must resolve into a Worktree of the \
Repository you work in or, outside source control, beneath your Workspace's \
directory; further is the user's to do. Another Agent's Session is refused, and \
a native Subagent's, which works inside its parent's process; a native \
Subagent's own call may act for its parent's Session. A Session at work Moves \
once its Turn and any native Subagents settle. After Moving your own, end your \
Turn: Shaman begins your next in the new directory itself, so you carry on there. \
Calling again replaces where it is bound; naming where it works cancels that. Answers with JSON of the shape \
{\"session_id\": \"...\", \"outcome\": \"...\", \"directory\": \"...\", \
\"message\": \"...\"}: \"outcome\" is \"moved\", \"pending\", \"already_there\" or \
\"pending_cancelled\", and \"message\" says what that means for you.";

pub(super) const SIDEKICK_DESCRIPTION: &str = concat!(
    "\
Move any other Session to another directory of the server it lives on: it keeps \
its conversation and works there from its next Turn. Takes \"session_id\", a \
Session's id as list_sessions gives it; \"origin\", the Remote it lives on as its \
row gives it, left out for one on this Shaman server; \"directory\", any directory \
of that server, another Workspace's among them, which that server alone reads and \
judges: an absolute path as it writes paths, one relative to where that Session \
works, or one beginning with ~ for its home; and optionally \"new_worktree\": true \
to Move it into a new Worktree of that directory's Repository, made now from the \
commit HEAD stands at, the Session's own where it works in that Repository, \
without uncommitted changes. Your own Session never Moves, nor does another \
Sidekick's, a native Subagent's, or a Subagent in the Sidekick Workspace beneath \
another Sidekick; a directory that is missing, a bare Repository, the Sidekick \
Workspace and a Remote that does not answer are refused too. A Session at work \
Moves once its Turn and any native Subagents settle, and works where it is until \
then. Calling again replaces where it is bound; naming where it works cancels \
that. Answers with JSON of the shape {\"session_id\": \"...\", \"origin\": \
\"...\", \"outcome\": \"...\", \"directory\": \"...\", \"message\": \"...\"}, \
\"origin\" left out for one on this server: \"outcome\" is \"moved\", \
\"pending\", \"already_there\" or \"pending_cancelled\", and \"message\" says \
what that means for you.",
    origin_of_a_session_read_named!()
);

/// The JSON Schema of `move_session`'s arguments, as an Agent that is `role` to
/// the Broker may give them.
pub(super) fn input_schema(role: BrokerRole) -> Value {
    let new_worktree = json!({
        "type": "boolean",
        "description": "true to Move the Session instead into a new Worktree of the \
            directory's Repository, which Shaman makes as you call.",
    });
    match role {
        BrokerRole::Agent => json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "The Session to Move: left out for your own, or the id of a \
                        Subagent spawned with spawn_subagent by you or by a Subagent beneath you.",
                },
                "directory": directory_property(
                    "within a Worktree of the Repository you work in or, outside source control, \
                     beneath your Workspace's directory",
                    "this machine's home",
                ),
                "new_worktree": new_worktree,
            },
            "required": MoveArguments::REQUIRED,
            "additionalProperties": false,
        }),
        BrokerRole::Sidekick => json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "The id of the Session to Move, as list_sessions gives it; \
                        your own never Moves.",
                },
                "origin": origins::origin_property(),
                "directory": directory_property(
                    "on the server it lives on, which reads and judges this, written as that \
                     server writes paths",
                    "that server's home",
                ),
                "new_worktree": new_worktree,
            },
            "required": MoveArguments::SIDEKICK_REQUIRED,
            "additionalProperties": false,
        }),
    }
}

/// The JSON Schema of `directory`, as what the caller reaches, `within`, and
/// whose `home` a `~` is, makes it.
fn directory_property(within: &str, home: &str) -> Value {
    json!({
        "type": "string",
        "description": format!(
            "Where the Session is to work, {within}: an absolute path, one relative to where \
             that Session works now, or one beginning with ~ for {home}."
        ),
    })
}

impl BrokerTools {
    /// Answers `move_session`: Moves the Session the call names, or the
    /// caller's own, within what the Server finds the caller reaches — at the
    /// Remote its `origin` names, for a Sidekick — and says what came of it. It
    /// runs apart from the call, since a call whose client has gone is dropped
    /// where it stands, and a Move stopped part of the way would leave a Worktree
    /// it made unused, a Session Moved without its Provider being told, or an act
    /// a Remote took unrecorded here.
    pub(super) async fn move_session(&self, call: ToolCall) -> Result<Value, ToolRefusal> {
        let role = call.caller.role();
        let moving = MoveArguments::read(role, &call.arguments)?;
        let origin = origins::origin(BrokerTool::MoveSession, &call.arguments)?;
        let remote = origins::row_origin(origin.clone());
        let caller = call.caller.session_id();
        let session_id = moving.session_id.unwrap_or(caller);
        let new_worktree = moving.new_worktree;
        let destination = moving.destination();
        let author = (role == BrokerRole::Sidekick).then(|| self.sidekick_author(&call));
        let operations = self.operations.clone();
        let outcome = tokio::spawn(async move {
            match author {
                Some(author) => operations
                    .move_session_at(&origin, session_id, destination, author)
                    .await
                    .map_err(|refusal| origins::act_refusal_on(refusal, session_id)),
                None => operations
                    .move_session_for_agent(caller, session_id, destination)
                    .await
                    .map_err(|refusal| ToolRefusal::new(refusal.to_string())),
            }
        })
        .await
        .map_err(|error| {
            tracing::error!(%session_id, "a Session's Move did not run to its end: {error}");
            ToolRefusal::new(
                "Shaman could not finish the Move, so the Session may or may not have Moved; \
                 Shaman's Log says why.",
            )
        })??;
        let speaker = Speaker::of(session_id, caller, remote.as_deref());
        Ok(
            serde_json::to_value(MoveAnswer::of(&outcome, speaker, new_worktree))
                .expect("a Move's answer always serializes"),
        )
    }
}

/// What `move_session` answers: the Session, by its Remote too where it lives on
/// one, what came of the Move, where the Session works or is bound, and what the
/// Agent is to make of that.
#[derive(Debug, Eq, PartialEq, Serialize)]
struct MoveAnswer {
    session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
    outcome: &'static str,
    directory: PathBuf,
    message: String,
}

impl MoveAnswer {
    /// The answer of `outcome` to the caller `speaker` speaks to, of a Move of
    /// the Session it speaks of, into a Worktree made for it where
    /// `new_worktree`.
    fn of(outcome: &MoveOutcome, speaker: Speaker<'_>, new_worktree: bool) -> Self {
        let (outcome, directory, message) = match outcome {
            MoveOutcome::Moved { session } => {
                let directory = &session.session.execution_directory.path;
                ("moved", directory, speaker.moved(directory, new_worktree))
            }
            MoveOutcome::AlreadyThere { session } => {
                let directory = &session.session.execution_directory.path;
                let session = speaker.subject();
                let there = speaker.place(directory);
                (
                    "already_there",
                    directory,
                    format!("{session} already works in {there}, so nothing changed."),
                )
            }
            MoveOutcome::PendingCancelled { session } => {
                let directory = &session.session.execution_directory.path;
                let session = speaker.subject();
                let there = speaker.place(directory);
                (
                    "pending_cancelled",
                    directory,
                    format!(
                        "{session} already works in {there}, so the Move pending to elsewhere is \
                         cancelled and it stays."
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
            session_id: speaker.session_id(),
            origin: speaker.remote().map(str::to_owned),
            outcome,
            directory: directory.to_owned(),
            message,
        }
    }
}

/// Whose Session an answer speaks of, as the Agent reading it knows that
/// Session: its own, or another by its id and the Remote it lives on, where it
/// lives on one, whose directories those it names are.
#[derive(Clone, Copy)]
enum Speaker<'a> {
    Own(SessionId),
    Other {
        session_id: SessionId,
        remote: Option<&'a str>,
    },
}

impl<'a> Speaker<'a> {
    /// A Session at a Remote is never the caller's own, whatever id it shares.
    fn of(session_id: SessionId, caller: SessionId, remote: Option<&'a str>) -> Self {
        if session_id == caller && remote.is_none() {
            Self::Own(session_id)
        } else {
            Self::Other { session_id, remote }
        }
    }

    fn session_id(self) -> SessionId {
        match self {
            Self::Own(session_id) | Self::Other { session_id, .. } => session_id,
        }
    }

    fn remote(self) -> Option<&'a str> {
        match self {
            Self::Own(_) => None,
            Self::Other { remote, .. } => remote,
        }
    }

    fn subject(self) -> String {
        match self {
            Self::Own(_) => "Your Session".to_owned(),
            Self::Other { session_id, .. } => format!("Session {session_id}"),
        }
    }

    /// `directory`, as the Agent reading is to take it: at the Remote it lies on.
    fn place(self, directory: &Path) -> String {
        match self.remote() {
            Some(remote) => format!("{} on the Remote `{remote}`", directory.display()),
            None => directory.display().to_string(),
        }
    }

    /// What is said of a Move that stands now, so `directory` is where the
    /// Session works.
    fn moved(self, directory: &Path, new_worktree: bool) -> String {
        let session = self.subject();
        let next = match self {
            Self::Own(_) => "your",
            Self::Other { .. } => "its",
        };
        let directory = self.place(directory);
        if new_worktree {
            format!(
                "{session} now works in the new Worktree Shaman made at {directory}, from the \
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
        let (to, from) = (self.place(to), self.place(from));
        let made =
            format!("Shaman made a new Worktree at {to} from the commit HEAD stands at now, and");
        // An Agent's own Session is woken where the Move lands, so it is told
        // to end its Turn for that; any other works on where it is.
        const OWN_TURN: &str = "this Turn and any native Subagents you have running";
        const OWN_NEXT: &str = "Shaman then begins your next Turn there itself, for you to carry \
                                on without waiting for the user";
        const OTHER_TURN: &str = "its Turn and any native Subagents it has running";
        const OTHER_NEXT: &str = "its next Turn works there";
        let (bound, turn, next, until) = match (self, new_worktree) {
            (Self::Own(_), false) => (
                format!("Your Session's Move to {to}"),
                OWN_TURN,
                OWN_NEXT,
                format!("End this Turn now rather than working on in {from}."),
            ),
            (Self::Own(_), true) => (
                format!("{made} your Session's Move there"),
                OWN_TURN,
                OWN_NEXT,
                format!(
                    "End this Turn now rather than working on in {from}. Commits you make after \
                     this call, and changes you leave uncommitted, stay on the old branch and \
                     never reach the new Worktree: if it needs them, commit and call \
                     move_session again before you end this Turn."
                ),
            ),
            (Self::Other { session_id, .. }, false) => (
                format!("Session {session_id}'s Move to {to}"),
                OTHER_TURN,
                OTHER_NEXT,
                format!("It keeps working in {from} until then."),
            ),
            (Self::Other { session_id, .. }, true) => (
                format!("{made} Session {session_id}'s Move there"),
                OTHER_TURN,
                OTHER_NEXT,
                format!(
                    "It keeps working in {from} until then. Commits made there after this call, \
                     and changes left uncommitted, stay on the old branch and never reach the \
                     new Worktree."
                ),
            ),
        };
        format!("{bound} is pending: it applies once {turn} settle, and {next}. {until}")
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
    /// Everything a Sidekick's call may name.
    const SIDEKICK_TAKES: [&'static str; 4] = ["session_id", "origin", "directory", "new_worktree"];
    /// What a Sidekick's call must name: its own Session could only be refused,
    /// so none is taken to be meant.
    const SIDEKICK_REQUIRED: [&'static str; 2] = ["session_id", "directory"];

    /// The arguments of a call by an Agent that is `role` to the Broker, each
    /// checked for the shape its schema gives it. The directory is taken as
    /// written, a relative path among them: only the Session it is read from,
    /// on its own Server, can say what it names.
    fn read(role: BrokerRole, arguments: &Map<String, Value>) -> Result<Self, ToolRefusal> {
        let takes: &[&str] = match role {
            BrokerRole::Agent => &Self::TAKES,
            BrokerRole::Sidekick => &Self::SIDEKICK_TAKES,
        };
        takes_only(BrokerTool::MoveSession, arguments, takes)?;
        let session_id = match arguments.get("session_id") {
            None | Some(Value::Null) if role == BrokerRole::Sidekick => {
                return Err(ToolRefusal::new(
                    "move_session needs `session_id`, the id of a Session as list_sessions gives \
                     it: your own is a Sidekick's, and never Moves.",
                ));
            }
            None | Some(Value::Null) => None,
            Some(named) => Some(serde_json::from_value(named.clone()).map_err(|_| {
                ToolRefusal::new(format!(
                    "move_session's `session_id` must be a Session's id; {named} is not one."
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

    fn arguments(value: Value) -> Map<String, Value> {
        let Value::Object(arguments) = value else {
            panic!("arguments are an object");
        };
        arguments
    }

    #[test]
    fn move_arguments_leave_the_session_to_the_caller_and_take_a_directory_as_written() {
        assert_eq!(
            MoveArguments::read(
                BrokerRole::Agent,
                &arguments(json!({ "directory": "../elsewhere" }))
            ),
            Ok(MoveArguments {
                session_id: None,
                directory: PathBuf::from("../elsewhere"),
                new_worktree: false,
            }),
            "a relative path is for the Server to read from the Session it Moves"
        );
        let session_id = SessionId::new();
        let read = MoveArguments::read(
            BrokerRole::Agent,
            &arguments(json!({
                "session_id": session_id,
                "directory": "~/work",
                "new_worktree": true,
            })),
        )
        .expect("every argument");
        assert_eq!(read.session_id, Some(session_id));
        assert_eq!(
            read.destination(),
            MoveDestination::NewWorktree {
                directory: PathBuf::from("~/work"),
            }
        );
        assert_eq!(
            MoveArguments::read(
                BrokerRole::Agent,
                &arguments(json!({
                    "session_id": null,
                    "directory": "here",
                    "new_worktree": null,
                }))
            )
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
                "move_session's `session_id` must be a Session's id; \"mine\" is not one.",
            ),
            (
                json!({ "directory": "here", "origin": "workstation" }),
                "move_session takes no argument `origin`; it takes `session_id`, `directory`, \
                 `new_worktree`.",
            ),
        ] {
            assert_eq!(
                MoveArguments::read(BrokerRole::Agent, &arguments(sent.clone())),
                Err(ToolRefusal::new(says)),
                "{sent}"
            );
        }
    }

    /// A Sidekick's call takes an `origin`, and must name a Session, since the
    /// one it would otherwise mean is its own, which never Moves. The rest is
    /// read as anyone's, and the directory taken as written, for the Session's
    /// own Server to read.
    #[test]
    fn a_sidekicks_arguments_take_an_origin_and_must_name_a_session() {
        let session_id = SessionId::new();
        let read = MoveArguments::read(
            BrokerRole::Sidekick,
            &arguments(json!({
                "session_id": session_id,
                "origin": "workstation",
                "directory": "~/work",
                "new_worktree": true,
            })),
        )
        .expect("every argument");
        assert_eq!(read.session_id, Some(session_id));
        assert_eq!(
            read.destination(),
            MoveDestination::NewWorktree {
                directory: PathBuf::from("~/work"),
            },
            "read by no one here"
        );
        for (sent, says) in [
            (
                json!({ "directory": "here" }),
                "move_session needs `session_id`, the id of a Session as list_sessions gives it: \
                 your own is a Sidekick's, and never Moves.",
            ),
            (
                json!({ "session_id": null, "directory": "here" }),
                "move_session needs `session_id`, the id of a Session as list_sessions gives it: \
                 your own is a Sidekick's, and never Moves.",
            ),
            (
                json!({ "session_id": "theirs", "directory": "here" }),
                "move_session's `session_id` must be a Session's id; \"theirs\" is not one.",
            ),
            (
                json!({ "session_id": session_id, "directory": "here", "root": "/" }),
                "move_session takes no argument `root`; it takes `session_id`, `origin`, \
                 `directory`, `new_worktree`.",
            ),
            (
                json!({ "session_id": session_id }),
                "move_session needs `directory`, where the Session is to work or, with \
                 `new_worktree`, a directory of the Repository to make a new Worktree of.",
            ),
        ] {
            assert_eq!(
                MoveArguments::read(BrokerRole::Sidekick, &arguments(sent.clone())),
                Err(ToolRefusal::new(says)),
                "{sent}"
            );
        }
        assert!(
            origins::origin(
                BrokerTool::MoveSession,
                &arguments(json!({ "origin": 7, "session_id": session_id }))
            )
            .is_err(),
            "and an `origin` is read as every Tool's is"
        );
    }

    /// A Sidekick is told what its form reaches and what it does not, in a
    /// schema that matches, and no ordinary Agent's words of a boundary.
    #[test]
    fn a_sidekicks_schema_requires_what_its_description_says_it_takes() {
        let schema = input_schema(BrokerRole::Sidekick);
        assert_eq!(schema["required"], json!(["session_id", "directory"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        for argument in MoveArguments::SIDEKICK_TAKES {
            assert!(
                schema["properties"].get(argument).is_some()
                    && SIDEKICK_DESCRIPTION.contains(&format!("\"{argument}\"")),
                "{argument} is in the schema and the description"
            );
        }
        assert_eq!(
            schema["properties"].as_object().map(Map::len),
            Some(MoveArguments::SIDEKICK_TAKES.len()),
            "and nothing else is"
        );
        for said in [
            "\"origin\" left out for one on this server",
            "\"pending\"",
            "\"already_there\"",
            "\"pending_cancelled\"",
            "Your own Session never Moves",
            "another Sidekick's",
            "a native Subagent's",
            "bare Repository",
            "the Sidekick Workspace",
            "a Remote that does not answer",
            "that server alone reads and judges",
            "A Session at work Moves once its Turn and any native Subagents settle",
            "the Session's own where it works in that Repository",
        ] {
            assert!(SIDEKICK_DESCRIPTION.contains(said), "{said}");
        }
        assert!(
            SIDEKICK_DESCRIPTION.ends_with(origin_of_a_session_read_named!()),
            "and which `origin` reaches a Session a read named: {SIDEKICK_DESCRIPTION}"
        );
        assert!(
            !SIDEKICK_DESCRIPTION.contains("Repository you work in"),
            "a Sidekick is held to no ordinary boundary"
        );
        let words = SIDEKICK_DESCRIPTION.split_whitespace().count();
        assert!(words <= 300, "the description runs to {words} words");
    }

    #[test]
    fn move_sessions_schema_requires_what_its_description_says_it_takes() {
        let schema = input_schema(BrokerRole::Agent);
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
        for refused in ["Another Agent's Session", "native Subagent's"] {
            assert!(DESCRIPTION.contains(refused), "{refused}");
        }
        let words = DESCRIPTION.split_whitespace().count();
        assert!(words <= 225, "the description runs to {words} words");
        for woken in [
            "After Moving your own, end your Turn",
            "Shaman begins your next in the new directory itself",
        ] {
            assert!(DESCRIPTION.contains(woken), "{woken}");
        }
        assert!(
            !DESCRIPTION.contains("Remote"),
            "an Agent other than a Sidekick is told of no Remote"
        );
    }

    /// A Tool every Agent is offered, changing what Shaman holds, and recorded as
    /// a Tool Call beside the Move's own row, whose arguments hold nothing a
    /// Transcript may not keep.
    #[test]
    fn move_session_is_every_agents_and_records_as_a_tool_call() {
        let tool = BrokerTool::MoveSession;
        assert_eq!(BrokerTool::named("move_session"), Some(tool));
        assert!(tool.is_offered_to(BrokerRole::Agent));
        assert!(tool.is_offered_to(BrokerRole::Sidekick));
        assert!(!tool.is_read_only());
        assert!(tool.takes_origin(), "from a Sidekick");
        assert!(!tool.is_recorded_by_its_row());
        let sent = json!({ "directory": "here" });
        assert_eq!(tool.recorded_arguments(&sent).as_ref(), &sent);
    }

    /// What a pending Move tells an Agent of a Move of its own Session teaches
    /// it to end its Turn, since Shaman wakes it where the Move lands, and of a
    /// new Worktree, that what it commits next is left behind; of a Subagent's,
    /// nothing is asked of it.
    #[test]
    fn a_pending_moves_answer_says_what_the_session_is_waiting_for_and_what_to_do() {
        let (to, from) = (Path::new("to"), Path::new("from"));
        let subagent = SessionId::new();
        let own = Speaker::Own(SessionId::new());
        let other = Speaker::Other {
            session_id: subagent,
            remote: None,
        };
        assert_eq!(
            own.pending(to, from, false),
            "Your Session's Move to to is pending: it applies once this Turn and any native \
             Subagents you have running settle, and Shaman then begins your next Turn there \
             itself, for you to carry on without waiting for the user. End this Turn now rather \
             than working on in from."
        );
        assert_eq!(
            own.pending(to, from, true),
            "Shaman made a new Worktree at to from the commit HEAD stands at now, and your \
             Session's Move there is pending: it applies once this Turn and any native Subagents \
             you have running settle, and Shaman then begins your next Turn there itself, for you \
             to carry on without waiting for the user. End this Turn now rather than working on \
             in from. Commits you make after this call, and changes you leave uncommitted, stay \
             on the old branch and never reach the new Worktree: if it needs them, commit and \
             call move_session again before you end this Turn."
        );
        assert_eq!(
            other.pending(to, from, false),
            format!(
                "Session {subagent}'s Move to to is pending: it applies once its Turn and any \
                 native Subagents it has running settle, and its next Turn works there. It keeps \
                 working in from until then."
            )
        );
        assert_eq!(
            other.pending(to, from, true),
            format!(
                "Shaman made a new Worktree at to from the commit HEAD stands at now, and Session \
                 {subagent}'s Move there is pending: it applies once its Turn and any native \
                 Subagents it has running settle, and its next Turn works there. It keeps working \
                 in from until then. Commits made there after this call, and changes left \
                 uncommitted, stay on the old branch and never reach the new Worktree."
            )
        );
    }

    /// What a Sidekick is told of a Session on a Remote says which Remote, since
    /// the directories named are that Remote's own.
    #[test]
    fn a_remotes_sessions_answer_says_which_remote_its_directories_are_on() {
        let (to, from) = (Path::new("to"), Path::new("from"));
        let session_id = SessionId::new();
        let remote = Speaker::Other {
            session_id,
            remote: Some("workstation"),
        };
        assert_eq!(
            remote.pending(to, from, false),
            format!(
                "Session {session_id}'s Move to to on the Remote `workstation` is pending: it \
                 applies once its Turn and any native Subagents it has running settle, and its \
                 next Turn works there. It keeps working in from on the Remote `workstation` \
                 until then."
            )
        );
        assert_eq!(
            remote.pending(to, from, true),
            format!(
                "Shaman made a new Worktree at to on the Remote `workstation` from the commit HEAD \
                 stands at now, and Session {session_id}'s Move there is pending: it applies once \
                 its Turn and any native Subagents it has running settle, and its next Turn works \
                 there. It keeps working in from on the Remote `workstation` until then. Commits \
                 made there after this call, and changes left uncommitted, stay on the old branch \
                 and never reach the new Worktree."
            )
        );
        assert_eq!(
            remote.moved(to, false),
            format!(
                "Session {session_id} now works in to on the Remote `workstation`; its next Turn \
                 works there."
            )
        );
        assert!(
            matches!(
                Speaker::of(session_id, session_id, Some("workstation")),
                Speaker::Other { .. }
            ),
            "a Session on a Remote is no caller's own, whatever id it shares"
        );
        assert!(matches!(
            Speaker::of(session_id, session_id, None),
            Speaker::Own(_)
        ));
    }

    #[test]
    fn a_move_that_stands_now_says_where_the_session_works() {
        let there = Path::new("there");
        let subagent = SessionId::new();
        assert_eq!(
            Speaker::Own(subagent).moved(there, false),
            "Your Session now works in there; your next Turn works there."
        );
        assert_eq!(
            Speaker::Other {
                session_id: subagent,
                remote: None,
            }
            .moved(there, true),
            format!(
                "Session {subagent} now works in the new Worktree Shaman made at there, from the \
                 commit HEAD stood at; its next Turn works there. Changes left uncommitted where \
                 it worked stayed there."
            )
        );
    }
}
