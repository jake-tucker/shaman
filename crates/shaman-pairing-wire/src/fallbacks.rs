//! What holds a definition's `x-unknown-kinds` mark to the fallback it
//! claims: every type the Pairing wire's schema marks is named here by its
//! Rust type, and a test reads a kind no build knows, in every shape the
//! type tells its kinds apart by, through that type's own `Deserialize`.
//!
//! A mark with no fallback behind it fails, and so does a marked type this
//! list does not name, so adding a mark means adding the type here.

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Reads `value` as the type a [`Reader`] is for, saying why it could not.
pub(crate) type Read = fn(Value) -> Result<(), String>;

/// A marked type by the name the schema defines it under, and its reader.
pub(crate) struct Reader {
    pub(crate) name: String,
    pub(crate) read: Read,
}

fn reader<T: JsonSchema + DeserializeOwned>() -> Reader {
    Reader {
        name: T::schema_name().into_owned(),
        read: |value| {
            serde_json::from_value::<T>(value)
                .map(drop)
                .map_err(|error| error.to_string())
        },
    }
}

macro_rules! readers {
    ($($type:ty),* $(,)?) => {
        vec![$(reader::<$type>()),*]
    };
}

/// Every type the Pairing wire's schema marks as reading a kind it does not
/// know.
pub(crate) fn readers() -> Vec<Reader> {
    use shaman_pairing::OfferAnswer;
    use shaman_protocol::{
        Activity, ActivityStatus, ApprovalOutcome, ApprovalPosture, ApprovalPostureApplication,
        ApprovalSubject, AttachmentKind, Author, CacheWritePrice, CheckoutBranchOutcome,
        CheckoutKind, CheckoutPreparationPlan, CheckoutReclaimPhase, CheckoutRevision,
        ClaudePermissionMode, CodexApprovalPolicy, CodexSandboxMode, CommandAction,
        CompactionRequest, CompactionTrigger, ContextSource, CopilotPermissions, CostBasis,
        CostCoverage, Decision, DirectorySourceControl, ExecutionDirectoryStatus, FileChange,
        InsightsTool, InterruptOutcome, LifecycleState, MessageRole, MessageStatus,
        ModelAdvisorySeverity, ModelAvailability, ModelOptionKind, ModelOptionRole,
        ModelOptionValue, ModelPriceCategory, MoveOutcome, PathStyle, PromptAdmission,
        PromptDelivery, PromptStatus, PromptWithdrawal, ProviderCatalogStatus,
        ProviderUnavailability, QuestionAnswer, QuestionnaireOutcome, ReadRefusal, ReadingSpan,
        RepositoryLocation, RouteAction, RouteDecision, RouteVerdict, ServerReference,
        SessionCatalogChange, SessionChange, SessionErrorCode, SessionListing, SessionStatus,
        SkillCatalogStatus, SkillPromptDelivery, SourceControlAvailability,
        SourceControlCapability, SubagentTreeChange, TranscriptItem, TurnStatus, UnreachableReason,
        UsageMetricsKey, WatchOutcomeStatus, Way, WorkspaceUnavailability,
    };
    readers![
        Activity,
        ActivityStatus,
        ApprovalOutcome,
        ApprovalPosture,
        ApprovalPostureApplication,
        ApprovalSubject,
        AttachmentKind,
        Author,
        CacheWritePrice,
        CheckoutBranchOutcome,
        CheckoutKind,
        CheckoutPreparationPlan,
        CheckoutReclaimPhase,
        CheckoutRevision,
        ClaudePermissionMode,
        CodexApprovalPolicy,
        CodexSandboxMode,
        CommandAction,
        CompactionRequest,
        CompactionTrigger,
        ContextSource,
        CopilotPermissions,
        CostBasis,
        CostCoverage,
        Decision,
        DirectorySourceControl,
        ExecutionDirectoryStatus,
        FileChange,
        InsightsTool,
        InterruptOutcome,
        LifecycleState,
        MessageRole,
        MessageStatus,
        ModelAdvisorySeverity,
        ModelAvailability,
        ModelOptionKind,
        ModelOptionRole,
        ModelOptionValue,
        ModelPriceCategory,
        MoveOutcome,
        OfferAnswer,
        PathStyle,
        PromptAdmission,
        PromptDelivery,
        PromptStatus,
        PromptWithdrawal,
        ProviderCatalogStatus,
        ProviderUnavailability,
        QuestionAnswer,
        QuestionnaireOutcome,
        ReadRefusal,
        ReadingSpan,
        RepositoryLocation,
        RouteAction,
        RouteDecision,
        RouteVerdict,
        ServerReference,
        SessionCatalogChange,
        SessionChange,
        SessionErrorCode,
        SessionListing,
        SessionStatus,
        SkillCatalogStatus,
        SkillPromptDelivery,
        SourceControlAvailability,
        SourceControlCapability,
        SubagentTreeChange,
        TranscriptItem,
        TurnStatus,
        UnreachableReason,
        UsageMetricsKey,
        WatchOutcomeStatus,
        Way,
        WorkspaceUnavailability,
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{TOLERATES_UNKNOWN_KINDS, invented_kinds, pairing_wire};

    /// Every mark in the schema stands for a fallback that reads a kind no
    /// build knows, whichever way the type tells its kinds apart.
    #[test]
    fn every_type_marked_as_reading_an_unknown_kind_reads_one() {
        let wire = pairing_wire();
        let marked: BTreeSet<String> = wire["$defs"]
            .as_object()
            .expect("the schema defines its types")
            .iter()
            .filter(|(_, definition)| definition.get(TOLERATES_UNKNOWN_KINDS).is_some())
            .map(|(name, _)| name.clone())
            .collect();
        let readers = readers();
        let named: BTreeSet<String> = readers.iter().map(|reader| reader.name.clone()).collect();
        let unnamed: Vec<_> = marked.difference(&named).collect();
        assert!(
            unnamed.is_empty(),
            "{unnamed:?} are marked as reading a kind they do not know; name each in \
             crates/shaman-pairing-wire/src/fallbacks.rs"
        );
        let unmarked: Vec<_> = named.difference(&marked).collect();
        assert!(
            unmarked.is_empty(),
            "{unmarked:?} are named as reading a kind they do not know, but nothing marks them"
        );
        for reader in readers {
            let samples = invented_kinds(&wire, &reader.name);
            assert!(
                !samples.is_empty(),
                "{} is marked, and has no kinds to sample",
                reader.name
            );
            for sample in samples {
                if let Err(error) = (reader.read)(sample.clone()) {
                    panic!(
                        "{} is marked as reading a kind it does not know, and refuses {sample}: \
                         {error}",
                        reader.name
                    );
                }
            }
        }
    }
}
