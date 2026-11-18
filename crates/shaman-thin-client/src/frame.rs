//! What frames an open Session's screen, whole on each change: the header,
//! the bar above the composer, the composer's state, the Intervention to
//! present and the Prompts queued behind the working Turn; and the
//! Interventions this screen dismissed, and the Prompts it has sent and its
//! Server has yet to be seen holding, which stand in the conversation and
//! among the queued as they are being sent.

use std::{
    collections::HashSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use shaman_protocol::{
    Activity, ActivityStatus, ApprovalId, PromptDelivery, PromptId, PromptStatus, SessionId,
    SessionSnapshot, SessionStatus, SessionTimestamp, WatchSummary,
};

use crate::{
    conversation::{ConversationItem, ItemKind},
    intervention::{self, Intervention, Standing},
    listing::Listed,
    tree::{InTree, TreeButton},
};

/// What frames an open Session's screen, whole on each change.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SessionFrame {
    /// The Session's Title.
    pub title: String,
    /// Where it stands, beneath the Title: "origin · workspace · branch",
    /// the Origin named only where the phone holds more than one Remote.
    pub place: String,
    /// Said where the Session, or the Remote it is on, is gone.
    pub gone: Option<String>,
    /// The bar above the composer, while the Session is Working or
    /// Monitoring, or one of its own Interventions waits out of sight.
    pub bar: Option<Bar>,
    /// The composer, where the Session takes Prompts from this phone: none
    /// where it is gone, or is a Subagent's.
    pub composer: Option<Composer>,
    /// The Prompts queued for after the working Turn, oldest first, which
    /// stand at the foot of the conversation.
    pub queued: Vec<QueuedPrompt>,
    /// The Session's own Intervention to present as a sheet: the oldest
    /// pending that this screen has not dismissed, while its Origin answers.
    pub intervention: Option<Intervention>,
    /// The button in the navigation bar that opens the tree the Session
    /// belongs to, where that tree has Subagents.
    pub tree: Option<TreeButton>,
}

/// The bar above the composer: what the Session's work is doing, since when,
/// and the one tap that stops it; or that an Intervention waits on the reader,
/// and the tap that brings it back.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Bar {
    pub kind: BarKind,
    /// "Working", "Monitoring" or "Approval waiting", or, while the Agent
    /// only waits on its Subagents, which: "Waiting on 2 subagents",
    /// "Waiting on Explorer".
    pub words: String,
    /// When the work began, as its Server gave it, which Swift counts the
    /// elapsed time from: none while an Intervention waits.
    pub since: Option<SystemTime>,
    /// What a Monitoring Session waits on: its one Watch, or how many.
    pub watching: Option<String>,
    /// The tap that stops it, and on a Subagent's Session whatever it
    /// delegated; none where a Subagent cannot be stopped alone.
    pub stop: Option<String>,
    /// The tap that presents the Interventions dismissed again: "Review",
    /// where there is anything to present.
    pub review: Option<String>,
}

/// What the bar stands for, by which Swift picks its colour and symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BarKind {
    Working,
    Monitoring,
    /// One of the Session's own Interventions waits out of sight.
    Waiting,
}

/// The composer at the foot of an open Session.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Composer {
    /// What the empty field says.
    pub placeholder: String,
    /// The offer a held send makes, while a Turn works: to queue the Prompt
    /// for after it rather than steer it.
    pub queue: Option<String>,
    /// Said above the composer while the Session's Origin is Unreachable,
    /// when the composer takes nothing.
    pub unreachable: Option<String>,
    /// Said beneath the composer after a send went unanswered: its text is
    /// back in the field, and it may have been sent.
    pub note: Option<String>,
}

/// A Prompt queued for after the working Turn: a dashed bubble at the foot.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct QueuedPrompt {
    /// The Prompt, as the core is handed it back.
    pub prompt: String,
    pub text: String,
    /// What it carried beside its text, where it carried anything.
    pub note: Option<String>,
    /// "Queued", or "Sending" while it is on its way.
    pub label: String,
    /// The act that delivers it now, steering the working Turn.
    pub steer_now: Option<String>,
    /// The act that withdraws it.
    pub cancel: Option<String>,
}

/// What an open Session's screen holds of its own beside what its Server
/// says: the Prompts it sent that the Server has yet to be seen holding, and
/// what a send that went unanswered left to say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Local {
    pub outgoing: Vec<Outgoing>,
    pub note: Option<String>,
    /// The Prompts a stop would withdraw now, as the Session last read: each
    /// admitted to steer or begin a Turn, and taken by none.
    pub withdrawable: Vec<(PromptId, String)>,
    /// The Prompts a stop whose answer was lost may have withdrawn, whose
    /// text is owed back to the composer once the Server shows one withdrawn.
    pub owed: Vec<(PromptId, String)>,
    /// The Session's own Interventions pending as it last read, which a
    /// dismissal takes in.
    pub pending: Vec<ApprovalId>,
    /// The Interventions swiped away on this screen, which wait behind
    /// "Review" for as long as it stays open.
    pub dismissed: HashSet<ApprovalId>,
}

/// A Prompt this screen sent, until its Server is seen holding it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Outgoing {
    pub id: PromptId,
    pub text: String,
    pub delivery: PromptDelivery,
    /// Whether its Server has answered that it took it.
    pub answered: bool,
}

impl Local {
    /// Lets go of every Prompt `snapshot` holds, which stands as its Server
    /// says from now on; answers whether any was.
    pub fn held_by(&mut self, snapshot: &SessionSnapshot) -> bool {
        let before = self.outgoing.len();
        self.outgoing
            .retain(|outgoing| !snapshot.prompts.iter().any(|held| held.id == outgoing.id));
        self.outgoing.len() != before
    }

    /// Notes which Prompts a stop would withdraw as `snapshot` stands, and
    /// takes out each owed Prompt it shows settled: answering the text of
    /// those it shows withdrawn, which are owed back to the composer.
    pub fn reconcile(&mut self, snapshot: &SessionSnapshot) -> Vec<String> {
        self.pending = snapshot.pending_approvals.clone();
        self.withdrawable = snapshot
            .prompts
            .iter()
            .filter(|prompt| {
                prompt.status == PromptStatus::Pending
                    && prompt.delivery == PromptDelivery::Steer
                    && prompt.taken.is_none()
            })
            .map(|prompt| (prompt.id, prompt.text.clone()))
            .collect();
        let mut withdrawn = Vec::new();
        self.owed.retain(|(id, text)| {
            match snapshot.prompts.iter().find(|prompt| prompt.id == *id) {
                // Still waiting: the stop may yet be seen to have taken it.
                Some(prompt)
                    if prompt.status == PromptStatus::Pending && prompt.taken.is_none() =>
                {
                    true
                }
                Some(prompt) => {
                    if prompt.status == PromptStatus::Cancelled {
                        withdrawn.push(text.clone());
                    }
                    false
                }
                None => true,
            }
        });
        withdrawn
    }

    /// Dismisses every Intervention pending now, until the reader asks for
    /// them again.
    pub fn dismiss(&mut self) {
        self.dismissed.extend(self.pending.iter().copied());
    }
}

/// Said beneath the composer after a send went unanswered.
pub(crate) const MAY_HAVE_BEEN_SENT: &str =
    "The server did not answer, so this prompt may have been sent";

/// The id a Prompt stands under in the conversation, before and after its
/// Server holds it.
pub(crate) fn prompt_item_id(id: PromptId) -> String {
    format!("prompt:{id}")
}

/// The Prompt `reference` names, as a queued bubble hands it back.
pub(crate) fn prompt_referenced(reference: &str) -> Option<PromptId> {
    serde_json::from_value(serde_json::Value::String(reference.to_owned())).ok()
}

/// What frames a Session: its Title and place as the listing and its
/// snapshot last said them, or that it is gone; what its work is doing; and
/// what this screen sent of its own. `unreachable` says whether its Origin
/// answered the last time it was asked, and `tree` what the tree the Session
/// belongs to says of it.
pub(crate) fn frame_of(
    listed: Option<&Listed>,
    snapshot: Option<&SessionSnapshot>,
    gone: Option<String>,
    local: &Local,
    unreachable: bool,
    tree: &InTree,
) -> SessionFrame {
    let Some(listed) = listed else {
        return SessionFrame {
            title: String::new(),
            place: String::new(),
            gone: Some(gone.unwrap_or_else(|| GONE_REMOTE.to_owned())),
            bar: None,
            composer: None,
            queued: Vec::new(),
            intervention: None,
            tree: None,
        };
    };
    let title = snapshot
        .map(|snapshot| snapshot.title.clone())
        .filter(|title| !title.is_empty())
        .or_else(|| listed.title.clone())
        .unwrap_or_default();
    let workspace = listed
        .workspace
        .as_ref()
        .map(|(_, name)| name.clone())
        .or_else(|| {
            snapshot.map(|snapshot| listed.paths.workspace_name(&snapshot.session.workspace))
        });
    // A Subagent's Session is read, not written to, and is named beneath its
    // Title by the Subagent's name and Model.
    let subagent =
        tree.subagent || snapshot.is_some_and(|snapshot| snapshot.session.parent.is_some());
    let place = match (&tree.named, subagent) {
        (Some(named), true) => named.clone(),
        _ => listed
            .origin
            .iter()
            .cloned()
            .chain(workspace)
            .chain(listed.branch.clone())
            .collect::<Vec<_>>()
            .join(" · "),
    };
    if gone.is_some() {
        return SessionFrame {
            title,
            place,
            gone,
            bar: None,
            composer: None,
            queued: Vec::new(),
            intervention: None,
            tree: None,
        };
    }
    let working = snapshot.is_some_and(|snapshot| snapshot.working_since().is_some());
    let composer = (!subagent).then(|| Composer {
        placeholder: if working {
            "Steer, or hold send to queue"
        } else {
            "Message"
        }
        .to_owned(),
        queue: working.then(|| "Queue for after this turn".to_owned()),
        unreachable: unreachable.then(|| unreachable_line(listed)),
        note: local.note.clone(),
    });
    let standing = snapshot.map_or(Standing::None, |snapshot| {
        intervention::standing(snapshot, &local.dismissed, unreachable)
    });
    let stops_work = !subagent || tree.stoppable;
    let bar_of = |snapshot: &SessionSnapshot| bar(snapshot, stops_work);
    let (bar, intervention) = match standing {
        Standing::None => (snapshot.and_then(bar_of), None),
        Standing::Presented(intervention) => (snapshot.and_then(bar_of), Some(*intervention)),
        Standing::Waiting { review } => (Some(waiting(review, stops_work)), None),
    };
    SessionFrame {
        title,
        place,
        gone,
        bar,
        composer,
        queued: queued(snapshot, local),
        intervention,
        tree: tree.button.clone(),
    }
}

/// The bar while the Session's own Interventions wait out of sight: orange,
/// with "Review" where it may bring them back, and Stop where `stops_work`.
fn waiting(review: bool, stops_work: bool) -> Bar {
    Bar {
        kind: BarKind::Waiting,
        words: intervention::APPROVAL_WAITING.to_owned(),
        since: None,
        watching: None,
        stop: stops_work.then(|| "Stop".to_owned()),
        review: review.then(|| intervention::REVIEW.to_owned()),
    }
}

/// Said where the Remote a Session was opened on is no longer held.
pub(crate) const GONE_REMOTE: &str =
    "This phone is no longer paired with the server this session is on";

/// Said above the composer while the Session's Origin is Unreachable,
/// naming it where the phone names Origins at all.
fn unreachable_line(listed: &Listed) -> String {
    let named = listed.origin.as_ref().map_or_else(
        || "The server is unreachable".to_owned(),
        |origin| format!("{origin} is unreachable"),
    );
    format!("{named}. Nothing sent goes anywhere until it answers.")
}

/// The bar, while the Session is Working, or else Monitoring. Stop stops a
/// Session's Watches whatever it is, and its work where `stops_work`.
fn bar(snapshot: &SessionSnapshot, stops_work: bool) -> Option<Bar> {
    let (kind, words, since, watching, stop) = if let Some(since) = snapshot.working_since() {
        (
            BarKind::Working,
            working_words(snapshot),
            since,
            None,
            stops_work,
        )
    } else {
        let since = snapshot.monitoring_since()?;
        (
            BarKind::Monitoring,
            "Monitoring".to_owned(),
            since,
            watching(&snapshot.watches),
            true,
        )
    };
    Some(Bar {
        kind,
        words,
        since: Some(instant(since)),
        watching,
        stop: stop.then(|| "Stop".to_owned()),
        review: None,
    })
}

/// What a Working Session's bar says: "Working", or, once its Turn has
/// settled with Subagents still at work or while its Turn's only work is its
/// Agent's wait on them, which it waits on: the one by its name, several by
/// how many.
fn working_words(snapshot: &SessionSnapshot) -> String {
    if snapshot.session.status == SessionStatus::Active && !snapshot.only_waiting_on_subagents() {
        return "Working".to_owned();
    }
    let mut seen = Vec::<SessionId>::new();
    let working = snapshot
        .activities
        .iter()
        .filter_map(|activity| match activity {
            Activity::Subagent {
                status: ActivityStatus::Active,
                session_id,
                name,
                ..
            } => Some((*session_id, name)),
            _ => None,
        })
        .filter(|(session_id, _)| {
            let new = !seen.contains(session_id);
            seen.push(*session_id);
            new
        })
        .map(|(_, name)| name.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>();
    match working.as_slice() {
        [one] if !one.is_empty() => format!("Waiting on {one}"),
        [_] => "Waiting on 1 subagent".to_owned(),
        [] => "Waiting on subagents".to_owned(),
        several => format!("Waiting on {} subagents", several.len()),
    }
}

/// What a Monitoring Session waits on, as the Working Indicator names it:
/// its one Watch by its description, or several by how many.
fn watching(watches: &[WatchSummary]) -> Option<String> {
    match watches {
        [] => None,
        [watch] => Some(watch.description.clone()),
        several => Some(format!("{} watches", several.len())),
    }
}

/// The Prompts queued for after the working Turn, oldest first, and then
/// those this screen is sending to be queued.
fn queued(snapshot: Option<&SessionSnapshot>, local: &Local) -> Vec<QueuedPrompt> {
    let mut held = snapshot
        .into_iter()
        .flat_map(|snapshot| &snapshot.prompts)
        .filter(|prompt| {
            prompt.status == PromptStatus::Pending && prompt.delivery == PromptDelivery::Queue
        })
        .collect::<Vec<_>>();
    held.sort_by_key(|prompt| prompt.admission_order);
    held.into_iter()
        .map(|prompt| QueuedPrompt {
            prompt: prompt.id.to_string(),
            text: prompt.text.clone(),
            note: crate::conversation::carried(prompt.attachments.len(), &prompt.skill_invocations),
            label: "Queued".to_owned(),
            steer_now: Some("Steer now".to_owned()),
            cancel: Some("Cancel".to_owned()),
        })
        .chain(
            local
                .outgoing
                .iter()
                .filter(|outgoing| outgoing.delivery == PromptDelivery::Queue)
                .map(|outgoing| QueuedPrompt {
                    prompt: outgoing.id.to_string(),
                    text: outgoing.text.clone(),
                    note: None,
                    label: "Sending".to_owned(),
                    steer_now: None,
                    cancel: None,
                }),
        )
        .collect()
}

/// `items`, with each Prompt this screen is sending to steer or begin a Turn
/// at their foot, as a bubble marked as sending until its Server answers.
pub(crate) fn with_outgoing(
    mut items: Vec<ConversationItem>,
    local: &Local,
) -> Vec<ConversationItem> {
    items.extend(
        local
            .outgoing
            .iter()
            .filter(|outgoing| outgoing.delivery != PromptDelivery::Queue)
            .map(|outgoing| ConversationItem {
                id: prompt_item_id(outgoing.id),
                kind: ItemKind::UserMessage {
                    text: outgoing.text.clone(),
                    note: None,
                    sending: !outgoing.answered,
                },
            }),
    );
    items
}

fn instant(timestamp: SessionTimestamp) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(timestamp.0)
}

#[cfg(test)]
mod tests;
