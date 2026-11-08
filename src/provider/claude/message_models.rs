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
//! Both the Model that ran and the Selection's are compared, and the Actual Model named, as Claude's
//! catalog names a Model — without a context tier or a snapshot date — so a Selection such as
//! `claude-opus-5-5[1m]` running `claude-opus-5-5` ran its own Model, a Message the API says
//! `claude-haiku-4-5-20251001` wrote names `claude-haiku-4-5` as the Turn's Model Share does, and
//! a Selection of that dated id running it ran its own Model too.
//!
//! What is held speaks only of the assistant message streaming now, so it is forgotten at each
//! boundary between them: the next `message_start`, the loop's `result`, and the end of the CLI's
//! process — which a Turn that ends any other way, such as an interrupt the CLI never answered,
//! comes to, since its child is replaced, and which rides ahead of all the replacement writes.
//!
//! A Subagent's conversation is not tracked here: what it ran on reaches its own row and Turn.

use crate::{
    protocol::{AgentSelection, ModelId},
    provider::{
        ProviderEvent, ProviderMessageId,
        actual_model::{self, ModelInForce, SelectedModel},
    },
};

/// The placeholder the CLI writes as the Model of an assistant message no Model wrote — an API
/// error it reports, or a local command's output.
const SYNTHETIC_MODEL: &str = "<synthetic>";

#[derive(Default)]
pub(super) struct MessageModels {
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
    pub(super) fn assistant_message_started(&mut self, model: Option<&str>) {
        self.forget();
        if let Some(model) = ran(model) {
            self.in_force.observe(model);
        }
    }

    /// What follows the start of a Message the loop's assistant message streams: its attribution
    /// where the Model is known, and otherwise the name a later snapshot attributes it by.
    pub(super) fn message_started(
        &mut self,
        selection: Option<&AgentSelection>,
    ) -> Option<ProviderEvent> {
        let selection = selection?;
        if self.in_force.is_known() {
            return self.in_force.attribution(selected(selection)).map(|model| {
                ProviderEvent::AgentMessageModelKnown {
                    message: None,
                    model,
                }
            });
        }
        self.named += 1;
        let message = ProviderMessageId::new(format!("claude-message-{}", self.named));
        self.awaiting.push(message.clone());
        Some(ProviderEvent::AgentMessageIdentified { message })
    }

    /// A snapshot of the assistant message the loop is streaming names the Model it ran on: the
    /// Messages it opened before then are attributed to it, and those it opens after it are
    /// attributed as they open.
    pub(super) fn snapshot_named(
        &mut self,
        model: Option<&str>,
        selection: Option<&AgentSelection>,
    ) -> Vec<ProviderEvent> {
        let (Some(model), Some(selection)) = (ran(model), selection) else {
            return Vec::new();
        };
        self.in_force.observe(model);
        let awaiting = std::mem::take(&mut self.awaiting);
        let Some(model) = self.in_force.attribution(selected(selection)) else {
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
        selection: Option<&AgentSelection>,
    ) -> Option<ProviderEvent> {
        let model = actual_model::attribution(selected(selection?), ran(model)?.as_str())?;
        Some(ProviderEvent::AgentMessageModelKnown {
            message: None,
            model,
        })
    }

    /// The loop's assistant messages are over — its `result` is out, or its process ended — so
    /// nothing held speaks of what comes next.
    pub(super) fn forget(&mut self) {
        self.in_force.forget();
        self.awaiting.clear();
    }
}

/// `selection`, compared as Claude's catalog names the Model it runs.
fn selected(selection: &AgentSelection) -> SelectedModel<'_> {
    SelectedModel::of(selection).running_as(super::canonical_model(selection.model.as_str()))
}

/// The Model `model` names where it is one that ran, rather than nothing or the CLI's placeholder,
/// as Claude's catalog names it. This is the one place the wire's `model` is judged.
fn ran(model: Option<&str>) -> Option<ModelId> {
    model
        .filter(|model| model.trim() != SYNTHETIC_MODEL)
        .and_then(|model| ModelId::named(super::canonical_model(model.trim())))
}
