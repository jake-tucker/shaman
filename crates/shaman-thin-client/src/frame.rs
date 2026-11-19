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
    Activity, ActivityStatus, PromptDelivery, PromptId, PromptStatus, SessionId, SessionSnapshot,
    SessionStatus, SessionTimestamp, WatchSummary,
};

use crate::{
    beginning::{self, Progress, Provisional},
    behind::Behind,
    conversation::{ConversationItem, ItemKind},
    intervention::{self, Intervention, InterventionId, Standing},
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
    /// Monitoring, one of its own Interventions waits out of sight, or one of
    /// a Subagent's beneath it waits.
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
    /// The references of every one of the Session's own Interventions still
    /// live, oldest first: presented, dismissed, out of sight on an
    /// Unreachable Origin, or answered and on its way to the Agent, which may
    /// refuse it. What the screen holds of one it no longer lists, such as
    /// answers half given, is let go.
    pub waiting: Vec<String>,
}

/// The bar above the composer: what the Session's work is doing, since when,
/// and the one tap that stops it; or that an Intervention waits on the reader,
/// its own or a Subagent's, and the tap that leads to it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Bar {
    pub kind: BarKind,
    /// "Working", "Monitoring", "Approval waiting" or "Questionnaire
    /// waiting", or, while the Agent only waits on its Subagents, which:
    /// "Waiting on 2 subagents", "Waiting on Explorer"; or who beneath it
    /// needs the reader: "Explorer needs approval", "Explorer needs an
    /// answer", "Explorer needs you", "2 subagents need you".
    pub words: String,
    /// When the work began, as its Server gave it, which Swift counts the
    /// elapsed time from: none while an Intervention waits.
    pub since: Option<SystemTime>,
    /// What a Monitoring Session waits on: its one Watch, or how many.
    pub watching: Option<String>,
    /// The tap that stops it, and on a Subagent's Session whatever it
    /// delegated; none where a Subagent cannot be stopped alone.
    pub stop: Option<String>,
    /// The tap that leads to the Intervention waiting, where it may: to the
    /// Session's own dismissed, presenting them again, or to a Subagent's.
    pub review: Option<Review>,
}

/// The bar's "Review", and where it leads.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Review {
    pub label: String,
    pub opens: ReviewOpens,
}

/// Where the bar's "Review" leads.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ReviewOpens {
    /// The Session's own Interventions dismissed, which the core's `review`
    /// presents again.
    Interventions,
    /// The Session `session` names, which Swift hands back unread as it
    /// pushes it: the one Subagent whose Intervention waits, where its sheet
    /// presents itself.
    Session { session: String },
    /// The tree the Session belongs to, as its sheet, where several
    /// Subagents' Interventions wait and each entry says what it waits on.
    Tree,
}

/// What the bar stands for, by which Swift picks its colour and symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BarKind {
    Working,
    Monitoring,
    /// One of the Session's own Interventions waits out of sight, or one of
    /// a Subagent's beneath it waits.
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
    /// or it and the phone can no longer work together, naming the side to
    /// update: the composer takes nothing meanwhile.
    pub unreachable: Option<String>,
    /// Said beneath the composer after a send went unanswered: its text is
    /// back in the field, and it may have been sent.
    pub note: Option<String>,
    /// Whether send is offered: not while a Session being begun waits on its
    /// Server, when the composer takes a draft and delivers nothing, nor
    /// while anything keeps the Session apart.
    pub can_send: bool,
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
    pub pending: Vec<InterventionId>,
    /// The Interventions swiped away on this screen, which wait behind
    /// "Review" for as long as it stays open.
    pub dismissed: HashSet<InterventionId>,
    /// The Session being begun, until its Server answers with the one it
    /// made: what the screen is drawn from in the meantime.
    pub provisional: Option<Provisional>,
    /// Whether the screen has gone.
    pub closed: bool,
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
        self.pending = intervention::pending(snapshot)
            .into_iter()
            .map(|(asked, _)| asked)
            .collect();
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

/// What frames a Session on the Remote `remote` names: its Title and place
/// as the listing and its snapshot last said them, or that it is gone; what
/// its work is doing; and what this screen sent of its own. `cut` says why
/// nothing sent from it goes anywhere for now, where something does: its
/// Origin did not answer the last time it was asked, or it and the phone no
/// longer work together; and `tree` what the tree the Session belongs to
/// says of it.
pub(crate) fn frame_of(
    remote: &str,
    listed: Option<&Listed>,
    snapshot: Option<&SessionSnapshot>,
    gone: Option<String>,
    local: &Local,
    cut: Option<Cut>,
    tree: &InTree,
) -> SessionFrame {
    // An Intervention waits out of sight whatever keeps the Session apart.
    let unreachable = cut.is_some();
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
            waiting: Vec::new(),
        };
    };
    if let (Some(provisional), None, None) = (&local.provisional, snapshot, &gone) {
        return provisional_frame(provisional, listed, local, cut);
    }
    // A Subagent's Session, which no listing carries, is titled by what its
    // tree says it was asked until it has been read.
    let title = snapshot
        .map(|snapshot| snapshot.title.clone())
        .filter(|title| !title.is_empty())
        .or_else(|| listed.title.clone())
        .or_else(|| tree.title.clone())
        .unwrap_or_default();
    let workspace = listed
        .workspace
        .as_ref()
        .map(|(_, name)| name.clone())
        .or_else(|| {
            snapshot.map(|snapshot| listed.paths.workspace_name(&snapshot.session.workspace))
        });
    // A Subagent's Session is read, not written to, and is named beneath its
    // Title by the Subagent's name and Model. Until it is known to be one, a
    // Session the listing does not carry — one pushed from a Subagent's row —
    // is taken for one, so no composer stands a moment and goes.
    let subagent = tree.subagent
        || snapshot.map_or(listed.title.is_none(), |snapshot| {
            snapshot.session.parent.is_some()
        });
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
            waiting: Vec::new(),
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
        unreachable: cut.map(|cut| cut_line(cut, listed)),
        note: local.note.clone(),
        can_send: cut.is_none(),
    });
    let standing = snapshot.map_or(Standing::None, |snapshot| {
        intervention::standing(snapshot, &local.dismissed, unreachable)
    });
    let stops_work = !subagent || tree.stoppable;
    let bar_of = |snapshot: &SessionSnapshot| bar(snapshot, stops_work);
    // A Subagent's Intervention never presents itself here: it marks the bar,
    // once the Session's own has been answered or swiped away.
    let beneath = || {
        snapshot
            .and_then(|snapshot| subagents_waiting(snapshot, tree, remote, unreachable))
            .map(|(words, opens)| waiting(opens, &words, stops_work))
    };
    let (bar, intervention) = match standing {
        Standing::None => (beneath().or_else(|| snapshot.and_then(bar_of)), None),
        Standing::Presented(intervention) => (snapshot.and_then(bar_of), Some(*intervention)),
        Standing::Waiting {
            review: false,
            words,
        } => (Some(waiting(None, words, stops_work)), None),
        Standing::Waiting {
            review: true,
            words,
        } => (
            beneath()
                .or_else(|| Some(waiting(Some(ReviewOpens::Interventions), words, stops_work))),
            None,
        ),
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
        waiting: snapshot.map_or_else(Vec::new, |snapshot| {
            intervention::live(snapshot)
                .into_iter()
                .map(InterventionId::reference)
                .collect()
        }),
    }
}

/// What frames a Session being begun, until its Server is seen holding it:
/// its Prompt's first line as the Title, and a bar reading "Creating
/// worktree" while a new Worktree is prepared and "Starting" after, with no
/// elapsed time and no Stop, while it waits; once refused, or unanswered
/// so that it may have begun, no bar, its row standing in the conversation,
/// and a composer whose Prompt is written instead.
fn provisional_frame(
    provisional: &Provisional,
    listed: &Listed,
    local: &Local,
    cut: Option<Cut>,
) -> SessionFrame {
    let starting = provisional.starting();
    SessionFrame {
        title: provisional.title(),
        place: provisional.place.clone(),
        gone: None,
        bar: starting.then(|| Bar {
            kind: BarKind::Working,
            words: provisional.bar_words().to_owned(),
            since: None,
            watching: None,
            stop: None,
            review: None,
        }),
        composer: Some(Composer {
            placeholder: if starting {
                "Message"
            } else {
                "Write a new prompt instead"
            }
            .to_owned(),
            queue: None,
            unreachable: cut.map(|cut| cut_line(cut, listed)),
            note: local.note.clone(),
            can_send: !starting && cut.is_none(),
        }),
        queued: Vec::new(),
        intervention: None,
        waiting: Vec::new(),
        tree: None,
    }
}

/// What the bar of a Session being begun reads until its Server answers.
pub(crate) const STARTING: &str = "Starting";

/// The id the row a beginning refused, or never answered, stands as is
/// under.
pub(crate) const REFUSED: &str = "refused";

/// The conversation of a Session being begun: its Prompt as a bubble, marked
/// as sending until its Server answers, and the row a refusal, or an answer
/// that never came, stands as.
pub(crate) fn provisional_items(provisional: &Provisional) -> Vec<ConversationItem> {
    let mut items = vec![ConversationItem {
        id: prompt_item_id(provisional.prompt.id),
        kind: ItemKind::UserMessage {
            text: provisional.prompt.text.clone(),
            note: None,
            sending: provisional.starting(),
        },
    }];
    let row = match &provisional.progress {
        Progress::Starting => None,
        Progress::Refused(why) => Some((beginning::refused_words(why), false)),
        Progress::MayHaveBegun => Some((beginning::MAYBE_BEGUN.to_owned(), true)),
    };
    if let Some((words, may_have_begun)) = row {
        items.push(ConversationItem {
            id: REFUSED.to_owned(),
            kind: ItemKind::Refused {
                words,
                may_have_begun,
                try_again: "Try again".to_owned(),
            },
        });
    }
    items
}

/// The bar while an Intervention waits on the reader out of sight: orange,
/// saying `words` of it, with "Review" where it `opens` something, and Stop
/// where `stops_work`.
fn waiting(opens: Option<ReviewOpens>, words: &str, stops_work: bool) -> Bar {
    Bar {
        kind: BarKind::Waiting,
        words: words.to_owned(),
        since: None,
        watching: None,
        stop: stops_work.then(|| "Stop".to_owned()),
        review: opens.map(|opens| Review {
            label: intervention::REVIEW.to_owned(),
            opens,
        }),
    }
}

/// What the bar says while Interventions wait in the Subagents beneath
/// `snapshot`'s Session, on the Remote `remote` names, and where its Review
/// leads: the one Subagent waiting by its name, pushed, or several by how
/// many, in the tree. Nothing is reviewed on an Unreachable Origin, as no
/// answer would go anywhere.
fn subagents_waiting(
    snapshot: &SessionSnapshot,
    tree: &InTree,
    remote: &str,
    unreachable: bool,
) -> Option<(String, Option<ReviewOpens>)> {
    let waiting = intervention::beneath(snapshot);
    let (words, opens) = match waiting.as_slice() {
        [] => return None,
        [(session_id, owed)] => {
            let named = tree
                .names
                .iter()
                .find(|(named, _)| named == session_id)
                .map(|(_, name)| name.as_str())
                .or_else(|| {
                    snapshot
                        .activities
                        .iter()
                        .find_map(|activity| match activity {
                            Activity::Subagent {
                                session_id: spawned,
                                name,
                                ..
                            } if spawned == session_id => Some(name.as_str()),
                            _ => None,
                        })
                })
                .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "A subagent".to_owned());
            (
                format!("{named} needs {}", owed.needs().unwrap_or("you")),
                ReviewOpens::Session {
                    session: crate::tree::reference(remote, *session_id),
                },
            )
        }
        several => (
            format!("{} subagents need you", several.len()),
            ReviewOpens::Tree,
        ),
    };
    Some((words, (!unreachable).then_some(opens)))
}

/// Said where the Remote a Session was opened on is no longer held.
pub(crate) const GONE_REMOTE: &str =
    "This phone is no longer paired with the server this session is on";

/// Why nothing sent from an open Session's screen goes anywhere for now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    /// Its Origin is Unreachable.
    Unreachable,
    /// Its Origin and the phone no longer work together, until the side
    /// behind is updated.
    Behind(Behind),
}

/// Said above the composer while `cut` keeps the Session apart, naming its
/// Origin where the phone names Origins at all.
fn cut_line(cut: Cut, listed: &Listed) -> String {
    let named = listed.origin.as_deref().unwrap_or("The server");
    match cut {
        Cut::Unreachable => {
            format!("{named} is unreachable. Nothing sent goes anywhere until it answers.")
        }
        Cut::Behind(behind) => behind.composer_line(named),
    }
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
