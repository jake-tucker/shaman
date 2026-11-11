//! What a scheduled Compaction is judged by (ADR 0073).
//!
//! [`ScheduledCompactionRules`] builds the [`ScheduledCompactionRule`] a
//! Session's scheduled Compaction is judged by from its Provider alone: the
//! Settings in force, whether that Provider is hosted, keeps a Cache
//! Lifetime, compacts on request and is turned on, and the timings. It is
//! installed once in the Session store, which every judgement asks — the
//! actor's, whatever wakes for a Session no actor runs for, and what is
//! published as pending — so none judges by a rule of its own.

use std::{sync::Arc, time::Duration};

use tokio::sync::watch;

use crate::protocol::{ProviderId, SettingsSnapshot};
use crate::provider::{CacheLifetime, ManualCompaction, ProviderRuntime};
use crate::sessions::{ScheduledCompactionRule, ScheduledCompactionRuleSource};

use super::ProviderOrchestrator;

/// When a scheduled Compaction falls due, how much of the cache's life it
/// needs left, and how long ahead of its moment a Session no actor runs for
/// is read in for it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScheduledCompactionTimings {
    pub(crate) lead: Duration,
    pub(crate) margin: Duration,
    pub(crate) wake_ahead: Duration,
}

/// Builds the rule a Session's scheduled Compaction is judged by from its
/// Provider alone, as the Settings stand when asked.
#[derive(Clone)]
pub(crate) struct ScheduledCompactionRules {
    runtimes: Arc<Vec<Arc<dyn ProviderRuntime>>>,
    settings: watch::Receiver<SettingsSnapshot>,
    timings: ScheduledCompactionTimings,
}

impl ScheduledCompactionRules {
    /// The rule a Session on `provider` is judged by now. A Provider this
    /// Server does not host cannot be asked.
    fn for_provider(&self, provider: &ProviderId) -> ScheduledCompactionRule {
        let in_force = self.settings.borrow();
        let provider_compacts = self
            .runtimes
            .iter()
            .find(|runtime| runtime.provider_id() == *provider)
            .is_some_and(|runtime| {
                runtime.cache_lifetime() != CacheLifetime::None
                    && runtime.manual_compaction() != ManualCompaction::Unsupported
            })
            && in_force.settings.provider_enabled(provider);
        ScheduledCompactionRule {
            settings: in_force.settings.compaction.scheduled,
            auto_settle: in_force.settings.sidebar.auto_settle,
            lead: self.timings.lead,
            margin: self.timings.margin,
            provider_compacts,
        }
    }
}

/// The store asks the same rules, to say what is pending with or without an
/// actor running: they read only the Settings in force and what each runtime
/// declares, so they may be asked under the store's lock.
impl ScheduledCompactionRuleSource for ScheduledCompactionRules {
    fn rule_for(&self, provider: &ProviderId) -> ScheduledCompactionRule {
        self.for_provider(provider)
    }

    fn lead(&self) -> Duration {
        self.timings.lead
    }
}

impl ProviderOrchestrator {
    /// Has the Session store judge every scheduled Compaction by these
    /// rules, the one copy of them: the actor, whatever wakes for a Session
    /// no actor runs for, and what is published as pending all ask the store.
    /// Its timings are fixed as the orchestrator is built, so tests inject
    /// theirs there.
    pub(super) fn install_scheduled_compaction_rules(&self) {
        self.sessions
            .judge_scheduled_compactions_by(Arc::new(self.scheduled_compaction_rules()));
    }

    /// The rules this orchestrator's scheduled Compactions are judged by.
    fn scheduled_compaction_rules(&self) -> ScheduledCompactionRules {
        ScheduledCompactionRules {
            runtimes: self.runtimes.clone(),
            settings: self.settings.clone(),
            timings: self.scheduled_compaction,
        }
    }
}
