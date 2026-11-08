//! Which Model the loop's own conversation wrote each Agent Message on.
//!
//! The CLI names the Model behind every assistant message of the loop's: the API's
//! `message_start` opens the message naming it, and the CLI's snapshot of each of its blocks
//! restates it in `model`. That is what the Selection's alias resolved to, or what the CLI fell
//! back to, and it holds for that assistant message alone, since the next may run elsewhere. The
//! loop's conversation streams, so a Message can open before anything has named its Model; such a
//! Message is named as it opens, and attributed once a snapshot names the Model. A Message the CLI
//! wrote itself carries its `<synthetic>` placeholder, which is no Model at all.
//!
//! A Subagent's conversation is not tracked here: what it ran on reaches its own row and Turn.

use crate::{
    protocol::{AgentSelection, ModelId},
    provider::{
        ProviderEvent, ProviderMessageId,
        actual_model::{self, ModelInForce},
    },
};

/// The placeholder the CLI writes as the Model of an assistant message no Model wrote — an API
/// error it reports, or a local command's output.
const SYNTHETIC_MODEL: &str = "<synthetic>";

/// The running Turn, by the count [`super::turn_in_flight::TurnInFlight::running_selection`]
/// tells it apart by, and the Agent Selection it runs under.
pub(super) type RunningTurn = (u64, AgentSelection);

#[derive(Default)]
pub(super) struct MessageModels {
    /// The Turn what is held here was learned in. Nothing learned in one Turn speaks of the next,
    /// whose Messages orchestration knows by no name given in this one.
    turn: Option<u64>,
    /// The Model the assistant message streaming now ran on, once something has named it.
    in_force: ModelInForce,
    /// The Messages of the assistant message streaming now that opened before its Model was
    /// named, by the name each was given as it opened.
    awaiting: Vec<ProviderMessageId>,
    /// How many Messages have been named, which keeps every name unique.
    named: u64,
}

impl MessageModels {
    /// An assistant message of the loop's opens, naming the Model it runs on where its opening
    /// chunk says.
    pub(super) fn assistant_message_started(
        &mut self,
        model: Option<&str>,
        turn: Option<&RunningTurn>,
    ) {
        self.forget();
        if let Some((ordinal, _)) = turn {
            self.turn = Some(*ordinal);
            if let Some(model) = ran(model) {
                self.in_force.observe(model);
            }
        }
    }

    /// What follows the start of a Message the loop's assistant message streams: its attribution
    /// where the Model is known, and otherwise the name a later snapshot attributes it by.
    pub(super) fn message_started(&mut self, turn: Option<&RunningTurn>) -> Vec<ProviderEvent> {
        let Some((ordinal, selection)) = turn else {
            return Vec::new();
        };
        self.enter(*ordinal);
        if self.in_force.is_known() {
            return self
                .in_force
                .attribution(selection)
                .map(|model| ProviderEvent::AgentMessageModelKnown {
                    message: None,
                    model,
                })
                .into_iter()
                .collect();
        }
        self.named += 1;
        let message = ProviderMessageId::new(format!("claude-message-{}", self.named));
        self.awaiting.push(message.clone());
        vec![ProviderEvent::AgentMessageIdentified { message }]
    }

    /// A snapshot of the assistant message the loop is streaming names the Model it ran on: the
    /// Messages it opened before then are attributed to it, and those it opens after it are
    /// attributed as they open.
    pub(super) fn snapshot_named(
        &mut self,
        model: Option<&str>,
        turn: Option<&RunningTurn>,
    ) -> Vec<ProviderEvent> {
        let (Some(model), Some((ordinal, selection))) = (ran(model), turn) else {
            return Vec::new();
        };
        self.enter(*ordinal);
        self.in_force.observe(model);
        let awaiting = std::mem::take(&mut self.awaiting);
        let Some(model) = self.in_force.attribution(selection) else {
            return Vec::new();
        };
        awaiting
            .into_iter()
            .map(|message| ProviderEvent::AgentMessageModelKnown {
                message: Some(message),
                model: model.clone(),
            })
            .collect()
    }

    /// The attribution of a Message a snapshot of the loop's carries whole, where none of it
    /// streamed: the Model that snapshot names, which speaks of nothing else.
    pub(super) fn whole_message(
        model: Option<&str>,
        turn: Option<&RunningTurn>,
    ) -> Option<ProviderEvent> {
        let (model, (_, selection)) = (ran(model)?, turn?);
        actual_model::attribution(selection, &ModelId::new(model)).map(|model| {
            ProviderEvent::AgentMessageModelKnown {
                message: None,
                model,
            }
        })
    }

    /// The loop's assistant messages are over — its `result` is out, or its process ended — so
    /// nothing held speaks of what comes next.
    pub(super) fn forget(&mut self) {
        self.in_force.forget();
        self.awaiting.clear();
    }

    fn enter(&mut self, ordinal: u64) {
        if self.turn != Some(ordinal) {
            self.forget();
            self.turn = Some(ordinal);
        }
    }
}

/// `model` where it names a Model that ran, rather than nothing or the CLI's placeholder.
fn ran(model: Option<&str>) -> Option<&str> {
    model.filter(|model| !model.trim().is_empty() && *model != SYNTHETIC_MODEL)
}
