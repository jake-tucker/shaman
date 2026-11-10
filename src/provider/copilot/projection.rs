//! Projection of Copilot's Session event stream onto Suru's attributed Provider events.
//!
//! Copilot reports one long timeline per Session rather than one stream per Turn, and the timeline
//! carries several conversations at once: the main agent's, and one for every subagent the runtime
//! delegates to, whose events ride under the envelope's `agentId`. [`CopilotCorrelation`] is the
//! running state the projection needs: whether a Suru Turn is in flight, the agent Message,
//! Reasoning blocks, and Tool executions each conversation still has open, and the Subagents the
//! lifecycle events have opened. Every event leaves here attributed to the conversation that
//! produced it, so orchestration lands a subagent's work in the Subagent's own child Session
//! rather than the parent's Transcript.
//!
//! The `subagent.*` lifecycle is where Subagents are spawned and their first stretch ends:
//! `subagent.started` opens the Subagent under the instance identity its work is attributed with,
//! and `subagent.completed` or `subagent.failed` — addressed by the spawning tool call — settles
//! that stretch. The spawning tool execution itself projects no row while the Subagent row answers
//! for the delegation ([`SPAWN_TOOL`](super::tools::SPAWN_TOOL)). Main-conversation content
//! outside a Turn Suru is running is dropped unless owed a Continuation; context snapshots can
//! refresh idle Sessions.
//! Events that contradict the recorded state fail the Session, and everything else becomes the
//! Provider events a Session consumes.
//!
//! Only the agent Message is strict about that: a Message the Transcript shows the reader as the
//! answer must be the answer. Reasoning and Tool work report on how the answer was reached, so a
//! Copilot report the projection cannot make sense of costs the reader that report rather than the
//! Turn it belongs to.
//!
//! A Delegation sent through `write_agent` — by the main agent or by a sibling Subagent — reaches
//! the Subagent as a `user.message` under its instance identity, reported when the Subagent
//! consumes it, and attributed to the Agent its `source` names
//! ([`CopilotCorrelation::project_subagent_message`]). Delivered into a working stretch
//! (`delivery: "steering"`) it **steers** that stretch, standing in the Subagent's current Turn at
//! the point it arrives (ADR 0032). The CLIs verified against, 1.0.87 and 1.0.88, never deliver
//! one that way, though: a message sent while the Subagent works is queued until its stretch has
//! ended, and one sent afterwards starts a run at once. Either way Copilot runs the settled agent
//! again on its own context with no second `subagent.started`, which **resumes** it (ADR 0033): a
//! new Turn in the Subagent's Session, opened by the message as its Delegation, whatever the
//! `delivery` says (docs/validation/0397-copilot-subagent-steer.md,
//! docs/validation/0398-copilot-subagent-resume.md). Nothing closes the resumed run either, so it
//! settles where the agent loop exits: the Subagent's `assistant.turn_end` after a model message
//! requesting no tools, with the next message it consumes and the loop's idle as backstops. The
//! `agent_idle` notification Copilot sometimes raises for it settles nothing. The send itself is
//! no work of the sender's, so its `write_agent` execution projects nothing in the sender's
//! Transcript ([`WRITE_AGENT_TOOL`](super::tools::WRITE_AGENT_TOOL)). Nor does a Broker call
//! that spawns, sends to, or stops a Subagent, which the Broker's own rows answer for.
//!
//! Which Activity records a tool execution is decided once, as it starts ([`ToolDisposition`]):
//! a shell execution is a Command, an edit a File Change naming the files it touches, an execution
//! another Activity records or that is Copilot's plumbing is nothing, and every other execution is
//! a Tool Call — named, given its arguments as input, streamed its output, and settled by
//! its completion. A File Change is settled by its completion too, but streams nothing: it records
//! which files changed and how, never what the Tool reported of it.
//!
//! A Suru Turn spans one stretch of Copilot's agentic loop: it opens when the Prompt is delivered
//! and settles on the loop's idle signal, `assistant.idle`, not on the per-model-call
//! `assistant.turn_end` — nor on `session.idle`, which Copilot holds back past the loop's end
//! while attached background work runs, and which therefore only echoes an idle already read
//! ([`drain_session_timeline`]). Idle is the Turn's only settle point, because Copilot emits it
//! mechanically whenever the loop
//! stops — including when it stopped on an error. An error therefore records what the Turn will
//! settle as rather than settling it: a Turn that settled early would leave its own trailing idle
//! to be read against whichever Turn had opened by the time it was projected.
//!
//! The idle speaks only for the main conversation — and for any resumed Subagent run still open,
//! which Copilot closes with nothing else: a spawned Subagent's streams live past it, which is
//! what lets a Subagent outlive the Turn (ADR 0015). Main output arriving after the settle, owed
//! to Subagents still working or just settled, begins a Continuation stretch that the loop's next
//! idle settles — unless a Prompt settles the Continuation first, in which case that stretch's
//! idle is owed nothing and is swallowed rather than read against the Prompt's own Turn.
//!
//! A **background** shell is the Copilot Watch (ADR 0030): the shell tool run with
//! `mode: "async"` — attached to Copilot's process, or with `detach: true` not — returns at once
//! and outlives the loop, and its completion notification wakes the loop again. Its start is read
//! off the Session's task roster rather than the tool call
//! ([`CopilotCorrelation::project_task_roster`]), and its settle off the `shell_completed` or
//! `shell_detached_completed` notification ([`CopilotCorrelation::project_system_notification`])
//! — or, for a shell the loop itself waits out or stops while it works, which raises none, off the
//! roster listing it ended (#379).
//!
//! Copilot compacting a conversation's context is a **Compaction** of that conversation:
//! `session.compaction_start` begins it and `session.compaction_complete` settles it, completed
//! with the `preCompactionTokens` and `postCompactionTokens` it measured and the `summaryContent`
//! it left, or failed with its `error`, so a failed attempt and its retry are two. The `model.*` events of the summarising
//! call, like `session.truncation`, project nothing. Copilot compacts in the background of the
//! Session rather than in a stretch of its loop, so the main conversation's compaction holds the
//! stretch it fell in open past the loop's idle — a steer meanwhile hands the stretch back to the
//! loop it wakes — and one reported while no Turn runs begins a Continuation of its own that its
//! settle settles (ADR 0042, [`CopilotCorrelation::project_compaction`]). Copilot owns any
//! Continuation it compacts in, so stopping it — by interrupt or by the next Prompt — cancels the
//! compaction (`session.history.cancelBackgroundCompaction`), which the loop's abort leaves
//! running. A cancel that finds nothing to cancel came too late: the compaction had already ended,
//! and Copilot's report of how, which may reach Suru after its answer, stands in the stretch the
//! compaction held — and settles that stretch as its loop's idle said, should the interrupt have
//! found nothing else to stop ([`CopilotCorrelation::cancel_answered`]). A Subagent's compaction is followed across its stretches, so one its stretch settled
//! without ends nowhere, least of all in the stretch resuming it, and one Copilot begins while the
//! Subagent works no stretch wakes it into a Continuation of its own Session, which the
//! compaction's end settles.
//!
//! A Compaction the user asks for is Copilot's `session.history.compact`, which runs no stretch of
//! the loop and reports no turn of its own: Suru opens the Turn it is asked in
//! ([`CopilotCorrelation::begin_manual_compaction`]), the same `session.compaction_start` and
//! `session.compaction_complete` report the compaction in it, and Copilot's answer to the request
//! settles it ([`CopilotCorrelation::project_manual_compaction_answer`]). Only reports under the
//! manual trigger Suru asks with decide anything of that Turn: one under another trigger, or none,
//! could be the end of a background compaction Suru stopped following, and records nothing there.
//! The SDK hands answers and events to Suru apart, so the answer waits for the compaction's own
//! end, whatever it says, so that its counts and the `compactionTokensUsed` that is the Turn's
//! Usage land on the Compaction and its Turn — unless Copilot rejected the request outright,
//! before it could compact anything. Only an end that never reaches Suru within the bound every
//! request waits under leaves the Turn to settle on the answer alone; the reports Copilot then
//! still owes would carry the trigger the next manual compaction's do, so the next request is
//! turned away rather than begun while they could still arrive. Stopping it is
//! `session.history.abortManualCompaction`, after which Copilot fails the compaction and the
//! request as cancelled: that is the stop Suru asked for, and settles both as interrupted. It is
//! no background compaction, so neither the background cancel nor the loop's abort ever reaches
//! it.
//!
//! On a Routing Tier the conversation's work arrives staged, and [`routes`] says what stands of it:
//! the first copy of each item, with the Messages of a Route's other phases passed over at its
//! commit (ADR 0069).

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
};

use futures_util::stream;
use github_copilot_sdk::{
    EventSubscription, SessionEvent,
    rpc::{TaskExecutionMode, TaskShellInfo, TaskShellInfoAttachmentMode, TaskStatus},
    session::Session as NativeSession,
    session_events::{
        AssistantIdleData, AssistantMessageData, AssistantMessageDeltaData,
        AssistantMessageStartData, AssistantReasoningData, AssistantReasoningDeltaData,
        AssistantUsageData, AutoTier, AutoTierSwitchFailureReason, ModelCallFailureData,
        ModelCallFailureSource, SessionAutoModeResolvedData, SessionAutoTierSwitchFailedData,
        SessionCompactionCompleteData, SessionErrorData, SessionEventType, SessionIdleData,
        SessionModelChangeData, SubagentCompletedData, SubagentFailedData, SubagentStartedData,
        SystemNotificationData, ToolExecutionCompleteContent, ToolExecutionCompleteData,
        ToolExecutionStartData, ToolShellOutputData, UserMessageData, UserMessageDelivery,
    },
    subscription::RecvErrorKind,
};
use tokio::{
    sync::{Notify, mpsc},
    time::{Duration, timeout},
};

use super::super::command_presentation::PresentedCommand;
use super::{
    COPILOT_FAILURE_FALLBACK, COPILOT_HARNESS_NAME,
    catalog::{AUTO_MODEL_ID, token_count},
    copilot_error,
    event_drain::EventDrainCheckpoint,
    model_evidence::WorkingModels,
    pricing::CopilotPricing,
    reported,
    routes::{self, Routes},
    session::{running_on, until_crash},
    skills::CopilotSkills,
    tools::{PresentedToolCall, ToolDisposition, presented_tool_call, tool_activity_id},
    transport::CopilotConnection,
};
use crate::protocol::{
    AgentSelection, ContextFill, Cost, ModelAttribution, ModelId, ModelOptionChoiceId,
    ModelOptionId, ModelOptionSelection, ModelOptionValue, NativeMeter, ProviderId,
    ROUTING_TIER_OPTION_ID, TurnId, Usage,
};
use crate::provider::{
    AttributedProviderEvent, ContextFillReport, ProviderActivityId, ProviderCommandStatus,
    ProviderError, ProviderEvent, ProviderEventAttribution, ProviderEventStream,
    ProviderFileChangeStatus, ProviderMessageId, ProviderSubagentId, ProviderSubagentStatus,
    ProviderToolCallStatus, ProviderWatchId, ProviderWatchOutcome, ReportedMetering,
    actual_model::{self, SelectedModel},
    concise_remote_message, exclusive_count, first_line,
    harness::SharedHarnessHandle,
    reasoning::{ReasoningSegment, ReasoningSummarySplitter},
    reported_count,
};

/// Everything the projection must remember between events for one Copilot Session.
pub(super) struct CopilotCorrelation {
    /// Where the Session works, which is where a relative path a Tool names a file by is looked up.
    execution_directory: PathBuf,
    /// The Suru Turn the main conversation's events currently belong to — the Turn a Prompt
    /// began, or the Continuation stretch late output opened.
    turn: Option<ActiveTurn>,
    /// Call IDs already observed in settled main-conversation stretches.
    previous_model_calls: HashSet<String>,
    /// Loop stretches whose idle is still to come after Suru stopped owning them: a Prompt
    /// settles a Continuation early (ADR 0015), and the stretch it cut off still ends with an
    /// idle of its own. Each owed idle is swallowed rather than read against the Turn that
    /// Prompt began.
    stale_stretches: u32,
    /// Each working Subagent's current stretch and the streams it has open, by the agent instance
    /// identity its events are attributed with.
    subagents: HashMap<String, WorkingSubagent>,
    /// The Subagent each spawning tool call opened: `subagent.completed` and `subagent.failed`
    /// name the spawn's tool call rather than the instance their own envelope names.
    spawns: HashMap<String, String>,
    /// Every spawning tool call whose `subagent.started` has arrived, kept past the Subagent's
    /// settle: the Subagent row answers for the delegation, so the spawn's own execution stays
    /// withheld however late it is reported — before its `subagent.started` or after the settle.
    delegations: HashSet<String>,
    /// Every Subagent instance ever opened on this timeline, kept past its settle: a message
    /// Copilot delivers to a settled Subagent resumes it (ADR 0033), and names the Agent that sent
    /// it by instance identity — a sibling that may have settled before the Subagent it wrote to
    /// received what it sent.
    agents: HashMap<String, KnownSubagent>,
    /// Whether a Subagent has settled since a Turn last began — or a background shell's completion
    /// woke the loop while no Turn was running. The output a completion provokes arrives only
    /// after the settle, so every settle leaves a Continuation owed to whatever that output turns
    /// out to be; a Turn beginning clears it, because from then on such output has a Turn to
    /// land in.
    late_settle_owes_continuation: bool,
    /// The background shells live as Watches, by the shell identity Copilot's completion
    /// notification names them by. All are the owning Session's: the task roster says nothing of
    /// which conversation ran the shell tool, and Copilot delivers the completion to the main
    /// loop.
    watches: HashSet<String>,
    /// Every background shell ever started as a Watch on this timeline, kept past its settle: the
    /// roster keeps listing a shell for a while after it ends, and a read racing the completion
    /// could otherwise start the settled shell's Watch a second time.
    watched_shells: HashSet<String>,
    /// Copilot's most recently published per-Model prices. The connection
    /// updates this on catalog discovery; each usage event reads it once while
    /// the call still belongs to an active Turn.
    pricing: CopilotPricing,
    /// Last Prompt delivered to this native timeline. Retained through idle so context
    /// observations can refresh a settled Session without opening a Continuation.
    context_turn: Option<TurnId>,
    context_sequence: u64,
    /// The main conversation's latest `tokenLimit`, which a Context Breakdown measures against:
    /// Copilot's context attribution names no window of its own Suru can trust.
    served_window: Option<u64>,
    /// The latest projected stretch is a Continuation rather than the captured Prompt.
    context_continuation: bool,
    /// Admission has begun but may fail before delivery; old Continuations cannot
    /// rebind observations to this new (possibly Model-invalidated) Turn.
    context_prompt_pending: bool,
    /// Where Copilot is with compacting the main conversation's context.
    compaction: Compacting,
    /// Counts the main conversation's stretches, so an interrupt still at work once the stretch it
    /// was for has settled stops nothing begun after it.
    stretches: u64,
    /// Raised when the timeline carries Copilot's next idle, for an interrupt waiting on the idle
    /// its abort ends in.
    abort_idle: Option<Arc<Notify>>,
    /// How many ends of a background compaction of the main conversation the timeline has
    /// carried to the projection's queue, and how many of those the projection has read: one
    /// carried and not yet read is on its way, ahead of whatever the timeline carries after it.
    compaction_ends_carried: u64,
    compaction_ends_read: u64,
    /// Raised when the timeline carries the end of a background compaction, for an interrupt
    /// waiting on the end of one its cancel found already over ([`Compacting::Ended`]).
    compaction_end: Option<Arc<Notify>>,
    /// An abort an interrupt gave up waiting on still owes the aborted idle it ends in, which
    /// belongs to the stretch the interrupt settled and so is swallowed wherever it lands. Copilot's
    /// idle names no run, so this is a count of one rather than a match: a genuine idle of a later
    /// Turn is never taken for it, since only an aborted idle is swallowed, and an aborted idle
    /// follows only an abort Suru sends — each of which forgives the debt before it goes out.
    owes_aborted_idle: bool,
    /// The Agent Selection the Copilot Session runs under, which a Continuation Copilot owns
    /// begins under.
    selection: AgentSelection,
    /// The Selection a tier Copilot refused left in force, until the Session has taken it in.
    tier_correction: Option<AgentSelection>,
    /// Copilot still owes reports of a manual compaction whose Turn settled on its answer alone.
    /// They carry the manual trigger a new manual compaction's would, so none begins until they
    /// are in — or until a stretch of the loop has run to its idle since, which every report
    /// Copilot wrote before that answer has reached Suru ahead of.
    manual_reports_owed: bool,
    /// The Routes of the stretch running, and of any stretch a Prompt cut off under them.
    routes: Routes,
}

/// Where Copilot is with compacting one conversation's context. It compacts in the background of
/// the Session rather than in a stretch of the conversation's loop, so a compaction can outlive
/// the stretch it began in — the main loop going idle, or a Subagent's stretch settling — and can
/// run while none does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Compacting {
    #[default]
    Idle,
    /// Copilot is summarising, and its `session.compaction_complete` is still to come.
    Running,
    /// Copilot is summarising, and an interrupt has asked it to stop: until Copilot answers, the
    /// compaction still holds its stretch open, and the interrupt settles that stretch.
    Cancelling,
    /// Copilot reported the compaction failing while the interrupt's cancel was still out. Only
    /// Copilot's answer says whether that failure is the cancel taking or the compaction's own, so
    /// the report is kept, and the stretch held, until it does.
    FailedWhileCancelling { error: Option<String> },
    /// The cancel found nothing to cancel, so the failure Copilot reported while it was out was
    /// the compaction's own: its Compaction settles failed, with Copilot's error, ahead of the
    /// stretch the interrupt stops ([`CopilotCorrelation::settle_turn`]).
    Failed { error: Option<String> },
    /// The cancel found nothing to cancel before Copilot's `session.compaction_complete` reached
    /// Suru: Copilot had already ended the compaction, and that report, still on its way, is its
    /// outcome. It is followed still, and holds the stretch it stands in until it lands there —
    /// settling that stretch too, as its loop's idle said, when `settles`: once the interrupt
    /// has left the stretch to it rather than to the idle of an abort it sends. A report that
    /// never comes within the interrupt's wait leaves it [`Self::Stopped`]
    /// ([`CopilotCorrelation::cancel_answered`]).
    Ended { settles: bool },
    /// Suru stopped following the compaction — cancelled it, or the stretch it stood in settled
    /// without it — and its Compaction settled with that stretch. Its
    /// `session.compaction_complete` is owed nothing, so it records nothing in whatever stretch
    /// runs by the time it arrives — unless another compaction starts first, which is read as the
    /// stopped one's end never coming. That ordering is assumed rather than verified against a live
    /// CLI: Copilot is taken to run one compaction of a conversation at a time and to report its
    /// end before the next one's start.
    Stopped,
}

impl Compacting {
    /// Whether the compaction still holds the stretch it stands in open.
    fn holds_stretch(&self) -> bool {
        matches!(
            self,
            Self::Running
                | Self::Cancelling
                | Self::FailedWhileCancelling { .. }
                | Self::Ended { .. }
        )
    }

    /// The compaction the stretch it stood in settled without: its end is owed nothing, or has
    /// already come.
    fn outlived(&mut self) {
        match self {
            Self::Running | Self::Cancelling | Self::Ended { .. } => *self = Self::Stopped,
            Self::FailedWhileCancelling { .. } | Self::Failed { .. } => *self = Self::Idle,
            Self::Idle | Self::Stopped => {}
        }
    }

    /// Reads a compaction report, returning whether it is one Suru follows: a start always is,
    /// and an end is unless it ends a compaction Suru stopped following.
    fn follows(&mut self, started: bool) -> bool {
        let before = std::mem::replace(self, if started { Self::Running } else { Self::Idle });
        started || before != Self::Stopped
    }
}

/// One stretch of Copilot's loop that Suru reads as a Turn, and the main conversation's state
/// within it.
struct ActiveTurn {
    /// Whether this Turn is a Continuation — a stretch the loop ran on its own, delivering what
    /// a Subagent's completion provoked, rather than one a Prompt began. The next delivered
    /// Prompt settles it rather than steering it (ADR 0015).
    continuation: bool,
    streams: ConversationStreams,
    /// What Copilot reported going wrong inside this Turn, which is what it settles as once the
    /// loop goes idle. The first report wins, because the failures after it are its consequences.
    failure: Option<String>,
    /// The idle the loop reported while Copilot was still compacting — `Some(aborted)` — which
    /// the stretch settles on once the compaction does. A Continuation a compaction began has its
    /// loop idle from the start. Loop output arriving, or a steer about to wake the loop, clears
    /// it: the loop is running again, and its own next idle is the stretch's.
    idled: Option<bool>,
    /// Whether Copilot owns this Continuation — it was reported with
    /// [`ProviderEvent::ContinuationStarted`] — so that the next Prompt or an interrupt stops what
    /// Copilot runs in it rather than Suru settling it alone.
    owned: bool,
    /// The manual compaction this Turn was begun for (ADR 0041), which Copilot runs as
    /// `session.history.compact` rather than a stretch of its loop: no idle ends it, and Copilot's
    /// answer to the request settles it. Nothing for any other Turn.
    manual: Option<ManualCompactionRun>,
}

impl ActiveTurn {
    fn new() -> Self {
        Self {
            continuation: false,
            streams: ConversationStreams::default(),
            failure: None,
            idled: None,
            owned: false,
            manual: None,
        }
    }

    fn continuation() -> Self {
        Self {
            continuation: true,
            ..Self::new()
        }
    }
}

/// Where a manual compaction Copilot runs for `session.history.compact` stands, as its reports,
/// Suru's abort of it and Copilot's answer have said.
#[derive(Debug)]
struct ManualCompactionRun {
    /// The Turn the request began, which Copilot's answer is matched to.
    turn_id: TurnId,
    reported: ManualReport,
    /// Copilot's account of why the compaction failed, from its `session.compaction_complete`,
    /// which the request's answer does not carry.
    failure: Option<String>,
    /// Copilot answered Suru's `session.history.abortManualCompaction` saying it aborted the
    /// compaction, so a failure from here on is that abort.
    aborted: bool,
    /// Copilot's answer to the request, held while the compaction's own end has still to reach
    /// Suru.
    answer: Option<ManualCompactionAnswer>,
}

impl ManualCompactionRun {
    fn new(turn_id: TurnId) -> Self {
        Self {
            turn_id,
            reported: ManualReport::Unreported,
            failure: None,
            aborted: false,
            answer: None,
        }
    }

    /// Whether Copilot's answer settles the Turn now rather than once the compaction's own end
    /// has reached Suru: only when that end is in, or when Copilot rejected the request outright,
    /// before it could compact anything. Any other answer may still have reports on their way,
    /// which the Turn waits for, however the answer reads.
    fn settles_on(&self, answer: &ManualCompactionAnswer) -> bool {
        match self.reported {
            ManualReport::Ended => true,
            ManualReport::Running => false,
            ManualReport::Unreported => matches!(
                answer,
                ManualCompactionAnswer::NotCompacted { rejected: true, .. }
            ),
        }
    }

    /// Whether Copilot still owes reports of this compaction once its Turn settles: the end of
    /// one it reported starting, or both reports of one it answered it made.
    fn owes_reports(&self) -> bool {
        match self.reported {
            ManualReport::Ended => false,
            ManualReport::Running => true,
            ManualReport::Unreported => {
                matches!(self.answer, Some(ManualCompactionAnswer::Compacted { .. }))
            }
        }
    }
}

/// How far Copilot has reported a manual compaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualReport {
    Unreported,
    Running,
    Ended,
}

/// Copilot's answer to `session.history.compact`, which settles the Turn the request runs in.
#[derive(Debug)]
pub(super) enum ManualCompactionAnswer {
    /// Copilot compacted the Session's context, leaving `summary` where its answer carried one.
    Compacted { summary: Option<String> },
    /// Copilot compacted nothing: it failed, or an abort cancelled it. `error` is its account of
    /// why, where the answer carried one. `rejected` says Copilot rejected the request outright,
    /// as a request it could not take, before it could compact anything or report doing so.
    NotCompacted {
        error: Option<String>,
        rejected: bool,
    },
}

/// Where Copilot's answer to a manual compaction joins the Session's timeline. Holds the timeline
/// only weakly, so the timeline still ends when Copilot's does.
#[derive(Clone)]
pub(super) struct ManualCompactionAnswers {
    timeline: mpsc::WeakUnboundedSender<Result<TimelineEvent, ProviderError>>,
}

impl ManualCompactionAnswers {
    /// Puts `answer`, Copilot's answer to the manual compaction `turn_id` was begun for, on the
    /// timeline. The SDK hands answers and events to Suru apart, so the compaction's own reports
    /// may still be on their way: should the Turn still be waiting on them `bound` on, the
    /// timeline then says so, so that the Turn settles on the answer.
    pub(super) async fn answer(
        &self,
        turn_id: TurnId,
        answer: ManualCompactionAnswer,
        bound: Duration,
    ) {
        if !self.send(TimelineEvent::ManualCompactionAnswered { turn_id, answer }) {
            return;
        }
        tokio::time::sleep(bound).await;
        self.send(TimelineEvent::ManualCompactionOverdue { turn_id });
    }

    /// Puts `event` on the timeline, answering whether the timeline was still there to take it.
    fn send(&self, event: TimelineEvent) -> bool {
        self.timeline
            .upgrade()
            .is_some_and(|timeline| timeline.send(Ok(event)).is_ok())
    }
}

/// One stretch of a Subagent's work still going: how it began, which says what settles it, and what
/// the Subagent's conversation still has open in it.
struct WorkingSubagent {
    stretch: Stretch,
    streams: ConversationStreams,
    /// What this stretch has shown of any Route it stages, which tells its committed copies from
    /// new work and speaks for nothing else: its owner's Turn answers only to its own.
    routes: Routes,
}

impl WorkingSubagent {
    fn new(stretch: Stretch) -> Self {
        Self {
            stretch,
            streams: ConversationStreams::default(),
            routes: Routes::default(),
        }
    }
}

/// How a working Subagent's stretch began.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stretch {
    /// Its spawn, which Copilot settles itself with `subagent.completed` or `subagent.failed`.
    Spawned,
    /// A resume (ADR 0033), which Copilot opens with the message it delivers and closes with
    /// nothing, so it settles where the agent loop exits: at the `assistant.turn_end` that follows
    /// a model message requesting no tools. `answered` records that the latest model message
    /// requested none, which makes the next turn end the exit.
    Resumed { answered: bool },
    /// A compaction Copilot began while the Subagent worked no stretch: a Continuation of the
    /// Subagent's own Session, in which no loop runs, which the compaction ending settles.
    Compacting,
}

impl Stretch {
    fn is_resumed(self) -> bool {
        matches!(self, Self::Resumed { .. })
    }

    /// Whether the next turn end is where the stretch's loop exits: only a resume's, and only
    /// once its latest model message requested no tools.
    fn exits_at_turn_end(self) -> bool {
        self == Self::Resumed { answered: true }
    }

    /// Records what the latest model message asked of the loop. A spawn keeps no such record:
    /// Copilot settles it itself.
    fn answer(&mut self, no_tools: bool) {
        if let Self::Resumed { answered } = self {
            *answered = no_tools;
        }
    }
}

/// What the projection keeps of a Subagent past its settle, for the resume that may run it again:
/// the name its rows carry, what its spawn was asked to do — the description of a resume whose
/// message carries nothing to read — and the Model it last ran on, which Copilot reports only at
/// spawn and which carries into the resumed Turn.
struct KnownSubagent {
    name: String,
    description: String,
    model: Option<String>,
    auto_selection: bool,
    previous_model_calls: HashSet<String>,
    /// Where Copilot is with compacting the Subagent's context, followed across its stretches: a
    /// compaction its stretch settled without must not end in the stretch that resumes it.
    compaction: Compacting,
}

/// What one conversation on the timeline still has open — the main agent's inside its Turn, or a
/// Subagent's for as long as the Subagent runs.
#[derive(Default)]
struct ConversationStreams {
    message: Option<ActiveMessage>,
    working_models: WorkingModels,
    /// The Reasoning blocks the conversation has open, in the order Copilot opened them. Copilot's
    /// loop reasons one block through at a time, so this is a short list rather than a
    /// map, and settling the conversation walks it in the order a reader met it.
    reasoning: Vec<ActiveReasoning>,
    /// The tool executions the conversation is still running, by the tool call identity Copilot
    /// gave each. Copilot runs several Tools at once, so a conversation holds as many as it
    /// started.
    tools: HashMap<String, RunningTool>,
    /// The cumulative reading for this conversation's one Suru Turn. Copilot
    /// emits one usage event per model call and gives it no Turn identity, so
    /// this state lives exactly as long as the surrounding Turn streams do.
    metering: Option<ReportedMetering>,
    observed_model_calls: HashSet<String>,
    recorded_model_calls: HashSet<String>,
    fixed_model_calls: HashSet<String>,
    call_auto_selections: HashMap<String, bool>,
    reported_model_calls: HashMap<String, ModelAttribution>,
    pending_model_messages: HashMap<String, Vec<PendingModelMessage>>,
    attributed_model_messages: HashMap<String, Vec<ProviderMessageId>>,
    anonymous_fixed_call: bool,
    anonymous_attributed_messages: Vec<ProviderMessageId>,
    identified_messages: HashSet<ProviderMessageId>,
}

impl ConversationStreams {
    fn observe_model_call(
        &mut self,
        id: Option<&str>,
        previous: &HashSet<String>,
        selected_auto: bool,
    ) -> bool {
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            return true;
        };
        if previous.contains(id) {
            return false;
        }
        self.observed_model_calls.insert(id.to_owned());
        self.call_auto_selections
            .entry(id.to_owned())
            .or_insert(selected_auto);
        true
    }

    fn call_selected_auto(&self, id: Option<&str>, fallback: bool) -> bool {
        id.and_then(|id| self.call_auto_selections.get(id).copied())
            .unwrap_or(fallback)
    }
}

struct PendingModelMessage {
    message: ProviderMessageId,
    actual: Option<ModelId>,
}

/// The agent Message Copilot is still streaming, and the text it has carried so far — against
/// which the completed Message's repeat of it is reconciled.
struct ActiveMessage {
    message_id: String,
    streamed: String,
}

/// A tool execution Copilot is still running, by what records it — and for the kinds a row records,
/// the output it has streamed so far, against which the completed execution's repeat of it is
/// reconciled.
enum RunningTool {
    /// A shell execution, recorded as a Command.
    Command { streamed_output: String },
    /// A file tool's execution, recorded as a File Change, which keeps none of its output.
    FileChange,
    /// Every other execution no more specific Activity records, recorded as a Tool Call.
    ToolCall { streamed_output: String },
    /// A [`SPAWN_TOOL`](super::tools::SPAWN_TOOL) execution, whose Tool Call is held back while
    /// the Subagent row answers for the delegation. A spawn that never opens its Subagent has no
    /// row answering for it, so the withheld Tool Call surfaces when the execution settles or the
    /// conversation stops — a failed delegation stays visible.
    Spawn {
        withheld: PresentedToolCall,
        /// What the execution hands the Subagent it spawns — the tool's `prompt` argument — kept
        /// for the `subagent.started` that opens it, which names the spawning tool call but never
        /// carries the prompt itself.
        prompt: Option<String>,
        streamed_output: String,
    },
    /// An execution another Activity records, or Copilot's plumbing, which projects nothing at
    /// all, whatever it came to.
    Unrecorded,
}

/// A Reasoning block Copilot still has open: the text it has streamed so far — against
/// which the completed block's repeat of it is reconciled — and the split holding its title back
/// until the head of the block resolves into one.
#[derive(Default)]
struct ActiveReasoning {
    reasoning_id: String,
    streamed: String,
    splitter: ReasoningSummarySplitter,
}

impl CopilotCorrelation {
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::working_in(
            std::env::temp_dir(),
            CopilotPricing::default(),
            tests::selection(),
        )
    }

    /// The projection for a Session working in `execution_directory`, costing its usage at
    /// `pricing`, under `selection`.
    pub(super) fn working_in(
        execution_directory: PathBuf,
        pricing: CopilotPricing,
        selection: AgentSelection,
    ) -> Self {
        Self {
            execution_directory,
            turn: None,
            previous_model_calls: HashSet::new(),
            stale_stretches: 0,
            subagents: HashMap::new(),
            spawns: HashMap::new(),
            delegations: HashSet::new(),
            agents: HashMap::new(),
            late_settle_owes_continuation: false,
            watches: HashSet::new(),
            watched_shells: HashSet::new(),
            pricing,
            context_turn: None,
            context_sequence: 0,
            served_window: None,
            context_continuation: false,
            context_prompt_pending: false,
            compaction: Compacting::Idle,
            stretches: 0,
            abort_idle: None,
            compaction_ends_carried: 0,
            compaction_ends_read: 0,
            compaction_end: None,
            owes_aborted_idle: false,
            selection,
            tier_correction: None,
            manual_reports_owed: false,
            routes: Routes::default(),
        }
    }

    /// Opens the Turn a Prompt is about to be delivered into. Copilot hosts one agentic loop per
    /// Session, so a second Turn cannot begin while a prompted one is running — but a Prompt
    /// delivered while a Continuation runs settles that Continuation rather than steering it
    /// (ADR 0015), so the stretch it cuts off becomes a stale one whose idle is owed nothing —
    /// unless its loop is already idle, and only a compaction held it open. A compaction the
    /// Continuation held settled with it, so Copilot's report of that one ending is owed nothing
    /// either.
    pub(super) fn begin_turn(&mut self) -> Result<(), ProviderError> {
        match self.turn.take() {
            None => {}
            Some(stretch) if stretch.continuation => {
                // Orchestration already settled the Continuation and the store settles whatever
                // its streams left open, so the stretch's state goes with it.
                if stretch.idled.is_none() {
                    self.stale_stretches += 1;
                }
                self.previous_model_calls
                    .extend(stretch.streams.observed_model_calls);
                self.compaction.outlived();
            }
            Some(turn) => {
                self.turn = Some(turn);
                return Err(copilot_error(
                    "Copilot started a Turn while another Turn was active",
                ));
            }
        }
        self.context_continuation = false;
        self.context_prompt_pending = true;
        self.stretches += 1;
        self.turn = Some(ActiveTurn::new());
        self.late_settle_owes_continuation = false;
        Ok(())
    }

    /// Bind observations at receipt, before the projection queue can fall behind a
    /// later Prompt or Model change. Startup reports during selection still belong
    /// to the previous Turn until the new Prompt is ready to be sent, by which time
    /// `selection` is in force on the Copilot Session.
    pub(super) fn context_prompt_ready(&mut self, turn_id: TurnId, selection: AgentSelection) {
        self.context_prompt_pending = false;
        self.context_turn = Some(turn_id);
        self.context_continuation = false;
        // A window belongs to the Selection it was served under; until Copilot reports one
        // under the new Selection, none is known.
        if selection != self.selection {
            self.served_window = None;
        }
        self.selection = selection;
    }

    /// Copilot refused the tier this Turn claimed, leaving another in force. The Selection says so
    /// from here on, and the Session learns of it before it next asks for a Selection.
    pub(super) fn project_tier_refused(
        &mut self,
        refusal: SessionAutoTierSwitchFailedData,
    ) -> Vec<AttributedProviderEvent> {
        let requested = refusal.requested_auto_tier.as_ref().map_or_else(
            || "provider-default routing".to_owned(),
            |tier| format!("Routing Tier `{}`", tier.as_str()),
        );
        let effective = refusal
            .effective_auto_tier
            .as_ref()
            .filter(|tier| **tier != AutoTier::Unknown)
            .map(|tier| tier.as_str().to_owned());
        let why = format!(
            "Copilot refused {requested} ({}). This Turn runs on {}.",
            switch_failure_words(&refusal.reason),
            running_on(effective.as_deref()),
        );
        let mut corrected = self.selection.clone().without_routing_tier();
        corrected
            .options
            .extend(effective.map(|tier| ModelOptionSelection {
                id: ModelOptionId::new(ROUTING_TIER_OPTION_ID),
                value: ModelOptionValue::Select {
                    choice: ModelOptionChoiceId::new(tier),
                },
            }));
        self.selection = corrected.clone();
        self.tier_correction = Some(corrected.clone());
        if self.turn.is_none() {
            return Vec::new();
        }
        vec![attributed(
            None,
            ProviderEvent::AgentSelectionCorrected {
                selection: corrected,
                why,
            },
        )]
    }

    /// The Selection a refused tier left in force, once, if one did since this was last asked.
    pub(super) fn take_tier_correction(&mut self) -> Option<AgentSelection> {
        self.tier_correction.take()
    }

    /// The window the main conversation's Model is served at, as Copilot last reported it under
    /// the Selection in force.
    pub(super) fn served_window(&self) -> Option<u64> {
        self.served_window
    }

    fn context_report(&mut self, event: &SessionEvent) -> Option<AttributedProviderEvent> {
        if event.parsed_type() != SessionEventType::SessionUsageInfo {
            return None;
        }
        // tokenLimit is the window the Session's Model is served at, not the catalog's larger
        // raw window Copilot never lets a prompt reach.
        // See docs/validation/0299-copilot-context-fill.md.
        let capacity_tokens = event
            .data
            .get("tokenLimit")
            .and_then(serde_json::Value::as_u64)
            .filter(|capacity| *capacity > 0);
        if event.agent_id.is_none() && capacity_tokens.is_some() {
            self.served_window = capacity_tokens;
        }
        let occupied_tokens = event.data.get("currentTokens")?.as_u64()?;
        let turn_id = if event.agent_id.is_some() {
            // Child Turn IDs are assigned by orchestration, under the native agent ID.
            None
        } else {
            Some(self.context_turn?)
        };
        self.context_sequence = self.context_sequence.saturating_add(1);
        Some(attributed(
            event.agent_id.as_deref(),
            ProviderEvent::ContextFill {
                report: ContextFillReport {
                    turn_id,
                    sequence: self.context_sequence,
                    fill: ContextFill {
                        occupied_tokens,
                        capacity_tokens,
                    },
                },
            },
        ))
    }

    /// Gives up the Turn opened by [`Self::begin_turn`] when the Prompt never reached Copilot.
    pub(super) fn abandon_turn(&mut self) {
        if let Some(turn) = self.turn.take() {
            self.previous_model_calls
                .extend(turn.streams.observed_model_calls);
        }
        // Readiness precedes native.send, which can still reject the Prompt.
        // Neither queued nor later observations may claim that failed delivery.
        self.context_turn = None;
        self.context_continuation = false;
        self.context_prompt_pending = true;
    }

    /// Opens the Turn a Compaction request began (ADR 0041), which Copilot runs as
    /// `session.history.compact`. Suru asks only while the Session is idle, so no Turn may be
    /// running: Copilot would take the compaction mid-Turn, report success, and lose it. Context
    /// readings bind to it once its Agent Selection is in force, as a Prompt's Turn's do
    /// ([`Self::context_prompt_ready`]).
    ///
    /// Nor does one begin while Copilot still owes the reports of an earlier manual compaction,
    /// which would be taken for this one's: the request is turned away, saying so.
    pub(super) fn begin_manual_compaction(&mut self, turn_id: TurnId) -> Result<(), ProviderError> {
        if self.turn.is_some() {
            return Err(copilot_error(
                "Copilot was asked to compact while a Turn was active",
            ));
        }
        if self.manual_reports_owed {
            return Err(copilot_error(
                "Copilot compaction refused: Copilot has yet to report an earlier compaction it \
                 answered, and that report could be taken for this one's; ask again once it has, \
                 or after the Agent's next answer",
            ));
        }
        self.context_continuation = false;
        self.context_prompt_pending = true;
        self.stretches += 1;
        self.turn = Some(ActiveTurn {
            manual: Some(ManualCompactionRun::new(turn_id)),
            ..ActiveTurn::new()
        });
        self.late_settle_owes_continuation = false;
        Ok(())
    }

    /// Whether the running Turn is a manual compaction's, which an interrupt aborts as one.
    pub(super) fn is_compacting_on_request(&self) -> bool {
        self.turn.as_ref().is_some_and(|turn| turn.manual.is_some())
    }

    /// Copilot answered Suru's abort of the manual compaction: `aborted` says it found one to
    /// abort. One it did not find had already ended, and its answer settles the Turn as it ended.
    pub(super) fn manual_compaction_aborted(&mut self, aborted: bool) {
        if let Some(manual) = self.turn.as_mut().and_then(|turn| turn.manual.as_mut()) {
            manual.aborted = aborted;
        }
    }

    /// Projects one of Copilot's reports on compacting the main conversation, as whichever of its
    /// two operations it belongs to. Suru asks for a manual compaction with the manual trigger,
    /// and Copilot reports it under that trigger; anything else it compacts is a background
    /// compaction ([`Self::project_compaction`]). Only a report under the manual trigger decides
    /// anything of a manual compaction's Turn: one under another trigger, or none, could be the
    /// end of a background compaction Suru stopped following — which it then is, owed nothing —
    /// and Copilot runs no other compaction of the conversation meanwhile. A report under the
    /// manual trigger with no manual compaction's Turn running belongs to one that has already
    /// settled, and is owed nothing either.
    fn project_main_compaction_report(
        &mut self,
        event: &SessionEvent,
    ) -> Vec<AttributedProviderEvent> {
        let started = event.parsed_type() == SessionEventType::SessionCompactionStart;
        let trigger = event
            .data
            .get("trigger")
            .and_then(serde_json::Value::as_str);
        let manual = trigger == Some(MANUAL_TRIGGER);
        if !started && !manual {
            self.compaction_ends_read += 1;
        }
        if self.is_compacting_on_request() {
            if manual {
                return self.project_manual_compaction(event);
            }
            if self.compaction == Compacting::Stopped && !started {
                self.compaction = Compacting::Idle;
            } else {
                tracing::warn!(
                    trigger,
                    "Copilot reported a compaction under no manual trigger while it ran a manual one"
                );
            }
            return Vec::new();
        }
        if manual {
            if !started {
                self.manual_reports_owed = false;
            }
            return Vec::new();
        }
        self.project_compaction(compaction_event(event))
    }

    /// Projects one of Copilot's reports on the manual compaction the running Turn was begun for:
    /// its start, and its end, with what the summarising call spent as the Turn's Usage — and,
    /// once Copilot has answered the request, the Turn's settle.
    fn project_manual_compaction(&mut self, event: &SessionEvent) -> Vec<AttributedProviderEvent> {
        let Some(turn) = self.turn.as_mut().filter(|turn| turn.manual.is_some()) else {
            return Vec::new();
        };
        let manual = turn
            .manual
            .as_mut()
            .expect("a manual compaction's Turn holds it");
        let reported = compaction_event(event);
        match &reported {
            ProviderEvent::CompactionStarted => manual.reported = ManualReport::Running,
            ProviderEvent::CompactionFailed { error } => {
                manual.reported = ManualReport::Ended;
                manual.failure.clone_from(error);
            }
            _ => manual.reported = ManualReport::Ended,
        }
        let ended = manual.reported == ManualReport::Ended;
        let answered = manual.answer.is_some();
        let mut projected = vec![attributed(None, reported)];
        if ended {
            if let Some(usage) = compaction_usage(event, &self.pricing, &mut turn.streams) {
                projected.push(attributed(None, usage));
            }
            if answered {
                projected.extend(self.settle_manual_compaction());
            }
        }
        projected
    }

    /// Takes Copilot's answer to the manual compaction `turn_id` was begun for, which settles its
    /// Turn — at once, unless the compaction's own end has still to reach Suru
    /// ([`ManualCompactionRun::settles_on`]); then that end settles it. An answer for a Turn no
    /// longer running is owed nothing.
    fn project_manual_compaction_answer(
        &mut self,
        turn_id: TurnId,
        answer: ManualCompactionAnswer,
    ) -> Vec<AttributedProviderEvent> {
        let Some(manual) = self.running_manual_compaction(turn_id) else {
            return Vec::new();
        };
        let settles = manual.settles_on(&answer);
        manual.answer = Some(answer);
        if settles {
            self.settle_manual_compaction()
        } else {
            Vec::new()
        }
    }

    /// The manual compaction `turn_id` was begun for answered a bound ago, and its own end has
    /// still not reached Suru: its Turn settles on the answer alone, and the reports, should they
    /// come, are owed nothing ([`Self::begin_manual_compaction`]).
    fn project_manual_compaction_overdue(
        &mut self,
        turn_id: TurnId,
    ) -> Vec<AttributedProviderEvent> {
        let Some(manual) = self.running_manual_compaction(turn_id) else {
            return Vec::new();
        };
        if manual.answer.is_none() {
            return Vec::new();
        }
        tracing::warn!(
            reported = ?manual.reported,
            "Copilot's report of a manual compaction it answered never reached Suru; its Turn \
             settles on the answer, without the counts and Usage the report would carry"
        );
        self.settle_manual_compaction()
    }

    /// The manual compaction the running Turn was begun for, if it is `turn_id`'s.
    fn running_manual_compaction(&mut self, turn_id: TurnId) -> Option<&mut ManualCompactionRun> {
        self.turn
            .as_mut()
            .and_then(|turn| turn.manual.as_mut())
            .filter(|manual| manual.turn_id == turn_id)
    }

    /// Settles the manual compaction's Turn on Copilot's answer to the request. A compaction it
    /// answered it made whose end never reached Suru completes as the answer says. One that did
    /// not compact fails the Turn with Copilot's account of why — or stops it, when Suru's abort
    /// is what cancelled it.
    fn settle_manual_compaction(&mut self) -> Vec<AttributedProviderEvent> {
        let mut turn = self
            .turn
            .take()
            .expect("a manual compaction's Turn is running");
        let manual = turn
            .manual
            .take()
            .expect("a manual compaction's Turn holds it");
        self.manual_reports_owed = manual.owes_reports();
        let mut projected = Vec::with_capacity(2);
        let outcome = match manual
            .answer
            .expect("a manual compaction's Turn settles on Copilot's answer")
        {
            ManualCompactionAnswer::Compacted { summary } => {
                if manual.reported != ManualReport::Ended {
                    projected.push(ProviderEvent::CompactionCompleted {
                        before_tokens: None,
                        after_tokens: None,
                        summary,
                    });
                }
                ProviderEvent::TurnCompleted
            }
            ManualCompactionAnswer::NotCompacted { .. } if manual.aborted => {
                ProviderEvent::TurnInterrupted
            }
            ManualCompactionAnswer::NotCompacted { error, .. } => ProviderEvent::TurnFailed {
                message: manual
                    .failure
                    .or(error)
                    .unwrap_or_else(|| NOT_COMPACTED.to_owned()),
            },
        };
        let reasoning = if matches!(outcome, ProviderEvent::TurnCompleted) {
            OpenReasoning::Complete
        } else {
            OpenReasoning::Release
        };
        projected.extend(settle_open_streams(
            &mut turn.streams,
            &self.delegations,
            reasoning,
        ));
        projected.push(outcome);
        projected
            .into_iter()
            .map(|settled| attributed(None, settled))
            .collect()
    }

    /// Whether a Turn is running, which is what makes a Prompt delivered now a steer rather than
    /// the start of another Turn, and what there is for an interrupt to stop.
    pub(super) fn is_turn_running(&self) -> bool {
        self.turn.is_some()
    }

    /// Whether a Route is running, whose phase Copilot carries on with after a Session disconnects
    /// and only an abort stops (docs/validation/copilot-fusion-routes.md).
    pub(super) fn is_route_running(&self) -> bool {
        self.turn.is_some() && self.routes.is_running()
    }

    /// Projects one of Copilot's reports on a Route, in the stretch it stands in. Only a report
    /// beginning a Route opens one: the rest find their Route there, or stand nowhere.
    fn project_route_lifecycle(&mut self, event: &SessionEvent) -> Vec<AttributedProviderEvent> {
        let open = if routes::files_a_route(event) {
            self.open_main_turn().is_some()
        } else {
            self.turn.is_some()
        };
        if !open {
            return Vec::new();
        }
        let decided = self.routes.read(event, self.stretches);
        if !decided.is_empty() && self.open_main_streams().is_none() {
            return Vec::new();
        }
        decided
            .into_iter()
            .map(|decided| attributed(None, decided))
            .collect()
    }

    fn project_current_model(&mut self, event: &SessionEvent) -> Option<AttributedProviderEvent> {
        if self.is_compacting_on_request() || self.compaction.holds_stretch() {
            return None;
        }
        let auto = self.selection.model.as_str() == AUTO_MODEL_ID;
        self.turn
            .as_mut()?
            .streams
            .working_models
            .read(event, auto)
            .map(|event| attributed(None, event))
    }

    fn clear_current_model(&mut self) -> Option<AttributedProviderEvent> {
        self.turn
            .as_mut()?
            .streams
            .working_models
            .clear()
            .map(|event| attributed(None, event))
    }

    /// The process hosting the Session is gone, and a staged phase's conversation with it. What
    /// the crash takes from the running stretch is the Turn's to settle, as it fails.
    fn project_routes_lost(&mut self) -> Vec<AttributedProviderEvent> {
        if self.turn.is_none() {
            return Vec::new();
        }
        self.routes
            .settle(self.stretches, true)
            .into_iter()
            .map(|lost| attributed(None, lost))
            .collect()
    }

    /// Whether main output arriving with no Turn active is owed a Continuation rather than being
    /// stray: some Subagent is still working, a Watch that may wake the loop is still live, or a
    /// Subagent or Watch just settled and its provoked output is still to come.
    fn owes_late_output(&self) -> bool {
        self.late_settle_owes_continuation || !self.subagents.is_empty() || !self.watches.is_empty()
    }

    /// Opens the main conversation's streams to the event in hand: the active Turn's, or the
    /// Continuation stretch that late output begins here — begun is what marks the owed
    /// Continuation delivered. `None` refuses an event outside any Turn, which is dropped.
    fn open_main_streams(&mut self) -> Option<&mut ConversationStreams> {
        self.open_main_turn().map(|turn| {
            // Output is the loop running, whatever idle it reported while a compaction held the
            // stretch open: the stretch is the loop's again, and its next idle settles it.
            turn.idled = None;
            &mut turn.streams
        })
    }

    /// The main conversation's Turn, as [`Self::open_main_streams`] opens it, for a report that
    /// is no output of the loop's.
    fn open_main_turn(&mut self) -> Option<&mut ActiveTurn> {
        if self.turn.is_none() {
            if !self.owes_late_output() {
                return None;
            }
            self.begin_continuation();
        }
        self.turn.as_mut()
    }

    /// Begins the Continuation stretch that main work arriving with no Turn active lands in, which
    /// the loop's next idle settles.
    fn begin_continuation(&mut self) {
        self.stretches += 1;
        self.turn = Some(ActiveTurn::continuation());
        self.context_continuation = true;
        self.late_settle_owes_continuation = false;
    }
}

/// What an interrupt is for, as it begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct InterruptScope {
    /// The main conversation's stretch it stops, if one is running.
    stretch: Option<u64>,
    /// Copilot is compacting the main conversation's context, which its loop's abort leaves
    /// running, so the interrupt cancels it first.
    pub(super) compacting: bool,
}

/// What an interrupt has left to do once any compaction it cancelled has stopped.
#[derive(Debug, PartialEq)]
pub(super) struct InterruptRemainder {
    /// Copilot's loop is running for the stretch the interrupt is for — the main conversation's,
    /// or a Subagent's working past it — which only its abort stops.
    pub(super) abort: bool,
    /// What settles a stretch whose loop had already stopped, which nothing else will report.
    pub(super) settled: Vec<AttributedProviderEvent>,
    /// The abort is for Subagents working past a stretch whose loop had already stopped, which
    /// the idle it ends in settles: the interrupt is not over until the timeline has carried it.
    pub(super) awaits_idle: bool,
    /// The interrupt stopped nothing: the stretch's loop had already stopped, and the compaction
    /// holding it open had already ended when the cancel reached Copilot. The stretch settles as
    /// it ended ([`ProviderInterruption::AlreadyEnded`](crate::provider::ProviderInterruption::AlreadyEnded)).
    pub(super) already_ended: bool,
}

/// Where the projection reads the Session's background tasks from when the timeline says they
/// changed: the Copilot Session itself, asked no longer than `request_timeout` — the bound every
/// request Suru makes of the Session's loop waits under — because the timeline projects no
/// further while the read is out.
pub(super) struct TaskRosterSource {
    pub(super) native: Arc<NativeSession>,
    pub(super) request_timeout: Duration,
}

/// Streams the Provider events projected from one Copilot Session's timeline, failing the stream
/// when the shared harness process hosting it dies, beside where the Session's manual compactions
/// put Copilot's answers on that timeline.
pub(super) fn provider_events(
    subscription: EventSubscription,
    harness: Arc<SharedHarnessHandle<CopilotConnection>>,
    drain: EventDrainCheckpoint,
    correlation: Arc<StdMutex<CopilotCorrelation>>,
    skills: CopilotSkills,
    approvals: Arc<super::approval::CopilotApprovals>,
    roster: TaskRosterSource,
) -> (ProviderEventStream, ManualCompactionAnswers) {
    // The SDK drops the oldest events on a subscriber that falls behind, and a dropped delta is
    // Transcript content Suru cannot get back, so the timeline is drained as fast as it arrives
    // and queued here rather than at the pace the Session's consumer reads.
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let answers = ManualCompactionAnswers {
        timeline: events_tx.downgrade(),
    };
    tokio::spawn(drain_session_timeline(
        subscription,
        events_tx,
        drain.clone(),
        correlation.clone(),
    ));
    let events = Box::pin(stream::unfold(
        CopilotEvents {
            events: events_rx,
            harness,
            drain,
            correlation,
            skills,
            approvals,
            roster,
            pending: VecDeque::new(),
            ended: false,
        },
        next_provider_event,
    ));
    (events, answers)
}

/// Moves Copilot's timeline off the SDK's bounded subscription as it arrives.
///
/// The main loop stopping is `assistant.idle`. Copilot follows it with `session.idle` only once
/// the attached background work the loop left running has ended too — after any loop that work
/// wakes has stopped in turn — so that `session.idle` echoes an idle already carried and is
/// dropped here, in timeline order, before it could be read against a Turn begun since. A
/// `session.idle` no `assistant.idle` went ahead of is read as the loop's idle itself: Copilot
/// is not verified to report every way its loop stops with both.
async fn drain_session_timeline(
    mut subscription: EventSubscription,
    events: mpsc::UnboundedSender<Result<TimelineEvent, ProviderError>>,
    drain: EventDrainCheckpoint,
    correlation: Arc<StdMutex<CopilotCorrelation>>,
) {
    let mut timeline = TimelineForwarding {
        events,
        drain,
        correlation,
        session_idle_echoes: false,
    };
    loop {
        match subscription.recv().await {
            Ok(event) => {
                if !timeline.forward(event) {
                    return;
                }
            }
            Err(error) => {
                if let RecvErrorKind::Lagged(lagged) = error.kind() {
                    let _ = timeline.events.send(Err(copilot_error(format!(
                        "Copilot produced events faster than Suru could record them: {} were lost",
                        lagged.skipped()
                    ))));
                    return;
                }
                // The SDK closes the timeline with the connection, dropping the events it had
                // read but not yet delivered; those are the process's last words before it went.
                for event in timeline.drain.dropped() {
                    if !timeline.forward(event) {
                        return;
                    }
                }
                return;
            }
        }
    }
}

/// What carries one Session's timeline into Suru's projection queue, event by event.
struct TimelineForwarding {
    events: mpsc::UnboundedSender<Result<TimelineEvent, ProviderError>>,
    drain: EventDrainCheckpoint,
    correlation: Arc<StdMutex<CopilotCorrelation>>,
    /// Whether the next main `session.idle` echoes an `assistant.idle` already carried.
    session_idle_echoes: bool,
}

impl TimelineForwarding {
    /// Queues `event` for projection, answering whether the projection is still there to take it.
    fn forward(&mut self, event: SessionEvent) -> bool {
        let event_id = event.id.clone();
        let main_idle = match (event.parsed_type(), event.agent_id.is_none()) {
            (SessionEventType::AssistantIdle, true) => {
                self.session_idle_echoes = true;
                true
            }
            (SessionEventType::SessionIdle, true) => {
                if std::mem::take(&mut self.session_idle_echoes) {
                    self.drain.delivered(event_id);
                    return true;
                }
                true
            }
            _ => false,
        };
        let compaction_end = is_background_compaction_end(&event);
        log_context_contents(&event);
        // Ephemeral context reports use the same lossless drain as durable
        // events, and keep their originating Turn while waiting for projection.
        let context = self
            .correlation
            .lock()
            .expect("Copilot correlation lock is not poisoned")
            .context_report(&event);
        let event = match context {
            Some(context) => TimelineEvent::Context(context),
            None => TimelineEvent::Native(event),
        };
        if self.events.send(Ok(event)).is_err() {
            return false;
        }
        if main_idle {
            self.correlation
                .lock()
                .expect("Copilot correlation lock is not poisoned")
                .idle_carried();
        }
        if compaction_end {
            self.correlation
                .lock()
                .expect("Copilot correlation lock is not poisoned")
                .compaction_end_carried();
        }
        self.drain.delivered(event_id);
        true
    }
}

/// Copilot's reason for refusing a tier, for reading.
fn switch_failure_words(reason: &AutoTierSwitchFailureReason) -> String {
    match reason {
        AutoTierSwitchFailureReason::PolicyRejected => "rejected by policy",
        AutoTierSwitchFailureReason::RequestFailed => "the request failed",
        AutoTierSwitchFailureReason::SetupFailed => "setup failed",
        AutoTierSwitchFailureReason::Unsupported => "not supported",
        AutoTierSwitchFailureReason::Unknown => "no reason Suru can read",
    }
    .to_owned()
}

/// Logs what Copilot reports filling the context window with: the breakdown behind each context
/// report, and the MCP servers the Session loaded, whose tool definitions count against it.
fn log_context_contents(event: &SessionEvent) {
    let count = |field: &str| event.data.get(field).and_then(serde_json::Value::as_i64);
    match event.parsed_type() {
        SessionEventType::SessionUsageInfo => tracing::debug!(
            agent = event.agent_id.as_deref(),
            current = count("currentTokens"),
            system = count("systemTokens"),
            tool_definitions = count("toolDefinitionsTokens"),
            conversation = count("conversationTokens"),
            limit = count("tokenLimit"),
            messages = count("messagesLength"),
            "Copilot context breakdown"
        ),
        SessionEventType::SessionMcpServersLoaded => {
            let servers = event
                .data
                .get("servers")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten();
            for server in servers {
                let field = |name: &str| server.get(name).and_then(serde_json::Value::as_str);
                tracing::debug!(
                    name = field("name"),
                    status = field("status"),
                    source = field("source"),
                    transport = field("transport"),
                    error = field("error"),
                    "Copilot MCP server loaded"
                );
            }
        }
        SessionEventType::SessionMcpServerStatusChanged => tracing::debug!(
            name = event
                .data
                .get("serverName")
                .and_then(serde_json::Value::as_str),
            status = event.data.get("status").and_then(serde_json::Value::as_str),
            error = event.data.get("error").and_then(serde_json::Value::as_str),
            "Copilot MCP server status changed"
        ),
        _ => {}
    }
}

enum TimelineEvent {
    Native(SessionEvent),
    Context(AttributedProviderEvent),
    /// Copilot's answer to the manual compaction `turn_id` was begun for.
    ManualCompactionAnswered {
        turn_id: TurnId,
        answer: ManualCompactionAnswer,
    },
    /// Copilot answered the manual compaction `turn_id` was begun for a bound ago.
    ManualCompactionOverdue {
        turn_id: TurnId,
    },
}

struct CopilotEvents {
    events: mpsc::UnboundedReceiver<Result<TimelineEvent, ProviderError>>,
    harness: Arc<SharedHarnessHandle<CopilotConnection>>,
    drain: EventDrainCheckpoint,
    correlation: Arc<StdMutex<CopilotCorrelation>>,
    skills: CopilotSkills,
    approvals: Arc<super::approval::CopilotApprovals>,
    roster: TaskRosterSource,
    pending: VecDeque<Result<AttributedProviderEvent, ProviderError>>,
    ended: bool,
}

async fn next_provider_event(
    mut events: CopilotEvents,
) -> Option<(
    Result<AttributedProviderEvent, ProviderError>,
    CopilotEvents,
)> {
    loop {
        if let Some(event) = events.pending.pop_front() {
            return Some((event, events));
        }
        if events.ended {
            return None;
        }
        let harness = events.harness.clone();
        // A process that dies takes every Session it hosted with it, so its exit races the
        // timeline rather than leaving the Session waiting on a pipe nobody is left to write to.
        let received = tokio::select! {
            biased;
            crashed = harness.crashed() => {
                end_with_crash(&mut events, crashed).await;
                continue;
            }
            received = events.events.recv() => received,
        };
        match received {
            // The SDK closes a Session's timeline with the connection, which can beat the
            // supervisor's account of the exit here; a process that went unasked is the crash
            // that account is about to report, not a timeline that simply ended.
            None if harness.connection().lost_unasked() => {
                let crashed = harness.crashed().await;
                end_with_crash(&mut events, crashed).await;
            }
            None => {
                events.ended = true;
                return None;
            }
            Some(Err(error)) => {
                events.ended = true;
                return Some((Err(error), events));
            }
            Some(Ok(event)) => {
                if let TimelineEvent::Native(event) = &event
                    && matches!(
                        event.parsed_type(),
                        SessionEventType::AssistantIdle
                            | SessionEventType::SessionIdle
                            | SessionEventType::SubagentCompleted
                            | SessionEventType::SubagentFailed
                    )
                {
                    events.approvals.wait_for_settled_decision().await;
                }
                let roster_changed = matches!(
                    &event,
                    TimelineEvent::Native(event)
                        if event.parsed_type() == SessionEventType::SessionBackgroundTasksChanged
                );
                queue_projected(&mut events, event);
                if roster_changed {
                    read_task_roster(&mut events).await;
                }
            }
        }
    }
}

/// Ends the timeline on the harness crash `crashed`, once every event the process wrote before it
/// died has been projected, settling the Watches the crash took with it.
///
/// The SDK stops delivering a Session's events the moment its connection closes, dropping any it
/// had read but not yet routed, so the drain is over either when the last event on the wire has
/// arrived or when the timeline closes without it.
async fn end_with_crash(events: &mut CopilotEvents, crashed: ProviderError) {
    let drained = events.drain.clone();
    let drained = drained.wait_until_drained();
    tokio::pin!(drained);
    loop {
        tokio::select! {
            biased;
            () = &mut drained => break,
            received = events.events.recv() => match received {
                Some(Ok(event)) => queue_projected(events, event),
                Some(Err(error)) => events.pending.push_back(Err(error)),
                None => break,
            },
        }
    }
    while let Ok(received) = events.events.try_recv() {
        match received {
            Ok(event) => queue_projected(events, event),
            Err(error) => events.pending.push_back(Err(error)),
        }
    }
    let lost = {
        let mut correlation = events
            .correlation
            .lock()
            .expect("Copilot correlation lock is not poisoned");
        let mut lost = correlation.project_routes_lost();
        lost.extend(correlation.project_watches_lost());
        lost
    };
    events.pending.extend(lost.into_iter().map(Ok));
    events.pending.push_back(Err(crashed));
    events.ended = true;
}

/// Reads the Session's background tasks the moment the timeline says they changed, and starts a
/// Watch for each background shell the read finds newly running.
///
/// `session.background_tasks_changed` carries no payload, so the roster Copilot's own task view
/// refreshes on it (`session.tasks.list`) is where a shell's attachment and execution modes are
/// recorded — and the
/// identity it lists the shell under is the `shellId` the shell's completion notification names,
/// so the Watch's start and settle agree on it. The timeline waits on the read, which puts the
/// Watch's start ahead of the idle that follows it: the Session reads Monitoring from the Turn's
/// settle rather than idle for a moment first. A read that fails costs the Session its Monitoring
/// reading, not the Session: the shell's completion still records its Watch and wakes the loop
/// into a Continuation headed by the Watch Outcome.
///
/// Why the roster rather than the tool call (github-copilot-sdk 1.0.15-preview.3; the CLI wire as
/// captured on 1.0.82 and 1.0.91):
/// - `ToolExecutionStartData` carries the shell tool's `arguments` — `mode: "async"` is how the
///   Model asks for a background shell, and `detach: true` beside it for a detached one — but no
///   shell identity, and `ToolExecutionStartShellToolInfo` only path hints and a display command.
/// - The tool's result names the shell only in text addressed to the Model (`<command started in
///   background with shellId: …>`); the one structured content carrying a `shellId`,
///   `ToolExecutionCompleteContentShellExit`, needs an exit code a background start never has.
/// - `SessionBackgroundTasksChangedData` is empty: it says only that the roster changed.
/// - `session.tasks.list` (`SessionRpcTasks::list`) answers `TaskShellInfo` entries whose `id`
///   is the shell identity the completion notification names, and whose `attachment_mode` and
///   `execution_mode` (`TaskShellInfoAttachmentMode::Detached`, `TaskExecutionMode::Background`)
///   say outright what the tool arguments only request — a synchronous shell is listed too. The
///   Rust SDK leaves `SystemNotificationData.kind` untyped; the
///   `shell_detached_completed { shellId, description? }` and
///   `shell_completed { shellId, exitCode, description? }` shapes are the CLI's wire, as issues
///   #380 and #387 and a 1.0.91 capture record them (and as the Node and .NET SDKs' generated
///   notification kinds type them).
async fn read_task_roster(events: &mut CopilotEvents) {
    const CONTEXT: &str = "Copilot task roster read failed";
    let listed = timeout(
        events.roster.request_timeout,
        until_crash(
            &events.harness,
            CONTEXT,
            events.roster.native.rpc().tasks().list(),
        ),
    )
    .await;
    let tasks = match listed {
        Ok(Ok(listed)) => listed.tasks,
        Ok(Err(error)) => {
            tracing::warn!("{error}");
            return;
        }
        Err(_) => {
            tracing::warn!(
                "{CONTEXT}: {COPILOT_HARNESS_NAME} timed out handling `session.tasks.list`"
            );
            return;
        }
    };
    let started = events
        .correlation
        .lock()
        .expect("Copilot correlation lock is not poisoned")
        .project_task_roster(&tasks);
    events.pending.extend(started.into_iter().map(Ok));
}

fn queue_projected(events: &mut CopilotEvents, event: TimelineEvent) {
    let event = match event {
        TimelineEvent::Context(mut context) => {
            let correlation = events
                .correlation
                .lock()
                .expect("Copilot correlation lock is not poisoned");
            if context.attribution == ProviderEventAttribution::OwningSession
                && correlation.context_turn.is_none()
            {
                // Startup may have failed after this observation entered the drain.
                return;
            }
            if let ProviderEvent::ContextFill { report } = &mut context.event
                && context.attribution == ProviderEventAttribution::OwningSession
                && !correlation.context_prompt_pending
                && correlation.context_continuation
                && report.turn_id == correlation.context_turn
            {
                // Preceding native content opened a Continuation in this same Prompt
                // generation. Its Turn ID belongs to orchestration, so bind in stream
                // order. A queued report from an older Prompt retains its explicit ID.
                report.turn_id = None;
            }
            events.pending.push_back(Ok(context));
            return;
        }
        TimelineEvent::ManualCompactionAnswered { turn_id, answer } => {
            let settled = events
                .correlation
                .lock()
                .expect("Copilot correlation lock is not poisoned")
                .project_manual_compaction_answer(turn_id, answer);
            events.pending.extend(settled.into_iter().map(Ok));
            return;
        }
        TimelineEvent::ManualCompactionOverdue { turn_id } => {
            let settled = events
                .correlation
                .lock()
                .expect("Copilot correlation lock is not poisoned")
                .project_manual_compaction_overdue(turn_id);
            events.pending.extend(settled.into_iter().map(Ok));
            return;
        }
        TimelineEvent::Native(event) => event,
    };
    match event.parsed_type() {
        SessionEventType::PermissionRequested => {
            if let Some(request_id) = event.data["requestId"]
                .as_str()
                .map(github_copilot_sdk::RequestId::new)
            {
                let attribution = event
                    .agent_id
                    .as_deref()
                    .map_or(ProviderEventAttribution::OwningSession, |agent| {
                        ProviderEventAttribution::Subagent(ProviderSubagentId::new(agent))
                    });
                events.approvals.observe(request_id, attribution);
            }
        }
        SessionEventType::PermissionCompleted => {
            if let Some(request_id) = event.data["requestId"]
                .as_str()
                .map(github_copilot_sdk::RequestId::new)
                && let Some(withdrawn) = events.approvals.complete(&request_id)
            {
                events.pending.push_back(Ok(withdrawn));
            }
        }
        _ => {}
    }
    if matches!(
        event.parsed_type(),
        SessionEventType::CommandsChanged | SessionEventType::SessionSkillsLoaded
    ) {
        events.skills.native_catalog_changed();
    }
    let projected = {
        let mut correlation = events
            .correlation
            .lock()
            .expect("Copilot correlation lock is not poisoned");
        project_session_event(&mut correlation, event)
    };
    match projected {
        Ok(projected) => events.pending.extend(projected.into_iter().map(Ok)),
        Err(error) => events.pending.push_back(Err(error)),
    }
}

/// The attribution one conversation's events land under.
fn attributed(subagent: Option<&str>, event: ProviderEvent) -> AttributedProviderEvent {
    AttributedProviderEvent {
        attribution: match subagent {
            None => ProviderEventAttribution::OwningSession,
            Some(subagent) => ProviderEventAttribution::Subagent(ProviderSubagentId::new(subagent)),
        },
        event,
    }
}

fn project_session_event(
    correlation: &mut CopilotCorrelation,
    event: SessionEvent,
) -> Result<Vec<AttributedProviderEvent>, ProviderError> {
    match event.parsed_type() {
        // The Subagent lifecycle is read before the envelope's attribution: it opens and settles
        // the Subagents themselves, whichever conversation's context Copilot stamped on it.
        SessionEventType::SubagentStarted => {
            return Ok(correlation.project_subagent_started(&event));
        }
        SessionEventType::SubagentCompleted => {
            return Ok(reported(&event).map_or_else(
                Vec::new,
                |completed: SubagentCompletedData| {
                    // A torn-down Subagent still reports completion, distinguished only by the
                    // flag; a delegation that never ran to the end did not do its work.
                    let status = if completed.cancelled.unwrap_or(false) {
                        ProviderSubagentStatus::Failed
                    } else {
                        ProviderSubagentStatus::Completed
                    };
                    correlation.project_subagent_settled(&completed.tool_call_id, status)
                },
            ));
        }
        SessionEventType::SubagentFailed => {
            return Ok(
                reported(&event).map_or_else(Vec::new, |failed: SubagentFailedData| {
                    correlation.project_subagent_settled(
                        &failed.tool_call_id,
                        ProviderSubagentStatus::Failed,
                    )
                }),
            );
        }
        // A notification wakes the main loop whichever conversation's context Copilot stamped
        // on it, so it is read before the envelope's attribution too.
        SessionEventType::SystemNotification => {
            return Ok(reported(&event).map_or_else(
                Vec::new,
                |notification: SystemNotificationData| {
                    correlation.project_system_notification(&notification)
                },
            ));
        }
        _ => {}
    }
    // An event the envelope attributes to a Subagent lands in that Subagent's own streams,
    // whether or not the main conversation has a Turn open — a Subagent outlives the Turn that
    // spawned it (ADR 0015). One attributed to an instance no Subagent holds lands nowhere.
    if let Some(subagent) = event.agent_id.clone() {
        match event.parsed_type() {
            SessionEventType::UserMessage => {
                return Ok(correlation.project_subagent_message(&subagent, &event));
            }
            SessionEventType::SessionAutoModeResolved | SessionEventType::SessionModelChange => {
                let (auto, reroute) = match event.parsed_type() {
                    SessionEventType::SessionAutoModeResolved => (
                        reported::<SessionAutoModeResolvedData>(&event).map(|_| true),
                        None,
                    ),
                    SessionEventType::SessionModelChange => {
                        let change = reported::<SessionModelChangeData>(&event);
                        (
                            change
                                .as_ref()
                                .map(|change| change.new_model == AUTO_MODEL_ID),
                            change.as_ref().and_then(refusal_fallback_reroute),
                        )
                    }
                    _ => unreachable!(),
                };
                let Some(auto) = auto else {
                    return Ok(Vec::new());
                };
                let Some(known) = correlation.agents.get_mut(&subagent) else {
                    return Ok(Vec::new());
                };
                known.auto_selection = auto;
                let Some(working) = correlation.subagents.get_mut(&subagent) else {
                    return Ok(Vec::new());
                };
                return Ok(reroute
                    .into_iter()
                    .chain(working.streams.working_models.selection_changed(auto))
                    .map(|event| attributed(Some(&subagent), event))
                    .collect());
            }
            SessionEventType::AssistantTurnEnd => {
                let auto = correlation
                    .agents
                    .get(&subagent)
                    .is_some_and(|known| known.auto_selection);
                let mut projected = correlation
                    .subagents
                    .get_mut(&subagent)
                    .and_then(|working| working.streams.working_models.read(&event, auto))
                    .map(|event| attributed(Some(&subagent), event))
                    .into_iter()
                    .collect::<Vec<_>>();
                projected.extend(correlation.project_subagent_turn_end(&subagent));
                return Ok(projected);
            }
            event_type if is_compaction_report(&event_type) => {
                let mut projected = if event_type == SessionEventType::SessionCompactionStart {
                    correlation
                        .subagents
                        .get_mut(&subagent)
                        .and_then(|working| working.streams.working_models.clear())
                        .map(|event| attributed(Some(&subagent), event))
                        .into_iter()
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                projected.extend(
                    correlation.project_subagent_compaction(&subagent, compaction_event(&event)),
                );
                return Ok(projected);
            }
            _ => {}
        }
        let Some(working) = correlation.subagents.get_mut(&subagent) else {
            return Ok(Vec::new());
        };
        let Some(known) = correlation.agents.get_mut(&subagent) else {
            return Ok(Vec::new());
        };
        if event.parsed_type() == SessionEventType::AssistantMessage {
            let no_tools = reported::<AssistantMessageData>(&event).is_some_and(|message| {
                message
                    .tool_requests
                    .is_none_or(|requests| requests.is_empty())
            });
            working.stretch.answer(no_tools);
        }
        let compacting = known.compaction.holds_stretch();
        let streams = &mut working.streams;
        if !compacting
            && matches!(
                event.parsed_type(),
                SessionEventType::AssistantTurnStart | SessionEventType::ModelCallStart
            )
        {
            streams.anonymous_fixed_call = false;
        }
        if event.parsed_type() == SessionEventType::AssistantUsage {
            let usage: AssistantUsageData = decode(&event)?;
            let previous_call = usage
                .api_call_id
                .as_deref()
                .is_some_and(|id| known.previous_model_calls.contains(id));
            let model = (!previous_call && !usage.model.is_empty()).then_some(usage.model.clone());
            let mut projected = Vec::new();
            if let Some(model) = model {
                known.model = Some(model.clone());
                projected.push(attributed(
                    None,
                    ProviderEvent::SubagentModelChanged {
                        subagent_id: ProviderSubagentId::new(subagent.clone()),
                        model: crate::protocol::ModelId::new(model),
                    },
                ));
            }
            projected.push(attributed(
                Some(&subagent),
                project_usage_event(streams, &event, &correlation.pricing)?,
            ));
            let conversation = is_conversation_model_call(
                &event,
                usage.interaction_type.as_deref(),
                usage.initiator.as_deref(),
                ConversationKind::Subagent,
                compacting,
            );
            let current_call = conversation
                && streams.observe_model_call(
                    usage.api_call_id.as_deref(),
                    &known.previous_model_calls,
                    known.auto_selection,
                );
            if current_call
                && !compacting
                && let Some(current) = streams.working_models.read(&event, known.auto_selection)
            {
                projected.push(attributed(Some(&subagent), current));
            }
            if current_call && usage.is_auto == Some(false) {
                projected.extend(
                    fixed_model_call(streams, usage.api_call_id.as_deref())
                        .into_iter()
                        .map(|event| attributed(Some(&subagent), event)),
                );
            }
            let auto = usage.is_auto == Some(true)
                || streams.call_selected_auto(usage.api_call_id.as_deref(), known.auto_selection);
            if current_call && auto {
                projected.extend(
                    reported_model_call(
                        streams,
                        &usage.model,
                        usage.api_call_id.as_deref(),
                        &ModelId::new(AUTO_MODEL_ID),
                        true,
                    )
                    .into_iter()
                    .map(|event| attributed(Some(&subagent), event)),
                );
            }
            return Ok(projected);
        }
        if event.parsed_type() == SessionEventType::ModelCallFailure {
            let Some(failure): Option<ModelCallFailureData> = reported(&event) else {
                return Ok(Vec::new());
            };
            if !is_conversation_model_failure(
                &event,
                &failure,
                ConversationKind::Subagent,
                compacting,
            ) || !streams.observe_model_call(
                failure.api_call_id.as_deref(),
                &known.previous_model_calls,
                known.auto_selection,
            ) {
                return Ok(Vec::new());
            }
            let mut projected = (!compacting)
                .then(|| streams.working_models.read(&event, known.auto_selection))
                .flatten()
                .map(|event| attributed(Some(&subagent), event))
                .into_iter()
                .collect::<Vec<_>>();
            if failure.is_auto == Some(false) {
                projected.extend(
                    fixed_model_call(streams, failure.api_call_id.as_deref())
                        .into_iter()
                        .map(|event| attributed(Some(&subagent), event)),
                );
            }
            let auto = failure.is_auto == Some(true)
                || streams.call_selected_auto(failure.api_call_id.as_deref(), known.auto_selection);
            if auto && let Some(model) = failure.model.as_deref() {
                projected.extend(
                    reported_model_call(
                        streams,
                        model,
                        failure.api_call_id.as_deref(),
                        &ModelId::new(AUTO_MODEL_ID),
                        false,
                    )
                    .into_iter()
                    .map(|event| attributed(Some(&subagent), event)),
                );
            }
            return Ok(projected);
        }
        let current_model =
            if compacting || is_previous_model_call(&event, &known.previous_model_calls) {
                None
            } else {
                streams.working_models.read(&event, known.auto_selection)
            };
        let auto = ModelId::new(AUTO_MODEL_ID);
        let routed_selection = known.auto_selection.then_some(&auto);
        let mut projected = current_model
            .map(|event| attributed(Some(&subagent), event))
            .into_iter()
            .collect::<Vec<_>>();
        projected.extend(
            project_conversation_event(
                streams,
                &mut correlation.delegations,
                &correlation.execution_directory,
                &mut working.routes,
                correlation.stretches,
                ModelRouting {
                    selected: routed_selection,
                    conversation: ConversationKind::Subagent,
                    previous_calls: &known.previous_model_calls,
                },
                &event,
            )?
            .into_iter()
            .map(|event| attributed(Some(&subagent), event)),
        );
        return Ok(projected);
    }
    if routes::is_lifecycle(&event) {
        let mut projected = correlation.project_route_lifecycle(&event);
        projected.extend(correlation.project_current_model(&event));
        return Ok(projected);
    }
    match event.parsed_type() {
        SessionEventType::AssistantTurnStart
        | SessionEventType::AssistantTurnEnd
        | SessionEventType::ModelCallStart
        | SessionEventType::ModelCallFinished
        | SessionEventType::Abort
        | SessionEventType::AgentInterrupted => {
            if matches!(
                event.parsed_type(),
                SessionEventType::AssistantTurnStart | SessionEventType::ModelCallStart
            ) && !has_parent_tool_call(&event)
                && let Some(turn) = correlation.turn.as_mut()
            {
                turn.streams.anonymous_fixed_call = false;
            }
            Ok(correlation
                .project_current_model(&event)
                .into_iter()
                .collect())
        }
        // Copilot switching Models of its own accord is a Reroute where it reports it. It is the
        // loop's own output, so like content it opens the Continuation stretch a loop woken
        // after its Turn settled owes, and stands ahead of whatever that loop goes on to say;
        // outside any Turn it is dropped.
        SessionEventType::SessionModelChange => {
            let Some(reroute) = reported::<SessionModelChangeData>(&event)
                .as_ref()
                .and_then(refusal_fallback_reroute)
            else {
                return Ok(Vec::new());
            };
            if correlation.open_main_streams().is_none() {
                return Ok(Vec::new());
            }
            Ok(vec![attributed(None, reroute)])
        }
        SessionEventType::SessionAutoTierSwitchFailed => Ok(reported(&event)
            .map(|refusal| correlation.project_tier_refused(refusal))
            .unwrap_or_default()),
        // A transient error is one Copilot's own loop recovers from by retrying, so it is not the
        // Turn's outcome and stays out of the Transcript.
        SessionEventType::SessionError if event.is_transient_error() => Ok(Vec::new()),
        SessionEventType::SessionError => {
            let failure: SessionErrorData = decode(&event)?;
            let current = correlation.clear_current_model();
            correlation.project_session_error(&failure);
            Ok(current.into_iter().collect())
        }
        SessionEventType::AssistantIdle | SessionEventType::SessionIdle => {
            let aborted = if event.parsed_type() == SessionEventType::AssistantIdle {
                decode::<AssistantIdleData>(&event)?.aborted
            } else {
                decode::<SessionIdleData>(&event)?.aborted
            }
            .unwrap_or(false);
            if correlation.owed_aborted_idle(aborted) {
                return Ok(Vec::new());
            }
            // No loop runs a manual compaction, and its answer settles its Turn.
            if correlation.is_compacting_on_request() {
                return Ok(Vec::new());
            }
            let mut projected = correlation.project_resumes_stopped(aborted);
            projected.extend(
                correlation
                    .project_loop_idle(aborted)
                    .into_iter()
                    .map(|projected| attributed(None, projected)),
            );
            Ok(projected)
        }
        event_type if is_compaction_report(&event_type) => {
            let mut projected = if event_type == SessionEventType::SessionCompactionStart {
                correlation
                    .clear_current_model()
                    .into_iter()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            projected.extend(correlation.project_main_compaction_report(&event));
            Ok(projected)
        }
        // A manual compaction's summarising call is the Turn's Usage as its
        // `compactionTokensUsed` reports it, once. The moment its call says
        // the conversation's Prompt Cache lapses is no Usage, and is taken.
        SessionEventType::AssistantUsage if correlation.is_compacting_on_request() => {
            Ok(reported::<AssistantUsageData>(&event)
                .filter(|_| !has_parent_tool_call(&event))
                .and_then(|usage| reported_lapse(&event, &usage))
                .map(|lapse| attributed(None, lapse))
                .into_iter()
                .collect())
        }
        SessionEventType::AssistantUsage => {
            if correlation.open_main_streams().is_none() {
                return Ok(Vec::new());
            }
            let usage: AssistantUsageData = decode(&event)?;
            let selection = (correlation.selection.model.as_str() == AUTO_MODEL_ID)
                .then(|| correlation.selection.model.clone());
            let compacting = correlation.compaction.holds_stretch();
            let turn = correlation
                .turn
                .as_mut()
                .expect("usage opened or found the main conversation's Turn");
            let mut projected = vec![attributed(
                None,
                project_usage_event(&mut turn.streams, &event, &correlation.pricing)?,
            )];
            let conversation = is_conversation_model_call(
                &event,
                usage.interaction_type.as_deref(),
                usage.initiator.as_deref(),
                ConversationKind::Main,
                compacting,
            );
            let current_call = conversation
                && turn.streams.observe_model_call(
                    usage.api_call_id.as_deref(),
                    &correlation.previous_model_calls,
                    selection.is_some(),
                );
            if current_call
                && !compacting
                && let Some(current) = turn
                    .streams
                    .working_models
                    .read(&event, selection.is_some())
            {
                projected.push(attributed(None, current));
            }
            // A call of the conversation that wrote or refreshed its Prompt
            // Cache says when that cache lapses; Suru takes the moment as given.
            if current_call && let Some(lapse) = reported_lapse(&event, &usage) {
                projected.push(attributed(None, lapse));
            }
            if current_call && usage.is_auto == Some(false) {
                projected.extend(
                    fixed_model_call(&mut turn.streams, usage.api_call_id.as_deref())
                        .into_iter()
                        .map(|event| attributed(None, event)),
                );
            }
            if let Some(selected) = selection
                && current_call
            {
                projected.extend(
                    reported_model_call(
                        &mut turn.streams,
                        &usage.model,
                        usage.api_call_id.as_deref(),
                        &selected,
                        true,
                    )
                    .into_iter()
                    .map(|event| attributed(None, event)),
                );
            }
            Ok(projected)
        }
        SessionEventType::ModelCallFailure => {
            if correlation.is_compacting_on_request() {
                return Ok(Vec::new());
            }
            let Some(failure): Option<ModelCallFailureData> = reported(&event) else {
                return Ok(Vec::new());
            };
            let compacting = correlation.compaction.holds_stretch();
            if !is_conversation_model_failure(&event, &failure, ConversationKind::Main, compacting)
            {
                return Ok(Vec::new());
            }
            let auto = correlation.selection.model.as_str() == AUTO_MODEL_ID;
            let Some(turn) = correlation.turn.as_mut() else {
                return Ok(Vec::new());
            };
            if !turn.streams.observe_model_call(
                failure.api_call_id.as_deref(),
                &correlation.previous_model_calls,
                auto,
            ) {
                return Ok(Vec::new());
            }
            let mut projected = (!compacting)
                .then(|| turn.streams.working_models.read(&event, auto))
                .flatten()
                .map(|event| attributed(None, event))
                .into_iter()
                .collect::<Vec<_>>();
            if failure.is_auto == Some(false) {
                projected.extend(
                    fixed_model_call(&mut turn.streams, failure.api_call_id.as_deref())
                        .into_iter()
                        .map(|event| attributed(None, event)),
                );
            }
            if auto && let Some(model) = failure.model.as_deref() {
                projected.extend(
                    reported_model_call(
                        &mut turn.streams,
                        model,
                        failure.api_call_id.as_deref(),
                        &correlation.selection.model,
                        false,
                    )
                    .into_iter()
                    .map(|event| attributed(None, event)),
                );
            }
            Ok(projected)
        }
        // Content the Transcript presents belongs to a Turn, so late content — like the error
        // above — opens a Continuation stretch for what it carries; everything else on the
        // timeline is passed over rather than opening a stretch it would put nothing in.
        // A tool execution's output or completion arriving with no Turn active reports on one a
        // settled stretch started — a background shell streams its output as it exits, after
        // the Turn that ran it — so it opens no stretch: a woken loop's own executions start
        // in the stretch they open.
        SessionEventType::ToolShellOutput | SessionEventType::ToolExecutionComplete
            if correlation.turn.is_none() =>
        {
            Ok(Vec::new())
        }
        event_type if is_conversation_content(&event_type) => {
            if correlation.open_main_streams().is_none() {
                return Ok(Vec::new());
            }
            let stretch = correlation.stretches;
            let routed_selection = (correlation.selection.model.as_str() == AUTO_MODEL_ID)
                .then_some(&correlation.selection.model);
            let compacting = correlation.compaction.holds_stretch();
            let previous_call = is_previous_model_call(&event, &correlation.previous_model_calls);
            let turn = correlation
                .turn
                .as_mut()
                .expect("the main conversation's streams live inside its Turn");
            let current = if compacting || previous_call || has_parent_tool_call(&event) {
                None
            } else {
                turn.streams
                    .working_models
                    .read(&event, routed_selection.is_some())
            };
            let mut projected = current
                .map(|event| attributed(None, event))
                .into_iter()
                .collect::<Vec<_>>();
            projected.extend(
                project_conversation_event(
                    &mut turn.streams,
                    &mut correlation.delegations,
                    &correlation.execution_directory,
                    &mut correlation.routes,
                    stretch,
                    ModelRouting {
                        selected: routed_selection,
                        conversation: ConversationKind::Main,
                        previous_calls: &correlation.previous_model_calls,
                    },
                    &event,
                )?
                .into_iter()
                .map(|projected| attributed(None, projected)),
            );
            Ok(projected)
        }
        _ => Ok(Vec::new()),
    }
}

/// The `cause` Copilot gives a model change when the Model it was running declined a request and
/// it switched to its configured refusal-fallback Model.
const REFUSAL_FALLBACK_CAUSE: &str = "refusal_fallback";

/// The Reroute a model change reports, where Copilot switched for a reason of its own. Only a
/// refusal fallback is one: a change the user's Selection asked for is none, and neither is Auto
/// choosing a Model for each call, which it reports otherwise. A `rate_limit_auto_switch` is
/// deliberately not one for now: it moves the rest of the Session onto Auto, which reaches wider
/// than a switch within one Turn, and how to show it is a decision of its own.
fn refusal_fallback_reroute(change: &SessionModelChangeData) -> Option<ProviderEvent> {
    if change.cause.as_deref() != Some(REFUSAL_FALLBACK_CAUSE) {
        return None;
    }
    Some(ProviderEvent::Rerouted {
        from: change.previous_model.as_deref().and_then(ModelId::named),
        to: ModelId::named(&change.new_model)?,
        reason: Some("the Model declined the request".to_owned()),
    })
}

fn project_usage_event(
    streams: &mut ConversationStreams,
    event: &SessionEvent,
    pricing: &CopilotPricing,
) -> Result<ProviderEvent, ProviderError> {
    let call = priced_call(event, pricing)?;
    Ok(metered(streams, call.model, call.usage, call.cost))
}

#[derive(Clone, Copy)]
enum ConversationKind {
    Main,
    Subagent,
}

struct ModelRouting<'a> {
    selected: Option<&'a ModelId>,
    conversation: ConversationKind,
    previous_calls: &'a HashSet<String>,
}

fn is_conversation_model_call(
    event: &SessionEvent,
    interaction_type: Option<&str>,
    initiator: Option<&str>,
    conversation: ConversationKind,
    compacting: bool,
) -> bool {
    if matches!(conversation, ConversationKind::Main) && has_parent_tool_call(event) {
        return false;
    }
    match (interaction_type, conversation) {
        (Some("conversation-agent"), ConversationKind::Main)
        | (Some("conversation-subagent"), ConversationKind::Subagent) => true,
        (None, ConversationKind::Main) => !compacting && initiator.is_none(),
        (None, ConversationKind::Subagent) => {
            !compacting && matches!(initiator, None | Some("sub-agent"))
        }
        _ => false,
    }
}

fn has_parent_tool_call(event: &SessionEvent) -> bool {
    event
        .data
        .get("parentToolCallId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|id| !id.is_empty())
}

fn is_previous_model_call(event: &SessionEvent, previous: &HashSet<String>) -> bool {
    event
        .data
        .get("apiCallId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|id| previous.contains(id))
}

fn is_conversation_model_failure(
    event: &SessionEvent,
    failure: &ModelCallFailureData,
    conversation: ConversationKind,
    compacting: bool,
) -> bool {
    matches!(
        (&failure.source, conversation),
        (ModelCallFailureSource::TopLevel, ConversationKind::Main)
            | (ModelCallFailureSource::Subagent, ConversationKind::Subagent)
    ) && is_conversation_model_call(
        event,
        failure.interaction_type.as_deref(),
        failure.initiator.as_deref(),
        conversation,
        compacting,
    )
}

fn reported_model_call(
    streams: &mut ConversationStreams,
    actual: &str,
    call_id: Option<&str>,
    selected: &ModelId,
    usage: bool,
) -> Vec<ProviderEvent> {
    let id = call_id.filter(|id| !id.is_empty());
    let provider = ProviderId::new(super::COPILOT_PROVIDER_ID);
    let Some(model) = actual_model::attribution(SelectedModel::new(&provider, selected), actual)
    else {
        return Vec::new();
    };
    if !usage && id.is_none() {
        return Vec::new();
    }
    let first = id.is_none_or(|id| streams.recorded_model_calls.insert(id.to_owned()));
    let mut projected = if first {
        vec![ProviderEvent::ModelCallRecorded {
            model: model.clone(),
        }]
    } else {
        Vec::new()
    };
    if let Some(id) = id.filter(|id| usage && !streams.fixed_model_calls.contains(*id)) {
        streams
            .reported_model_calls
            .insert(id.to_owned(), model.clone());
        if let Some(messages) = streams.pending_model_messages.remove(id) {
            for pending in messages {
                let mut attribution = model.clone();
                if let Some(actual) = pending.actual {
                    attribution.actual = actual;
                }
                remember_attributed_message(streams, id, &pending.message);
                projected.push(ProviderEvent::AgentMessageModelKnown {
                    message: Some(pending.message),
                    model: attribution,
                });
            }
        }
    }
    projected
}

fn remember_attributed_message(
    streams: &mut ConversationStreams,
    call_id: &str,
    message: &ProviderMessageId,
) {
    let messages = streams
        .attributed_model_messages
        .entry(call_id.to_owned())
        .or_default();
    if !messages.contains(message) {
        messages.push(message.clone());
    }
}

fn fixed_model_call(
    streams: &mut ConversationStreams,
    call_id: Option<&str>,
) -> Vec<ProviderEvent> {
    let Some(id) = call_id.filter(|id| !id.is_empty()) else {
        streams.anonymous_fixed_call = true;
        return std::mem::take(&mut streams.anonymous_attributed_messages)
            .into_iter()
            .map(|message| ProviderEvent::AgentMessageModelCleared { message })
            .collect();
    };
    streams.fixed_model_calls.insert(id.to_owned());
    streams.reported_model_calls.remove(id);
    streams.pending_model_messages.remove(id);
    streams
        .attributed_model_messages
        .remove(id)
        .unwrap_or_default()
        .into_iter()
        .map(|message| ProviderEvent::AgentMessageModelCleared { message })
        .collect()
}
/// What one model call's `assistant.usage` reports it consumed, in Suru's disjoint Usage parts,
/// and its Cost at the catalog `pricing` holds — absent where that prices nothing for the call's
/// Model. A Turn adds it to the Turn's reading; an Errand to the Errand's.
pub(super) fn priced_usage(
    event: &SessionEvent,
    pricing: &CopilotPricing,
) -> Result<(Usage, Option<Cost>), ProviderError> {
    priced_call(event, pricing).map(|call| (call.usage, call.cost))
}

/// One model call's `assistant.usage` report as [`priced_call`] reads it: its Usage, its Cost, and
/// the Model it names as the one the call ran on, absent where it names none. [`priced_usage`] is
/// this without the Model.
struct PricedCall {
    model: Option<ModelId>,
    usage: Usage,
    cost: Option<Cost>,
}

fn priced_call(
    event: &SessionEvent,
    pricing: &CopilotPricing,
) -> Result<PricedCall, ProviderError> {
    let cache_ttl_seconds = reported_cache_ttl(event);
    let reported: AssistantUsageData = decode(event)?;
    let usage = Usage {
        fresh_input_tokens: exclusive_count(
            reported.input_tokens,
            [reported.cache_read_tokens, reported.cache_write_tokens],
        ),
        cache_read_tokens: reported_count(reported.cache_read_tokens),
        cache_write_tokens: reported_count(reported.cache_write_tokens),
        output_tokens: exclusive_count(reported.output_tokens, [reported.reasoning_tokens]),
        reasoning_tokens: reported_count(reported.reasoning_tokens),
        native_meter: reported.cost.and_then(NativeMeter::from_units),
    };
    let cost = pricing.cost(
        &reported.model,
        reported.max_prompt_tokens,
        cache_ttl_seconds,
        &usage,
    );
    Ok(PricedCall {
        model: ModelId::named(&reported.model),
        usage,
        cost,
    })
}

/// Adds one model call's `usage`, costing `reported_cost`, to the reading the conversation's Turn
/// has accumulated, and to the running Model Share of `model`, the Model the call reported running
/// on, answering the Turn's Usage so far. A call naming no Model adds to the Turn's total alone,
/// unattributed.
fn metered(
    streams: &mut ConversationStreams,
    model: Option<ModelId>,
    usage: Usage,
    reported_cost: Option<Cost>,
) -> ProviderEvent {
    ReportedMetering::accumulate_on(&mut streams.metering, model, usage, reported_cost).event()
}

/// What a call's `assistant.usage` says of the Prompt Cache it wrote or
/// refreshed: when it lapses, on the Model the call names, kept for as long
/// as its `cacheTtlSeconds` says, by a call able to send as much as its
/// `maxPromptTokens`, which Copilot prices the call's tier by. None for a
/// call that touched no cache.
fn reported_lapse(event: &SessionEvent, usage: &AssistantUsageData) -> Option<ProviderEvent> {
    Some(ProviderEvent::PromptCacheLapseReported {
        lapses_at: usage.cache_expires_at.as_deref().and_then(lapse_moment)?,
        model: ModelId::named(&usage.model),
        lifetime: reported_cache_ttl(event)
            .and_then(|seconds| u64::try_from(seconds).ok())
            .filter(|seconds| *seconds > 0)
            .map(std::time::Duration::from_secs),
        prompt_limit_tokens: usage.max_prompt_tokens.and_then(token_count),
    })
}

/// The moment a call's `cacheExpiresAt` names, an RFC 3339 timestamp, as
/// milliseconds since the Unix epoch; none where it names no moment Suru can
/// read, or one before the epoch.
fn lapse_moment(cache_expires_at: &str) -> Option<crate::protocol::SessionTimestamp> {
    let moment = cache_expires_at.parse::<jiff::Timestamp>().ok()?;
    u64::try_from(moment.as_millisecond())
        .ok()
        .map(crate::protocol::SessionTimestamp)
}

fn reported_cache_ttl(event: &SessionEvent) -> Option<i64> {
    // The SDK receives this protocol field but does not expose it publicly;
    // retain it from the event payload so the catalog's one-hour write price
    // can be selected when it is the applicable published rate.
    event.data.get("cacheTtlSeconds")?.as_i64()
}

/// Whether an event is one of Copilot's reports on compacting a conversation's context.
fn is_compaction_report(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::SessionCompactionStart | SessionEventType::SessionCompactionComplete
    )
}

/// Whether `event` reports the end of a compaction of the main conversation Copilot ran in the
/// background of the Session — any but the manual one Suru asks for.
fn is_background_compaction_end(event: &SessionEvent) -> bool {
    event.agent_id.is_none()
        && event.parsed_type() == SessionEventType::SessionCompactionComplete
        && event
            .data
            .get("trigger")
            .and_then(serde_json::Value::as_str)
            != Some(MANUAL_TRIGGER)
}

/// The Provider event a compaction report is, for whichever conversation it compacted: the start,
/// or how it ended — completed with the context Copilot measured before and after it and the
/// summary it left, or failed with Copilot's account of why. Copilot's trigger is not read: whether a Compaction was
/// automatic is Suru's to say (ADR 0041). An end Suru cannot read still ends the compaction,
/// failed, since nothing else will.
fn compaction_event(event: &SessionEvent) -> ProviderEvent {
    if event.parsed_type() == SessionEventType::SessionCompactionStart {
        return ProviderEvent::CompactionStarted;
    }
    match reported::<SessionCompactionCompleteData>(event) {
        Some(completed) if completed.success => ProviderEvent::CompactionCompleted {
            before_tokens: reported_count(completed.pre_compaction_tokens),
            after_tokens: reported_count(completed.post_compaction_tokens),
            summary: completed.summary_content,
        },
        Some(failed) => ProviderEvent::CompactionFailed {
            error: failed.error,
        },
        None => ProviderEvent::CompactionFailed { error: None },
    }
}

/// Why a manual compaction's Turn fails when Copilot answered that it compacted nothing and said
/// nothing of why.
const NOT_COMPACTED: &str = "Copilot did not compact the Session's context";

/// The trigger Suru asks Copilot to compact on request under, which Copilot reports the
/// compaction's events under.
const MANUAL_TRIGGER: &str = "manual";

/// What a compaction's summarising call spent, from the `compactionTokensUsed` its
/// `session.compaction_complete` carries in the shape `assistant.usage` reports a model call's,
/// priced as one: the Usage of the manual compaction's Turn. Nothing where Copilot reported none.
fn compaction_usage(
    event: &SessionEvent,
    pricing: &CopilotPricing,
    streams: &mut ConversationStreams,
) -> Option<ProviderEvent> {
    let used = reported::<SessionCompactionCompleteData>(event)?.compaction_tokens_used?;
    let usage = Usage {
        fresh_input_tokens: exclusive_count(
            used.input_tokens,
            [used.cache_read_tokens, used.cache_write_tokens],
        ),
        cache_read_tokens: reported_count(used.cache_read_tokens),
        cache_write_tokens: reported_count(used.cache_write_tokens),
        output_tokens: reported_count(used.output_tokens),
        ..Usage::default()
    };
    let reported_cost = used
        .model
        .as_deref()
        .and_then(|model| pricing.cost(model, None, None, &usage));
    let model = used.model.as_deref().and_then(ModelId::named);
    Some(metered(streams, model, usage, reported_cost))
}

/// Whether an event carries conversation content this projection presents.
fn is_conversation_content(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::AssistantMessageStart
            | SessionEventType::AssistantMessageDelta
            | SessionEventType::AssistantMessage
            | SessionEventType::ToolExecutionStart
            | SessionEventType::ToolShellOutput
            | SessionEventType::ToolExecutionComplete
            | SessionEventType::AssistantReasoningDelta
            | SessionEventType::AssistantReasoning
    )
}

/// Projects one conversation's content event — the main agent's inside its Turn, or a Subagent's
/// for as long as it runs — onto the Provider events that carry it. `delegations` is the roster
/// of spawning tool calls whose Subagent row already answers for them, which is what tells a
/// withheld spawn's settle apart from a delegation that never happened; `execution_directory` is
/// where the Session works, which every conversation in it shares. `routes` is this conversation's
/// own, and with `stretch`, the stretch of the loop the event falls in, tells the first copy of a
/// Route's staged item from its committed twin.
fn project_conversation_event(
    streams: &mut ConversationStreams,
    delegations: &mut HashSet<String>,
    execution_directory: &Path,
    routes: &mut Routes,
    stretch: u64,
    routing: ModelRouting<'_>,
    event: &SessionEvent,
) -> Result<Vec<ProviderEvent>, ProviderError> {
    let fusion = routes::attribution(event);
    match event.parsed_type() {
        SessionEventType::AssistantMessageStart => {
            let started: AssistantMessageStartData = decode(event)?;
            if routes.holds_message(&started.message_id) {
                return Ok(Vec::new());
            }
            project_message_started(streams, started.message_id)
        }
        SessionEventType::AssistantMessageDelta => {
            let delta: AssistantMessageDeltaData = decode(event)?;
            if routes.holds_message(&delta.message_id) {
                return Ok(Vec::new());
            }
            Ok(project_message_delta(
                streams,
                &delta.message_id,
                delta.delta_content,
            ))
        }
        SessionEventType::AssistantMessage => {
            let message: AssistantMessageData = decode(event)?;
            let concrete = |model: Option<&str>| {
                model
                    .and_then(ModelId::named)
                    .filter(|model| model.as_str() != AUTO_MODEL_ID)
            };
            let actual = concrete(message.model.as_deref()).or_else(|| {
                concrete(
                    fusion
                        .as_ref()
                        .and_then(|route| route.source_model.as_deref()),
                )
            });
            let unscoped_child = matches!(routing.conversation, ConversationKind::Main)
                && has_parent_tool_call(event);
            let reported_id = message.api_call_id.as_deref().filter(|id| !id.is_empty());
            let current_call = !unscoped_child
                && streams.observe_model_call(
                    reported_id,
                    routing.previous_calls,
                    routing.selected.is_some(),
                );
            let auto_call = current_call
                && !reported_id.is_some_and(|id| streams.fixed_model_calls.contains(id))
                && !(reported_id.is_none() && streams.anonymous_fixed_call);
            let call_id = reported_id.filter(|_| auto_call);
            let reported_call = call_id
                .and_then(|id| streams.reported_model_calls.get(id))
                .cloned();
            let auto = ModelId::new(AUTO_MODEL_ID);
            let selected = (auto_call
                && streams.call_selected_auto(reported_id, routing.selected.is_some()))
            .then_some(&auto)
            .or_else(|| reported_call.as_ref().map(|call| &call.selected));
            let provider = ProviderId::new(super::COPILOT_PROVIDER_ID);
            let model = selected.and_then(|selected| {
                actual.as_ref().map_or_else(
                    || reported_call.clone(),
                    |actual| {
                        actual_model::attribution(
                            SelectedModel::new(&provider, selected),
                            actual.as_str(),
                        )
                    },
                )
            });
            let mut projected = if let Some(fusion) = fusion {
                if !routes.admit_message(&fusion, &message, stretch) {
                    let message_id = ProviderMessageId::new(message.message_id.clone());
                    if streams.identified_messages.contains(&message_id) {
                        if let Some(model) = &model {
                            let mut associated_calls = Vec::new();
                            for (id, entries) in &mut streams.pending_model_messages {
                                for pending in entries {
                                    if pending.message == message_id {
                                        pending.actual = Some(model.actual.clone());
                                        associated_calls.push(id.clone());
                                    }
                                }
                            }
                            if let Some(call_id) = call_id {
                                associated_calls.push(call_id.to_owned());
                            }
                            for id in associated_calls {
                                remember_attributed_message(streams, &id, &message_id);
                            }
                            if reported_id.is_none()
                                && !streams.anonymous_attributed_messages.contains(&message_id)
                            {
                                streams
                                    .anonymous_attributed_messages
                                    .push(message_id.clone());
                            }
                        } else if let Some(call_id) = call_id
                            && selected.is_some()
                        {
                            let pending = streams
                                .pending_model_messages
                                .entry(call_id.to_owned())
                                .or_default();
                            if !pending.iter().any(|entry| entry.message == message_id) {
                                pending.push(PendingModelMessage {
                                    message: message_id.clone(),
                                    actual: None,
                                });
                            }
                        }
                    }
                    return Ok(model
                        .filter(|_| streams.identified_messages.contains(&message_id))
                        .map(|model| ProviderEvent::AgentMessageModelKnown {
                            message: Some(message_id),
                            model,
                        })
                        .into_iter()
                        .collect());
                }
                project_routed_message(streams, &message)?
            } else {
                project_message_completed(streams, &message.message_id, &message.content)?
            };
            if let Some(mut index) = projected
                .iter()
                .rposition(|event| matches!(event, ProviderEvent::AgentMessageCompleted))
            {
                if let Some(call_id) = call_id.filter(|_| {
                    routing.selected.is_some()
                        || matches!(routing.conversation, ConversationKind::Subagent)
                }) {
                    let provider_message = ProviderMessageId::new(message.message_id.clone());
                    if !projected.iter().any(|event| {
                        matches!(event, ProviderEvent::AgentMessageIdentified { message }
                            if message == &provider_message)
                    }) {
                        projected.insert(
                            index,
                            ProviderEvent::AgentMessageIdentified {
                                message: provider_message.clone(),
                            },
                        );
                        index += 1;
                    }
                    if model.is_none() {
                        streams
                            .pending_model_messages
                            .entry(call_id.to_owned())
                            .or_default()
                            .push(PendingModelMessage {
                                message: provider_message,
                                actual,
                            });
                    }
                }
                if reported_id.is_none() && model.is_some() {
                    let provider_message = ProviderMessageId::new(message.message_id.clone());
                    if !projected.iter().any(|event| {
                        matches!(event, ProviderEvent::AgentMessageIdentified { message }
                            if message == &provider_message)
                    }) {
                        projected.insert(
                            index,
                            ProviderEvent::AgentMessageIdentified {
                                message: provider_message,
                            },
                        );
                        index += 1;
                    }
                }
                if let Some(model) = model {
                    projected.insert(
                        index,
                        ProviderEvent::AgentMessageModelKnown {
                            message: None,
                            model,
                        },
                    );
                }
            }
            streams
                .identified_messages
                .extend(projected.iter().filter_map(|event| {
                    if let ProviderEvent::AgentMessageIdentified { message } = event {
                        Some(message.clone())
                    } else {
                        None
                    }
                }));
            if let Some(call_id) = call_id
                && projected
                    .iter()
                    .any(|event| matches!(event, ProviderEvent::AgentMessageModelKnown { .. }))
            {
                let message_id = ProviderMessageId::new(message.message_id.clone());
                if streams.identified_messages.contains(&message_id) {
                    remember_attributed_message(streams, call_id, &message_id);
                }
            } else if reported_id.is_none()
                && projected
                    .iter()
                    .any(|event| matches!(event, ProviderEvent::AgentMessageModelKnown { .. }))
            {
                let message_id = ProviderMessageId::new(message.message_id.clone());
                if streams.identified_messages.contains(&message_id)
                    && !streams.anonymous_attributed_messages.contains(&message_id)
                {
                    streams.anonymous_attributed_messages.push(message_id);
                }
            }
            Ok(projected)
        }
        SessionEventType::ToolExecutionStart => Ok(reported(event).map_or_else(
            Vec::new,
            |started: ToolExecutionStartData| {
                if fusion.is_some_and(|fusion| {
                    !routes.admit_tool(&fusion, &started.tool_call_id, stretch)
                }) {
                    return Vec::new();
                }
                project_tool_started(streams, &started, execution_directory)
            },
        )),
        // A running shell streams its output as `tool.shell_output` chunks, each appending to what
        // came before. The `tool.execution_partial_result` Copilot still sends beside them is the
        // output so far, repeated whole each time, so it carries nothing the chunks have not
        // (docs/validation/copilot-tool-output-streaming.md).
        SessionEventType::ToolShellOutput => Ok(reported(event).map_or_else(
            Vec::new,
            |output: ToolShellOutputData| {
                project_tool_output(streams, &output.tool_call_id, output.text)
            },
        )),
        SessionEventType::ToolExecutionComplete => Ok(reported(event).map_or_else(
            Vec::new,
            |completed: ToolExecutionCompleteData| {
                project_tool_completed(streams, delegations, &completed)
            },
        )),
        SessionEventType::AssistantReasoningDelta => Ok(reported(event).map_or_else(
            Vec::new,
            |delta: AssistantReasoningDeltaData| {
                project_reasoning_delta(streams, &delta.reasoning_id, &delta.delta_content)
            },
        )),
        SessionEventType::AssistantReasoning => Ok(reported(event).map_or_else(
            Vec::new,
            |reasoning: AssistantReasoningData| {
                project_reasoning_completed(streams, &reasoning.reasoning_id, &reasoning.content)
            },
        )),
        _ => Ok(Vec::new()),
    }
}

impl CopilotCorrelation {
    /// Opens the Subagent a `subagent.started` reports, in the conversation that spawned it. The
    /// envelope's instance identity is what the Subagent's every event is attributed with; a
    /// started that carries none leaves the spawning tool call standing in, so the Subagent's row
    /// and settle still reach the Transcript even though no work can ever be attributed to it.
    /// The spawn's Delegation is the prompt its tool call's execution carried, where that
    /// execution started first; the runtime's own delegation, which never surfaces as one, opens
    /// with none.
    fn project_subagent_started(&mut self, event: &SessionEvent) -> Vec<AttributedProviderEvent> {
        let Some(started) = reported::<SubagentStartedData>(event) else {
            return Vec::new();
        };
        let subagent = event
            .agent_id
            .clone()
            .unwrap_or_else(|| started.tool_call_id.clone());
        if self.subagents.contains_key(&subagent) {
            return Vec::new();
        }
        // A spawn out of a Subagent's own conversation — the spawning tool call is one that
        // Subagent is running — recurses one level down; every other spawn is the main agent's,
        // and lands only inside a Turn Suru is running or the Continuation late work opens.
        let spawner = self.spawning_conversation(&started.tool_call_id);
        if spawner.is_none() && self.open_main_streams().is_none() {
            return Vec::new();
        }
        self.subagents
            .insert(subagent.clone(), WorkingSubagent::new(Stretch::Spawned));
        self.spawns
            .insert(started.tool_call_id.clone(), subagent.clone());
        self.delegations.insert(started.tool_call_id.clone());
        // Copilot's display name is the readable one, but a spawn made through its task tool
        // writes the invocation's description there, leaving the configured name the stable one.
        let name = if started.agent_display_name.is_empty() {
            started.agent_name
        } else {
            started.agent_display_name
        };
        let model = started.model.filter(|model| !model.is_empty());
        self.agents.insert(
            subagent.clone(),
            KnownSubagent {
                name: name.clone(),
                description: started.agent_description.clone(),
                auto_selection: model.as_deref() == Some(AUTO_MODEL_ID),
                model: model.clone(),
                previous_model_calls: HashSet::new(),
                compaction: Compacting::Idle,
            },
        );
        let delegation = self.spawn_prompt(spawner.as_deref(), &started.tool_call_id);
        let mut projected = vec![attributed(
            spawner.as_deref(),
            ProviderEvent::SubagentStarted {
                subagent_id: ProviderSubagentId::new(subagent.clone()),
                name,
                description: started.agent_description,
                delegation,
            },
        )];
        if let Some(model) = model {
            projected.push(attributed(
                spawner.as_deref(),
                ProviderEvent::SubagentModelChanged {
                    subagent_id: ProviderSubagentId::new(subagent),
                    model: crate::protocol::ModelId::new(model),
                },
            ));
        }
        projected
    }

    /// The prompt the spawning tool call handed its Subagent, read off its withheld execution in
    /// the conversation that ran it — a Subagent's own, or the main agent's.
    fn spawn_prompt(&self, spawner: Option<&str>, tool_call_id: &str) -> Option<String> {
        let streams = match spawner {
            Some(subagent) => &self.subagents.get(subagent)?.streams,
            None => &self.turn.as_ref()?.streams,
        };
        match streams.tools.get(tool_call_id)? {
            RunningTool::Spawn { prompt, .. } => prompt.clone(),
            _ => None,
        }
    }

    /// The Subagent whose conversation ran `tool_call_id`, or `None` for the main agent's own —
    /// which is also the answer for a spawn whose tool call never surfaced as an execution, the
    /// shape the runtime's own delegation takes.
    fn spawning_conversation(&self, tool_call_id: &str) -> Option<String> {
        self.subagents
            .iter()
            .find(|(_, working)| working.streams.tools.contains_key(tool_call_id))
            .map(|(subagent, _)| subagent.clone())
    }

    /// Reads a message Copilot delivered to a Subagent's own loop, which Copilot reports when the
    /// Subagent consumes it. Whoever sent it, it is attributed to the Agent `source` names —
    /// `agent-<id>` names a sibling Subagent by its instance identity, and every other origin, the
    /// main agent's `agent-<session id>` among them, is the owning Session's.
    ///
    /// One delivered into the stretch a working Subagent is in (`delivery: "steering"`) is a
    /// steer: a Delegation standing at the point the Subagent received it, which is where this
    /// event arrives (ADR 0032). The CLIs verified against never deliver a `write_agent` that way
    /// (docs/validation/0397-copilot-subagent-steer.md).
    ///
    /// Any other message a working spawn receives is its own prompt, which the spawning tool call
    /// already carried in as the Delegation opening its Turn: Copilot holds every later send back
    /// until the stretch ends, so no other message reaches a spawn while it works. Every message a settled Subagent consumes runs it again on its own context — whatever
    /// its `delivery`: `queued` when it was sent during the stretch and held until that stretch's
    /// `subagent.completed`, `idle` when it was sent afterwards — so it is a resume (ADR 0033),
    /// opening the new Turn as its Delegation ([`Self::project_resumed`]). Its arrival is also
    /// proof the previous run ended, so it settles a resume of that Subagent still open before it
    /// begins the next — or the Continuation a compaction of its context began, which that
    /// compaction outlasts. A message for an instance no Subagent holds stands nowhere.
    fn project_subagent_message(
        &mut self,
        subagent: &str,
        event: &SessionEvent,
    ) -> Vec<AttributedProviderEvent> {
        let Some(message) = reported::<UserMessageData>(event) else {
            return Vec::new();
        };
        // A Subagent that queued a copy of its own send to itself is not its own sender: the
        // resume that copy begins is the owning Session's.
        let sender = message
            .source
            .as_deref()
            .and_then(|source| source.strip_prefix("agent-"))
            .filter(|sender| *sender != subagent && self.agents.contains_key(*sender))
            .map(str::to_owned);
        match self.subagents.get(subagent).map(|working| working.stretch) {
            Some(_) if message.delivery == Some(UserMessageDelivery::Steering) => {
                if message.content.trim().is_empty() {
                    return Vec::new();
                }
                vec![attributed(
                    sender.as_deref(),
                    ProviderEvent::SubagentSteered {
                        subagent_id: ProviderSubagentId::new(subagent),
                        delegation: message.content,
                    },
                )]
            }
            Some(Stretch::Spawned) => Vec::new(),
            Some(Stretch::Resumed { .. } | Stretch::Compacting) => {
                let mut projected =
                    self.project_stretch_settled(subagent, ProviderSubagentStatus::Completed);
                projected.extend(self.project_resumed(subagent, sender, message.content));
                projected
            }
            None => self.project_resumed(subagent, sender, message.content),
        }
    }

    /// Resumes a settled Subagent with the message it consumed (ADR 0033): routes its work to it
    /// again, in a stretch its loop's exit settles, and reports the resume from the Agent that
    /// sent it, the message opening the resumed Turn as its Delegation and its first line
    /// describing the stretch's row. The Model the Subagent last ran on carries into that Turn,
    /// because Copilot reports a Subagent's Model only at spawn. A resume the owning Session
    /// delegates while no Turn is running begins a Continuation to hold its row, which the loop's
    /// next idle settles. An instance no Subagent ever held has nothing to resume.
    fn project_resumed(
        &mut self,
        subagent: &str,
        sender: Option<String>,
        content: String,
    ) -> Vec<AttributedProviderEvent> {
        let Some(known) = self.agents.get(subagent) else {
            return Vec::new();
        };
        let delegation = (!content.trim().is_empty()).then_some(content);
        let description = delegation
            .as_deref()
            .and_then(first_line)
            .unwrap_or_else(|| known.description.clone());
        let (name, model) = (known.name.clone(), known.model.clone());
        if sender.is_none() && self.turn.is_none() {
            self.begin_continuation();
        }
        self.subagents.insert(
            subagent.to_owned(),
            WorkingSubagent::new(Stretch::Resumed { answered: false }),
        );
        let mut projected = vec![attributed(
            sender.as_deref(),
            ProviderEvent::SubagentResumed {
                subagent_id: ProviderSubagentId::new(subagent),
                name,
                description,
                delegation,
            },
        )];
        if let Some(model) = model {
            projected.push(attributed(
                None,
                ProviderEvent::SubagentModelChanged {
                    subagent_id: ProviderSubagentId::new(subagent),
                    model: crate::protocol::ModelId::new(model),
                },
            ));
        }
        projected
    }

    /// Settles a resumed Subagent's stretch where its agent loop exits: the turn end following a
    /// model message that requested no tools. Like any Subagent settle, the output it provokes in
    /// the loop that reads it arrives only afterwards, so it leaves a Continuation owed to it. A
    /// spawn's turn ends settle nothing: Copilot settles a spawn itself.
    fn project_subagent_turn_end(&mut self, subagent: &str) -> Vec<AttributedProviderEvent> {
        let exits = self
            .subagents
            .get(subagent)
            .is_some_and(|working| working.stretch.exits_at_turn_end());
        if !exits {
            return Vec::new();
        }
        self.late_settle_owes_continuation = true;
        self.project_stretch_settled(subagent, ProviderSubagentStatus::Completed)
    }

    /// Settles every resumed stretch still open when the loop goes idle — interrupted, when the
    /// idle is an abort's — since Copilot defers the idle while any agent runs. Spawns are
    /// Copilot's own to settle.
    fn project_resumes_stopped(&mut self, aborted: bool) -> Vec<AttributedProviderEvent> {
        let mut resumed = self
            .subagents
            .iter()
            .filter(|(_, working)| working.stretch.is_resumed())
            .map(|(subagent, _)| subagent.clone())
            .collect::<Vec<_>>();
        resumed.sort_unstable();
        let status = if aborted {
            ProviderSubagentStatus::Interrupted
        } else {
            ProviderSubagentStatus::Completed
        };
        resumed
            .iter()
            .flat_map(|subagent| self.project_stretch_settled(subagent, status))
            .collect()
    }

    /// Settles the Subagent the spawning tool call names. A settle for a spawn never opened lands
    /// nowhere, like any other unrouted Subagent event.
    fn project_subagent_settled(
        &mut self,
        tool_call_id: &str,
        status: ProviderSubagentStatus,
    ) -> Vec<AttributedProviderEvent> {
        let Some(subagent) = self.spawns.remove(tool_call_id) else {
            return Vec::new();
        };
        // The output this completion provokes arrives only after the settle, so the settle
        // leaves a Continuation owed to it (ADR 0015).
        self.late_settle_owes_continuation = true;
        self.project_stretch_settled(&subagent, status)
    }

    /// Settles the stretch a Subagent is working, closing whatever its streams leave open — the
    /// split-held Reasoning title above all — before the settle itself drops the routes. A
    /// compaction still running settles with the stretch, and its end is owed nothing.
    fn project_stretch_settled(
        &mut self,
        subagent: &str,
        status: ProviderSubagentStatus,
    ) -> Vec<AttributedProviderEvent> {
        let mut streams = self
            .subagents
            .remove(subagent)
            .map(|working| working.streams)
            .unwrap_or_default();
        if let Some(known) = self.agents.get_mut(subagent) {
            known.compaction.outlived();
            known
                .previous_model_calls
                .extend(streams.observed_model_calls.iter().cloned());
        }
        let mut projected: Vec<AttributedProviderEvent> =
            settle_open_streams(&mut streams, &self.delegations, OpenReasoning::Complete)
                .into_iter()
                .map(|event| attributed(Some(subagent), event))
                .collect();
        projected.push(attributed(
            None,
            ProviderEvent::SubagentCompleted {
                subagent_id: ProviderSubagentId::new(subagent),
                status,
            },
        ));
        projected
    }

    /// Starts a Watch for every background shell the Session's task roster lists as running that
    /// is not one already, and — while a Turn is running — settles the Watch of every shell the
    /// roster lists as ended or no longer lists at all.
    ///
    /// A background shell is one whose tool call returned at once, leaving it to run while the
    /// loop works on and goes idle: a *detached* shell, or an *attached* one run in the
    /// background (`mode: "async"`). An attached shell run synchronously is listed too, but its
    /// tool call is still waiting on it, so it is the Command that is running rather than a Watch;
    /// and an agent task is a Subagent. A shell listed already ended is left alone: there is
    /// nothing left to wait on, and the completion notification it raises records its Watch
    /// whole.
    ///
    /// Copilot delivers a shell's completion as a notification only once its loop is idle. A shell
    /// the loop waits out with `read_bash` or kills with `stop_bash` while it works raises none,
    /// so the roster is all that says it ended: listed completed, failed, or cancelled, or —
    /// stopped — gone from it. Its Watch settles there, waking nothing, since the loop that ended
    /// it is already working. While the loop is idle the roster is left to the notification, which
    /// the CLI raises just after the roster lists the shell ended, and which wakes the loop.
    ///
    /// The roster's entries are the SDK's `TaskShellInfo` and `TaskAgentInfo`, told apart by
    /// their `type`, which is why an entry that is not a shell this build can read is passed
    /// over — with a warning, since a shell entry that no longer parses is the CLI drifting from
    /// the SDK — rather than failing the read.
    pub(super) fn project_task_roster(
        &mut self,
        tasks: &[serde_json::Value],
    ) -> Vec<AttributedProviderEvent> {
        let shells = tasks
            .iter()
            .filter(|task| task.get("type").and_then(serde_json::Value::as_str) == Some("shell"))
            .filter_map(|task| {
                serde_json::from_value::<TaskShellInfo>(task.clone())
                    .inspect_err(|error| {
                        tracing::warn!(
                            "passed over a Copilot task roster shell Suru could not read: {error}"
                        );
                    })
                    .ok()
            })
            .filter(|shell| !shell.id.is_empty())
            .collect::<Vec<_>>();
        let mut projected = Vec::new();
        if self.turn.is_some() {
            let mut ended = self
                .watches
                .iter()
                .filter_map(|watched| {
                    let outcome = match shells.iter().find(|shell| shell.id == *watched) {
                        None => ProviderWatchOutcome::Stopped,
                        Some(shell) => match shell.status {
                            TaskStatus::Completed => ProviderWatchOutcome::Completed,
                            TaskStatus::Failed => ProviderWatchOutcome::Failed,
                            TaskStatus::Cancelled => ProviderWatchOutcome::Stopped,
                            _ => return None,
                        },
                    };
                    Some((watched.clone(), outcome))
                })
                .collect::<Vec<_>>();
            ended.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            for (shell, outcome) in ended {
                self.watches.remove(&shell);
                projected.push(attributed(
                    None,
                    ProviderEvent::WatchSettled {
                        watch_id: ProviderWatchId::new(shell),
                        outcome,
                        summary: None,
                        woke_agent: false,
                    },
                ));
            }
        }
        for shell in shells {
            let background = shell.attachment_mode == TaskShellInfoAttachmentMode::Detached
                || shell.execution_mode == Some(TaskExecutionMode::Background);
            if !background
                || !matches!(shell.status, TaskStatus::Running | TaskStatus::Idle)
                || !self.watched_shells.insert(shell.id.clone())
            {
                continue;
            }
            self.watches.insert(shell.id.clone());
            let description = [shell.description, shell.command]
                .into_iter()
                .find(|text| !text.trim().is_empty())
                .unwrap_or_else(|| shell.id.clone());
            projected.push(attributed(
                None,
                ProviderEvent::WatchStarted {
                    watch_id: ProviderWatchId::new(shell.id),
                    description,
                },
            ));
        }
        projected
    }

    /// Reads a notification Copilot delivers to the main loop. The one this projection acts on is
    /// a background shell completing, naming the shell by `shellId`: an attached one
    /// (`kind.type: "shell_completed"`) settles its Watch as completed or failed by the
    /// `exitCode` it carries, and a detached one (`"shell_detached_completed"`), which carries no
    /// exit code, as completed. Either settles with the notification's text, less the
    /// `<system_notification>` wrapper the loop reads it in, as Copilot's account of how it
    /// settled.
    ///
    /// The notification is what wakes the loop, and orchestration owes the woken output a
    /// Continuation only through the settle of a Watch it knows. So a shell never seen as a Watch
    /// — its roster read failed, or found it already finished because it ran shorter than the
    /// round trip — starts its Watch here, described in the notification's own words, and
    /// settles it at once: the woken work is still shown, headed by how the shell settled. A
    /// shell whose Watch already settled — stopped by an interrupt, which records nothing (ADR
    /// 0030) — wakes nothing Suru shows. While a Turn is running the woken work lands in it and
    /// nothing is owed. Every other kind wakes nothing this projection follows.
    fn project_system_notification(
        &mut self,
        notification: &SystemNotificationData,
    ) -> Vec<AttributedProviderEvent> {
        let kind = &notification.kind;
        let outcome = match kind.get("type").and_then(serde_json::Value::as_str) {
            Some("shell_detached_completed") => ProviderWatchOutcome::Completed,
            Some("shell_completed") => {
                match kind.get("exitCode").and_then(serde_json::Value::as_i64) {
                    Some(0) | None => ProviderWatchOutcome::Completed,
                    Some(_) => ProviderWatchOutcome::Failed,
                }
            }
            _ => return Vec::new(),
        };
        let Some(shell) = kind
            .get("shellId")
            .and_then(serde_json::Value::as_str)
            .filter(|shell| !shell.is_empty())
        else {
            return Vec::new();
        };
        let mut projected = Vec::with_capacity(2);
        if self.watched_shells.insert(shell.to_owned()) {
            let description = kind
                .get("description")
                .and_then(serde_json::Value::as_str)
                .filter(|description| !description.trim().is_empty())
                .unwrap_or(shell)
                .to_owned();
            projected.push(attributed(
                None,
                ProviderEvent::WatchStarted {
                    watch_id: ProviderWatchId::new(shell),
                    description,
                },
            ));
        } else if !self.watches.remove(shell) {
            return Vec::new();
        }
        if self.turn.is_none() {
            self.late_settle_owes_continuation = true;
        }
        projected.push(attributed(
            None,
            ProviderEvent::WatchSettled {
                watch_id: ProviderWatchId::new(shell),
                outcome,
                summary: notification_text(&notification.content),
                woke_agent: true,
            },
        ));
        projected
    }

    /// Those of `watches` still live, which are all a stop has left to stop.
    pub(super) fn live_watches(&self, watches: &[ProviderWatchId]) -> Vec<String> {
        watches
            .iter()
            .map(ProviderWatchId::as_str)
            .filter(|shell| self.watches.contains(*shell))
            .map(str::to_owned)
            .collect()
    }

    /// Background shells the Session stopped and Copilot confirmed cancelling. Each still a live
    /// Watch settles as stopped, waking nothing, so the Session stops Monitoring on the
    /// confirmation alone; a completion notification arriving afterwards finds the Watch already
    /// settled and repeats nothing.
    pub(super) fn project_watches_stopped(
        &mut self,
        shells: &[String],
    ) -> Vec<AttributedProviderEvent> {
        shells
            .iter()
            .filter(|shell| self.watches.remove(*shell))
            .map(|shell| {
                attributed(
                    None,
                    ProviderEvent::WatchSettled {
                        watch_id: ProviderWatchId::new(shell.clone()),
                        outcome: ProviderWatchOutcome::Stopped,
                        summary: None,
                        woke_agent: false,
                    },
                )
            })
            .collect()
    }

    /// The harness process hosting the Session died. A background shell may outlive it, but nothing is
    /// left to deliver its completion to the loop, so it can wake nothing: every live Watch
    /// settles as lost.
    fn project_watches_lost(&mut self) -> Vec<AttributedProviderEvent> {
        let mut lost = std::mem::take(&mut self.watches)
            .into_iter()
            .collect::<Vec<_>>();
        lost.sort_unstable();
        lost.into_iter()
            .map(|shell| {
                attributed(
                    None,
                    ProviderEvent::WatchSettled {
                        watch_id: ProviderWatchId::new(shell),
                        outcome: ProviderWatchOutcome::Lost,
                        summary: None,
                        woke_agent: false,
                    },
                )
            })
            .collect()
    }

    /// Records what the Turn will settle as when Copilot reports an error — authentication,
    /// quota, rate limit, and the rest — naming the category Copilot typed it as, so a remote
    /// failure reads without opening the Log. The loop stopping is what settles the Turn on it.
    fn project_session_error(&mut self, failure: &SessionErrorData) {
        let Some(turn) = self.open_main_turn() else {
            return;
        };
        let kind = failure.error_type.replace(['_', '-'], " ");
        turn.failure.get_or_insert_with(|| {
            concise_remote_message(
                &format!("Copilot {kind} error: {}", failure.message),
                COPILOT_FAILURE_FALLBACK,
            )
        });
    }

    /// Projects one of Copilot's reports on compacting the main conversation's context: `event` is
    /// what [`compaction_event`] read it as.
    ///
    /// Copilot compacts in the background of the Session rather than in a stretch of its loop, so
    /// a report arriving while no Turn runs begins a Continuation of its own, whose loop is idle:
    /// the compaction settling settles it, since no idle will. Copilot owns that Continuation, and
    /// a Continuation late output began becomes Copilot's once Copilot compacts in it: either is
    /// reported with [`ProviderEvent::ContinuationStarted`], so an interrupt, or the next Prompt,
    /// cancels the compaction rather than leaving it to finish in a Turn Suru no longer holds it
    /// in. The end of a compaction Suru stopped following is owed nothing, and one an interrupt is
    /// cancelling leaves the stretch for the interrupt to settle — as does one the cancel found
    /// already over, until the interrupt has decided whether an abort's idle settles it instead.
    fn project_compaction(&mut self, event: ProviderEvent) -> Vec<AttributedProviderEvent> {
        let started = matches!(event, ProviderEvent::CompactionStarted);
        if !started {
            match &self.compaction {
                Compacting::Stopped => {
                    self.compaction = Compacting::Idle;
                    return Vec::new();
                }
                // The end raced the cancel. One that completed did compact; one that failed is
                // kept until Copilot answers the cancel, which says whose failure it was. Either
                // way a stretch the compaction held is left for the interrupt to settle.
                Compacting::Cancelling => {
                    return match event {
                        ProviderEvent::CompactionFailed { error } => {
                            self.compaction = Compacting::FailedWhileCancelling { error };
                            Vec::new()
                        }
                        event => {
                            self.compaction = Compacting::Idle;
                            if self.turn.is_some() {
                                vec![attributed(None, event)]
                            } else {
                                Vec::new()
                            }
                        }
                    };
                }
                // The end of the one already kept.
                Compacting::FailedWhileCancelling { .. } | Compacting::Failed { .. } => {
                    return Vec::new();
                }
                // The end of one Copilot had already finished when the cancel reached it, which
                // stands as Copilot reports it, in the stretch it held — and settles that
                // stretch only once the interrupt has left it to.
                Compacting::Ended { settles } => {
                    let settles = *settles;
                    self.compaction = Compacting::Idle;
                    let Some(turn) = self.turn.as_ref() else {
                        return Vec::new();
                    };
                    let idled = turn.idled.filter(|_| settles);
                    let mut projected = vec![attributed(None, event)];
                    if let Some(aborted) = idled {
                        projected.extend(
                            self.settle_turn(aborted)
                                .into_iter()
                                .map(|settled| attributed(None, settled)),
                        );
                    }
                    return projected;
                }
                Compacting::Idle | Compacting::Running => {}
            }
        }
        self.compaction = if started {
            Compacting::Running
        } else {
            Compacting::Idle
        };
        let mut projected = Vec::with_capacity(3);
        if self.turn.is_none() {
            self.begin_continuation();
            if let Some(turn) = self.turn.as_mut() {
                turn.idled = Some(false);
            }
        }
        if let Some(turn) = self.turn.as_mut()
            && turn.continuation
            && !turn.owned
        {
            turn.owned = true;
            projected.push(attributed(
                None,
                ProviderEvent::ContinuationStarted {
                    selection: self.selection.clone(),
                },
            ));
        }
        projected.push(attributed(None, event));
        if !started && let Some(aborted) = self.turn.as_ref().and_then(|turn| turn.idled) {
            projected.extend(
                self.settle_turn(aborted)
                    .into_iter()
                    .map(|settled| attributed(None, settled)),
            );
        }
        projected
    }

    /// A compaction report for a Subagent's conversation, which stands in the stretch the Subagent
    /// is working. One Copilot begins while the Subagent works none is work of the Subagent's own,
    /// as its own Watch waking it would be: it begins a Continuation of the Subagent's Session,
    /// which the compaction ending settles, since no loop runs there to end it. The end of a
    /// compaction a settled stretch held records nothing, least of all in the stretch resuming
    /// the Subagent.
    fn project_subagent_compaction(
        &mut self,
        subagent: &str,
        event: ProviderEvent,
    ) -> Vec<AttributedProviderEvent> {
        let started = matches!(event, ProviderEvent::CompactionStarted);
        let Some(known) = self.agents.get_mut(subagent) else {
            return Vec::new();
        };
        if !known.compaction.follows(started) {
            return Vec::new();
        }
        let mut projected = Vec::with_capacity(3);
        if !self.subagents.contains_key(subagent) {
            self.subagents.insert(
                subagent.to_owned(),
                WorkingSubagent::new(Stretch::Compacting),
            );
            projected.push(attributed(
                None,
                ProviderEvent::SubagentWoken {
                    subagent_id: ProviderSubagentId::new(subagent),
                },
            ));
        }
        projected.push(attributed(Some(subagent), event));
        if !started
            && self
                .subagents
                .get(subagent)
                .is_some_and(|working| working.stretch == Stretch::Compacting)
        {
            projected
                .extend(self.project_stretch_settled(subagent, ProviderSubagentStatus::Completed));
        }
        projected
    }

    /// Begins an interrupt of whatever runs, returning what it is for. A compaction Copilot is
    /// running is being cancelled from here: until Copilot answers, it still holds its stretch
    /// open, and the interrupt settles that stretch once it has — or, should the cancel find the
    /// compaction already over, leaves it to the compaction's own end.
    pub(super) fn begin_interrupt(&mut self) -> InterruptScope {
        self.sending_abort();
        let compacting = self.compaction == Compacting::Running;
        if compacting {
            self.compaction = Compacting::Cancelling;
        }
        InterruptScope {
            stretch: self.turn.as_ref().map(|_| self.stretches),
            compacting,
        }
    }

    /// Copilot failed to cancel the compaction the interrupt began cancelling, which is still
    /// followed. A failure Copilot reported meanwhile was the compaction's own, since the cancel
    /// never took, and settles it with Copilot's error. If it ended and its stretch is held on it,
    /// nothing is left to settle that stretch but this: it settles as its loop's idle said.
    pub(super) fn cancel_failed(&mut self) -> Vec<AttributedProviderEvent> {
        let mut projected = Vec::new();
        match std::mem::take(&mut self.compaction) {
            Compacting::Cancelling => {
                self.compaction = Compacting::Running;
                return projected;
            }
            Compacting::FailedWhileCancelling { error } => {
                if self.turn.is_some() {
                    projected.push(attributed(None, ProviderEvent::CompactionFailed { error }));
                }
            }
            other => self.compaction = other,
        }
        if let Some(aborted) = self.turn.as_ref().and_then(|turn| turn.idled)
            && !self.compaction.holds_stretch()
        {
            projected.extend(
                self.settle_turn(aborted)
                    .into_iter()
                    .map(|settled| attributed(None, settled)),
            );
        }
        projected
    }

    /// Reads Copilot's answer to the cancel the interrupt `scope` began, if one was sent:
    /// `cancelled` says it found the compaction still running to cancel. A failure Copilot
    /// reported while the cancel was out is the cancel taking when it found something to cancel,
    /// and the compaction's own when it found nothing. Finding nothing before any report of the
    /// compaction's end means Copilot had already ended it, and the report still on its way is
    /// its outcome: the compaction is followed still ([`Compacting::Ended`]).
    ///
    /// Answers what the interrupt waits on before it goes on: the timeline carrying that report,
    /// which it then projects ahead of anything that follows — the idle of any abort the interrupt
    /// sends, and anything of the Turn after — in the stretch the compaction stood in. Nothing,
    /// when the timeline already has. A compaction an earlier interrupt found over is waited on
    /// the same way.
    pub(super) fn cancel_answered(
        &mut self,
        scope: InterruptScope,
        cancelled: bool,
    ) -> Option<Arc<Notify>> {
        let current = scope.stretch == Some(self.stretches) && self.turn.is_some();
        self.compaction = match std::mem::take(&mut self.compaction) {
            Compacting::Cancelling if !cancelled && current => Compacting::Ended { settles: false },
            Compacting::Cancelling => Compacting::Stopped,
            Compacting::FailedWhileCancelling { error } if !cancelled && current => {
                Compacting::Failed { error }
            }
            Compacting::FailedWhileCancelling { .. } => Compacting::Idle,
            other => other,
        };
        let on_its_way = self.compaction_ends_carried > self.compaction_ends_read;
        if !matches!(self.compaction, Compacting::Ended { .. }) || on_its_way {
            return None;
        }
        let carried = Arc::new(Notify::new());
        self.compaction_end = Some(carried.clone());
        Some(carried)
    }

    /// The timeline has carried the end of a background compaction of the main conversation — to
    /// the projection's queue, ahead of whatever follows it — which an interrupt may be waiting
    /// on.
    fn compaction_end_carried(&mut self) {
        self.compaction_ends_carried += 1;
        if let Some(carried) = self.compaction_end.take() {
            carried.notify_one();
        }
    }

    /// Copilot's report of the end of a compaction its cancel found already over never reached
    /// the timeline within the interrupt's wait. Suru follows the compaction no further: it
    /// settles with the stretch it stood in, as one the interrupt stopped, and its report, should
    /// it come after all, is owed nothing.
    pub(super) fn compaction_end_missing(&mut self) {
        self.compaction_end = None;
        if matches!(self.compaction, Compacting::Ended { .. }) {
            tracing::warn!(
                "Copilot found no compaction left to cancel, but its report of how that compaction \
                 ended never reached Suru; it settles with its Turn as stopped"
            );
            self.compaction = Compacting::Stopped;
        }
    }

    /// What is left of the interrupt `scope` began once Copilot has answered the cancel it sent,
    /// if any ([`Self::cancel_answered`]). Only the stretch it was for is its to stop: one that
    /// settled meanwhile left nothing running, and whatever began since is no business of this
    /// interrupt. A loop still running is aborted, and the idle the abort ends in settles its
    /// stretch. A stretch whose loop had already stopped settles here as interrupted, since
    /// nothing else will report its end — unless Subagents work on past it: the abort they need
    /// ends in an idle of the loop's own, which settles the stretch instead, so that idle cannot
    /// land in whatever Turn begins next. The interrupt waits for it ([`Self::expect_abort_idle`]).
    ///
    /// A stopped loop's stretch that a compaction held is stopped by nothing, though, when the
    /// cancel found the compaction already over: the interrupt stopped nothing, and the stretch
    /// settles as its loop's idle said — on the compaction's end once that lands, if it has yet
    /// to.
    pub(super) fn finish_interrupt(
        &mut self,
        scope: InterruptScope,
        cancelled: bool,
    ) -> InterruptRemainder {
        let running = InterruptRemainder {
            abort: true,
            settled: Vec::new(),
            awaits_idle: false,
            already_ended: false,
        };
        let Some(stretch) = scope.stretch else {
            return running;
        };
        let Some(idled) = self
            .turn
            .as_ref()
            .filter(|_| self.stretches == stretch)
            .map(|turn| turn.idled)
        else {
            return InterruptRemainder {
                abort: false,
                ..running
            };
        };
        let subagents_work = self
            .subagents
            .values()
            .any(|working| working.stretch != Stretch::Compacting);
        // The end of a compaction the cancel found already over settles the stretch should its
        // loop have gone idle by then — a working loop may yet — unless the idle of the abort its
        // Subagents need is to.
        if let Compacting::Ended { settles } = &mut self.compaction {
            *settles = idled.is_none() || !subagents_work;
        }
        let Some(idled) = idled else {
            return running;
        };
        if self.compaction == Compacting::Running {
            self.compaction = Compacting::Stopped;
        }
        if subagents_work {
            return InterruptRemainder {
                awaits_idle: true,
                ..running
            };
        }
        if matches!(self.compaction, Compacting::Ended { .. }) {
            return InterruptRemainder {
                abort: false,
                already_ended: true,
                ..running
            };
        }
        // A compaction whose end Copilot reported while the cancel was out, and which the cancel
        // then found over, stopped nothing either.
        let already_ended = scope.compacting
            && !cancelled
            && matches!(
                self.compaction,
                Compacting::Idle | Compacting::Failed { .. }
            );
        let aborted = if already_ended { idled } else { true };
        InterruptRemainder {
            abort: false,
            settled: self
                .settle_turn(aborted)
                .into_iter()
                .map(|settled| attributed(None, settled))
                .collect(),
            awaits_idle: false,
            already_ended,
        }
    }

    /// Readies the signal that the abort an interrupt is about to send has ended in the idle that
    /// settles the stretch it was for ([`InterruptRemainder::awaits_idle`]): the timeline carrying
    /// Copilot's next idle raises it, ahead of anything the Turn after could produce.
    pub(super) fn expect_abort_idle(&mut self) -> Arc<Notify> {
        let carried = Arc::new(Notify::new());
        self.abort_idle = Some(carried.clone());
        carried
    }

    /// An abort is about to go out, so the next aborted idle is its own: none is owed any longer
    /// to one an interrupt gave up waiting on.
    pub(super) fn sending_abort(&mut self) {
        self.owes_aborted_idle = false;
    }

    /// Whether the aborted idle in hand is the one an earlier abort still owes, which settles
    /// nothing.
    fn owed_aborted_idle(&mut self, aborted: bool) -> bool {
        aborted && std::mem::take(&mut self.owes_aborted_idle)
    }

    /// The timeline has carried Copilot's main loop going idle — to the projection's queue, ahead
    /// of whatever follows it — which an interrupt may be waiting on.
    fn idle_carried(&mut self) {
        if let Some(carried) = self.abort_idle.take() {
            carried.notify_one();
        }
    }

    /// The abort an interrupt sent ended in no idle within the wait: settles, as interrupted, the
    /// stretch `scope` was for if it is still waiting on that idle, since nothing else will, and
    /// owes that idle should it come after all. The end of a compaction the cancel found already
    /// over, which the timeline has carried by now, will: it settles the stretch so instead, once
    /// it has stood there.
    pub(super) fn abort_idle_missing(
        &mut self,
        scope: InterruptScope,
    ) -> Vec<AttributedProviderEvent> {
        self.abort_idle = None;
        self.owes_aborted_idle = true;
        let Some(turn) = self
            .turn
            .as_mut()
            .filter(|turn| scope.stretch == Some(self.stretches) && turn.idled.is_some())
        else {
            return Vec::new();
        };
        if let Compacting::Ended { settles } = &mut self.compaction {
            *settles = true;
            turn.idled = Some(true);
            return Vec::new();
        }
        self.settle_turn(true)
            .into_iter()
            .map(|settled| attributed(None, settled))
            .collect()
    }

    /// A steer is about to reach Copilot. A loop already idle, with only a compaction holding the
    /// stretch open, runs again on it, so the stretch is the loop's until its next idle rather
    /// than the compaction's to settle — whichever of the steered loop's output and the
    /// compaction's end comes first. Returns the idle it held, for [`Self::steer_undelivered`].
    pub(super) fn steer_delivering(&mut self) -> Option<bool> {
        self.turn.as_mut().and_then(|turn| turn.idled.take())
    }

    /// The steer [`Self::steer_delivering`] prepared for never reached Copilot, whose loop is as
    /// idle as it was.
    pub(super) fn steer_undelivered(&mut self, idled: Option<bool>) {
        if let Some(turn) = self.turn.as_mut()
            && idled.is_some()
        {
            turn.idled = idled;
        }
    }

    /// Settles the Turn on the signal that Copilot's agentic loop has stopped: the stretch is the
    /// Turn, so its idle is the Turn's outcome — whatever the loop met on the way there, and an
    /// idle the abort produced is an interruption. The idle of a stale stretch — a Continuation
    /// the next Prompt already settled — is owed nothing and settles nothing.
    ///
    /// Copilot compacts in the background of the Session, so its loop can go idle while it still
    /// summarises. The stretch is not over until the compaction is — settling it now would fail a
    /// Compaction Copilot is about to complete — so the idle is held, and the compaction settling
    /// settles the stretch as the idle said (ADR 0042).
    fn project_loop_idle(&mut self, aborted: bool) -> Vec<ProviderEvent> {
        // Every report Copilot wrote before answering an earlier manual compaction has reached
        // Suru ahead of an idle of its loop since, so any still owed will never come.
        self.manual_reports_owed = false;
        if self.stale_stretches > 0 {
            self.stale_stretches -= 1;
            self.routes.forget_before(self.stretches);
            return Vec::new();
        }
        if self.compaction.holds_stretch()
            && let Some(turn) = self.turn.as_mut()
        {
            turn.idled = Some(aborted);
            return Vec::new();
        }
        self.settle_turn(aborted)
    }

    /// Settles the main conversation's stretch on its loop having stopped — on an abort, if
    /// `aborted`. A compaction whose failure an interrupt found to be its own settles failed
    /// first.
    fn settle_turn(&mut self, aborted: bool) -> Vec<ProviderEvent> {
        let Some(mut turn) = self.turn.take() else {
            return Vec::new();
        };
        self.previous_model_calls
            .extend(turn.streams.observed_model_calls.iter().cloned());
        let mut projected = Vec::new();
        if let Compacting::Failed { error } = &mut self.compaction {
            projected.push(ProviderEvent::CompactionFailed {
                error: error.take(),
            });
            self.compaction = Compacting::Idle;
        }
        let outcome = match (turn.failure, aborted) {
            (Some(message), _) => ProviderEvent::TurnFailed { message },
            (None, true) => ProviderEvent::TurnInterrupted,
            (None, false) => ProviderEvent::TurnCompleted,
        };
        // A loop that stops mid-Message or mid-block leaves neither running forever. A block a
        // failure or an abort cut off is left to the Turn's settle, which closes it as the Turn
        // settled (ADR 0039).
        let reasoning = if matches!(outcome, ProviderEvent::TurnCompleted) {
            OpenReasoning::Complete
        } else {
            OpenReasoning::Release
        };
        projected.extend(settle_open_streams(
            &mut turn.streams,
            &self.delegations,
            reasoning,
        ));
        projected.extend(self.routes.settle(
            self.stretches,
            !matches!(outcome, ProviderEvent::TurnCompleted),
        ));
        projected.push(outcome);
        projected
    }
}

/// The text of a notification as a reader should see it: Copilot hands the loop its
/// notifications wrapped in `<system_notification>` tags, which are addressed to the Model rather
/// than to anyone reading the Transcript.
fn notification_text(content: &str) -> Option<String> {
    let text = content.trim();
    let text = text
        .strip_prefix("<system_notification>")
        .and_then(|text| text.strip_suffix("</system_notification>"))
        .unwrap_or(text)
        .trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn project_message_started(
    streams: &mut ConversationStreams,
    message_id: String,
) -> Result<Vec<ProviderEvent>, ProviderError> {
    if streams.message.is_some() {
        return Err(copilot_error(
            "Copilot started a second agent Message before completing the first",
        ));
    }
    streams.message = Some(ActiveMessage {
        message_id,
        streamed: String::new(),
    });
    Ok(vec![ProviderEvent::AgentMessageStarted])
}

/// Streams the next of the agent Message into the Transcript, opening the Message on the first
/// delta when Copilot never started it: a Subagent's Message streams with no start of its own,
/// announcing itself by being written into the way a Reasoning block does. The main
/// conversation's Messages do get starts, but a delta is Transcript content either way, so every
/// conversation opens on whichever of the two arrives first rather than losing the Turn to a
/// missing announcement.
fn project_message_delta(
    streams: &mut ConversationStreams,
    message_id: &str,
    delta: String,
) -> Vec<ProviderEvent> {
    let mut projected = Vec::with_capacity(2);
    if streams.message.is_none() {
        streams.message = Some(ActiveMessage {
            message_id: message_id.to_owned(),
            streamed: String::new(),
        });
        projected.push(ProviderEvent::AgentMessageStarted);
    }
    let message = streams
        .message
        .as_mut()
        .expect("the Message is open, having been opened just now when it was not");
    if message.message_id != message_id {
        return Vec::new();
    }
    message.streamed.push_str(&delta);
    projected.push(ProviderEvent::AgentMessageDelta { content: delta });
    projected
}

/// Completes the streaming Message, or stands in for one Copilot never streamed: a Model call that
/// only asked for tools reports an empty Message, and one short enough to arrive whole reports it
/// without a start or a single delta.
fn project_message_completed(
    streams: &mut ConversationStreams,
    message_id: &str,
    content: &str,
) -> Result<Vec<ProviderEvent>, ProviderError> {
    let Some(message) = streams.message.as_ref() else {
        if content.is_empty() {
            return Ok(Vec::new());
        }
        return Ok(vec![
            ProviderEvent::AgentMessageStarted,
            ProviderEvent::AgentMessageDelta {
                content: content.to_owned(),
            },
            ProviderEvent::AgentMessageCompleted,
        ]);
    };
    if message.message_id != message_id {
        return Ok(Vec::new());
    }
    let Some(remaining) = content.strip_prefix(&message.streamed) else {
        return Err(copilot_error(
            "Copilot completed an agent Message with content that did not match its stream",
        ));
    };
    let mut projected = Vec::with_capacity(if remaining.is_empty() { 1 } else { 2 });
    if !remaining.is_empty() {
        projected.push(ProviderEvent::AgentMessageDelta {
            content: remaining.to_owned(),
        });
    }
    projected.push(ProviderEvent::AgentMessageCompleted);
    streams.message = None;
    Ok(projected)
}

/// Projects a Message a Route's phase staged, or its commit carried first: it arrives whole, with
/// the Reasoning that led to it inside, which a Route's phases report no other way. A Model call
/// that only asked for Tools has nothing in it for the Transcript, though it may have reasoned. It
/// is named, for the commit that may pass it over to speak of.
fn project_routed_message(
    streams: &mut ConversationStreams,
    message: &AssistantMessageData,
) -> Result<Vec<ProviderEvent>, ProviderError> {
    let mut projected = Vec::new();
    if let Some(reasoning) = message
        .reasoning_text
        .as_deref()
        .filter(|reasoning| !reasoning.trim().is_empty())
    {
        projected.extend(project_reasoning_completed(
            streams,
            &message.message_id,
            reasoning,
        ));
    }
    if message.content.is_empty() {
        return Ok(projected);
    }
    // A Message that streamed after all is completed as any streamed Message is. Any other left
    // open will never finish now: this one is whole, and the Transcript holds one open at a time.
    match streams.message.as_ref() {
        Some(open) if open.message_id == message.message_id => {
            projected.push(ProviderEvent::AgentMessageIdentified {
                message: ProviderMessageId::new(message.message_id.clone()),
            });
            projected.extend(project_message_completed(
                streams,
                &message.message_id,
                &message.content,
            )?);
            return Ok(projected);
        }
        Some(_) => {
            streams.message = None;
            projected.push(ProviderEvent::AgentMessageCompleted);
        }
        None => {}
    }
    projected.extend([
        ProviderEvent::AgentMessageStarted,
        ProviderEvent::AgentMessageIdentified {
            message: ProviderMessageId::new(message.message_id.clone()),
        },
        ProviderEvent::AgentMessageDelta {
            content: message.content.clone(),
        },
        ProviderEvent::AgentMessageCompleted,
    ]);
    Ok(projected)
}

/// Opens the row a tool execution is recorded as: a Command for a shell execution, a File Change
/// for an edit, a Tool Call for one no more specific Activity records, and nothing for the rest —
/// a spawn's Tool Call withheld while the Subagent row represents its delegation. Copilot reports
/// no working directory of its own for a command, so only one that changes directory as it opens
/// names where it runs; any other leaves it to the Session's Workspace, which the Session already
/// carries.
fn project_tool_started(
    streams: &mut ConversationStreams,
    started: &ToolExecutionStartData,
    execution_directory: &Path,
) -> Vec<ProviderEvent> {
    if streams.tools.contains_key(&started.tool_call_id) {
        // A repeated start reports nothing new: the first record keeps its streamed output and
        // its withheld Tool Call.
        return Vec::new();
    }
    let activity_id = tool_activity_id(&started.tool_call_id);
    let (running, projected) = match ToolDisposition::of(started, execution_directory) {
        ToolDisposition::Command(PresentedCommand { command, cwd }) => (
            RunningTool::Command {
                streamed_output: String::new(),
            },
            vec![ProviderEvent::CommandStarted {
                activity_id,
                command,
                cwd,
            }],
        ),
        ToolDisposition::FileChange(changes) => (
            RunningTool::FileChange,
            vec![ProviderEvent::FileChangeStarted {
                activity_id,
                changes,
            }],
        ),
        ToolDisposition::ToolCall => {
            let PresentedToolCall {
                name,
                server,
                input,
            } = presented_tool_call(started);
            (
                RunningTool::ToolCall {
                    streamed_output: String::new(),
                },
                vec![ProviderEvent::ToolCallStarted {
                    activity_id,
                    name,
                    server,
                    input: Some(input),
                }],
            )
        }
        ToolDisposition::Spawn => {
            let prompt = started
                .arguments
                .as_ref()
                .and_then(|arguments| arguments.get("prompt"))
                .and_then(serde_json::Value::as_str)
                .filter(|prompt| !prompt.trim().is_empty())
                .map(str::to_owned);
            (
                RunningTool::Spawn {
                    withheld: presented_tool_call(started),
                    prompt,
                    streamed_output: String::new(),
                },
                Vec::new(),
            )
        }
        ToolDisposition::WriteAgent
        | ToolDisposition::Questionnaire
        | ToolDisposition::BrokerRow
        | ToolDisposition::Plumbing => (RunningTool::Unrecorded, Vec::new()),
    };
    streams.tools.insert(started.tool_call_id.clone(), running);
    projected
}

/// Streams the next of a running Command's or Tool Call's output into the Transcript. A File
/// Change keeps none.
fn project_tool_output(
    streams: &mut ConversationStreams,
    tool_call_id: &str,
    output: String,
) -> Vec<ProviderEvent> {
    let activity_id = tool_activity_id(tool_call_id);
    match streams.tools.get_mut(tool_call_id) {
        Some(RunningTool::Command { streamed_output }) => {
            streamed_output.push_str(&output);
            vec![ProviderEvent::CommandOutputDelta {
                activity_id,
                content: output,
            }]
        }
        Some(RunningTool::ToolCall { streamed_output }) => {
            streamed_output.push_str(&output);
            vec![ProviderEvent::ToolCallOutputDelta {
                activity_id,
                content: output,
            }]
        }
        Some(RunningTool::Spawn {
            streamed_output, ..
        }) => {
            streamed_output.push_str(&output);
            Vec::new()
        }
        Some(RunningTool::FileChange | RunningTool::Unrecorded) | None => Vec::new(),
    }
}

/// Settles the row a tool execution is recorded as on what it came to, carrying whatever of its
/// output the stream had not already reached.
fn project_tool_completed(
    streams: &mut ConversationStreams,
    delegations: &mut HashSet<String>,
    completed: &ToolExecutionCompleteData,
) -> Vec<ProviderEvent> {
    match streams.tools.remove(&completed.tool_call_id) {
        Some(RunningTool::Command { streamed_output }) => {
            settle_command_events(completed, &streamed_output)
        }
        Some(RunningTool::FileChange) => vec![ProviderEvent::FileChangeCompleted {
            activity_id: tool_activity_id(&completed.tool_call_id),
            status: if completed.success {
                ProviderFileChangeStatus::Completed
            } else {
                ProviderFileChangeStatus::Failed
            },
        }],
        Some(RunningTool::ToolCall { streamed_output }) => {
            settle_tool_call_events(completed, &streamed_output)
        }
        Some(RunningTool::Spawn {
            withheld,
            streamed_output,
            ..
        }) => {
            if delegations.remove(&completed.tool_call_id) {
                return Vec::new();
            }
            // The spawn never opened its Subagent, so no row answers for the delegation: the
            // withheld Tool Call surfaces here, carrying what the execution reported went wrong.
            let mut projected =
                surface_withheld_spawn(&completed.tool_call_id, withheld, streamed_output.clone());
            projected.extend(settle_tool_call_events(completed, &streamed_output));
            projected
        }
        Some(RunningTool::Unrecorded) | None => Vec::new(),
    }
}

/// Opens the Tool Call a withheld spawn would have been, now that no Subagent row will answer for
/// the delegation, replaying the output the withholding kept back.
fn surface_withheld_spawn(
    tool_call_id: &str,
    withheld: PresentedToolCall,
    streamed_output: String,
) -> Vec<ProviderEvent> {
    let activity_id = tool_activity_id(tool_call_id);
    let PresentedToolCall {
        name,
        server,
        input,
    } = withheld;
    let mut projected = vec![ProviderEvent::ToolCallStarted {
        activity_id: activity_id.clone(),
        name,
        server,
        input: Some(input),
    }];
    if !streamed_output.is_empty() {
        projected.push(ProviderEvent::ToolCallOutputDelta {
            activity_id,
            content: streamed_output,
        });
    }
    projected
}

/// Settles a Command on what its tool execution came to, carrying whatever of its output the
/// stream had not already reached.
fn settle_command_events(
    completed: &ToolExecutionCompleteData,
    streamed_output: &str,
) -> Vec<ProviderEvent> {
    let activity_id = tool_activity_id(&completed.tool_call_id);
    let mut projected = Vec::with_capacity(2);
    if let Some(trailing) = trailing_output(completed, streamed_output) {
        projected.push(ProviderEvent::CommandOutputDelta {
            activity_id: activity_id.clone(),
            content: trailing,
        });
    }
    projected.push(ProviderEvent::CommandCompleted {
        activity_id,
        status: if completed.success {
            ProviderCommandStatus::Completed
        } else {
            ProviderCommandStatus::Failed
        },
        exit_status: command_exit_status(completed),
    });
    projected
}

/// The exit code a completed shell execution reports: the `shell_exit` part of its result, or
/// failing that the experimental `shellExecution` facts Copilot keeps beside the result. A code
/// outside what a process can exit with is no exit status at all.
fn command_exit_status(completed: &ToolExecutionCompleteData) -> Option<i32> {
    completed
        .result
        .as_ref()
        .and_then(|result| result.contents.as_deref())
        .unwrap_or_default()
        .iter()
        .find_map(|part| match part {
            ToolExecutionCompleteContent::ShellExit(shell_exit) => Some(shell_exit.exit_code),
            _ => None,
        })
        .or_else(|| {
            completed
                .shell_execution
                .as_ref()
                .map(|execution| execution.exit_code)
        })
        .and_then(|code| i32::try_from(code).ok())
}

/// Settles a Tool Call on what its tool execution came to: the rest of its result's text, or the
/// error a failed one reports instead, and a count of the result's parts that are not text.
fn settle_tool_call_events(
    completed: &ToolExecutionCompleteData,
    streamed_output: &str,
) -> Vec<ProviderEvent> {
    let activity_id = tool_activity_id(&completed.tool_call_id);
    let mut projected = Vec::with_capacity(2);
    if let Some(trailing) = trailing_output(completed, streamed_output) {
        projected.push(ProviderEvent::ToolCallOutputDelta {
            activity_id: activity_id.clone(),
            content: trailing,
        });
    }
    projected.push(ProviderEvent::ToolCallCompleted {
        activity_id,
        status: if completed.success {
            ProviderToolCallStatus::Completed
        } else {
            ProviderToolCallStatus::Failed
        },
        omitted_parts: omitted_result_parts(completed),
    });
    projected
}

/// How many parts of a completed execution's result are not text — images, audio, resources — and
/// so are left out of its output. Text, and the shell output a terminal part reports, is output.
fn omitted_result_parts(completed: &ToolExecutionCompleteData) -> u32 {
    let omitted = completed
        .result
        .as_ref()
        .and_then(|result| result.contents.as_deref())
        .unwrap_or_default()
        .iter()
        .filter(|part| {
            matches!(
                part,
                ToolExecutionCompleteContent::Image(_)
                    | ToolExecutionCompleteContent::Audio(_)
                    | ToolExecutionCompleteContent::ResourceLink(_)
                    | ToolExecutionCompleteContent::Resource(_)
            )
        })
        .count();
    u32::try_from(omitted).unwrap_or(u32::MAX)
}

/// What the completed execution adds to the output already streamed: the rest of a result the
/// stream had not reached, and for a failed execution what went wrong. The same rule settles a
/// Command and a Tool Call.
///
/// A result that is not what streamed extends adds nothing to a successful execution. Copilot cuts
/// the result it hands the Model down for token efficiency, so the stream is the fuller record, and
/// replacing it would show the reader the same output twice.
///
/// A failed execution may report a result, an error, or both, and nothing ranks one over the
/// other, so both reach the output: the rest of the result first, then the error's message. A
/// failure reports why rather than more output, so neither a result that does not continue the
/// stream nor the error continues it: each is added below what the execution had produced rather
/// than onto the end of its last line. An error the output already ends with — the result
/// restating it, or the stream having carried it — is not repeated.
fn trailing_output(completed: &ToolExecutionCompleteData, streamed: &str) -> Option<String> {
    let mut trailing = String::new();
    if let Some(result) = completed.result.as_ref() {
        let reported = result
            .detailed_content
            .as_deref()
            .unwrap_or(result.content.as_str());
        let reported = without_shell_exit_marker(completed, reported, streamed);
        match reported.strip_prefix(streamed) {
            Some(rest) => trailing.push_str(rest),
            None if !completed.success && !reported.is_empty() => {
                push_below(streamed, &mut trailing, reported);
            }
            None => {}
        }
    }
    let error = completed
        .error
        .as_ref()
        .filter(|_| !completed.success)
        .map_or("", |error| error.message.as_str());
    let restated = format!("{streamed}{trailing}")
        .trim_end()
        .ends_with(error.trim_end());
    if !error.trim().is_empty() && !restated {
        push_below(streamed, &mut trailing, error);
    }
    (!trailing.is_empty()).then_some(trailing)
}

/// `reported` without the marker closing an ended shell's result, which tells the Model how the
/// shell exited and is no output of the shell's.
///
/// The `shell_exit` part reporting the same exit spells the marker out, so anything else reading
/// like one stays output, whatever printed it. Copilot sets the marker on a line of its own,
/// breaking the line itself after output that left it open, so a line break the stream never
/// carried goes with the marker (docs/validation/copilot-tool-output-streaming.md).
fn without_shell_exit_marker<'a>(
    completed: &ToolExecutionCompleteData,
    reported: &'a str,
    streamed: &str,
) -> &'a str {
    let marker = completed
        .result
        .as_ref()
        .and_then(|result| result.contents.as_deref())
        .unwrap_or_default()
        .iter()
        .find_map(|part| match part {
            ToolExecutionCompleteContent::ShellExit(exit) => Some(format!(
                "<shellId: {} completed with exit code {}>",
                exit.shell_id, exit.exit_code
            )),
            _ => None,
        });
    let Some(output) = marker.and_then(|marker| reported.strip_suffix(marker.as_str())) else {
        return reported;
    };
    match output.strip_suffix('\n') {
        Some(unbroken) if unbroken == streamed => unbroken,
        _ => output,
    }
}

/// Adds `text` to `trailing` on a line of its own below everything the output holds so far — what
/// streamed, then what `trailing` already adds to it.
fn push_below(streamed: &str, trailing: &mut String, text: &str) {
    let so_far_ends_a_line = if trailing.is_empty() {
        streamed.is_empty() || streamed.ends_with('\n')
    } else {
        trailing.ends_with('\n')
    };
    if !so_far_ends_a_line {
        trailing.push('\n');
    }
    trailing.push_str(text);
}

/// Names the Activity one Reasoning block projects onto, in the identity space
/// [`tool_activity_id`] explains.
fn reasoning_activity_id(reasoning_id: &str) -> ProviderActivityId {
    ProviderActivityId::new(format!("reasoning:{reasoning_id}"))
}

/// Streams the next of a Reasoning block into the Transcript, opening the block on the first of it
/// Copilot sends: Copilot announces a block by reasoning into it rather than with an event of its
/// own.
fn project_reasoning_delta(
    streams: &mut ConversationStreams,
    reasoning_id: &str,
    delta: &str,
) -> Vec<ProviderEvent> {
    let mut projected = Vec::new();
    if !streams
        .reasoning
        .iter()
        .any(|block| block.reasoning_id == reasoning_id)
    {
        projected.push(ProviderEvent::ReasoningStarted {
            activity_id: reasoning_activity_id(reasoning_id),
        });
        streams.reasoning.push(ActiveReasoning {
            reasoning_id: reasoning_id.to_owned(),
            ..ActiveReasoning::default()
        });
    }
    let block = streams
        .reasoning
        .iter_mut()
        .find(|block| block.reasoning_id == reasoning_id)
        .expect("the block is open, having been opened just now when it was not");
    block.streamed.push_str(delta);
    let segment = block.splitter.push(delta);
    projected.extend(reasoning_segment_events(reasoning_id, segment));
    projected
}

/// Settles the block Copilot has reasoned through, or stands in for one it never streamed: a Model
/// that reasons without streaming reports the block whole and only once.
fn project_reasoning_completed(
    streams: &mut ConversationStreams,
    reasoning_id: &str,
    content: &str,
) -> Vec<ProviderEvent> {
    let mut block = streams
        .reasoning
        .iter()
        .position(|block| block.reasoning_id == reasoning_id)
        .map_or_else(ActiveReasoning::default, |open| {
            streams.reasoning.remove(open)
        });
    let mut projected = Vec::new();
    if block.streamed.is_empty() {
        projected.push(ProviderEvent::ReasoningStarted {
            activity_id: reasoning_activity_id(reasoning_id),
        });
    }
    // The completed block repeats what streamed and carries whatever the stream had not reached.
    // When the two disagree the stream already showed the reader a coherent block, so repeating the
    // completed one on top of it would double the text rather than correct it.
    let remaining = content
        .strip_prefix(block.streamed.as_str())
        .unwrap_or_default();
    if !remaining.is_empty() {
        projected.extend(reasoning_segment_events(
            reasoning_id,
            block.splitter.push(remaining),
        ));
    }
    projected.extend(settle_reasoning(reasoning_id, &mut block));
    projected
}

/// Settles the Activity a Reasoning block streamed into, releasing whatever its split still
/// withholds, so no block is left open for a Turn to settle around.
fn settle_reasoning(reasoning_id: &str, block: &mut ActiveReasoning) -> Vec<ProviderEvent> {
    let mut projected = reasoning_segment_events(reasoning_id, block.splitter.finish());
    projected.push(ProviderEvent::ReasoningCompleted {
        activity_id: reasoning_activity_id(reasoning_id),
    });
    projected
}

/// What settling a stopped conversation does with the Reasoning blocks it left open.
#[derive(Clone, Copy)]
enum OpenReasoning {
    /// The conversation finished its work, so each block completes.
    Complete,
    /// Something cut the conversation off: each block releases what its split withholds and is
    /// left running, for the Turn's settle to close as the Turn settled.
    Release,
}

/// Settles everything a stopped conversation left open: its Reasoning blocks, the Message it was
/// still streaming, and any withheld spawn no Subagent row ever answered for — surfaced here,
/// because a Tool Call the store never saw is one the store cannot settle, and the delegation
/// would otherwise vanish with the conversation. Open Commands, File Changes and Tool Calls are
/// otherwise left to the store — their outcome is Copilot's to report, not ours to invent — which
/// settles a Turn's open Activities from its own snapshot; only the split here knows the title it
/// is still withholding, which would otherwise go with the block.
fn settle_open_streams(
    streams: &mut ConversationStreams,
    delegations: &HashSet<String>,
    reasoning: OpenReasoning,
) -> Vec<ProviderEvent> {
    let mut projected = Vec::new();
    for mut block in std::mem::take(&mut streams.reasoning) {
        let reasoning_id = std::mem::take(&mut block.reasoning_id);
        projected.extend(match reasoning {
            OpenReasoning::Complete => settle_reasoning(&reasoning_id, &mut block),
            OpenReasoning::Release => {
                reasoning_segment_events(&reasoning_id, block.splitter.finish())
            }
        });
    }
    if streams.message.take().is_some() {
        projected.push(ProviderEvent::AgentMessageCompleted);
    }
    for (tool_call_id, running) in streams.tools.drain() {
        if let RunningTool::Spawn {
            withheld,
            streamed_output,
            ..
        } = running
            && !delegations.contains(&tool_call_id)
        {
            projected.extend(surface_withheld_spawn(
                &tool_call_id,
                withheld,
                streamed_output,
            ));
        }
    }
    projected
}

/// Lowers one split step of a Reasoning block onto the Provider events that carry it, dropping the
/// step that resolved nothing because the split is still withholding the head.
fn reasoning_segment_events(reasoning_id: &str, segment: ReasoningSegment) -> Vec<ProviderEvent> {
    let mut projected = Vec::new();
    if let Some(title) = segment.title {
        projected.push(ProviderEvent::ReasoningTitleChanged {
            activity_id: reasoning_activity_id(reasoning_id),
            title,
        });
    }
    if !segment.content.is_empty() {
        projected.push(ProviderEvent::ReasoningDelta {
            activity_id: reasoning_activity_id(reasoning_id),
            content: segment.content,
        });
    }
    projected
}

fn decode<T: serde::de::DeserializeOwned>(event: &SessionEvent) -> Result<T, ProviderError> {
    event.typed_data().ok_or_else(|| {
        copilot_error(format!(
            "Copilot sent an invalid `{}` event",
            event.event_type
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{RouteAction, RouteDecision, RouteStep, RouteVerdict};
    use crate::provider::ToolCallInput;
    use crate::provider::copilot::COPILOT_PROVIDER_ID;
    use serde_json::json;

    fn event(event_type: &str, data: serde_json::Value) -> SessionEvent {
        SessionEvent {
            id: "fixture-event".to_owned(),
            timestamp: "2026-01-01T00:00:00Z".to_owned(),
            parent_id: None,
            ephemeral: None,
            agent_id: None,
            debug_cli_received_at_ms: None,
            debug_ws_forwarded_at_ms: None,
            event_type: event_type.to_owned(),
            data,
        }
    }

    fn agent_event(agent: &str, event_type: &str, data: serde_json::Value) -> SessionEvent {
        SessionEvent {
            agent_id: Some(agent.to_owned()),
            ..event(event_type, data)
        }
    }

    fn project_attributed(
        correlation: &mut CopilotCorrelation,
        event: SessionEvent,
    ) -> Vec<AttributedProviderEvent> {
        let event_type = event.event_type.clone();
        project_session_event(correlation, event)
            .unwrap_or_else(|error| panic!("`{event_type}` projects cleanly, got: {error}"))
    }

    /// Projects an event whose every projection belongs to the owning Session, unwrapped to the
    /// bare events for assertion.
    fn project(
        correlation: &mut CopilotCorrelation,
        event_type: &str,
        data: serde_json::Value,
    ) -> Vec<ProviderEvent> {
        project_attributed(correlation, event(event_type, data))
            .into_iter()
            .map(|attributed| {
                assert_eq!(
                    attributed.attribution,
                    ProviderEventAttribution::OwningSession,
                    "the event lands in the owning Session, got {:?}",
                    attributed.attribution
                );
                attributed.event
            })
            .collect()
    }

    fn in_turn() -> CopilotCorrelation {
        let mut correlation = CopilotCorrelation::new();
        correlation.begin_turn().expect("open the Turn");
        correlation
    }

    fn spawn_started(agent: &str, tool_call_id: &str) -> SessionEvent {
        agent_event(
            agent,
            "subagent.started",
            json!({
                "toolCallId": tool_call_id,
                "agentName": "researcher",
                "agentDisplayName": "Researcher",
                "agentDescription": "Scout the workspace",
            }),
        )
    }

    /// A correlation whose Turn has one Subagent open under instance `agent-1`.
    fn with_subagent() -> CopilotCorrelation {
        let mut correlation = in_turn();
        project_attributed(&mut correlation, spawn_started("agent-1", "t-spawn"));
        correlation
    }

    fn subagent(id: &str) -> ProviderEventAttribution {
        ProviderEventAttribution::Subagent(ProviderSubagentId::new(id))
    }

    /// The Prompt Cache lapses each projected event reports, wherever it lands.
    fn lapses(
        projected: &[AttributedProviderEvent],
    ) -> Vec<(&ProviderEventAttribution, &ProviderEvent)> {
        projected
            .iter()
            .filter(|attributed| {
                matches!(
                    attributed.event,
                    ProviderEvent::PromptCacheLapseReported { .. }
                )
            })
            .map(|attributed| (&attributed.attribution, &attributed.event))
            .collect()
    }

    #[test]
    fn a_conversation_call_states_when_the_cache_it_wrote_lapses_how_long_it_is_kept_and_its_limit()
    {
        let mut correlation = in_turn();
        let projected = project_attributed(
            &mut correlation,
            event(
                "assistant.usage",
                json!({
                    "apiCallId": "c1", "model": "claude-fixture", "inputTokens": 10,
                    "cacheExpiresAt": "2026-01-01T00:05:00.250Z", "cacheTtlSeconds": 3600,
                    "maxPromptTokens": 872_000,
                }),
            ),
        );
        assert_eq!(
            lapses(&projected),
            [(
                &ProviderEventAttribution::OwningSession,
                &ProviderEvent::PromptCacheLapseReported {
                    lapses_at: crate::protocol::SessionTimestamp(1_767_225_900_250),
                    model: Some(ModelId::new("claude-fixture")),
                    lifetime: Some(std::time::Duration::from_secs(3600)),
                    prompt_limit_tokens: Some(872_000),
                }
            )]
        );
    }

    #[test]
    fn a_call_states_no_lapse_without_a_readable_moment_after_the_epoch() {
        for expires in [None, Some("soon"), Some("1969-12-31T23:59:59Z")] {
            let mut correlation = in_turn();
            let mut data =
                json!({ "apiCallId": "c1", "model": "claude-fixture", "inputTokens": 1 });
            if let Some(expires) = expires {
                data["cacheExpiresAt"] = json!(expires);
            }
            let projected = project_attributed(&mut correlation, event("assistant.usage", data));
            assert!(
                lapses(&projected).is_empty(),
                "{expires:?} reports no lapse: {projected:?}"
            );
            assert!(
                projected
                    .iter()
                    .any(|attributed| matches!(attributed.event, ProviderEvent::Usage { .. })),
                "the call's Usage stands whatever it says of the cache"
            );
        }
    }

    #[test]
    fn neither_a_subagents_call_nor_one_outside_the_conversation_states_its_lapse() {
        let data = |interaction: &str| {
            json!({
                "apiCallId": "c1", "model": "claude-fixture", "inputTokens": 1,
                "interactionType": interaction, "cacheExpiresAt": "2026-01-01T00:05:00Z",
            })
        };
        let mut correlation = with_subagent();
        let child = project_attributed(
            &mut correlation,
            agent_event("agent-1", "assistant.usage", data("conversation-subagent")),
        );
        assert!(lapses(&child).is_empty(), "{child:?}");
        let mut correlation = in_turn();
        let aside = project_attributed(
            &mut correlation,
            event("assistant.usage", data("conversation-compaction")),
        );
        assert!(lapses(&aside).is_empty(), "{aside:?}");
    }

    #[test]
    fn an_auto_parent_never_supplies_a_subagents_model_or_auto_selection() {
        let mut correlation = with_subagent();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        let root = project_attributed(
            &mut correlation,
            event(
                "model.call_start",
                json!({ "turnId": "root", "model": "root-model" }),
            ),
        );
        assert!(matches!(
            &root[..],
            [AttributedProviderEvent {
                attribution: ProviderEventAttribution::OwningSession,
                event: ProviderEvent::CurrentModelChanged { model: Some(model) },
            }] if model.actual.as_str() == "root-model"
        ));

        assert!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "model.call_start",
                    json!({ "turnId": "child", "model": "child-model" }),
                ),
            )
            .is_empty(),
            "a child's dispatch has no Auto evidence of its own yet"
        );
        let child_usage = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({ "model": "child-model", "inputTokens": 1 }),
            ),
        );
        assert!(!child_usage.iter().any(|event| matches!(
            event.event,
            ProviderEvent::ModelCallRecorded { .. } | ProviderEvent::CurrentModelChanged { .. }
        )));

        let resolved = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.auto_mode_resolved",
                json!({ "chosenModel": "child-model" }),
            ),
        );
        assert!(
            resolved.is_empty(),
            "Auto resolution cannot revive a dispatch whose usage has already arrived"
        );
        let child_end = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "model.call_finished",
                json!({
                    "turnId": "child", "dispatchDurationMs": 1,
                    "editClassifierVersion": 1, "outcome": "success",
                }),
            ),
        );
        assert!(child_end.is_empty());
        let child_next = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "model.call_start",
                json!({ "turnId": "child-next", "model": "child-model" }),
            ),
        );
        assert!(matches!(
            &child_next[..],
            [AttributedProviderEvent {
                attribution: ProviderEventAttribution::Subagent(agent),
                event: ProviderEvent::CurrentModelChanged { model: Some(model) },
            }] if agent.as_str() == "agent-1" && model.actual.as_str() == "child-model"
        ));
        let child_end = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "model.call_finished",
                json!({
                    "turnId": "child-next", "dispatchDurationMs": 1,
                    "editClassifierVersion": 1, "outcome": "success",
                }),
            ),
        );
        assert!(
            child_end.is_empty(),
            "the child's Model stands between calls"
        );
        let root_end = project(
            &mut correlation,
            "model.call_finished",
            json!({
                "turnId": "root", "dispatchDurationMs": 1,
                "editClassifierVersion": 1, "outcome": "success",
            }),
        );
        assert!(root_end.is_empty(), "the root's Model stands between calls");
    }

    #[test]
    fn a_subagents_error_withdraws_the_model_it_held() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.auto_mode_resolved",
                json!({ "chosenModel": "child-model" }),
            ),
        );
        let started = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "model.call_start",
                json!({ "turnId": "child", "model": "child-model" }),
            ),
        );
        assert!(started.iter().any(|event| matches!(
            &event.event,
            ProviderEvent::CurrentModelChanged { model: Some(model) }
                if model.actual.as_str() == "child-model"
        )));
        let transient = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.error",
                json!({ "errorType": "model_call", "message": "retrying" }),
            ),
        );
        assert!(
            !transient
                .iter()
                .any(|event| matches!(event.event, ProviderEvent::CurrentModelChanged { .. })),
            "a transient error is one the loop retries through"
        );
        let failed = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.error",
                json!({ "errorType": "quota", "message": "out of credits" }),
            ),
        );
        assert!(failed.iter().any(|event| matches!(
            &event.event,
            ProviderEvent::CurrentModelChanged { model: None }
        )));
    }

    #[test]
    fn a_subagents_per_call_auto_evidence_does_not_label_unrelated_work() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "model.call_start",
                json!({ "turnId": "child", "model": "auto-model" }),
            ),
        );
        let usage = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({
                    "apiCallId": "auto-call", "model": "auto-model",
                    "isAuto": true, "inputTokens": 1,
                }),
            ),
        );
        assert!(
            usage
                .iter()
                .any(|event| matches!(event.event, ProviderEvent::ModelCallRecorded { .. }))
        );
        assert!(
            !usage
                .iter()
                .any(|event| matches!(event.event, ProviderEvent::CurrentModelChanged { .. }))
        );

        let auto_message = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message",
                json!({
                    "messageId": "auto-message", "apiCallId": "auto-call",
                    "content": "Auto answer",
                }),
            ),
        );
        assert!(auto_message.iter().any(|event| matches!(
            &event.event,
            ProviderEvent::AgentMessageModelKnown { model, .. }
                if model.selected.as_str() == "auto" && model.actual.as_str() == "auto-model"
        )));

        let fixed_message = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message",
                json!({
                    "messageId": "fixed-message", "apiCallId": "fixed-call",
                    "content": "Fixed answer", "model": "fixed-model",
                }),
            ),
        );
        assert!(
            !fixed_message
                .iter()
                .any(|event| matches!(event.event, ProviderEvent::AgentMessageModelKnown { .. }))
        );

        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message",
                json!({
                    "messageId": "late-message", "apiCallId": "late-call",
                    "content": "Later answer", "model": "message-model",
                }),
            ),
        );
        let late_usage = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({
                    "apiCallId": "late-call", "model": "usage-model",
                    "isAuto": true, "inputTokens": 1,
                }),
            ),
        );
        assert!(late_usage.iter().any(|event| matches!(
            &event.event,
            ProviderEvent::AgentMessageModelKnown { message: Some(message), model }
                if message == &ProviderMessageId::new("late-message")
                    && model.selected.as_str() == "auto"
                    && model.actual.as_str() == "message-model"
        )));
    }

    #[test]
    fn anonymous_fixed_usage_withholds_or_retracts_anonymous_auto_labels() {
        for usage_first in [true, false] {
            let mut correlation = in_turn();
            correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
            let usage = || {
                event(
                    "assistant.usage",
                    json!({ "model": "fixed-model", "isAuto": false, "inputTokens": 1 }),
                )
            };
            let message = || {
                event(
                    "assistant.message",
                    json!({ "messageId": "m1", "model": "fixed-model", "content": "Answer" }),
                )
            };
            if usage_first {
                project_attributed(&mut correlation, usage());
                let emitted = project_attributed(&mut correlation, message());
                assert!(!emitted.iter().any(|event| matches!(
                    event.event,
                    ProviderEvent::AgentMessageModelKnown { .. }
                )));
            } else {
                project_attributed(&mut correlation, message());
                let emitted = project_attributed(&mut correlation, usage());
                assert!(emitted.iter().any(|event| matches!(
                    &event.event,
                    ProviderEvent::AgentMessageModelCleared { message }
                        if message == &ProviderMessageId::new("m1")
                )));
            }
        }
    }

    #[test]
    fn a_childs_prior_fixed_message_is_not_claimed_after_it_switches_to_auto() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message",
                json!({
                    "messageId": "fixed", "apiCallId": "old-call",
                    "model": "fixed-model", "content": "Answer",
                }),
            ),
        );
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.auto_mode_resolved",
                json!({ "chosenModel": "auto-model" }),
            ),
        );
        let usage = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({ "apiCallId": "old-call", "model": "fixed-model", "inputTokens": 1 }),
            ),
        );
        assert!(!usage.iter().any(|event| matches!(
            event.event,
            ProviderEvent::ModelCallRecorded { .. } | ProviderEvent::AgentMessageModelKnown { .. }
        )));
    }

    #[test]
    fn a_child_call_keeps_the_selection_seen_before_a_later_model_change() {
        for initially_auto in [false, true] {
            for first_report in ["assistant.usage", "model.call_failure"] {
                let mut correlation = with_subagent();
                if initially_auto {
                    project_attributed(
                        &mut correlation,
                        agent_event(
                            "agent-1",
                            "session.auto_mode_resolved",
                            json!({ "chosenModel": "first-model" }),
                        ),
                    );
                }
                let data = match first_report {
                    "assistant.usage" => {
                        json!({ "apiCallId": "c1", "model": "first-model", "inputTokens": 1 })
                    }
                    _ => json!({
                        "source": "subagent", "apiCallId": "c1", "model": "first-model",
                    }),
                };
                project_attributed(&mut correlation, agent_event("agent-1", first_report, data));
                let (change, data) = if initially_auto {
                    (
                        "session.model_change",
                        json!({ "newModel": "fixed-model", "previousModel": "auto" }),
                    )
                } else {
                    (
                        "session.auto_mode_resolved",
                        json!({ "chosenModel": "auto-model" }),
                    )
                };
                project_attributed(&mut correlation, agent_event("agent-1", change, data));
                let message = project_attributed(
                    &mut correlation,
                    agent_event(
                        "agent-1",
                        "assistant.message",
                        json!({
                            "messageId": "m1", "apiCallId": "c1",
                            "model": "first-model", "content": "Answer",
                        }),
                    ),
                );
                assert_eq!(
                    message.iter().any(|event| matches!(
                        &event.event,
                        ProviderEvent::AgentMessageModelKnown { model, .. }
                            if model.actual.as_str() == "first-model"
                    )),
                    initially_auto,
                    "initially_auto={initially_auto}, first_report={first_report}"
                );
            }
        }
    }

    #[test]
    fn fixed_calls_still_appear_in_a_routed_turns_call_history() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        let usage = project(
            &mut correlation,
            "assistant.usage",
            json!({ "apiCallId": "c1", "model": "fixed-model", "isAuto": false, "inputTokens": 1 }),
        );
        assert!(usage.iter().any(|event| matches!(
            event,
            ProviderEvent::ModelCallRecorded { model }
                if model.actual.as_str() == "fixed-model"
        )));
    }

    #[test]
    fn a_fixed_child_call_retracts_its_message_without_changing_the_childs_auto_selection() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.auto_mode_resolved",
                json!({ "chosenModel": "child-model" }),
            ),
        );
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message",
                json!({
                    "messageId": "m1", "apiCallId": "fixed",
                    "model": "fixed-model", "content": "Answer",
                }),
            ),
        );
        let fixed = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({
                    "apiCallId": "fixed", "model": "fixed-model",
                    "isAuto": false, "inputTokens": 1,
                }),
            ),
        );
        assert!(fixed.iter().any(|event| matches!(
            &event.event,
            ProviderEvent::AgentMessageModelCleared { message }
                if message == &ProviderMessageId::new("m1")
        )));
        let auto = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({ "apiCallId": "next", "model": "child-model", "inputTokens": 1 }),
            ),
        );
        assert!(auto.iter().any(|event| matches!(
            &event.event,
            ProviderEvent::ModelCallRecorded { model }
                if model.actual.as_str() == "child-model"
        )));
    }

    #[test]
    fn a_subagents_auto_selection_survives_a_resume_but_not_a_model_change() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.auto_mode_resolved",
                json!({ "chosenModel": "child-model" }),
            ),
        );
        let usage = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({ "apiCallId": "first", "model": "child-model", "inputTokens": 1 }),
            ),
        );
        assert!(
            usage
                .iter()
                .any(|event| matches!(event.event, ProviderEvent::ModelCallRecorded { .. }))
        );
        project_attributed(
            &mut correlation,
            event(
                "subagent.completed",
                json!({
                    "toolCallId": "t-spawn", "agentName": "researcher",
                    "agentDisplayName": "Researcher",
                }),
            ),
        );
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "user.message",
                json!({ "content": "Continue", "delivery": "idle" }),
            ),
        );
        let resumed = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "model.call_start",
                json!({ "turnId": "resumed", "model": "child-model" }),
            ),
        );
        assert!(resumed.iter().any(|event| matches!(
            &event.event,
            ProviderEvent::CurrentModelChanged { model: Some(model) }
                if model.actual.as_str() == "child-model"
        )));
        let changed = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.model_change",
                json!({ "newModel": "fixed-model", "previousModel": "auto" }),
            ),
        );
        assert!(changed.iter().any(|event| matches!(
            event.event,
            ProviderEvent::CurrentModelChanged { model: None }
        )));
        let fixed_message = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message",
                json!({ "messageId": "fixed", "content": "Fixed", "model": "fixed-model" }),
            ),
        );
        assert!(
            !fixed_message
                .iter()
                .any(|event| matches!(event.event, ProviderEvent::AgentMessageModelKnown { .. }))
        );
    }

    #[test]
    fn a_subagents_refusal_fallback_is_its_own_reroute_and_still_ends_its_auto_selection() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.auto_mode_resolved",
                json!({ "chosenModel": "child-model" }),
            ),
        );
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "model.call_start",
                json!({ "turnId": "t1", "model": "child-model" }),
            ),
        );
        let changed = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "session.model_change",
                json!({
                    "previousModel": "child-model",
                    "newModel": "fallback-model",
                    "cause": "refusal_fallback",
                }),
            ),
        );
        let subagent = ProviderEventAttribution::Subagent(ProviderSubagentId::new("agent-1"));
        assert_eq!(
            changed,
            [
                AttributedProviderEvent {
                    attribution: subagent.clone(),
                    event: ProviderEvent::Rerouted {
                        from: Some(ModelId::new("child-model")),
                        to: ModelId::new("fallback-model"),
                        reason: Some("the Model declined the request".to_owned()),
                    },
                },
                AttributedProviderEvent {
                    attribution: subagent,
                    event: ProviderEvent::CurrentModelChanged { model: None },
                },
            ],
            "the Reroute lands in the Subagent's own Transcript, and the change still ends Auto"
        );
    }

    #[test]
    fn unclassified_sampling_and_child_messages_are_not_root_auto_evidence() {
        let mut correlation = with_subagent();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        let sampling = project(
            &mut correlation,
            "assistant.usage",
            json!({
                "apiCallId": "sample", "model": "sample-model", "inputTokens": 1,
                "initiator": "mcp-sampling",
            }),
        );
        assert!(
            sampling
                .iter()
                .any(|event| matches!(event, ProviderEvent::Usage { .. }))
        );
        assert!(
            !sampling
                .iter()
                .any(|event| matches!(event, ProviderEvent::ModelCallRecorded { .. }))
        );
        let child = project(
            &mut correlation,
            "assistant.message",
            json!({
                "messageId": "child-message", "content": "Child answer",
                "model": "child-model", "parentToolCallId": "t-spawn",
            }),
        );
        assert!(
            !child
                .iter()
                .any(|event| matches!(event, ProviderEvent::AgentMessageModelKnown { .. }))
        );
    }

    #[test]
    fn interaction_type_remains_authoritative_over_the_billing_initiator() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        let classified = project(
            &mut correlation,
            "assistant.usage",
            json!({
                "apiCallId": "c1", "model": "author-model", "inputTokens": 1,
                "initiator": "mcp-sampling", "interactionType": "conversation-agent",
            }),
        );
        assert!(classified.iter().any(|event| matches!(
            event,
            ProviderEvent::ModelCallRecorded { model }
                if model.actual.as_str() == "author-model"
        )));
    }

    #[test]
    fn usage_and_failure_in_either_order_record_one_identified_call() {
        for usage_first in [true, false] {
            let mut correlation = in_turn();
            correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
            let usage = || {
                event(
                    "assistant.usage",
                    json!({ "apiCallId": "c1", "model": "alpha", "isAuto": true, "inputTokens": 1 }),
                )
            };
            let failure = || {
                event(
                    "model.call_failure",
                    json!({ "apiCallId": "c1", "model": "alpha", "isAuto": true, "source": "top_level" }),
                )
            };
            let reports = if usage_first {
                [usage(), failure()]
            } else {
                [failure(), usage()]
            };
            let recorded = reports
                .into_iter()
                .flat_map(|event| project_attributed(&mut correlation, event))
                .filter(|event| matches!(event.event, ProviderEvent::ModelCallRecorded { .. }))
                .count();
            assert_eq!(recorded, 1, "usage_first={usage_first}");
        }
    }

    #[test]
    fn a_call_explicitly_reported_as_fixed_does_not_inherit_the_auto_selection() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        project(
            &mut correlation,
            "assistant.usage",
            json!({
                "apiCallId": "fixed", "model": "fixed-model", "isAuto": false, "inputTokens": 1,
            }),
        );
        let message = project(
            &mut correlation,
            "assistant.message",
            json!({
                "messageId": "m1", "apiCallId": "fixed",
                "model": "fixed-model", "content": "Answer",
            }),
        );
        assert!(
            !message
                .iter()
                .any(|event| matches!(event, ProviderEvent::AgentMessageModelKnown { .. }))
        );
    }

    #[test]
    fn a_failed_auto_call_is_recorded_once_and_keeps_the_current_model() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        project(
            &mut correlation,
            "model.call_start",
            json!({ "turnId": "first", "model": "alpha" }),
        );
        let failed = project(
            &mut correlation,
            "model.call_failure",
            json!({
                "turnId": "first", "source": "top_level", "model": "alpha",
                "apiCallId": "attempt", "isAuto": true,
            }),
        );
        assert!(
            !failed
                .iter()
                .any(|event| matches!(event, ProviderEvent::CurrentModelChanged { .. })),
            "the retry runs on the Model nothing has contradicted"
        );
        assert!(failed.iter().any(|event| matches!(
            event,
            ProviderEvent::ModelCallRecorded { model } if model.actual.as_str() == "alpha"
        )));
        let usage = project(
            &mut correlation,
            "assistant.usage",
            json!({ "apiCallId": "attempt", "model": "alpha", "inputTokens": 1 }),
        );
        assert!(
            !usage
                .iter()
                .any(|event| matches!(event, ProviderEvent::ModelCallRecorded { .. }))
        );
    }

    #[test]
    fn anonymous_failure_and_usage_do_not_invent_two_identifiable_calls() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        let failure = project(
            &mut correlation,
            "model.call_failure",
            json!({ "model": "alpha", "isAuto": true, "source": "top_level" }),
        );
        let usage = project(
            &mut correlation,
            "assistant.usage",
            json!({ "model": "alpha", "isAuto": true, "inputTokens": 1 }),
        );
        assert_eq!(
            failure
                .iter()
                .chain(&usage)
                .filter(|event| matches!(event, ProviderEvent::ModelCallRecorded { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn sampling_failures_neither_clear_nor_enter_a_conversations_calls() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        project(
            &mut correlation,
            "model.call_start",
            json!({ "turnId": "active", "model": "author-model" }),
        );
        let failure = project(
            &mut correlation,
            "model.call_failure",
            json!({
                "source": "mcp_sampling", "model": "sample-model", "isAuto": true,
                "apiCallId": "sample", "initiator": "mcp-sampling",
            }),
        );
        assert!(failure.is_empty());
        let next = project(
            &mut correlation,
            "model.call_start",
            json!({ "turnId": "next", "model": "next-model" }),
        );
        assert!(
            next.iter()
                .any(|event| matches!(event, ProviderEvent::CurrentModelChanged { model: None })),
            "the sampling failure left the active call's evidence in place"
        );
    }

    #[test]
    fn a_committed_fusion_twin_can_identify_its_already_staged_message() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        project(
            &mut correlation,
            "session.fusion_resolved",
            resolved("r", &["primary"]),
        );
        project(
            &mut correlation,
            "assistant.message",
            staged_message("m1", "Answer", fusion("r", "primary", false)),
        );
        let mut committed = staged_message("m1", "Answer", fusion("r", "primary", true));
        committed["fusion"]["sourceModel"] = json!("author-model");
        let projected = project(&mut correlation, "assistant.message", committed);
        assert!(matches!(
            &projected[..],
            [ProviderEvent::AgentMessageModelKnown { message: Some(message), model }]
                if message == &ProviderMessageId::new("m1")
                    && model.actual.as_str() == "author-model"
        ));
    }

    #[test]
    fn a_fusion_twins_explicit_author_is_not_replaced_by_later_usage() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        project(
            &mut correlation,
            "session.fusion_resolved",
            resolved("r", &["primary"]),
        );
        let mut staged = staged_message("m1", "Answer", fusion("r", "primary", false));
        staged["apiCallId"] = json!("c1");
        project(&mut correlation, "assistant.message", staged);
        let mut committed = staged_message("m1", "Answer", fusion("r", "primary", true));
        committed["fusion"]["sourceModel"] = json!("author-model");
        project(&mut correlation, "assistant.message", committed);
        let usage = project(
            &mut correlation,
            "assistant.usage",
            json!({ "apiCallId": "c1", "model": "call-model", "inputTokens": 1 }),
        );
        assert!(usage.iter().any(|event| matches!(
            event,
            ProviderEvent::AgentMessageModelKnown { message: Some(message), model }
                if message == &ProviderMessageId::new("m1")
                    && model.actual.as_str() == "author-model"
        )));
    }

    #[test]
    fn phase_usage_ends_live_phase_evidence_before_a_tool_runs() {
        for child in [false, true] {
            let mut correlation = with_subagent();
            correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
            if child {
                project_attributed(
                    &mut correlation,
                    agent_event(
                        "agent-1",
                        "session.auto_mode_resolved",
                        json!({ "chosenModel": "alpha" }),
                    ),
                );
            }
            let scoped = |kind, data| {
                if child {
                    agent_event("agent-1", kind, data)
                } else {
                    event(kind, data)
                }
            };
            let started = project_attributed(
                &mut correlation,
                scoped(
                    "assistant.fusion_phase_started",
                    json!({
                        "fusionId": "r", "phaseId": "p1", "phaseKind": "primary",
                        "role": "solver", "conversationScope": "root",
                        "pattern": "single", "model": "alpha",
                    }),
                ),
            );
            assert!(started.iter().any(|event| matches!(
                &event.event,
                ProviderEvent::CurrentModelChanged { model: Some(model) }
                    if model.actual.as_str() == "alpha"
            )));
            let used = project_attributed(
                &mut correlation,
                scoped(
                    "assistant.usage",
                    json!({ "model": "alpha", "inputTokens": 1 }),
                ),
            );
            assert!(
                !used
                    .iter()
                    .any(|event| matches!(event.event, ProviderEvent::CurrentModelChanged { .. })),
                "alpha stands until something contradicts it"
            );
            project_attributed(
                &mut correlation,
                scoped(
                    "tool.execution_start",
                    json!({ "toolCallId": "t1", "toolName": "custom", "arguments": {} }),
                ),
            );
            let next = project_attributed(
                &mut correlation,
                scoped(
                    "model.call_start",
                    json!({ "turnId": "next", "model": "beta" }),
                ),
            );
            assert!(next.iter().any(|event| matches!(
                &event.event,
                ProviderEvent::CurrentModelChanged { model: Some(model) }
                    if model.actual.as_str() == "beta"
            )));
        }
    }

    #[test]
    fn a_fusion_twins_call_id_can_bind_later_usage_to_the_staged_message() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        project(
            &mut correlation,
            "session.fusion_resolved",
            resolved("r", &["primary"]),
        );
        project(
            &mut correlation,
            "assistant.message",
            staged_message("m1", "Answer", fusion("r", "primary", false)),
        );
        let mut committed = staged_message("m1", "Answer", fusion("r", "primary", true));
        committed["apiCallId"] = json!("c1");
        assert!(project(&mut correlation, "assistant.message", committed).is_empty());
        let usage = project(
            &mut correlation,
            "assistant.usage",
            json!({ "apiCallId": "c1", "model": "author-model", "inputTokens": 1 }),
        );
        assert!(usage.iter().any(|event| matches!(
            event,
            ProviderEvent::AgentMessageModelKnown { message: Some(message), model }
                if message == &ProviderMessageId::new("m1")
                    && model.actual.as_str() == "author-model"
        )));
    }

    #[test]
    fn late_usage_with_a_known_previous_call_id_does_not_enter_the_next_turns_calls() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        project(
            &mut correlation,
            "assistant.message",
            json!({ "messageId": "m1", "apiCallId": "old-call", "content": "First" }),
        );
        project(&mut correlation, "session.idle", json!({}));
        correlation.begin_turn().expect("begin the next Turn");
        let late = project(
            &mut correlation,
            "assistant.usage",
            json!({ "apiCallId": "old-call", "model": "old-model", "inputTokens": 1 }),
        );
        assert!(
            late.iter()
                .any(|event| matches!(event, ProviderEvent::Usage { .. }))
        );
        assert!(!late.iter().any(|event| matches!(
            event,
            ProviderEvent::ModelCallRecorded { .. } | ProviderEvent::AgentMessageModelKnown { .. }
        )));
    }

    #[test]
    fn call_ids_from_a_fixed_turn_are_not_misattributed_to_a_later_auto_turn() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "assistant.usage",
            json!({ "apiCallId": "old-fixed", "model": "fixed", "inputTokens": 1 }),
        );
        project(&mut correlation, "session.idle", json!({}));
        correlation.begin_turn().expect("begin the next Turn");
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        let late = project(
            &mut correlation,
            "assistant.usage",
            json!({ "apiCallId": "old-fixed", "model": "fixed", "inputTokens": 1 }),
        );
        assert!(
            !late
                .iter()
                .any(|event| matches!(event, ProviderEvent::ModelCallRecorded { .. }))
        );
    }

    #[test]
    fn late_child_usage_does_not_enter_the_resumed_subagents_calls() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({ "apiCallId": "old-child", "model": "old", "isAuto": true, "inputTokens": 1 }),
            ),
        );
        project_attributed(
            &mut correlation,
            event(
                "subagent.completed",
                json!({
                    "toolCallId": "t-spawn", "agentName": "researcher",
                    "agentDisplayName": "Researcher",
                }),
            ),
        );
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "user.message",
                json!({ "content": "Continue", "delivery": "idle" }),
            ),
        );
        let late = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({ "apiCallId": "old-child", "model": "old", "isAuto": true, "inputTokens": 1 }),
            ),
        );
        assert!(!late.iter().any(|event| matches!(
            event.event,
            ProviderEvent::ModelCallRecorded { .. } | ProviderEvent::SubagentModelChanged { .. }
        )));
    }

    #[test]
    fn model_calls_exclude_other_conversations_and_background_work_without_losing_usage() {
        for (child, interaction, parent_tool, expected) in [
            (false, None, false, true),
            (false, Some("conversation-agent"), false, true),
            (false, Some("conversation-subagent"), false, false),
            (false, Some("conversation-compaction"), false, false),
            (false, Some("conversation-background"), false, false),
            (false, Some("conversation-sampling"), false, false),
            (false, Some("future-interaction"), false, false),
            (false, None, true, false),
            (true, None, false, true),
            (true, Some("conversation-subagent"), false, true),
            (true, Some("conversation-agent"), false, false),
            (true, Some("conversation-compaction"), false, false),
        ] {
            let mut correlation = with_subagent();
            correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
            let mut data = json!({
                "apiCallId": "call", "model": "reported-model",
                "isAuto": true, "inputTokens": 1, "interactionType": interaction,
            });
            if parent_tool {
                data["parentToolCallId"] = json!("child-tool");
            }
            let event = if child {
                agent_event("agent-1", "assistant.usage", data)
            } else {
                event("assistant.usage", data)
            };
            let projected = project_attributed(&mut correlation, event);
            assert!(
                projected.iter().any(|event| matches!(
                    &event.event,
                    ProviderEvent::Usage { usage, .. } if usage.fresh_input_tokens == Some(1)
                )),
                "usage must remain accounted for {child:?} {interaction:?} {parent_tool:?}"
            );
            assert_eq!(
                projected
                    .iter()
                    .any(|event| matches!(event.event, ProviderEvent::ModelCallRecorded { .. })),
                expected,
                "call attribution for child={child}, interaction={interaction:?}, parent_tool={parent_tool}",
            );
        }
    }

    #[test]
    fn unclassified_compaction_usage_is_not_a_conversation_model_call() {
        for child in [false, true] {
            let mut correlation = with_subagent();
            correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
            let scoped = |event_type, data| {
                if child {
                    agent_event("agent-1", event_type, data)
                } else {
                    event(event_type, data)
                }
            };
            project_attributed(
                &mut correlation,
                scoped("session.compaction_start", compaction_started()),
            );
            let usage = project_attributed(
                &mut correlation,
                scoped(
                    "assistant.usage",
                    json!({ "model": "summary", "isAuto": true, "inputTokens": 1 }),
                ),
            );
            assert!(
                usage
                    .iter()
                    .any(|event| matches!(event.event, ProviderEvent::Usage { .. }))
            );
            assert!(
                !usage
                    .iter()
                    .any(|event| matches!(event.event, ProviderEvent::ModelCallRecorded { .. }))
            );
            project_attributed(
                &mut correlation,
                scoped("session.compaction_complete", compaction_completed()),
            );
            let after = project_attributed(
                &mut correlation,
                scoped(
                    "assistant.usage",
                    json!({ "model": "answer", "isAuto": true, "inputTokens": 1 }),
                ),
            );
            assert!(
                after
                    .iter()
                    .any(|event| matches!(event.event, ProviderEvent::ModelCallRecorded { .. }))
            );
        }
    }

    #[test]
    fn a_timeline_outside_a_turn_projects_nothing() {
        let mut correlation = CopilotCorrelation::new();
        assert!(
            project(
                &mut correlation,
                "assistant.message_start",
                json!({ "messageId": "m1" }),
            )
            .is_empty()
        );
        assert!(project(&mut correlation, "session.idle", json!({})).is_empty());
    }

    #[test]
    fn a_completed_message_carries_the_content_the_stream_had_not_reached() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "assistant.message_start",
            json!({ "messageId": "m1" }),
        );
        project(
            &mut correlation,
            "assistant.message_delta",
            json!({ "messageId": "m1", "deltaContent": "Hello" }),
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m1", "content": "Hello from Copilot" }),
            ),
            [
                ProviderEvent::AgentMessageDelta {
                    content: " from Copilot".to_owned()
                },
                ProviderEvent::AgentMessageCompleted,
            ]
        );
    }

    #[test]
    fn a_message_copilot_never_started_opens_on_its_first_delta() {
        // Copilot streams a Subagent's Message without ever starting it: the deltas simply
        // begin, the way a Reasoning block announces itself by being reasoned into.
        let mut correlation = with_subagent();
        assert_eq!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "assistant.message_delta",
                    json!({ "messageId": "sub-m1", "deltaContent": "Scouting" }),
                ),
            ),
            [
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::AgentMessageStarted,
                },
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::AgentMessageDelta {
                        content: "Scouting".to_owned()
                    },
                },
            ]
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "assistant.message",
                    json!({ "messageId": "sub-m1", "content": "Scouting the workspace." }),
                ),
            ),
            [
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::AgentMessageDelta {
                        content: " the workspace.".to_owned()
                    },
                },
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::AgentMessageCompleted,
                },
            ],
            "the completed Message reconciles against the stream the delta opened"
        );
    }

    #[test]
    fn a_message_that_never_streamed_still_reaches_the_transcript_whole() {
        let mut correlation = in_turn();
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m1", "content": "Whole" }),
            ),
            [
                ProviderEvent::AgentMessageStarted,
                ProviderEvent::AgentMessageDelta {
                    content: "Whole".to_owned()
                },
                ProviderEvent::AgentMessageCompleted,
            ]
        );
    }

    #[test]
    fn a_message_that_only_asked_for_tools_adds_nothing_to_the_transcript() {
        let mut correlation = in_turn();
        assert!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m1", "content": "" }),
            )
            .is_empty()
        );
    }

    #[test]
    fn content_belonging_to_another_message_is_dropped() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "assistant.message_start",
            json!({ "messageId": "m1" }),
        );
        assert!(
            project(
                &mut correlation,
                "assistant.message_delta",
                json!({ "messageId": "other", "deltaContent": "wrong Message content" }),
            )
            .is_empty()
        );
    }

    #[test]
    fn a_completion_that_contradicts_the_stream_fails_the_session() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "assistant.message_start",
            json!({ "messageId": "m1" }),
        );
        project(
            &mut correlation,
            "assistant.message_delta",
            json!({ "messageId": "m1", "deltaContent": "Hello" }),
        );
        let failure = project_session_event(
            &mut correlation,
            event(
                "assistant.message",
                json!({ "messageId": "m1", "content": "something else entirely" }),
            ),
        )
        .expect_err("a completion that contradicts the stream fails the Session");
        assert!(
            failure.to_string().contains("did not match its stream"),
            "the failure says what contradicted what, got: {failure}"
        );
    }

    #[test]
    fn reasoning_streams_into_the_transcript_with_the_title_it_leads_with_kept_apart() {
        let mut correlation = in_turn();
        let block = reasoning_activity_id("r1");
        assert_eq!(
            project(
                &mut correlation,
                "assistant.reasoning_delta",
                json!({ "reasoningId": "r1", "deltaContent": "**Reading the seam**\n\nOpening" }),
            ),
            [
                ProviderEvent::ReasoningStarted {
                    activity_id: block.clone()
                },
                ProviderEvent::ReasoningTitleChanged {
                    activity_id: block.clone(),
                    title: "Reading the seam".to_owned()
                },
                ProviderEvent::ReasoningDelta {
                    activity_id: block.clone(),
                    content: "Opening".to_owned()
                },
            ]
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.reasoning",
                json!({ "reasoningId": "r1", "content": "**Reading the seam**\n\nOpening the projection." }),
            ),
            [
                ProviderEvent::ReasoningDelta {
                    activity_id: block.clone(),
                    content: " the projection.".to_owned()
                },
                ProviderEvent::ReasoningCompleted { activity_id: block },
            ]
        );
    }

    #[test]
    fn reasoning_a_model_never_streamed_still_reaches_the_transcript_whole() {
        let mut correlation = in_turn();
        let block = reasoning_activity_id("r1");
        assert_eq!(
            project(
                &mut correlation,
                "assistant.reasoning",
                json!({ "reasoningId": "r1", "content": "**Weighing it up**\n\nBoth seams work." }),
            ),
            [
                ProviderEvent::ReasoningStarted {
                    activity_id: block.clone()
                },
                ProviderEvent::ReasoningTitleChanged {
                    activity_id: block.clone(),
                    title: "Weighing it up".to_owned()
                },
                ProviderEvent::ReasoningDelta {
                    activity_id: block.clone(),
                    content: "Both seams work.".to_owned()
                },
                ProviderEvent::ReasoningCompleted { activity_id: block },
            ]
        );
    }

    #[test]
    fn an_aborted_idle_leaves_the_reasoning_it_cut_off_to_the_turns_settle() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "assistant.reasoning_delta",
            json!({ "reasoningId": "r1", "deltaContent": "**Reading the seam**" }),
        );

        assert_eq!(
            project(&mut correlation, "session.idle", json!({ "aborted": true })),
            [
                ProviderEvent::ReasoningTitleChanged {
                    activity_id: reasoning_activity_id("r1"),
                    title: "Reading the seam".to_owned()
                },
                ProviderEvent::TurnInterrupted,
            ],
            "a block cut short keeps the title its split was still withholding, and settles \
             with the Turn rather than completing"
        );
    }

    #[test]
    fn an_idle_completes_the_reasoning_the_turn_never_finished() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "assistant.reasoning_delta",
            json!({ "reasoningId": "r1", "deltaContent": "**Reading the seam**" }),
        );

        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [
                ProviderEvent::ReasoningTitleChanged {
                    activity_id: reasoning_activity_id("r1"),
                    title: "Reading the seam".to_owned()
                },
                ProviderEvent::ReasoningCompleted {
                    activity_id: reasoning_activity_id("r1")
                },
                ProviderEvent::TurnCompleted,
            ]
        );
    }

    #[test]
    fn a_command_streams_its_output_into_the_transcript_and_settles_on_its_outcome() {
        let mut correlation = in_turn();
        let command = tool_activity_id("t1");
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_start",
                json!({
                    "toolCallId": "t1",
                    "toolName": "bash",
                    "arguments": { "command": "cargo nextest run" },
                }),
            ),
            [ProviderEvent::CommandStarted {
                activity_id: command.clone(),
                command: "cargo nextest run".to_owned(),
                cwd: None,
            }]
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.shell_output",
                json!({ "toolCallId": "t1", "text": "running tests\n", "sequence": 0 }),
            ),
            [ProviderEvent::CommandOutputDelta {
                activity_id: command.clone(),
                content: "running tests\n".to_owned(),
            }]
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_complete",
                json!({
                    "toolCallId": "t1",
                    "success": true,
                    "result": { "content": "running tests\nall green\n" },
                }),
            ),
            [
                ProviderEvent::CommandOutputDelta {
                    activity_id: command.clone(),
                    content: "all green\n".to_owned(),
                },
                ProviderEvent::CommandCompleted {
                    activity_id: command,
                    status: ProviderCommandStatus::Completed,
                    exit_status: None,
                },
            ]
        );
    }

    #[test]
    fn a_tool_call_streams_its_output_and_settles_counting_what_it_left_out() {
        let mut correlation = in_turn();
        let tool_call = tool_activity_id("t1");
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_start",
                json!({
                    "toolCallId": "t1",
                    "toolName": "browser-screenshot",
                    "mcpServerName": "browser",
                    "mcpToolName": "screenshot",
                    "arguments": { "url": "https://example.com" },
                }),
            ),
            [ProviderEvent::ToolCallStarted {
                activity_id: tool_call.clone(),
                name: "screenshot".to_owned(),
                server: Some("browser".to_owned()),
                input: Some(ToolCallInput::rendered("url=https://example.com")),
            }]
        );
        assert!(
            project(
                &mut correlation,
                "tool.execution_progress",
                json!({ "toolCallId": "t1", "progressMessage": "Loading the page" }),
            )
            .is_empty(),
            "a progress message is no output"
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.shell_output",
                json!({ "toolCallId": "t1", "text": "Captured ", "sequence": 0 }),
            ),
            [ProviderEvent::ToolCallOutputDelta {
                activity_id: tool_call.clone(),
                content: "Captured ".to_owned(),
            }]
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_complete",
                json!({
                    "toolCallId": "t1",
                    "success": true,
                    "result": {
                        "content": "Captured the page",
                        "contents": [
                            { "type": "text", "text": "Captured the page" },
                            { "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" },
                            { "type": "audio", "data": "UklGRg==", "mimeType": "audio/wav" },
                        ],
                    },
                }),
            ),
            [
                ProviderEvent::ToolCallOutputDelta {
                    activity_id: tool_call.clone(),
                    content: "the page".to_owned(),
                },
                ProviderEvent::ToolCallCompleted {
                    activity_id: tool_call,
                    status: ProviderToolCallStatus::Completed,
                    omitted_parts: 2,
                },
            ]
        );
    }

    #[test]
    fn copilots_plumbing_projects_nothing_at_all() {
        let mut correlation = in_turn();
        for tool in ["report_intent", "task_complete"] {
            for (event_type, data) in [
                (
                    "tool.execution_start",
                    json!({ "toolCallId": tool, "toolName": tool, "arguments": {} }),
                ),
                (
                    "tool.shell_output",
                    json!({ "toolCallId": tool, "text": "noted", "sequence": 0 }),
                ),
                (
                    "tool.execution_complete",
                    json!({ "toolCallId": tool, "success": true, "result": { "content": "ok" } }),
                ),
            ] {
                assert!(
                    project(&mut correlation, event_type, data).is_empty(),
                    "`{tool}` projects nothing at `{event_type}`"
                );
            }
        }
    }

    #[test]
    fn a_command_that_failed_settles_as_failed_with_what_copilot_said_went_wrong() {
        let mut correlation = in_turn();
        let command = tool_activity_id("t1");
        project(
            &mut correlation,
            "tool.execution_start",
            json!({
                "toolCallId": "t1",
                "toolName": "bash",
                "arguments": { "command": "cargo nextest run" },
            }),
        );

        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_complete",
                json!({
                    "toolCallId": "t1",
                    "success": false,
                    "error": { "message": "command timed out" },
                }),
            ),
            [
                ProviderEvent::CommandOutputDelta {
                    activity_id: command.clone(),
                    content: "command timed out".to_owned(),
                },
                ProviderEvent::CommandCompleted {
                    activity_id: command,
                    status: ProviderCommandStatus::Failed,
                    exit_status: None,
                },
            ]
        );
    }

    #[test]
    fn a_command_settles_with_the_exit_code_its_shell_exit_reports() {
        let mut correlation = in_turn();
        let command = tool_activity_id("t1");
        project(
            &mut correlation,
            "tool.execution_start",
            json!({
                "toolCallId": "t1",
                "toolName": "bash",
                "arguments": { "command": "cargo nextest run" },
            }),
        );

        let settled = project(
            &mut correlation,
            "tool.execution_complete",
            json!({
                "toolCallId": "t1",
                "success": false,
                "result": {
                    "content": "1 test failed",
                    "contents": [
                        { "type": "text", "text": "1 test failed" },
                        { "type": "shell_exit", "shellId": "s1", "exitCode": 101 },
                    ],
                },
                "shellExecution": { "exitCode": 7 },
                "error": { "message": "1 test failed" },
            }),
        );

        assert_eq!(
            settled.last(),
            Some(&ProviderEvent::CommandCompleted {
                activity_id: command,
                status: ProviderCommandStatus::Failed,
                exit_status: Some(101),
            })
        );
    }

    #[test]
    fn a_command_whose_result_reports_no_shell_exit_settles_with_its_shell_executions_exit_code() {
        let mut correlation = in_turn();
        let command = tool_activity_id("t1");
        project(
            &mut correlation,
            "tool.execution_start",
            json!({
                "toolCallId": "t1",
                "toolName": "bash",
                "arguments": { "command": "cargo nextest run" },
            }),
        );

        let settled = project(
            &mut correlation,
            "tool.execution_complete",
            json!({
                "toolCallId": "t1",
                "success": false,
                "shellExecution": { "exitCode": 2 },
                "error": { "message": "no such file" },
            }),
        );

        assert_eq!(
            settled.last(),
            Some(&ProviderEvent::CommandCompleted {
                activity_id: command,
                status: ProviderCommandStatus::Failed,
                exit_status: Some(2),
            })
        );
    }

    #[test]
    fn a_command_that_failed_part_way_keeps_the_reason_off_the_output_it_had_produced() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "tool.execution_start",
            json!({
                "toolCallId": "t1",
                "toolName": "bash",
                "arguments": { "command": "cargo nextest run" },
            }),
        );
        project(
            &mut correlation,
            "tool.shell_output",
            json!({ "toolCallId": "t1", "text": "running tests", "sequence": 0 }),
        );

        let projected = project(
            &mut correlation,
            "tool.execution_complete",
            json!({
                "toolCallId": "t1",
                "success": false,
                "error": { "message": "command timed out" },
            }),
        );
        let [ProviderEvent::CommandOutputDelta { content, .. }, _] = projected.as_slice() else {
            panic!("a failed command carries its reason into its output, got {projected:?}");
        };
        assert_eq!(content, "\ncommand timed out");
    }

    #[test]
    fn a_failed_commands_error_follows_a_result_reported_beside_it() {
        let mut correlation = in_turn();
        let complete = |tool_call_id: &str, result: &str, error: &str| {
            json!({
                "toolCallId": tool_call_id,
                "success": false,
                "result": { "content": result },
                "error": { "message": error },
            })
        };
        let trailing = |correlation: &mut CopilotCorrelation,
                        tool_call_id: &str,
                        streamed: &str,
                        completed: serde_json::Value| {
            project(
                correlation,
                "tool.execution_start",
                json!({
                    "toolCallId": tool_call_id,
                    "toolName": "bash",
                    "arguments": { "command": "cargo nextest run" },
                }),
            );
            if !streamed.is_empty() {
                project(
                    correlation,
                    "tool.shell_output",
                    json!({ "toolCallId": tool_call_id, "text": streamed, "sequence": 0 }),
                );
            }
            project(correlation, "tool.execution_complete", completed)
                .into_iter()
                .filter_map(|event| match event {
                    ProviderEvent::CommandOutputDelta { content, .. } => Some(content),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(
            trailing(
                &mut correlation,
                "t-empty",
                "",
                complete("t-empty", "", "command timed out")
            ),
            ["command timed out"],
            "an empty result leaves the error as the whole output"
        );
        assert_eq!(
            trailing(
                &mut correlation,
                "t-repeated",
                "running tests",
                complete("t-repeated", "running tests", "command timed out"),
            ),
            ["\ncommand timed out"],
            "a result repeating the stream leaves the error on a line of its own below it"
        );
        assert_eq!(
            trailing(
                &mut correlation,
                "t-extended",
                "running tests\n",
                complete(
                    "t-extended",
                    "running tests\n1 failed\n",
                    "command timed out"
                ),
            ),
            ["1 failed\ncommand timed out"],
            "the rest of the result comes first, and the error after it"
        );
        assert_eq!(
            trailing(
                &mut correlation,
                "t-restated",
                "",
                complete("t-restated", "command timed out", "command timed out"),
            ),
            ["command timed out"],
            "an error the result already reports is not repeated"
        );
        assert_eq!(
            trailing(
                &mut correlation,
                "t-finished",
                "command ti",
                complete("t-finished", "command timed out", "command timed out"),
            ),
            ["med out"],
            "nor is one the stream began and the result finished"
        );
    }

    #[test]
    fn an_idle_settles_the_turn_and_whatever_it_left_streaming() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "assistant.message_start",
            json!({ "messageId": "m1" }),
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [
                ProviderEvent::AgentMessageCompleted,
                ProviderEvent::TurnCompleted
            ]
        );
        assert!(
            project(&mut correlation, "session.idle", json!({})).is_empty(),
            "the Turn settles once"
        );
    }

    #[test]
    fn an_aborted_idle_settles_the_turn_as_interrupted() {
        let mut correlation = in_turn();
        assert_eq!(
            project(&mut correlation, "session.idle", json!({ "aborted": true })),
            [ProviderEvent::TurnInterrupted]
        );
    }

    #[test]
    fn a_copilot_error_fails_the_turn_with_the_category_copilot_typed_it_as() {
        let mut correlation = in_turn();
        assert!(
            project(
                &mut correlation,
                "session.error",
                json!({ "errorType": "rate_limit", "message": "too many requests" }),
            )
            .is_empty(),
            "the loop stopping is what settles the Turn on the error"
        );
        let projected = project(&mut correlation, "session.idle", json!({}));
        let [ProviderEvent::TurnFailed { message }] = projected.as_slice() else {
            panic!("a Copilot error fails the Turn, got {projected:?}");
        };
        assert_eq!(message, "Copilot rate limit error: too many requests");
    }

    #[test]
    fn the_first_error_is_what_the_turn_settles_as() {
        let mut correlation = in_turn();
        for message in ["the cause", "a consequence"] {
            project(
                &mut correlation,
                "session.error",
                json!({ "errorType": "quota", "message": message }),
            );
        }
        let projected = project(&mut correlation, "session.idle", json!({}));
        let [ProviderEvent::TurnFailed { message }] = projected.as_slice() else {
            panic!("a Copilot error fails the Turn, got {projected:?}");
        };
        assert_eq!(message, "Copilot quota error: the cause");
    }

    #[test]
    fn a_failed_turns_idle_settles_that_turn_rather_than_the_one_after_it() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.error",
            json!({ "errorType": "authentication", "message": "sign in again" }),
        );
        project(&mut correlation, "session.idle", json!({}));

        correlation
            .begin_turn()
            .expect("the Session takes the next Prompt");
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m2", "content": "Second answer" }),
            ),
            [
                ProviderEvent::AgentMessageStarted,
                ProviderEvent::AgentMessageDelta {
                    content: "Second answer".to_owned()
                },
                ProviderEvent::AgentMessageCompleted,
            ],
            "the Turn after a failed one runs rather than settling on the failed one's idle"
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted]
        );
    }

    #[test]
    fn a_transient_error_leaves_the_turn_running() {
        let mut correlation = in_turn();
        assert!(
            project(
                &mut correlation,
                "session.error",
                json!({ "errorType": "model_call", "message": "retrying" }),
            )
            .is_empty()
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted],
            "the Turn Copilot recovered inside still completes"
        );
    }

    #[test]
    fn a_subagent_started_opens_the_subagent_in_the_owning_session() {
        let mut correlation = in_turn();
        assert_eq!(
            project_attributed(&mut correlation, spawn_started("agent-1", "t-spawn")),
            [AttributedProviderEvent {
                attribution: ProviderEventAttribution::OwningSession,
                event: ProviderEvent::SubagentStarted {
                    subagent_id: ProviderSubagentId::new("agent-1"),
                    name: "Researcher".to_owned(),
                    description: "Scout the workspace".to_owned(),
                    delegation: None,
                },
            }],
            "a spawn whose tool call never surfaced as an execution carries no Delegation"
        );
        assert!(
            project_attributed(&mut correlation, spawn_started("agent-1", "t-spawn")).is_empty(),
            "a repeated started opens nothing twice"
        );
    }

    #[test]
    fn a_subagent_started_carries_the_prompt_its_spawning_tool_call_handed_it() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "tool.execution_start",
            json!({
                "toolCallId": "t-spawn",
                "toolName": "task",
                "arguments": {
                    "agent_type": "explore",
                    "description": "Scout the workspace",
                    "prompt": "Find every TODO marker and report the files.",
                },
            }),
        );
        let projected = project_attributed(&mut correlation, spawn_started("agent-1", "t-spawn"));
        let [
            AttributedProviderEvent {
                event: ProviderEvent::SubagentStarted { delegation, .. },
                ..
            },
        ] = projected.as_slice()
        else {
            panic!("the spawn opens one Subagent, got {projected:?}");
        };
        assert_eq!(
            delegation.as_deref(),
            Some("Find every TODO marker and report the files."),
            "the task tool's prompt is the spawn's Delegation"
        );

        // A spawn out of a Subagent's own tool call reads the prompt from that Subagent's
        // conversation.
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "tool.execution_start",
                json!({
                    "toolCallId": "t-nested",
                    "toolName": "task",
                    "arguments": { "prompt": "Read the TODO in src/main.rs." },
                }),
            ),
        );
        let nested = project_attributed(&mut correlation, spawn_started("agent-2", "t-nested"));
        let [
            AttributedProviderEvent {
                event: ProviderEvent::SubagentStarted { delegation, .. },
                ..
            },
        ] = nested.as_slice()
        else {
            panic!("the nested spawn opens one Subagent, got {nested:?}");
        };
        assert_eq!(delegation.as_deref(), Some("Read the TODO in src/main.rs."));
    }

    #[test]
    fn a_spawn_outside_any_turn_with_nothing_owed_lands_nowhere() {
        let mut correlation = CopilotCorrelation::new();
        assert!(
            project_attributed(&mut correlation, spawn_started("agent-1", "t-spawn")).is_empty()
        );
        assert!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "assistant.message",
                    json!({ "messageId": "sub-m1", "content": "orphan" }),
                ),
            )
            .is_empty(),
            "work attributed to the dropped spawn lands nowhere too"
        );
    }

    #[test]
    fn a_subagents_stream_lands_in_the_subagent_while_the_main_message_streams_its_own() {
        let mut correlation = with_subagent();
        project(
            &mut correlation,
            "assistant.message_start",
            json!({ "messageId": "m1" }),
        );

        // The subagent's whole Message arrives while the main agent's is still open: it lands
        // attributed to the Subagent rather than colliding with the main Message.
        assert_eq!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "assistant.message",
                    json!({ "messageId": "sub-m1", "content": "Scouted." }),
                ),
            ),
            [
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::AgentMessageStarted,
                },
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::AgentMessageDelta {
                        content: "Scouted.".to_owned()
                    },
                },
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::AgentMessageCompleted,
                },
            ]
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message_delta",
                json!({ "messageId": "m1", "deltaContent": "Still the main answer" }),
            ),
            [ProviderEvent::AgentMessageDelta {
                content: "Still the main answer".to_owned()
            }],
            "the main Message streams on undisturbed"
        );
    }

    #[test]
    fn a_subagents_stream_keeps_landing_after_the_turn_settled() {
        let mut correlation = with_subagent();
        project(&mut correlation, "session.idle", json!({}));

        let projected = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "tool.execution_start",
                json!({ "toolCallId": "t-sub", "toolName": "bash", "arguments": { "command": "cargo audit" } }),
            ),
        );
        assert_eq!(
            projected,
            [AttributedProviderEvent {
                attribution: subagent("agent-1"),
                event: ProviderEvent::CommandStarted {
                    activity_id: tool_activity_id("t-sub"),
                    command: "cargo audit".to_owned(),
                    cwd: None,
                },
            }],
            "a Subagent outlives the Turn that spawned it"
        );
    }

    #[test]
    fn a_subagent_settle_closes_its_open_streams_and_reports_the_outcome() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.reasoning_delta",
                json!({ "reasoningId": "sub-r1", "deltaContent": "**Weighing the findings**" }),
            ),
        );

        assert_eq!(
            project_attributed(
                &mut correlation,
                event(
                    "subagent.completed",
                    json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "Researcher" }),
                ),
            ),
            [
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::ReasoningTitleChanged {
                        activity_id: reasoning_activity_id("sub-r1"),
                        title: "Weighing the findings".to_owned(),
                    },
                },
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::ReasoningCompleted {
                        activity_id: reasoning_activity_id("sub-r1"),
                    },
                },
                AttributedProviderEvent {
                    attribution: ProviderEventAttribution::OwningSession,
                    event: ProviderEvent::SubagentCompleted {
                        subagent_id: ProviderSubagentId::new("agent-1"),
                        status: ProviderSubagentStatus::Completed,
                    },
                },
            ],
            "a block cut short keeps the title its split was still withholding"
        );
        assert!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "assistant.message",
                    json!({ "messageId": "sub-m2", "content": "too late" }),
                ),
            )
            .is_empty(),
            "nothing more of the settled Subagent's lands anywhere"
        );
    }

    #[test]
    fn a_cancelled_completion_and_a_failure_both_settle_the_subagent_as_failed() {
        for (event_type, data) in [
            (
                "subagent.completed",
                json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "Researcher", "cancelled": true }),
            ),
            (
                "subagent.failed",
                json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "Researcher", "error": "the researcher crashed" }),
            ),
        ] {
            let mut correlation = with_subagent();
            let projected = project_attributed(&mut correlation, event(event_type, data));
            assert_eq!(
                projected,
                [AttributedProviderEvent {
                    attribution: ProviderEventAttribution::OwningSession,
                    event: ProviderEvent::SubagentCompleted {
                        subagent_id: ProviderSubagentId::new("agent-1"),
                        status: ProviderSubagentStatus::Failed,
                    },
                }],
                "`{event_type}` settles the Subagent as failed"
            );
        }
    }

    #[test]
    fn work_attributed_to_an_unknown_instance_lands_nowhere() {
        let mut correlation = in_turn();
        assert!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "never-started",
                    "assistant.message",
                    json!({ "messageId": "m9", "content": "whose is this" }),
                ),
            )
            .is_empty()
        );
    }

    #[test]
    fn late_output_owed_to_a_settled_subagent_begins_a_continuation_settled_by_the_next_idle() {
        let mut correlation = with_subagent();
        project(&mut correlation, "session.idle", json!({}));
        project_attributed(
            &mut correlation,
            event(
                "subagent.completed",
                json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "Researcher" }),
            ),
        );

        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m2", "content": "The scout came back." }),
            ),
            [
                ProviderEvent::AgentMessageStarted,
                ProviderEvent::AgentMessageDelta {
                    content: "The scout came back.".to_owned()
                },
                ProviderEvent::AgentMessageCompleted,
            ],
            "output the settle provoked projects with no Turn active"
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted],
            "the stretch's own idle settles the Continuation"
        );
        assert!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m3", "content": "stray" }),
            )
            .is_empty(),
            "with nothing owed any longer, stray output is dropped as it always was"
        );
    }

    #[test]
    fn a_prompt_during_a_continuation_swallows_the_stale_stretches_idle() {
        let mut correlation = with_subagent();
        project(&mut correlation, "session.idle", json!({}));
        project_attributed(
            &mut correlation,
            event(
                "subagent.completed",
                json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "Researcher" }),
            ),
        );
        project(
            &mut correlation,
            "assistant.message_start",
            json!({ "messageId": "m2" }),
        );

        correlation
            .begin_turn()
            .expect("a Prompt settles the Continuation rather than colliding with it");
        assert!(
            project(&mut correlation, "session.idle", json!({})).is_empty(),
            "the stale stretch's idle settles nothing"
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m3", "content": "Prompted answer" }),
            ),
            [
                ProviderEvent::AgentMessageStarted,
                ProviderEvent::AgentMessageDelta {
                    content: "Prompted answer".to_owned()
                },
                ProviderEvent::AgentMessageCompleted,
            ]
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted],
            "the Prompt's own Turn settles on its own idle"
        );
    }

    #[test]
    fn a_spawn_tool_execution_is_represented_by_the_subagent_row_alone() {
        let mut correlation = in_turn();
        assert!(
            project(
                &mut correlation,
                "tool.execution_start",
                json!({
                    "toolCallId": "t-spawn",
                    "toolName": "task",
                    "arguments": { "agent_type": "explore", "prompt": "Scout the workspace" },
                }),
            )
            .is_empty(),
            "the spawning tool call opens no row of its own"
        );
        assert!(
            matches!(
                project_attributed(&mut correlation, spawn_started("agent-1", "t-spawn"))
                    .as_slice(),
                [AttributedProviderEvent {
                    event: ProviderEvent::SubagentStarted { .. },
                    ..
                }]
            ),
            "the Subagent row is the delegation's whole representation"
        );
        assert!(
            project(
                &mut correlation,
                "tool.execution_complete",
                json!({ "toolCallId": "t-spawn", "success": true, "result": { "content": "done" } }),
            )
            .is_empty(),
            "the spawn's completion settles nothing: the Subagent lifecycle already did"
        );
    }

    #[test]
    fn a_spawn_execution_reported_after_its_subagent_opened_stays_withheld() {
        // The runtime's own delegation may report `subagent.started` before — or without — the
        // spawning tool call ever surfacing as an execution.
        let mut correlation = in_turn();
        project_attributed(&mut correlation, spawn_started("agent-1", "t-spawn"));
        assert!(
            project(
                &mut correlation,
                "tool.execution_start",
                json!({ "toolCallId": "t-spawn", "toolName": "task", "arguments": {} }),
            )
            .is_empty(),
        );

        assert!(
            project(
                &mut correlation,
                "tool.execution_complete",
                json!({ "toolCallId": "t-spawn", "success": true, "result": { "content": "done" } }),
            )
            .is_empty(),
            "the Subagent row already answers for the delegation, whichever event opened it first"
        );
    }

    #[test]
    fn a_spawn_that_never_opened_its_subagent_surfaces_as_the_tool_call_it_was() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "tool.execution_start",
            json!({
                "toolCallId": "t-spawn",
                "toolName": "task",
                "arguments": { "agent_type": "no-such-agent" },
            }),
        );

        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_complete",
                json!({
                    "toolCallId": "t-spawn",
                    "success": false,
                    "error": { "message": "unknown agent type" },
                }),
            ),
            [
                ProviderEvent::ToolCallStarted {
                    activity_id: tool_activity_id("t-spawn"),
                    name: "task".to_owned(),
                    server: None,
                    input: Some(ToolCallInput::rendered("agent_type=no-such-agent")),
                },
                ProviderEvent::ToolCallOutputDelta {
                    activity_id: tool_activity_id("t-spawn"),
                    content: "unknown agent type".to_owned(),
                },
                ProviderEvent::ToolCallCompleted {
                    activity_id: tool_activity_id("t-spawn"),
                    status: ProviderToolCallStatus::Failed,
                    omitted_parts: 0,
                },
            ],
            "with no Subagent row answering for the delegation, the failed spawn stays visible"
        );
    }

    #[test]
    fn a_repeated_spawn_execution_start_does_not_forget_the_delegation() {
        let mut correlation = in_turn();
        let start = json!({ "toolCallId": "t-spawn", "toolName": "task", "arguments": {} });
        project(&mut correlation, "tool.execution_start", start.clone());
        project_attributed(&mut correlation, spawn_started("agent-1", "t-spawn"));
        assert!(project(&mut correlation, "tool.execution_start", start).is_empty());

        assert!(
            project(
                &mut correlation,
                "tool.execution_complete",
                json!({ "toolCallId": "t-spawn", "success": true, "result": { "content": "done" } }),
            )
            .is_empty(),
            "a repeated start neither reopens the row nor un-delegates the spawn"
        );
    }

    #[test]
    fn a_withheld_spawn_still_open_at_the_turns_settle_surfaces_for_the_store_to_settle() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "tool.execution_start",
            json!({ "toolCallId": "t-spawn", "toolName": "task", "arguments": { "agent_type": "explore" } }),
        );

        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [
                ProviderEvent::ToolCallStarted {
                    activity_id: tool_activity_id("t-spawn"),
                    name: "task".to_owned(),
                    server: None,
                    input: Some(ToolCallInput::rendered("agent_type=explore")),
                },
                ProviderEvent::TurnCompleted,
            ],
            "a delegation no Subagent row ever answered for does not vanish with the Turn"
        );
    }

    #[test]
    fn a_spawn_out_of_a_subagents_own_tool_call_recurses_one_level_down() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "tool.execution_start",
                json!({ "toolCallId": "t-nested", "toolName": "task", "arguments": {} }),
            ),
        );

        let projected = project_attributed(&mut correlation, spawn_started("agent-2", "t-nested"));
        let [AttributedProviderEvent { attribution, event }] = projected.as_slice() else {
            panic!("a nested spawn opens one Subagent, got {projected:?}");
        };
        assert_eq!(
            *attribution,
            subagent("agent-1"),
            "the grandchild hangs under the Subagent whose tool call spawned it"
        );
        assert!(matches!(
            event,
            ProviderEvent::SubagentStarted { subagent_id, .. }
                if *subagent_id == ProviderSubagentId::new("agent-2")
        ));
    }

    #[test]
    fn a_started_without_an_instance_identity_still_opens_the_row() {
        let mut correlation = in_turn();
        let projected = project_attributed(
            &mut correlation,
            event(
                "subagent.started",
                json!({
                    "toolCallId": "t-spawn",
                    "agentName": "researcher",
                    "agentDisplayName": "",
                    "agentDescription": "Scout the workspace",
                }),
            ),
        );
        let [
            AttributedProviderEvent {
                event:
                    ProviderEvent::SubagentStarted {
                        subagent_id, name, ..
                    },
                ..
            },
        ] = projected.as_slice()
        else {
            panic!("the spawn still opens the Subagent, got {projected:?}");
        };
        assert_eq!(
            *subagent_id,
            ProviderSubagentId::new("t-spawn"),
            "the spawning tool call stands in for the missing instance identity"
        );
        assert_eq!(
            name, "researcher",
            "an empty display name falls back to the configured one"
        );

        assert_eq!(
            project_attributed(
                &mut correlation,
                event(
                    "subagent.completed",
                    json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "" }),
                ),
            ),
            [AttributedProviderEvent {
                attribution: ProviderEventAttribution::OwningSession,
                event: ProviderEvent::SubagentCompleted {
                    subagent_id: ProviderSubagentId::new("t-spawn"),
                    status: ProviderSubagentStatus::Completed,
                },
            }],
            "the settle still reaches the row it opened"
        );
    }

    /// One shell as `session.tasks.list` lists it.
    fn listed_shell(id: &str, attachment: &str, status: &str) -> serde_json::Value {
        json!({
            "type": "shell",
            "id": id,
            "description": "Serve the docs",
            "command": "mdbook serve",
            "status": status,
            "startedAt": "2026-01-01T00:00:00Z",
            "attachmentMode": attachment,
        })
    }

    fn detached_completion(shell: &str) -> serde_json::Value {
        json!({
            "content": "<system_notification>\nDetached shell \"Serve the docs\" (shellId: shell-1) has completed.\n</system_notification>",
            "kind": { "type": "shell_detached_completed", "shellId": shell, "description": "Serve the docs" },
        })
    }

    fn roster(
        correlation: &mut CopilotCorrelation,
        tasks: &[serde_json::Value],
    ) -> Vec<ProviderEvent> {
        correlation
            .project_task_roster(tasks)
            .into_iter()
            .map(|attributed| {
                assert_eq!(
                    attributed.attribution,
                    ProviderEventAttribution::OwningSession
                );
                attributed.event
            })
            .collect()
    }

    #[test]
    fn a_running_detached_shell_on_the_roster_starts_a_watch_under_its_shell_identity() {
        let mut correlation = in_turn();
        assert_eq!(
            roster(
                &mut correlation,
                &[
                    listed_shell("shell-1", "detached", "running"),
                    json!({ "type": "agent", "id": "agent-1", "description": "Audit" }),
                ],
            ),
            [ProviderEvent::WatchStarted {
                watch_id: ProviderWatchId::new("shell-1"),
                description: "Serve the docs".to_owned(),
            }]
        );
        assert!(
            roster(
                &mut correlation,
                &[listed_shell("shell-1", "detached", "running")]
            )
            .is_empty(),
            "a shell already watched is not announced again"
        );
    }

    #[test]
    fn an_attached_shell_run_in_the_background_starts_a_watch() {
        let mut correlation = in_turn();
        let mut shell = listed_shell("0", "attached", "running");
        shell["executionMode"] = json!("background");
        assert_eq!(
            roster(&mut correlation, &[shell]),
            [ProviderEvent::WatchStarted {
                watch_id: ProviderWatchId::new("0"),
                description: "Serve the docs".to_owned(),
            }]
        );
    }

    #[test]
    fn synchronous_and_already_ended_shells_on_the_roster_are_no_watches() {
        let mut correlation = in_turn();
        let mut synchronous = listed_shell("synchronous", "attached", "running");
        synchronous["executionMode"] = json!("sync");
        assert!(
            roster(
                &mut correlation,
                &[
                    synchronous,
                    listed_shell("ended", "detached", "completed"),
                    listed_shell("cancelled", "detached", "cancelled"),
                ],
            )
            .is_empty()
        );
    }

    #[test]
    fn a_detached_shell_described_by_nothing_is_described_by_its_command() {
        let mut correlation = in_turn();
        let mut shell = listed_shell("shell-1", "detached", "running");
        shell["description"] = json!(" ");
        assert_eq!(
            roster(&mut correlation, &[shell]),
            [ProviderEvent::WatchStarted {
                watch_id: ProviderWatchId::new("shell-1"),
                description: "mdbook serve".to_owned(),
            }]
        );
    }

    #[test]
    fn a_detached_shells_completion_settles_its_watch_and_begins_a_continuation() {
        let mut correlation = in_turn();
        roster(
            &mut correlation,
            &[listed_shell("shell-1", "detached", "running")],
        );
        project(&mut correlation, "session.idle", json!({}));

        assert_eq!(
            project(
                &mut correlation,
                "system.notification",
                detached_completion("shell-1")
            ),
            [ProviderEvent::WatchSettled {
                watch_id: ProviderWatchId::new("shell-1"),
                outcome: ProviderWatchOutcome::Completed,
                summary: Some(
                    r#"Detached shell "Serve the docs" (shellId: shell-1) has completed."#
                        .to_owned()
                ),
                woke_agent: true,
            }],
            "the notification's text, out of its wrapper, is how the Watch settled"
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m2", "content": "The docs server stopped." }),
            )
            .len(),
            3,
            "the woken loop's output opens a Continuation rather than being dropped"
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted]
        );
        assert!(
            project(
                &mut correlation,
                "system.notification",
                detached_completion("shell-1")
            )
            .is_empty(),
            "a settled Watch settles once"
        );
        assert!(
            roster(
                &mut correlation,
                &[listed_shell("shell-1", "detached", "running")]
            )
            .is_empty(),
            "a roster read racing the completion does not start the settled Watch again"
        );
    }

    #[test]
    fn a_live_watch_owes_the_loops_output_a_continuation() {
        let mut correlation = in_turn();
        roster(
            &mut correlation,
            &[listed_shell("shell-1", "detached", "running")],
        );
        project(&mut correlation, "session.idle", json!({}));

        assert!(
            !project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m2", "content": "Checking the server." }),
            )
            .is_empty()
        );
    }

    #[test]
    fn a_completion_for_a_shell_never_seen_records_its_watch_whole_and_owes_the_woken_loop_a_continuation()
     {
        let mut correlation = in_turn();
        project(&mut correlation, "session.idle", json!({}));

        assert_eq!(
            project(
                &mut correlation,
                "system.notification",
                detached_completion("unseen")
            ),
            [
                ProviderEvent::WatchStarted {
                    watch_id: ProviderWatchId::new("unseen"),
                    description: "Serve the docs".to_owned(),
                },
                ProviderEvent::WatchSettled {
                    watch_id: ProviderWatchId::new("unseen"),
                    outcome: ProviderWatchOutcome::Completed,
                    summary: Some(
                        r#"Detached shell "Serve the docs" (shellId: shell-1) has completed."#
                            .to_owned()
                    ),
                    woke_agent: true,
                },
            ],
            "orchestration owes the woken output a Continuation only through a Watch it knows"
        );
        assert!(
            !project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m2", "content": "Woken." }),
            )
            .is_empty(),
            "the loop wakes all the same, and its output is shown"
        );
        project(&mut correlation, "session.idle", json!({}));
        assert!(
            project(
                &mut correlation,
                "system.notification",
                detached_completion("unseen")
            )
            .is_empty(),
            "a repeated completion records nothing twice"
        );
    }

    #[test]
    fn a_completion_for_a_shell_suru_stopped_wakes_nothing_suru_shows() {
        let mut correlation = in_turn();
        roster(
            &mut correlation,
            &[listed_shell("shell-1", "detached", "running")],
        );
        project(&mut correlation, "session.idle", json!({}));
        correlation.project_watches_stopped(&["shell-1".to_owned()]);

        assert!(
            project(
                &mut correlation,
                "system.notification",
                detached_completion("shell-1")
            )
            .is_empty()
        );
        assert!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m2", "content": "stray" }),
            )
            .is_empty(),
            "a Watch stopped by an interrupt wakes nothing Suru records"
        );
    }

    #[test]
    fn an_attached_shell_completing_with_a_failing_exit_code_settles_its_watch_failed() {
        let mut correlation = in_turn();
        let mut shell = listed_shell("0", "attached", "running");
        shell["executionMode"] = json!("background");
        roster(&mut correlation, &[shell]);
        project(&mut correlation, "assistant.idle", json!({}));

        assert_eq!(
            project(
                &mut correlation,
                "system.notification",
                json!({
                    "content": "<system_notification>\nShell command \"Serve the docs\" (shellId: 0) has completed.\n</system_notification>",
                    "kind": { "type": "shell_completed", "shellId": "0", "exitCode": 2 },
                }),
            ),
            [ProviderEvent::WatchSettled {
                watch_id: ProviderWatchId::new("0"),
                outcome: ProviderWatchOutcome::Failed,
                summary: Some(
                    r#"Shell command "Serve the docs" (shellId: 0) has completed."#.to_owned()
                ),
                woke_agent: true,
            }]
        );
    }

    #[test]
    fn other_notifications_wake_nothing_this_projection_follows() {
        let mut correlation = in_turn();
        project(&mut correlation, "session.idle", json!({}));

        assert!(
            project(
                &mut correlation,
                "system.notification",
                json!({ "content": "idle", "kind": { "type": "agent_idle", "agentId": "a" } }),
            )
            .is_empty()
        );
        assert!(
            project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m2", "content": "stray" }),
            )
            .is_empty()
        );
    }

    #[test]
    fn stopped_watches_settle_as_stopped_and_a_later_completion_repeats_nothing() {
        let mut correlation = in_turn();
        roster(
            &mut correlation,
            &[
                listed_shell("shell-1", "detached", "running"),
                listed_shell("shell-2", "detached", "running"),
            ],
        );
        project(&mut correlation, "session.idle", json!({}));
        assert_eq!(
            correlation.live_watches(&[
                ProviderWatchId::new("shell-1"),
                ProviderWatchId::new("gone")
            ]),
            ["shell-1"]
        );

        let stopped = correlation.project_watches_stopped(&["shell-1".to_owned()]);
        assert_eq!(
            stopped
                .into_iter()
                .map(|event| event.event)
                .collect::<Vec<_>>(),
            [ProviderEvent::WatchSettled {
                watch_id: ProviderWatchId::new("shell-1"),
                outcome: ProviderWatchOutcome::Stopped,
                summary: None,
                woke_agent: false,
            }]
        );
        assert!(
            correlation
                .project_watches_stopped(&["shell-1".to_owned()])
                .is_empty()
        );
        let lost = correlation.project_watches_lost();
        assert_eq!(
            lost.into_iter()
                .map(|event| event.event)
                .collect::<Vec<_>>(),
            [ProviderEvent::WatchSettled {
                watch_id: ProviderWatchId::new("shell-2"),
                outcome: ProviderWatchOutcome::Lost,
                summary: None,
                woke_agent: false,
            }],
            "a process that dies loses every Watch still live"
        );
    }

    #[test]
    fn a_notification_out_of_its_wrapper_is_what_a_reader_sees() {
        assert_eq!(
            notification_text("<system_notification>\n  Shell done.\n</system_notification>\n"),
            Some("Shell done.".to_owned())
        );
        assert_eq!(
            notification_text("Unwrapped."),
            Some("Unwrapped.".to_owned())
        );
        assert_eq!(
            notification_text("<system_notification></system_notification>"),
            None
        );
    }

    /// A `user.message` Copilot delivers to `agent`'s own loop, as `delivery`, from `source`.
    fn delivered_message(
        agent: &str,
        delivery: &str,
        source: Option<&str>,
        content: &str,
    ) -> SessionEvent {
        let mut data = json!({ "content": content, "delivery": delivery, "turnId": "1" });
        if let Some(source) = source {
            data["source"] = json!(source);
        }
        agent_event(agent, "user.message", data)
    }

    fn steer(subagent: &str, delegation: &str) -> ProviderEvent {
        ProviderEvent::SubagentSteered {
            subagent_id: ProviderSubagentId::new(subagent),
            delegation: delegation.to_owned(),
        }
    }

    #[test]
    fn a_message_steering_a_working_subagent_is_a_steer_the_main_agent_sent() {
        let mut correlation = with_subagent();
        assert_eq!(
            project_attributed(
                &mut correlation,
                delivered_message(
                    "agent-1",
                    "steering",
                    Some("agent-94bf4cb4-74ac-43d9-8903-038759ca8a86"),
                    "Also say PINEAPPLE.",
                ),
            ),
            [AttributedProviderEvent {
                attribution: ProviderEventAttribution::OwningSession,
                event: steer("agent-1", "Also say PINEAPPLE."),
            }],
            "the main agent sends as `agent-<its session id>`, which names no Subagent"
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "steering", None, "And MANGO."),
            ),
            [AttributedProviderEvent {
                attribution: ProviderEventAttribution::OwningSession,
                event: steer("agent-1", "And MANGO."),
            }],
            "a steer naming no sender is the owning Session's"
        );
    }

    #[test]
    fn a_message_a_sibling_sent_steers_on_that_siblings_behalf_even_once_it_settled() {
        let mut correlation = with_subagent();
        project_attributed(&mut correlation, spawn_started("agent-2", "t-sibling"));
        assert_eq!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "steering", Some("agent-agent-2"), "Say MANGO."),
            ),
            [AttributedProviderEvent {
                attribution: subagent("agent-2"),
                event: steer("agent-1", "Say MANGO."),
            }]
        );

        project_attributed(
            &mut correlation,
            agent_event(
                "agent-2",
                "subagent.completed",
                json!({ "toolCallId": "t-sibling", "agentName": "researcher", "agentDisplayName": "Researcher" }),
            ),
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "steering", Some("agent-agent-2"), "Say KIWI."),
            ),
            [AttributedProviderEvent {
                attribution: subagent("agent-2"),
                event: steer("agent-1", "Say KIWI."),
            }],
            "the sender is still the sibling that sent it after that sibling settled"
        );
    }

    #[test]
    fn a_message_that_does_not_steer_a_working_subagent_is_no_steer() {
        let mut correlation = with_subagent();
        for delivery in ["idle", "queued", "later"] {
            assert!(
                project_attributed(
                    &mut correlation,
                    delivered_message("agent-1", delivery, None, "Read the notes."),
                )
                .is_empty(),
                "a `{delivery}` delivery begins or awaits a run of its own rather than steering"
            );
        }
        assert!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "user.message",
                    json!({ "content": "Read the notes." })
                ),
            )
            .is_empty(),
            "a message with no delivery reported steers nothing"
        );
        assert!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "steering", None, "  "),
            )
            .is_empty(),
            "a steer carrying nothing to read stands nowhere"
        );
    }

    #[test]
    fn a_message_for_an_instance_no_subagent_holds_stands_nowhere() {
        let mut correlation = with_subagent();
        for delivery in ["steering", "queued", "idle"] {
            assert!(
                project_attributed(
                    &mut correlation,
                    delivered_message("agent-9", delivery, None, "Who?"),
                )
                .is_empty(),
                "an instance no Subagent holds has no Session for a `{delivery}` message to reach"
            );
        }
    }

    #[test]
    fn a_main_agent_message_is_not_a_subagent_steer() {
        let mut correlation = with_subagent();
        assert!(
            project(
                &mut correlation,
                "user.message",
                json!({ "content": "Answer in French", "delivery": "steering" }),
            )
            .is_empty(),
            "the main loop's own messages are the user's Prompts, which Suru already holds"
        );
    }

    /// The Broker's Tools reach Copilot as executions on the MCP server `suru`. A call that spawns,
    /// sends to, or stops a Subagent is no work a Transcript presents — the Broker adds the row
    /// that stands for what it did — so the execution projects nothing in the conversation that
    /// made it, the main agent's or a native Subagent's, while a call to any other MCP server is a
    /// Tool Call like any other Tool's.
    #[test]
    fn a_broker_call_affecting_a_subagent_adds_nothing_to_the_transcript_of_its_agent() {
        let mut correlation = with_subagent();
        let broker_call = |tool_call_id: &str| {
            json!({
                "toolCallId": tool_call_id,
                "toolName": "suru-spawn_subagent",
                "mcpServerName": "suru",
                "mcpToolName": "spawn_subagent",
                "arguments": { "provider": "codex", "model": "gpt-5.5", "prompt": "Map it." },
            })
        };
        for (event_type, data) in [
            ("tool.execution_start", broker_call("t-broker")),
            (
                "tool.shell_output",
                json!({ "toolCallId": "t-broker", "text": "Spawning", "sequence": 0 }),
            ),
            (
                "tool.execution_complete",
                json!({
                    "toolCallId": "t-broker",
                    "success": true,
                    "result": { "content": "Spawned the Subagent." },
                }),
            ),
        ] {
            assert!(
                project(&mut correlation, event_type, data).is_empty(),
                "the main agent's Broker call projects nothing at `{event_type}`"
            );
        }
        for event in [
            agent_event(
                "agent-1",
                "tool.execution_start",
                broker_call("t-sub-broker"),
            ),
            agent_event(
                "agent-1",
                "tool.execution_complete",
                json!({
                    "toolCallId": "t-sub-broker",
                    "success": false,
                    "error": { "message": "The spawn was refused." },
                }),
            ),
        ] {
            assert!(
                project_attributed(&mut correlation, event).is_empty(),
                "a Subagent's Broker call projects nothing in its own conversation either"
            );
        }

        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_start",
                json!({
                    "toolCallId": "t-linear",
                    "toolName": "linear-list_issues",
                    "mcpServerName": "linear",
                    "mcpToolName": "list_issues",
                    "arguments": {},
                }),
            ),
            [ProviderEvent::ToolCallStarted {
                activity_id: tool_activity_id("t-linear"),
                name: "list_issues".to_owned(),
                server: Some("linear".to_owned()),
                input: Some(ToolCallInput::rendered("")),
            }],
            "another MCP server's call is a Tool Call"
        );
    }

    #[test]
    fn a_write_agent_execution_adds_nothing_to_the_senders_transcript() {
        let mut correlation = with_subagent();
        let write_agent = |tool_call_id: &str| {
            json!({
                "toolCallId": tool_call_id,
                "toolName": "write_agent",
                "arguments": { "agent_id": "agent-1", "message": "Also say PINEAPPLE." },
            })
        };
        assert!(
            project(
                &mut correlation,
                "tool.execution_start",
                write_agent("t-write")
            )
            .is_empty()
        );
        assert!(
            project(
                &mut correlation,
                "tool.shell_output",
                json!({ "toolCallId": "t-write", "text": "Message delivered", "sequence": 0 }),
            )
            .is_empty()
        );
        assert!(
            project(
                &mut correlation,
                "tool.execution_complete",
                json!({
                    "toolCallId": "t-write",
                    "success": true,
                    "result": { "content": "Message delivered to agent agent-1." },
                }),
            )
            .is_empty(),
            "the send is a Delegation standing in the Subagent it reached, not a Command"
        );
        assert!(
            project(
                &mut correlation,
                "tool.execution_start",
                write_agent("t-refused")
            )
            .is_empty()
        );
        assert!(
            project(
                &mut correlation,
                "tool.execution_complete",
                json!({
                    "toolCallId": "t-refused",
                    "success": false,
                    "error": { "message": "No agent agent-1" },
                }),
            )
            .is_empty(),
            "a send that delivered nothing stands nowhere either"
        );

        // A sibling's send out of its own conversation is absorbed there just the same.
        project_attributed(&mut correlation, spawn_started("agent-2", "t-sibling"));
        for event in [
            agent_event(
                "agent-2",
                "tool.execution_start",
                write_agent("t-sib-write"),
            ),
            agent_event(
                "agent-2",
                "tool.execution_complete",
                json!({ "toolCallId": "t-sib-write", "success": true }),
            ),
        ] {
            assert!(project_attributed(&mut correlation, event).is_empty());
        }

        // One still running when the loop stops surfaces nothing for the store to settle.
        project(
            &mut correlation,
            "tool.execution_start",
            write_agent("t-open"),
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted]
        );
    }

    /// The main agent's own identity as a sender: `agent-<its Copilot Session id>`, which names
    /// no Subagent.
    const MAIN_AGENT: &str = "agent-94bf4cb4-74ac-43d9-8903-038759ca8a86";

    /// A correlation whose Turn spawned `agent-1`, whose stretch has since settled.
    fn with_settled_subagent() -> CopilotCorrelation {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "subagent.completed",
                json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "Researcher" }),
            ),
        );
        correlation
    }

    fn resumed(subagent: &str, description: &str, delegation: Option<&str>) -> ProviderEvent {
        ProviderEvent::SubagentResumed {
            subagent_id: ProviderSubagentId::new(subagent),
            name: "Researcher".to_owned(),
            description: description.to_owned(),
            delegation: delegation.map(str::to_owned),
        }
    }

    fn settled(subagent: &str, status: ProviderSubagentStatus) -> AttributedProviderEvent {
        AttributedProviderEvent {
            attribution: ProviderEventAttribution::OwningSession,
            event: ProviderEvent::SubagentCompleted {
                subagent_id: ProviderSubagentId::new(subagent),
                status,
            },
        }
    }

    /// The model message `agent` closes one agent-loop turn with: one requesting `tools`, or with
    /// none, the answer that exits the loop.
    fn loop_message(agent: &str, message_id: &str, content: &str, tools: &[&str]) -> SessionEvent {
        let requests = tools
            .iter()
            .map(|tool| json!({ "toolCallId": tool, "name": "bash", "arguments": {} }))
            .collect::<Vec<_>>();
        agent_event(
            agent,
            "assistant.message",
            json!({ "messageId": message_id, "content": content, "toolRequests": requests }),
        )
    }

    fn turn_end(agent: &str, turn_id: &str) -> SessionEvent {
        agent_event(agent, "assistant.turn_end", json!({ "turnId": turn_id }))
    }

    #[test]
    fn a_message_a_settled_subagent_consumes_resumes_it_whatever_its_delivery() {
        for delivery in ["queued", "idle", "steering"] {
            let mut correlation = with_settled_subagent();
            assert_eq!(
                project_attributed(
                    &mut correlation,
                    delivered_message(
                        "agent-1",
                        delivery,
                        Some(MAIN_AGENT),
                        "Also say PINEAPPLE.\nThen stop."
                    ),
                ),
                [AttributedProviderEvent {
                    attribution: ProviderEventAttribution::OwningSession,
                    event: resumed(
                        "agent-1",
                        "Also say PINEAPPLE.",
                        Some("Also say PINEAPPLE.\nThen stop."),
                    ),
                }],
                "a `{delivery}` message the settled Subagent consumed runs it again: the main \
                 agent's resume, opened by the message, described by its first line"
            );
            assert_eq!(
                project_attributed(
                    &mut correlation,
                    agent_event(
                        "agent-1",
                        "assistant.message",
                        json!({ "messageId": "r1", "content": "PINEAPPLE" }),
                    ),
                ),
                [
                    ProviderEvent::AgentMessageStarted,
                    ProviderEvent::AgentMessageDelta {
                        content: "PINEAPPLE".to_owned(),
                    },
                    ProviderEvent::AgentMessageCompleted,
                ]
                .map(|event| AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event,
                }),
                "the resume routes the Subagent's work to it again"
            );
        }
    }

    #[test]
    fn a_resume_a_sibling_sent_is_that_siblings_even_once_it_settled() {
        let mut correlation = with_settled_subagent();
        project_attributed(&mut correlation, spawn_started("agent-2", "t-sibling"));
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-2",
                "subagent.completed",
                json!({ "toolCallId": "t-sibling", "agentName": "researcher", "agentDisplayName": "Researcher" }),
            ),
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "queued", Some("agent-agent-2"), "Say MANGO."),
            ),
            [AttributedProviderEvent {
                attribution: subagent("agent-2"),
                event: resumed("agent-1", "Say MANGO.", Some("Say MANGO.")),
            }]
        );
    }

    #[test]
    fn a_resume_carries_the_model_the_subagent_last_ran_on() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.usage",
                json!({ "model": "gpt-5.6-luna", "inputTokens": 10, "outputTokens": 2 }),
            ),
        );
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "subagent.completed",
                json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "Researcher" }),
            ),
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "idle", Some(MAIN_AGENT), "Once more."),
            ),
            [
                AttributedProviderEvent {
                    attribution: ProviderEventAttribution::OwningSession,
                    event: resumed("agent-1", "Once more.", Some("Once more.")),
                },
                AttributedProviderEvent {
                    attribution: ProviderEventAttribution::OwningSession,
                    event: ProviderEvent::SubagentModelChanged {
                        subagent_id: ProviderSubagentId::new("agent-1"),
                        model: crate::protocol::ModelId::new("gpt-5.6-luna"),
                    },
                },
            ],
            "Copilot reports a Subagent's Model change only at spawn, so it follows the resume"
        );
    }

    #[test]
    fn a_resume_that_carries_nothing_to_read_is_described_as_the_subagent_was() {
        let mut correlation = with_settled_subagent();
        assert_eq!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "queued", None, "  "),
            ),
            [AttributedProviderEvent {
                attribution: ProviderEventAttribution::OwningSession,
                event: resumed("agent-1", "Scout the workspace", None),
            }]
        );
    }

    #[test]
    fn the_spawns_own_prompt_is_no_resume() {
        let mut correlation = with_subagent();
        assert!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "idle", Some(MAIN_AGENT), "Scout the workspace."),
            )
            .is_empty(),
            "the spawn's prompt arrives before the Subagent has worked, and its spawn already \
             carried it in"
        );
    }

    #[test]
    fn a_resumed_turn_settles_at_the_turn_end_after_a_message_requesting_no_tools() {
        let mut correlation = with_settled_subagent();
        project_attributed(
            &mut correlation,
            delivered_message("agent-1", "idle", Some(MAIN_AGENT), "Run it again."),
        );
        project_attributed(
            &mut correlation,
            loop_message("agent-1", "r1", "", &["t-bash"]),
        );
        assert!(
            project_attributed(&mut correlation, turn_end("agent-1", "0")).is_empty(),
            "a model message requesting tools keeps the loop going"
        );
        project_attributed(
            &mut correlation,
            loop_message("agent-1", "r2", "Done.", &[]),
        );
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.reasoning",
                json!({ "reasoningId": "rs1", "content": "Wrapping up." }),
            ),
        );
        assert_eq!(
            project_attributed(&mut correlation, turn_end("agent-1", "1")),
            [settled("agent-1", ProviderSubagentStatus::Completed)],
            "the loop exits at the turn end after an answer requesting no tools"
        );
        assert!(
            project_attributed(
                &mut correlation,
                loop_message("agent-1", "r3", "Late.", &[])
            )
            .is_empty(),
            "the settled Subagent is routed no longer"
        );
        assert!(
            project_attributed(&mut correlation, turn_end("agent-1", "2")).is_empty(),
            "and its stretch settles once"
        );
    }

    #[test]
    fn a_spawns_turn_end_settles_nothing() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            loop_message("agent-1", "s1", "Found it.", &[]),
        );
        assert!(
            project_attributed(&mut correlation, turn_end("agent-1", "0")).is_empty(),
            "Copilot settles a spawn's stretch itself, with `subagent.completed`"
        );
    }

    #[test]
    fn a_later_message_settles_a_resume_still_open_before_beginning_the_next() {
        let mut correlation = with_settled_subagent();
        project_attributed(
            &mut correlation,
            delivered_message("agent-1", "queued", Some(MAIN_AGENT), "First."),
        );
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message_delta",
                json!({ "messageId": "r1", "deltaContent": "Half" }),
            ),
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "queued", Some(MAIN_AGENT), "Second."),
            ),
            [
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::AgentMessageCompleted,
                },
                settled("agent-1", ProviderSubagentStatus::Completed),
                AttributedProviderEvent {
                    attribution: ProviderEventAttribution::OwningSession,
                    event: resumed("agent-1", "Second.", Some("Second.")),
                },
            ],
            "Copilot delivers the next message only once the previous run has ended"
        );
    }

    #[test]
    fn the_loops_idle_settles_a_resume_still_open() {
        let mut correlation = with_settled_subagent();
        project_attributed(
            &mut correlation,
            delivered_message("agent-1", "queued", Some(MAIN_AGENT), "Again."),
        );
        assert_eq!(
            project_attributed(&mut correlation, event("session.idle", json!({}))),
            [
                settled("agent-1", ProviderSubagentStatus::Completed),
                AttributedProviderEvent {
                    attribution: ProviderEventAttribution::OwningSession,
                    event: ProviderEvent::TurnCompleted,
                },
            ]
        );
    }

    #[test]
    fn an_aborted_idle_settles_a_resume_interrupted() {
        let mut correlation = with_settled_subagent();
        project_attributed(
            &mut correlation,
            delivered_message("agent-1", "queued", Some(MAIN_AGENT), "Again."),
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                event("session.idle", json!({ "aborted": true }))
            ),
            [
                settled("agent-1", ProviderSubagentStatus::Interrupted),
                AttributedProviderEvent {
                    attribution: ProviderEventAttribution::OwningSession,
                    event: ProviderEvent::TurnInterrupted,
                },
            ]
        );
    }

    #[test]
    fn a_resume_the_main_agent_sends_with_no_turn_running_begins_a_continuation() {
        let mut correlation = with_settled_subagent();
        project(&mut correlation, "session.idle", json!({}));
        assert!(!correlation.is_turn_running());
        assert_eq!(
            project_attributed(
                &mut correlation,
                delivered_message("agent-1", "idle", Some(MAIN_AGENT), "Again."),
            ),
            [AttributedProviderEvent {
                attribution: ProviderEventAttribution::OwningSession,
                event: resumed("agent-1", "Again.", Some("Again.")),
            }]
        );
        assert!(
            correlation.is_turn_running(),
            "the resume's row stands in a Continuation the loop's next idle settles"
        );
        assert_eq!(
            project_attributed(&mut correlation, event("session.idle", json!({}))),
            [
                settled("agent-1", ProviderSubagentStatus::Completed),
                AttributedProviderEvent {
                    attribution: ProviderEventAttribution::OwningSession,
                    event: ProviderEvent::TurnCompleted,
                },
            ]
        );
    }

    #[test]
    fn an_agent_idle_notification_projects_nothing() {
        let mut correlation = with_settled_subagent();
        project_attributed(
            &mut correlation,
            delivered_message("agent-1", "queued", Some(MAIN_AGENT), "Again."),
        );
        assert!(
            project(
                &mut correlation,
                "system.notification",
                json!({
                    "content": "<system_notification>\nAgent \"researcher\" has finished.\n</system_notification>",
                    "kind": {
                        "type": "agent_idle",
                        "agentId": "agent-1",
                        "agentType": "task",
                        "description": "Scout the workspace",
                        "displayName": "Researcher",
                    },
                }),
            )
            .is_empty(),
            "Copilot names a resumed agent idle in some runs and not others, so it settles nothing"
        );
    }

    fn compaction_started() -> serde_json::Value {
        json!({ "currentTokens": 182_000, "trigger": "threshold" })
    }

    fn compaction_completed() -> serde_json::Value {
        json!({ "success": true, "preCompactionTokens": 182_000, "postCompactionTokens": 31_000 })
    }

    fn compacted() -> ProviderEvent {
        ProviderEvent::CompactionCompleted {
            before_tokens: Some(182_000),
            after_tokens: Some(31_000),
            summary: None,
        }
    }

    pub(super) fn selection() -> AgentSelection {
        AgentSelection {
            provider: crate::protocol::ProviderId::new("copilot"),
            model: crate::protocol::ModelId::new("claude-fixture"),
            options: Vec::new(),
        }
    }

    /// A correlation whose first Turn settled under `selection()` with nothing left owing.
    fn after_a_turn() -> CopilotCorrelation {
        let mut correlation = in_turn();
        correlation.context_prompt_ready(TurnId::new(), selection());
        project(&mut correlation, "session.idle", json!({}));
        correlation
    }

    #[test]
    fn a_compaction_report_settles_as_copilot_says_whatever_it_was_triggered_by() {
        let mut correlation = in_turn();
        for trigger in [
            "threshold",
            "context_limit_retry",
            "memory_pressure",
            "model_switch",
        ] {
            assert_eq!(
                project(
                    &mut correlation,
                    "session.compaction_start",
                    json!({ "trigger": trigger }),
                ),
                [ProviderEvent::CompactionStarted]
            );
            assert_eq!(
                project(
                    &mut correlation,
                    "session.compaction_complete",
                    json!({ "success": false, "trigger": trigger, "error": "too long" }),
                ),
                [ProviderEvent::CompactionFailed {
                    error: Some("too long".to_owned())
                }]
            );
        }
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                json!({ "preCompactionTokens": 1 }),
            ),
            [ProviderEvent::CompactionFailed { error: None }],
            "an end Suru cannot read still ends the compaction"
        );
    }

    #[test]
    fn copilots_truncation_and_summarising_model_calls_project_nothing() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        for (event_type, data) in [
            (
                "session.truncation",
                json!({
                    "messagesRemovedDuringTruncation": 4,
                    "performedBy": "BasicTruncator",
                    "postTruncationMessagesLength": 10,
                    "postTruncationTokensInMessages": 9000,
                    "preTruncationMessagesLength": 14,
                    "preTruncationTokensInMessages": 12000,
                    "tokenLimit": 200_000,
                    "tokensRemovedDuringTruncation": 3000,
                }),
            ),
            (
                "model.call_start",
                json!({ "turnId": "compaction-1", "model": "claude-fixture" }),
            ),
            (
                "model.call_failure",
                json!({ "turnId": "compaction-1", "statusCode": 429 }),
            ),
            (
                "model.call_finished",
                json!({
                    "turnId": "compaction-1",
                    "dispatchDurationMs": 40_000.0,
                    "editClassifierVersion": 1,
                    "outcome": "success",
                }),
            ),
        ] {
            assert!(
                project(&mut correlation, event_type, data.clone()).is_empty(),
                "`{event_type}` records nothing"
            );
            assert!(
                project_attributed(
                    &mut with_subagent(),
                    agent_event("agent-1", event_type, data)
                )
                .is_empty(),
                "a sub-agent's `{event_type}` records nothing"
            );
        }
    }

    #[test]
    fn summarising_model_calls_while_the_loop_is_idle_neither_wake_it_nor_open_a_turn() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        assert!(project(&mut correlation, "session.idle", json!({})).is_empty());
        assert!(
            project(
                &mut correlation,
                "model.call_start",
                json!({ "turnId": "compaction-1" }),
            )
            .is_empty()
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted(), ProviderEvent::TurnCompleted],
            "the compaction's own model call leaves the held idle standing"
        );
        assert!(
            project(
                &mut correlation,
                "model.call_finished",
                json!({ "turnId": "compaction-1" }),
            )
            .is_empty(),
            "a model call reported after the Turn settled opens no Continuation"
        );
    }

    #[test]
    fn an_idle_mid_compaction_settles_the_turn_as_the_idle_said_once_the_compaction_ends() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        assert!(
            project(&mut correlation, "session.idle", json!({ "aborted": true })).is_empty(),
            "the Turn is held open while Copilot compacts"
        );
        assert!(correlation.is_turn_running());
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                json!({ "success": false, "error": "too long" }),
            ),
            [
                ProviderEvent::CompactionFailed {
                    error: Some("too long".to_owned())
                },
                ProviderEvent::TurnInterrupted
            ]
        );
        assert!(!correlation.is_turn_running());
    }

    #[test]
    fn loop_output_after_a_held_idle_leaves_the_turn_to_the_loops_next_idle() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        project(&mut correlation, "session.idle", json!({}));
        project(
            &mut correlation,
            "assistant.message",
            json!({ "messageId": "m-steered", "content": "Steered." }),
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted()],
            "the loop is running again, so the compaction ending settles nothing"
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted]
        );
    }

    #[test]
    fn a_compaction_while_no_turn_runs_begins_a_continuation_copilot_owns_and_its_end_settles() {
        let mut correlation = after_a_turn();
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_start",
                compaction_started(),
            ),
            [
                ProviderEvent::ContinuationStarted {
                    selection: selection()
                },
                ProviderEvent::CompactionStarted
            ]
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted(), ProviderEvent::TurnCompleted]
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [
                ProviderEvent::ContinuationStarted {
                    selection: selection()
                },
                compacted(),
                ProviderEvent::TurnCompleted
            ],
            "an end with no start still records its Compaction, in a Continuation it settles"
        );
    }

    #[test]
    fn a_compaction_continuation_woken_by_the_loop_settles_on_the_loops_idle() {
        let mut correlation = after_a_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        project(
            &mut correlation,
            "assistant.message",
            json!({ "messageId": "m-woken", "content": "Woken." }),
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted()]
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted]
        );
    }

    /// A correlation whose Turn's loop went idle while Copilot still compacts.
    fn held_by_a_compaction() -> CopilotCorrelation {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        assert!(project(&mut correlation, "session.idle", json!({})).is_empty());
        correlation
    }

    fn interrupted(settled: bool) -> Vec<AttributedProviderEvent> {
        if settled {
            vec![attributed(None, ProviderEvent::TurnInterrupted)]
        } else {
            Vec::new()
        }
    }

    /// What is left of the interrupt `scope` began once Copilot has answered its cancel, if one
    /// went out, as `cancelled` says — an answer that leaves the interrupt nothing to wait on.
    fn finished(
        correlation: &mut CopilotCorrelation,
        scope: InterruptScope,
        cancelled: bool,
    ) -> InterruptRemainder {
        assert!(
            correlation.cancel_answered(scope, cancelled).is_none(),
            "nothing is left to wait on"
        );
        correlation.finish_interrupt(scope, cancelled)
    }

    #[test]
    fn an_interrupt_with_nothing_compacting_aborts_the_loop_alone() {
        let mut correlation = in_turn();
        let scope = correlation.begin_interrupt();
        assert!(!scope.compacting);
        assert_eq!(
            finished(&mut correlation, scope, scope.compacting),
            InterruptRemainder {
                abort: true,
                settled: Vec::new(),
                awaits_idle: false,
                already_ended: false,
            }
        );
    }

    #[test]
    fn an_interrupt_with_no_turn_running_aborts_the_subagents_working_past_it() {
        let mut correlation = with_subagent();
        project(&mut correlation, "session.idle", json!({}));
        let scope = correlation.begin_interrupt();
        assert_eq!(
            finished(&mut correlation, scope, scope.compacting),
            InterruptRemainder {
                abort: true,
                settled: Vec::new(),
                awaits_idle: false,
                already_ended: false,
            }
        );
    }

    #[test]
    fn an_interrupt_while_the_loop_works_and_copilot_compacts_stops_both_and_waits_on_neither() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        let scope = correlation.begin_interrupt();
        assert!(scope.compacting);
        assert_eq!(
            finished(&mut correlation, scope, scope.compacting),
            InterruptRemainder {
                abort: true,
                settled: Vec::new(),
                awaits_idle: false,
                already_ended: false,
            },
            "a working loop's own aborted idle settles its Turn"
        );
        assert!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            )
            .is_empty(),
            "the cancelled compaction's end is owed nothing, even one the cancel was too late for"
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({ "aborted": true })),
            [ProviderEvent::TurnInterrupted],
            "the aborted idle waits on no compaction"
        );
    }

    #[test]
    fn an_interrupt_of_a_turn_only_a_compaction_holds_settles_it_with_nothing_to_abort() {
        let mut correlation = held_by_a_compaction();
        let scope = correlation.begin_interrupt();
        assert!(scope.compacting);
        assert_eq!(
            finished(&mut correlation, scope, scope.compacting),
            InterruptRemainder {
                abort: false,
                settled: interrupted(true),
                awaits_idle: false,
                already_ended: false,
            }
        );
        assert!(
            project(
                &mut correlation,
                "session.compaction_complete",
                json!({ "success": false, "error": "Compaction cancelled" }),
            )
            .is_empty(),
            "the cancelled compaction's end is owed nothing: its Compaction settled with the Turn"
        );
    }

    #[test]
    fn an_interrupt_of_a_turn_a_compaction_holds_still_aborts_the_subagents_working_past_it() {
        let mut correlation = with_subagent();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        project(&mut correlation, "session.idle", json!({}));
        let scope = correlation.begin_interrupt();
        assert_eq!(
            finished(&mut correlation, scope, scope.compacting),
            InterruptRemainder {
                abort: true,
                settled: Vec::new(),
                awaits_idle: true,
                already_ended: false,
            },
            "the abort the Subagent needs ends in an idle of the loop's own, which settles the Turn"
        );
        let carried = correlation.expect_abort_idle();
        assert!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "subagent.completed",
                    json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "Researcher", "cancelled": true }),
                ),
            )
            .iter()
            .all(|event| event.event != ProviderEvent::TurnInterrupted)
        );
        correlation.idle_carried();
        assert!(
            futures_util::FutureExt::now_or_never(carried.notified()).is_some(),
            "the interrupt hears the timeline carried the idle it waits on"
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({ "aborted": true })),
            [ProviderEvent::TurnInterrupted],
            "the abort's own idle settles the Turn it was for, with nothing compacting to hold it"
        );
        assert!(
            correlation.abort_idle_missing(scope).is_empty(),
            "a Turn its idle settled is not settled again"
        );
    }

    #[test]
    fn a_held_turn_whose_subagents_abort_ends_in_no_idle_settles_when_the_wait_runs_out() {
        let mut correlation = with_subagent();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        project(&mut correlation, "session.idle", json!({}));
        let scope = correlation.begin_interrupt();
        assert!(finished(&mut correlation, scope, scope.compacting).awaits_idle);
        correlation.expect_abort_idle();
        assert_eq!(correlation.abort_idle_missing(scope), interrupted(true));
    }

    #[test]
    fn an_idle_while_an_interrupt_cancels_the_compaction_leaves_the_turn_to_the_interrupt() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        let scope = correlation.begin_interrupt();
        assert!(
            project(&mut correlation, "session.idle", json!({})).is_empty(),
            "the compaction still holds the Turn until Copilot answers the cancel"
        );
        assert!(
            project(
                &mut correlation,
                "session.compaction_complete",
                json!({ "success": false, "error": "Compaction cancelled" }),
            )
            .is_empty(),
            "the cancel taking settles nothing on its own"
        );
        assert_eq!(
            finished(&mut correlation, scope, scope.compacting),
            InterruptRemainder {
                abort: false,
                settled: interrupted(true),
                awaits_idle: false,
                already_ended: false,
            },
            "the loop stopped while the cancel was out, so nothing is left to abort"
        );
    }

    #[test]
    fn an_interrupt_stops_nothing_of_a_turn_after_the_one_it_was_for() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        let scope = correlation.begin_interrupt();
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted()],
            "a compaction that completed before the cancel took did compact"
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted],
            "with nothing compacting, the loop's idle settles its Turn"
        );
        correlation
            .begin_turn()
            .expect("the next Prompt begins its Turn");

        assert_eq!(
            finished(&mut correlation, scope, scope.compacting),
            InterruptRemainder {
                abort: false,
                settled: Vec::new(),
                awaits_idle: false,
                already_ended: false,
            },
            "the Turn the interrupt was for settled, and the one after is not its to stop"
        );
        assert!(correlation.is_turn_running());
    }

    #[test]
    fn a_failure_reported_while_the_cancel_is_out_is_the_cancel_if_it_took() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        let scope = correlation.begin_interrupt();
        assert!(
            project(
                &mut correlation,
                "session.compaction_complete",
                json!({ "success": false, "error": "Compaction cancelled" }),
            )
            .is_empty(),
            "whose failure it was waits on Copilot's answer to the cancel"
        );
        assert!(
            project(&mut correlation, "session.idle", json!({})).is_empty(),
            "and until then it holds the Turn"
        );
        assert_eq!(
            finished(&mut correlation, scope, scope.compacting).settled,
            interrupted(true),
            "the cancel took, so the failure was it, and the interrupt settles the Turn"
        );
    }

    #[test]
    fn a_failure_reported_while_the_cancel_is_out_is_the_compactions_own_if_it_failed() {
        let mut correlation = held_by_a_compaction();
        correlation.begin_interrupt();
        project(
            &mut correlation,
            "session.compaction_complete",
            json!({ "success": false, "error": "too long" }),
        );
        assert_eq!(
            correlation.cancel_failed(),
            [
                attributed(
                    None,
                    ProviderEvent::CompactionFailed {
                        error: Some("too long".to_owned())
                    }
                ),
                attributed(None, ProviderEvent::TurnCompleted)
            ],
            "the cancel never took, so Copilot's failure stands, and the Turn it held settles"
        );
    }

    #[test]
    fn a_failure_reported_while_the_cancel_is_out_is_the_compactions_own_if_nothing_was_cancelled()
    {
        let mut held = held_by_a_compaction();
        let scope = held.begin_interrupt();
        project(
            &mut held,
            "session.compaction_complete",
            json!({ "success": false, "error": "too long" }),
        );
        let failed = attributed(
            None,
            ProviderEvent::CompactionFailed {
                error: Some("too long".to_owned()),
            },
        );
        assert_eq!(
            finished(&mut held, scope, false),
            InterruptRemainder {
                abort: false,
                settled: vec![
                    failed.clone(),
                    attributed(None, ProviderEvent::TurnCompleted)
                ],
                awaits_idle: false,
                already_ended: true,
            },
            "the failure stands, and the interrupt stopped nothing of the Turn it held, which \
             settles as its loop's idle said"
        );

        let mut working = in_turn();
        project(
            &mut working,
            "session.compaction_start",
            compaction_started(),
        );
        let scope = working.begin_interrupt();
        project(
            &mut working,
            "session.compaction_complete",
            json!({ "success": false, "error": "too long" }),
        );
        assert!(finished(&mut working, scope, false).abort);
        assert_eq!(
            project_attributed(
                &mut working,
                event("session.idle", json!({ "aborted": true }))
            ),
            [failed, attributed(None, ProviderEvent::TurnInterrupted)],
            "the failure stands ahead of the Turn the abort's idle settles"
        );
    }

    /// Answers the cancel the interrupt `scope` began that Copilot found nothing to cancel, before
    /// the compaction's end has reached the timeline, which the interrupt then waits on.
    fn found_nothing_to_cancel(correlation: &mut CopilotCorrelation, scope: InterruptScope) {
        assert!(scope.compacting);
        let end = correlation
            .cancel_answered(scope, false)
            .expect("the interrupt waits on the end of a compaction the cancel found over");
        assert!(
            futures_util::FutureExt::now_or_never(end.notified()).is_none(),
            "the end has yet to reach the timeline"
        );
        correlation.compaction_end_carried();
        assert!(
            futures_util::FutureExt::now_or_never(end.notified()).is_some(),
            "the interrupt hears the timeline carried the end it waits on"
        );
    }

    #[test]
    fn a_compaction_the_cancel_found_over_settles_the_turn_it_held_as_its_end_says() {
        for (end, outcome) in [
            (compaction_completed(), compacted()),
            (
                json!({ "success": false, "error": "too long" }),
                ProviderEvent::CompactionFailed {
                    error: Some("too long".to_owned()),
                },
            ),
        ] {
            let mut held = held_by_a_compaction();
            let scope = held.begin_interrupt();
            found_nothing_to_cancel(&mut held, scope);
            assert_eq!(
                held.finish_interrupt(scope, false),
                InterruptRemainder {
                    abort: false,
                    settled: Vec::new(),
                    awaits_idle: false,
                    already_ended: true,
                },
                "the interrupt stopped nothing, and leaves the Turn to the compaction's end"
            );
            assert_eq!(
                project(&mut held, "session.compaction_complete", end),
                [outcome, ProviderEvent::TurnCompleted],
                "the end stands as Copilot reports it, and settles the Turn as its idle said"
            );
        }
    }

    #[test]
    fn an_interrupt_waits_on_no_end_of_a_compaction_the_timeline_already_carried() {
        let mut held = held_by_a_compaction();
        let scope = held.begin_interrupt();
        held.compaction_end_carried();
        assert!(
            held.cancel_answered(scope, false).is_none(),
            "the end is already on its way to the projection"
        );
        assert!(held.finish_interrupt(scope, false).already_ended);
        assert_eq!(
            project(
                &mut held,
                "session.compaction_complete",
                compaction_completed()
            ),
            [compacted(), ProviderEvent::TurnCompleted]
        );

        let mut retried = in_turn();
        project(
            &mut retried,
            "session.compaction_start",
            compaction_started(),
        );
        project(
            &mut retried,
            "session.compaction_complete",
            json!({ "success": false, "error": "too long" }),
        );
        retried.compaction_end_carried();
        project(
            &mut retried,
            "session.compaction_start",
            compaction_started(),
        );
        let scope = retried.begin_interrupt();
        assert!(
            retried.cancel_answered(scope, false).is_some(),
            "the end of an earlier attempt, which the projection has read, is no end still to come"
        );
    }

    #[test]
    fn a_compaction_whose_end_never_follows_the_cancel_settles_with_the_turn_the_interrupt_stops() {
        let mut held = held_by_a_compaction();
        let scope = held.begin_interrupt();
        assert!(held.cancel_answered(scope, false).is_some());
        held.compaction_end_missing();
        assert_eq!(
            held.finish_interrupt(scope, false),
            InterruptRemainder {
                abort: false,
                settled: interrupted(true),
                awaits_idle: false,
                already_ended: false,
            }
        );
        assert!(
            project(
                &mut held,
                "session.compaction_complete",
                compaction_completed()
            )
            .is_empty(),
            "its late end is owed nothing"
        );
        assert!(!held.is_turn_running(), "and begins no Continuation");
    }

    #[test]
    fn a_compaction_the_cancel_found_over_while_the_loop_works_stands_ahead_of_the_aborts_idle() {
        let mut working = in_turn();
        project(
            &mut working,
            "session.compaction_start",
            compaction_started(),
        );
        let scope = working.begin_interrupt();
        found_nothing_to_cancel(&mut working, scope);
        assert!(working.finish_interrupt(scope, false).abort);
        assert_eq!(
            project(
                &mut working,
                "session.compaction_complete",
                compaction_completed()
            ),
            [compacted()],
            "the working loop's abort is what settles its Turn"
        );
        assert_eq!(
            project(&mut working, "session.idle", json!({ "aborted": true })),
            [ProviderEvent::TurnInterrupted]
        );

        let mut idled = in_turn();
        project(&mut idled, "session.compaction_start", compaction_started());
        let scope = idled.begin_interrupt();
        found_nothing_to_cancel(&mut idled, scope);
        assert!(idled.finish_interrupt(scope, false).abort);
        assert!(
            project(&mut idled, "session.idle", json!({})).is_empty(),
            "a loop going idle on its own before the abort reached it is held on the end"
        );
        assert_eq!(
            project(
                &mut idled,
                "session.compaction_complete",
                compaction_completed()
            ),
            [compacted(), ProviderEvent::TurnCompleted],
            "which settles its Turn as that idle said"
        );
    }

    #[test]
    fn a_held_turns_subagents_abort_settles_it_after_a_compaction_the_cancel_found_over() {
        for abort_idle_comes in [true, false] {
            let mut correlation = with_subagent();
            project(
                &mut correlation,
                "session.compaction_start",
                compaction_started(),
            );
            project(&mut correlation, "session.idle", json!({}));
            let scope = correlation.begin_interrupt();
            found_nothing_to_cancel(&mut correlation, scope);
            assert!(correlation.finish_interrupt(scope, false).awaits_idle);
            correlation.expect_abort_idle();
            if abort_idle_comes {
                assert_eq!(
                    project(
                        &mut correlation,
                        "session.compaction_complete",
                        compaction_completed()
                    ),
                    [compacted()],
                    "the end ahead of the abort's idle settles nothing: that idle is the Turn's"
                );
                assert_eq!(
                    project(&mut correlation, "session.idle", json!({ "aborted": true })),
                    [ProviderEvent::TurnInterrupted]
                );
            } else {
                assert!(
                    correlation.abort_idle_missing(scope).is_empty(),
                    "the end the timeline carried is left to settle the Turn"
                );
                assert_eq!(
                    project(
                        &mut correlation,
                        "session.compaction_complete",
                        compaction_completed()
                    ),
                    [compacted(), ProviderEvent::TurnInterrupted],
                    "which it does as the interrupt stopped it"
                );
            }
        }
    }

    #[test]
    fn a_second_interrupt_leaves_the_turn_to_the_end_of_a_compaction_found_over() {
        let mut held = held_by_a_compaction();
        let scope = held.begin_interrupt();
        found_nothing_to_cancel(&mut held, scope);
        assert!(held.finish_interrupt(scope, false).already_ended);

        let again = held.begin_interrupt();
        assert!(!again.compacting, "nothing is left to cancel");
        assert!(held.cancel_answered(again, false).is_none());
        assert_eq!(
            held.finish_interrupt(again, false),
            InterruptRemainder {
                abort: false,
                settled: Vec::new(),
                awaits_idle: false,
                already_ended: true,
            }
        );
        assert_eq!(
            project(
                &mut held,
                "session.compaction_complete",
                compaction_completed()
            ),
            [compacted(), ProviderEvent::TurnCompleted]
        );
    }

    /// A correlation whose held Turn an interrupt settled itself once the abort its Subagent
    /// needed ended in no idle within the wait, and whose next Turn is running.
    fn owing_an_aborted_idle() -> CopilotCorrelation {
        let mut correlation = with_subagent();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        project(&mut correlation, "session.idle", json!({}));
        let scope = correlation.begin_interrupt();
        assert!(finished(&mut correlation, scope, true).awaits_idle);
        correlation.expect_abort_idle();
        assert_eq!(correlation.abort_idle_missing(scope), interrupted(true));
        correlation
            .begin_turn()
            .expect("the next Prompt begins its Turn");
        correlation
    }

    #[test]
    fn an_aborted_idle_an_earlier_abort_owes_settles_nothing_of_the_turn_after_it() {
        let mut correlation = owing_an_aborted_idle();
        assert!(
            project(&mut correlation, "session.idle", json!({ "aborted": true })).is_empty(),
            "the late idle belongs to the Turn the interrupt settled"
        );
        assert!(correlation.is_turn_running());
        assert_eq!(
            project(&mut correlation, "session.idle", json!({ "aborted": true })),
            [ProviderEvent::TurnInterrupted],
            "only one is owed"
        );
    }

    #[test]
    fn a_turns_own_idle_is_never_taken_for_the_one_an_earlier_abort_owes() {
        let mut correlation = owing_an_aborted_idle();
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted],
            "an idle no abort ended in is the Turn's own"
        );

        let mut correlation = owing_an_aborted_idle();
        correlation.begin_interrupt();
        assert_eq!(
            project(&mut correlation, "session.idle", json!({ "aborted": true })),
            [ProviderEvent::TurnInterrupted],
            "an abort sent since owes its own idle, which settles the Turn it stops"
        );
    }

    #[test]
    fn a_compaction_copilot_failed_to_cancel_is_followed_and_may_be_cancelled_again() {
        let mut correlation = held_by_a_compaction();
        correlation.begin_interrupt();
        assert!(correlation.cancel_failed().is_empty());
        let retried = correlation.begin_interrupt();
        assert!(
            retried.compacting,
            "the next interrupt cancels the compaction still running"
        );
        assert!(correlation.cancel_failed().is_empty());
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted(), ProviderEvent::TurnCompleted],
            "its end settles the Turn it held, as its loop's idle said"
        );
    }

    #[test]
    fn a_compaction_that_ended_while_a_failed_cancel_was_out_releases_the_turn_it_held() {
        let mut correlation = held_by_a_compaction();
        correlation.begin_interrupt();
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted()]
        );
        assert_eq!(
            correlation.cancel_failed(),
            [attributed(None, ProviderEvent::TurnCompleted)],
            "the interrupt that would have settled the Turn failed, so its idle does"
        );
    }

    #[test]
    fn a_compaction_starting_after_one_suru_stopped_following_is_recorded_again() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        correlation.begin_interrupt();
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_start",
                compaction_started(),
            ),
            [ProviderEvent::CompactionStarted],
            "a new start is read as the stopped one's end never coming"
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted()]
        );
    }

    #[test]
    fn a_steer_into_a_turn_a_compaction_holds_hands_the_turn_back_to_the_loop() {
        let mut correlation = held_by_a_compaction();
        assert_eq!(correlation.steer_delivering(), Some(false));
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted()],
            "the compaction ending before the steered loop speaks settles nothing"
        );
        assert!(
            !project(
                &mut correlation,
                "assistant.message",
                json!({ "messageId": "m-steered", "content": "Steered." }),
            )
            .is_empty(),
            "the steered loop answers in the Turn"
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted]
        );
    }

    #[test]
    fn a_steer_that_never_reached_copilot_leaves_the_turn_to_its_compaction() {
        let mut correlation = held_by_a_compaction();
        let idled = correlation.steer_delivering();
        correlation.steer_undelivered(idled);
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            ),
            [compacted(), ProviderEvent::TurnCompleted]
        );
    }

    #[test]
    fn a_compaction_in_a_continuation_late_output_began_makes_it_copilots() {
        let mut correlation = with_settled_subagent();
        project(&mut correlation, "session.idle", json!({}));
        project(
            &mut correlation,
            "assistant.message_start",
            json!({ "messageId": "m-late" }),
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_start",
                compaction_started(),
            ),
            [
                ProviderEvent::ContinuationStarted {
                    selection: selection()
                },
                ProviderEvent::CompactionStarted
            ],
            "Copilot owns the Continuation it compacts in, so a Prompt stops the compaction"
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_start",
                compaction_started(),
            ),
            [ProviderEvent::CompactionStarted],
            "and says so once"
        );
    }

    #[test]
    fn a_continuation_only_a_compaction_held_owes_the_turn_after_it_no_idle() {
        let mut correlation = after_a_turn();
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );

        correlation.begin_turn().expect("a Prompt begins its Turn");
        assert!(
            project(
                &mut correlation,
                "session.compaction_complete",
                compaction_completed(),
            )
            .is_empty(),
            "the compaction settled with the Continuation the Prompt cut off"
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted],
            "no loop ran in the Continuation, so the Prompt's own idle is the Prompt's"
        );
    }

    #[test]
    fn a_subagent_compaction_its_stretch_settled_without_ends_nowhere_once_it_resumes() {
        let mut correlation = with_subagent();
        project_attributed(
            &mut correlation,
            agent_event("agent-1", "session.compaction_start", compaction_started()),
        );
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "subagent.completed",
                json!({ "toolCallId": "t-spawn", "agentName": "researcher", "agentDisplayName": "Researcher" }),
            ),
        );
        project_attributed(
            &mut correlation,
            delivered_message("agent-1", "idle", None, "Run it again."),
        );
        assert!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "session.compaction_complete",
                    compaction_completed()
                ),
            )
            .is_empty(),
            "the end belongs to the stretch that settled, not the one resuming the Subagent"
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                agent_event("agent-1", "session.compaction_start", compaction_started()),
            ),
            [AttributedProviderEvent {
                attribution: subagent("agent-1"),
                event: ProviderEvent::CompactionStarted,
            }],
            "the resumed stretch's own compaction stands in it"
        );
    }

    #[test]
    fn a_subagent_compaction_starting_while_it_works_no_stretch_wakes_it_until_the_end() {
        let mut correlation = with_settled_subagent();
        assert_eq!(
            project_attributed(
                &mut correlation,
                agent_event("agent-1", "session.compaction_start", compaction_started()),
            ),
            [
                attributed(
                    None,
                    ProviderEvent::SubagentWoken {
                        subagent_id: ProviderSubagentId::new("agent-1")
                    }
                ),
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: ProviderEvent::CompactionStarted,
                }
            ],
            "the compaction is the Subagent's own work, in a Continuation of its Session"
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "session.compaction_complete",
                    compaction_completed()
                ),
            ),
            [
                AttributedProviderEvent {
                    attribution: subagent("agent-1"),
                    event: compacted(),
                },
                settled("agent-1", ProviderSubagentStatus::Completed)
            ],
            "no loop runs in it, so the compaction ending settles it"
        );
    }

    #[test]
    fn a_resume_while_a_compaction_holds_the_subagent_settles_that_continuation_first() {
        let mut correlation = with_settled_subagent();
        project_attributed(
            &mut correlation,
            agent_event("agent-1", "session.compaction_start", compaction_started()),
        );
        let resumed = project_attributed(
            &mut correlation,
            delivered_message("agent-1", "idle", None, "Run it again."),
        );
        assert_eq!(
            resumed[0],
            settled("agent-1", ProviderSubagentStatus::Completed),
            "the resume settles the Continuation the compaction began before it begins its own"
        );
        assert!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "session.compaction_complete",
                    compaction_completed()
                ),
            )
            .is_empty(),
            "the compaction outlasted its Continuation, so its end records nothing in the resume"
        );
    }

    #[test]
    fn a_subagents_compaction_lands_in_its_own_session_and_an_unknown_ones_nowhere() {
        let mut correlation = with_subagent();
        assert_eq!(
            project_attributed(
                &mut correlation,
                agent_event("agent-1", "session.compaction_start", compaction_started()),
            ),
            [AttributedProviderEvent {
                attribution: subagent("agent-1"),
                event: ProviderEvent::CompactionStarted,
            }]
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "session.compaction_complete",
                    compaction_completed()
                ),
            ),
            [AttributedProviderEvent {
                attribution: subagent("agent-1"),
                event: compacted(),
            }]
        );
        assert!(
            project_attributed(
                &mut correlation,
                agent_event("agent-9", "session.compaction_start", compaction_started()),
            )
            .is_empty()
        );
        assert_eq!(
            project(&mut correlation, "session.idle", json!({})),
            [ProviderEvent::TurnCompleted],
            "a Subagent's compaction holds nothing of the main conversation's open"
        );
    }

    /// A correlation whose Session sits idle after a Turn, then asked to compact on request in the
    /// Turn it answers.
    fn compacting_on_request() -> (CopilotCorrelation, TurnId) {
        compacting_on_request_after(after_a_turn())
    }

    fn compacting_on_request_after(
        mut correlation: CopilotCorrelation,
    ) -> (CopilotCorrelation, TurnId) {
        let turn = TurnId::new();
        correlation
            .begin_manual_compaction(turn)
            .expect("an idle Session takes a manual compaction");
        correlation.context_prompt_ready(turn, selection());
        (correlation, turn)
    }

    fn answered(
        correlation: &mut CopilotCorrelation,
        turn: TurnId,
        answer: ManualCompactionAnswer,
    ) -> Vec<ProviderEvent> {
        correlation
            .project_manual_compaction_answer(turn, answer)
            .into_iter()
            .map(|attributed| attributed.event)
            .collect()
    }

    fn overdue(correlation: &mut CopilotCorrelation, turn: TurnId) -> Vec<ProviderEvent> {
        correlation
            .project_manual_compaction_overdue(turn)
            .into_iter()
            .map(|attributed| attributed.event)
            .collect()
    }

    #[test]
    fn a_requested_compactions_call_states_its_lapse_and_adds_no_usage() {
        let (mut correlation, _turn) = compacting_on_request();
        let projected = project_attributed(
            &mut correlation,
            event(
                "assistant.usage",
                json!({
                    "apiCallId": "summary", "model": "claude-fixture", "inputTokens": 9_000,
                    "cacheExpiresAt": "2026-01-01T00:05:00Z",
                }),
            ),
        );
        assert_eq!(
            projected
                .iter()
                .map(|attributed| &attributed.event)
                .collect::<Vec<_>>(),
            [&ProviderEvent::PromptCacheLapseReported {
                lapses_at: crate::protocol::SessionTimestamp(1_767_225_900_000),
                model: Some(ModelId::new("claude-fixture")),
                lifetime: None,
                prompt_limit_tokens: None,
            }],
            "the call's Usage is the Compaction's to report, once"
        );
    }

    fn manual_started() -> serde_json::Value {
        json!({ "trigger": "manual", "currentTokens": 182_000 })
    }

    fn manual_completed() -> serde_json::Value {
        json!({
            "success": true,
            "trigger": "manual",
            "preCompactionTokens": 182_000,
            "postCompactionTokens": 31_000,
            "summaryContent": "<overview>Half done.</overview>",
            "compactionTokensUsed": {
                "inputTokens": 1_500,
                "cacheReadTokens": 500,
                "cacheWriteTokens": 100,
                "outputTokens": 40,
                "model": "claude-fixture",
            },
        })
    }

    /// The compaction `manual_completed` ends, with the counts and summary it reports.
    fn manual_compacted() -> ProviderEvent {
        ProviderEvent::CompactionCompleted {
            before_tokens: Some(182_000),
            after_tokens: Some(31_000),
            summary: Some("<overview>Half done.</overview>".to_owned()),
        }
    }

    /// The counts of the one model call `manual_completed` reports, as a Usage reports them.
    fn manual_usage(event: &ProviderEvent) -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
        let ProviderEvent::Usage { usage, .. } = event else {
            panic!("a Usage is reported: {event:?}");
        };
        (
            usage.fresh_input_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
            usage.output_tokens,
        )
    }

    #[test]
    fn a_manual_compaction_is_reported_in_its_own_turn_and_settled_by_copilots_answer() {
        let (mut correlation, turn) = compacting_on_request();
        assert!(correlation.is_compacting_on_request());
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_start",
                manual_started()
            ),
            [ProviderEvent::CompactionStarted],
            "no Continuation begins: the Turn Suru opened holds it"
        );
        assert!(
            project(&mut correlation, "session.idle", json!({})).is_empty(),
            "no loop runs a manual compaction, so no idle settles its Turn"
        );
        assert!(
            project(
                &mut correlation,
                "assistant.usage",
                json!({ "model": "claude-fixture", "inputTokens": 9, "outputTokens": 9 })
            )
            .is_empty(),
            "what the summarising call spent is told once, by the compaction's end"
        );
        let ended = project(
            &mut correlation,
            "session.compaction_complete",
            manual_completed(),
        );
        assert_eq!(ended[0], manual_compacted());
        assert_eq!(
            manual_usage(&ended[1]),
            (Some(900), Some(500), Some(100), Some(40))
        );
        assert_eq!(
            answered(
                &mut correlation,
                turn,
                ManualCompactionAnswer::Compacted { summary: None }
            ),
            [ProviderEvent::TurnCompleted]
        );
        assert!(!correlation.is_turn_running());
    }

    #[test]
    fn an_answer_ahead_of_the_compactions_reports_waits_for_them() {
        let (mut correlation, turn) = compacting_on_request();
        assert!(
            answered(
                &mut correlation,
                turn,
                ManualCompactionAnswer::Compacted { summary: None }
            )
            .is_empty(),
            "a compaction Copilot made has reports still to come"
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_start",
                manual_started()
            ),
            [ProviderEvent::CompactionStarted],
            "in the Turn it was asked in, not a Continuation"
        );
        let ended = project(
            &mut correlation,
            "session.compaction_complete",
            manual_completed(),
        );
        assert_eq!(ended.len(), 3, "{ended:?}");
        assert_eq!(ended[0], manual_compacted(), "its counts land on it");
        assert_eq!(
            manual_usage(&ended[1]),
            (Some(900), Some(500), Some(100), Some(40)),
            "its Usage lands on its Turn"
        );
        assert_eq!(ended[2], ProviderEvent::TurnCompleted);
        assert!(!correlation.is_turn_running());
        assert!(
            overdue(&mut correlation, turn).is_empty(),
            "a Turn that settled is owed no overdue settle"
        );
        assert!(
            correlation.begin_manual_compaction(TurnId::new()).is_ok(),
            "nothing is owed of a compaction whose reports are in"
        );
    }
    #[test]
    fn only_a_request_copilot_rejected_outright_settles_ahead_of_its_reports() {
        let failure = |rejected| ManualCompactionAnswer::NotCompacted {
            error: Some("No messages to compact".to_owned()),
            rejected,
        };
        let refused = ProviderEvent::TurnFailed {
            message: "No messages to compact".to_owned(),
        };
        let (mut correlation, turn) = compacting_on_request();
        assert_eq!(
            answered(&mut correlation, turn, failure(true)),
            std::slice::from_ref(&refused),
            "Copilot rejected the request before it could compact anything"
        );
        assert!(
            correlation.begin_manual_compaction(TurnId::new()).is_ok(),
            "and owes no report of it"
        );

        // Any other failure may have reports still on their way.
        let (mut correlation, turn) = compacting_on_request();
        assert!(answered(&mut correlation, turn, failure(false)).is_empty());
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_start",
                manual_started()
            ),
            [ProviderEvent::CompactionStarted]
        );
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_complete",
                json!({ "success": false, "trigger": "manual", "error": "Too long to summarise" }),
            ),
            [
                ProviderEvent::CompactionFailed {
                    error: Some("Too long to summarise".to_owned()),
                },
                ProviderEvent::TurnFailed {
                    message: "Too long to summarise".to_owned(),
                },
            ],
            "the reports record the Compaction, which says why the Turn failed"
        );

        // Reports that never come leave the Turn to settle on the answer once overdue, owing
        // nothing of a compaction Copilot never reported and says it did not make.
        let (mut correlation, turn) = compacting_on_request();
        assert!(answered(&mut correlation, turn, failure(false)).is_empty());
        assert_eq!(overdue(&mut correlation, turn), [refused]);
        assert!(correlation.begin_manual_compaction(TurnId::new()).is_ok());
    }
    #[test]
    fn an_answered_compaction_whose_end_never_comes_settles_on_the_answer_once_overdue() {
        let (mut correlation, turn) = compacting_on_request();
        project(
            &mut correlation,
            "session.compaction_start",
            manual_started(),
        );
        assert!(
            answered(
                &mut correlation,
                turn,
                ManualCompactionAnswer::Compacted {
                    summary: Some("<overview>Half done.</overview>".to_owned()),
                }
            )
            .is_empty(),
            "the compaction's end is still to come"
        );
        assert!(
            overdue(&mut correlation, TurnId::new()).is_empty(),
            "another compaction's wait settles nothing of this one"
        );
        assert_eq!(
            overdue(&mut correlation, turn),
            [
                ProviderEvent::CompactionCompleted {
                    before_tokens: None,
                    after_tokens: None,
                    summary: Some("<overview>Half done.</overview>".to_owned()),
                },
                ProviderEvent::TurnCompleted,
            ],
            "Copilot answered that it compacted"
        );
        assert!(
            project(
                &mut correlation,
                "session.compaction_complete",
                manual_completed()
            )
            .is_empty(),
            "the end of a manual compaction whose Turn settled records nothing"
        );
        assert!(!correlation.is_turn_running(), "and begins no Continuation");
        assert!(
            correlation.begin_manual_compaction(TurnId::new()).is_ok(),
            "its reports are in, so nothing is owed"
        );
        correlation.abandon_turn();
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_start",
                compaction_started()
            ),
            [
                ProviderEvent::ContinuationStarted {
                    selection: selection()
                },
                ProviderEvent::CompactionStarted,
            ],
            "a compaction Copilot begins in the background after is its own"
        );
    }

    /// A correlation whose Turn an interrupt settled while Copilot compacted in its background,
    /// cancelling a compaction whose end has yet to come.
    fn after_a_cancelled_compaction() -> CopilotCorrelation {
        let mut correlation = in_turn();
        correlation.context_prompt_ready(TurnId::new(), selection());
        project(
            &mut correlation,
            "session.compaction_start",
            compaction_started(),
        );
        project(&mut correlation, "session.idle", json!({}));
        let scope = correlation.begin_interrupt();
        assert!(scope.compacting);
        let remainder = finished(&mut correlation, scope, true);
        assert!(!remainder.abort, "{remainder:?}");
        assert!(!correlation.is_turn_running());
        correlation
    }

    #[test]
    fn a_cancelled_background_compactions_end_never_reaches_the_manual_compaction_after_it() {
        let stale = || {
            json!({
                "success": false,
                "trigger": "threshold",
                "error": "Compaction cancelled",
                "compactionTokensUsed": { "inputTokens": 9_000, "outputTokens": 10 },
            })
        };
        for stale_before_the_start in [true, false] {
            let (mut correlation, turn) =
                compacting_on_request_after(after_a_cancelled_compaction());
            if stale_before_the_start {
                assert!(
                    project(&mut correlation, "session.compaction_complete", stale()).is_empty(),
                    "the cancelled compaction's end is owed nothing"
                );
            }
            assert_eq!(
                project(
                    &mut correlation,
                    "session.compaction_start",
                    manual_started()
                ),
                [ProviderEvent::CompactionStarted]
            );
            if !stale_before_the_start {
                assert!(
                    project(&mut correlation, "session.compaction_complete", stale()).is_empty(),
                    "the cancelled compaction's end settles nothing of the manual one"
                );
            }
            let ended = project(
                &mut correlation,
                "session.compaction_complete",
                manual_completed(),
            );
            assert_eq!(
                ended[0],
                manual_compacted(),
                "the manual compaction ends as it did"
            );
            assert_eq!(
                manual_usage(&ended[1]),
                (Some(900), Some(500), Some(100), Some(40)),
                "with its own Usage alone"
            );
            assert_eq!(
                answered(
                    &mut correlation,
                    turn,
                    ManualCompactionAnswer::Compacted { summary: None }
                ),
                [ProviderEvent::TurnCompleted]
            );
        }
    }

    #[test]
    fn reports_naming_no_trigger_decide_nothing_of_a_manual_compaction() {
        // The cancelled compaction's end, naming no trigger, after the manual one began.
        let (mut correlation, turn) = compacting_on_request_after(after_a_cancelled_compaction());
        assert_eq!(
            project(
                &mut correlation,
                "session.compaction_start",
                manual_started()
            ),
            [ProviderEvent::CompactionStarted]
        );
        assert!(
            project(
                &mut correlation,
                "session.compaction_complete",
                json!({
                    "success": false,
                    "error": "Compaction cancelled",
                    "compactionTokensUsed": { "inputTokens": 9_000, "outputTokens": 10 },
                }),
            )
            .is_empty(),
            "it could be either compaction's end, so it ends neither"
        );
        let ended = project(
            &mut correlation,
            "session.compaction_complete",
            manual_completed(),
        );
        assert_eq!(ended[0], manual_compacted());
        assert_eq!(
            manual_usage(&ended[1]),
            (Some(900), Some(500), Some(100), Some(40))
        );
        assert_eq!(
            answered(
                &mut correlation,
                turn,
                ManualCompactionAnswer::Compacted { summary: None }
            ),
            [ProviderEvent::TurnCompleted]
        );

        // With nothing owed, a report naming no trigger still decides nothing.
        let (mut correlation, _) = compacting_on_request();
        for (report, data) in [
            ("session.compaction_start", json!({})),
            ("session.compaction_complete", compaction_completed()),
        ] {
            assert!(project(&mut correlation, report, data).is_empty());
        }
        assert!(correlation.is_compacting_on_request());
    }
    #[test]
    fn a_manual_compactions_turn_settles_as_copilot_answered_and_as_suru_aborted_it() {
        let failure = "Compaction failed: the model returned an empty summary";
        for (aborted, ending, answer, expected) in [
            // Copilot's own failure, in its words.
            (
                None,
                Some(json!({ "success": false, "trigger": "manual", "error": failure })),
                ManualCompactionAnswer::NotCompacted {
                    error: Some("RPC said otherwise".to_owned()),
                    rejected: false,
                },
                ProviderEvent::TurnFailed {
                    message: failure.to_owned(),
                },
            ),
            // Copilot rejecting the request before it compacted anything.
            (
                None,
                None,
                ManualCompactionAnswer::NotCompacted {
                    error: Some("No messages to compact".to_owned()),
                    rejected: true,
                },
                ProviderEvent::TurnFailed {
                    message: "No messages to compact".to_owned(),
                },
            ),
            (
                None,
                None,
                ManualCompactionAnswer::NotCompacted {
                    error: None,
                    rejected: true,
                },
                ProviderEvent::TurnFailed {
                    message: NOT_COMPACTED.to_owned(),
                },
            ),
            // The cancellation Suru's abort asked for.
            (
                Some(true),
                Some(
                    json!({ "success": false, "trigger": "manual", "error": "Compaction Cancelled" }),
                ),
                ManualCompactionAnswer::NotCompacted {
                    error: Some("Compaction Cancelled".to_owned()),
                    rejected: false,
                },
                ProviderEvent::TurnInterrupted,
            ),
            // An abort that found nothing to abort leaves the compaction's own end standing.
            (
                Some(false),
                Some(json!({ "success": false, "trigger": "manual", "error": failure })),
                ManualCompactionAnswer::NotCompacted {
                    error: None,
                    rejected: false,
                },
                ProviderEvent::TurnFailed {
                    message: failure.to_owned(),
                },
            ),
            (
                Some(false),
                Some(manual_completed()),
                ManualCompactionAnswer::Compacted { summary: None },
                ProviderEvent::TurnCompleted,
            ),
        ] {
            let (mut correlation, turn) = compacting_on_request();
            if let Some(ending) = ending {
                project(
                    &mut correlation,
                    "session.compaction_start",
                    manual_started(),
                );
                if let Some(aborted) = aborted {
                    correlation.manual_compaction_aborted(aborted);
                }
                project(&mut correlation, "session.compaction_complete", ending);
            } else if let Some(aborted) = aborted {
                correlation.manual_compaction_aborted(aborted);
            }
            assert_eq!(answered(&mut correlation, turn, answer), [expected]);
            assert!(!correlation.is_turn_running());
        }
    }

    #[test]
    fn no_manual_compaction_begins_while_copilot_owes_an_earlier_ones_reports() {
        let (mut correlation, turn) = compacting_on_request();
        assert!(
            answered(
                &mut correlation,
                turn,
                ManualCompactionAnswer::Compacted { summary: None }
            )
            .is_empty()
        );
        assert_eq!(
            overdue(&mut correlation, turn),
            [
                ProviderEvent::CompactionCompleted {
                    before_tokens: None,
                    after_tokens: None,
                    summary: None,
                },
                ProviderEvent::TurnCompleted,
            ]
        );
        for _ in 0..2 {
            assert!(
                correlation.begin_manual_compaction(TurnId::new()).is_err(),
                "its reports, under the manual trigger, would be taken for the next one's"
            );
        }
        assert!(!correlation.is_turn_running(), "nothing begins");

        // A stretch of the loop running to its idle has every report Copilot wrote before its
        // answer ahead of it, so none is owed any longer.
        correlation.begin_turn().expect("open the Turn");
        project(&mut correlation, "session.idle", json!({}));
        assert!(correlation.begin_manual_compaction(TurnId::new()).is_ok());
    }

    #[test]
    fn a_manual_compaction_is_refused_while_a_turn_runs() {
        let mut correlation = in_turn();
        assert!(correlation.begin_manual_compaction(TurnId::new()).is_err());
        assert!(
            !correlation.is_compacting_on_request(),
            "the Turn running is left as it was"
        );
    }

    /// A `session.usage_info` report reaching the timeline drain, which reads it at receipt.
    fn usage_info(
        correlation: &mut CopilotCorrelation,
        agent: Option<&str>,
        tokens: u64,
        limit: u64,
    ) {
        let report = json!({ "currentTokens": tokens, "tokenLimit": limit });
        let received = match agent {
            Some(agent) => agent_event(agent, "session.usage_info", report),
            None => event("session.usage_info", report),
        };
        correlation.context_report(&received);
    }

    #[test]
    fn the_served_window_is_the_main_conversations_latest_token_limit() {
        let mut correlation = with_subagent();
        correlation.context_prompt_ready(TurnId::new(), selection());
        assert_eq!(correlation.served_window(), None, "nothing reported yet");

        usage_info(&mut correlation, None, 12_400, 200_000);
        usage_info(&mut correlation, Some("agent-1"), 2_000, 64_000);
        usage_info(&mut correlation, None, 13_000, 0);

        assert_eq!(
            correlation.served_window(),
            Some(200_000),
            "a Subagent's window and a zero limit leave the main conversation's as it was"
        );
    }

    #[test]
    fn a_turn_under_another_selection_forgets_the_served_window_until_copilot_reports_one() {
        let mut correlation = after_a_turn();
        usage_info(&mut correlation, None, 12_400, 200_000);

        correlation.begin_turn().expect("open the second Turn");
        correlation.context_prompt_ready(TurnId::new(), selection());
        assert_eq!(
            correlation.served_window(),
            Some(200_000),
            "the same Selection keeps its window"
        );

        project(&mut correlation, "session.idle", json!({}));
        correlation.begin_turn().expect("open the third Turn");
        correlation.context_prompt_ready(
            TurnId::new(),
            AgentSelection {
                model: crate::protocol::ModelId::new("gpt-fixture"),
                ..selection()
            },
        );
        assert_eq!(correlation.served_window(), None);
    }

    /// The attribution a Route's phase puts on what it stages, and on what its commit re-sends.
    fn fusion(route: &str, phase: &str, commit: bool) -> serde_json::Value {
        let mut attribution = json!({
            "fusionId": route,
            "syntheticModel": "hydrafusion",
            "policy": "standard",
            "pattern": "cascade",
            "phaseId": format!("{route}:{phase}"),
            "phaseKind": phase,
            "role": "solver",
        });
        if commit {
            attribution["commitId"] = json!(format!("{route}:commit"));
        }
        attribution
    }

    fn staged_message(id: &str, content: &str, fusion: serde_json::Value) -> serde_json::Value {
        json!({ "messageId": id, "content": content, "fusion": fusion })
    }

    fn staged_start(
        id: &str,
        tool: &str,
        args: serde_json::Value,
        fusion: serde_json::Value,
    ) -> serde_json::Value {
        json!({ "toolCallId": id, "toolName": tool, "arguments": args, "fusion": fusion })
    }

    fn staged_complete(id: &str, result: &str, fusion: serde_json::Value) -> serde_json::Value {
        json!({
            "toolCallId": id,
            "success": true,
            "result": { "content": result },
            "fusion": fusion,
        })
    }

    fn commit_of(route: &str, phase: &str) -> serde_json::Value {
        json!({
            "fusionId": route,
            "commitId": format!("{route}:commit"),
            "sourcePhaseId": format!("{route}:{phase}"),
            "sourceModel": "gpt-fixture",
            "kind": "text",
            "toolCallId": null,
        })
    }

    fn phase_started(route: &str, phase: &str) -> serde_json::Value {
        json!({
            "fusionId": route,
            "phaseId": format!("{route}:{phase}"),
            "phaseKind": phase,
            "role": "solver",
            "conversationScope": "root",
            "pattern": "cascade",
            "model": "gpt-fixture",
        })
    }

    fn resolved(route: &str, plan: &[&str]) -> serde_json::Value {
        json!({
            "contractVersion": 1,
            "fusionId": route,
            "turnId": format!("{route}:turn"),
            "syntheticModel": "hydrafusion",
            "policy": "standard",
            "pattern": if plan.len() > 1 { "cascade" } else { "single" },
            "phasePlan": plan.iter().map(|kind| json!({
                "kind": kind, "role": "solver", "scope": "root", "conditional": false,
            })).collect::<Vec<_>>(),
            "primaryModel": "gpt-primary",
            "judgeModel": "gpt-judge",
            "repairModel": "claude-repair",
            "fallbackModel": "gpt-fallback",
            "followUpModel": "gpt-primary",
        })
    }

    /// A Turn whose Route has begun a phase, which is what there is to discard.
    fn in_route(route: &str) -> CopilotCorrelation {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.fusion_resolved",
            resolved(route, &["primary"]),
        );
        project(
            &mut correlation,
            "assistant.fusion_phase_started",
            phase_started(route, "primary"),
        );
        correlation
    }

    #[test]
    fn auto_fusion_attributes_the_staged_model_and_preserves_supersession() {
        let mut correlation = in_turn();
        correlation.selection.model = ModelId::new(AUTO_MODEL_ID);
        project(
            &mut correlation,
            "session.fusion_resolved",
            resolved("r", &["primary", "repair"]),
        );
        let started = project(
            &mut correlation,
            "assistant.fusion_phase_started",
            phase_started("r", "primary"),
        );
        assert!(started.iter().any(|event| matches!(
            event,
            ProviderEvent::CurrentModelChanged { model: Some(model) }
                if model.actual.as_str() == "gpt-fixture"
        )));

        let mut attributed = fusion("r", "primary", false);
        attributed["sourceModel"] = json!("gpt-solver");
        let mut message = staged_message("draft", "First attempt", attributed);
        message["model"] = json!(" ");
        let staged = project(&mut correlation, "assistant.message", message);
        assert!(staged.iter().any(|event| matches!(
            event,
            ProviderEvent::AgentMessageModelKnown { model, .. }
                if model.selected.as_str() == "auto" && model.actual.as_str() == "gpt-solver"
        )));
        let committed = project(
            &mut correlation,
            "session.fusion_commit_started",
            commit_of("r", "repair"),
        );
        assert_eq!(
            superseded_messages(&committed),
            [ProviderMessageId::new("draft")]
        );
    }

    fn superseded_messages(events: &[ProviderEvent]) -> Vec<ProviderMessageId> {
        events
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::AgentMessageSuperseded { message } => Some(message.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_committed_resend_of_a_settled_command_adds_nothing() {
        let mut correlation = in_route("r");
        let staged = fusion("r", "primary", false);
        let committed = fusion("r", "primary", true);
        let args = json!({ "command": "ls" });

        assert!(matches!(
            project(
                &mut correlation,
                "tool.execution_start",
                staged_start("t1", "bash", args.clone(), staged.clone())
            )[..],
            [ProviderEvent::CommandStarted { .. }]
        ));
        let output = project(
            &mut correlation,
            "tool.execution_complete",
            staged_complete("t1", "a.txt\n", staged),
        );
        assert!(
            matches!(
                &output[..],
                [
                    ProviderEvent::CommandOutputDelta { content, .. },
                    ProviderEvent::CommandCompleted { .. },
                ] if content == "a.txt\n"
            ),
            "a Command that streamed nothing takes its whole result, once: {output:?}"
        );

        project(
            &mut correlation,
            "session.fusion_commit_started",
            commit_of("r", "primary"),
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_start",
                staged_start("t1", "bash", args, committed.clone())
            ),
            [],
            "the re-send opens no second row"
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_complete",
                staged_complete("t1", "a.txt\n", committed)
            ),
            [],
            "and appends no second copy of its output"
        );
    }

    #[test]
    fn a_committed_resend_of_a_settled_tool_call_or_file_change_adds_nothing() {
        for (tool, args) in [
            ("web_search", json!({ "query": "suru" })),
            (
                "edit",
                json!({ "path": "src/main.rs", "old_str": "a", "new_str": "b" }),
            ),
        ] {
            let mut correlation = in_route("r");
            let staged = fusion("r", "primary", false);
            let committed = fusion("r", "primary", true);

            assert_eq!(
                project(
                    &mut correlation,
                    "tool.execution_start",
                    staged_start("t1", tool, args.clone(), staged.clone())
                )
                .len(),
                1,
                "{tool} opens its row"
            );
            assert!(
                !project(
                    &mut correlation,
                    "tool.execution_complete",
                    staged_complete("t1", "done", staged)
                )
                .is_empty()
            );
            project(
                &mut correlation,
                "session.fusion_commit_started",
                commit_of("r", "primary"),
            );
            assert_eq!(
                project(
                    &mut correlation,
                    "tool.execution_start",
                    staged_start("t1", tool, args, committed.clone())
                ),
                []
            );
            assert_eq!(
                project(
                    &mut correlation,
                    "tool.execution_complete",
                    staged_complete("t1", "done", committed)
                ),
                []
            );
        }
    }

    /// Shell output streams once and has no durable copy. A Tool still running when its phase ends
    /// settles on its committed twin, carrying only what the stream had not reached.
    #[test]
    fn a_staged_tool_still_running_at_its_commit_settles_on_its_twin_without_doubling_its_output() {
        let mut correlation = in_route("r");
        let staged = fusion("r", "primary", false);
        let committed = fusion("r", "primary", true);
        let args = json!({ "command": "ls" });
        project(
            &mut correlation,
            "tool.execution_start",
            staged_start("t1", "bash", args.clone(), staged.clone()),
        );
        project(
            &mut correlation,
            "tool.shell_output",
            json!({ "toolCallId": "t1", "stream": "stdout", "text": "a\n", "sequence": 0, "fusion": staged }),
        );

        project(
            &mut correlation,
            "session.fusion_commit_started",
            commit_of("r", "primary"),
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_start",
                staged_start("t1", "bash", args, committed.clone())
            ),
            []
        );
        let settled = project(
            &mut correlation,
            "tool.execution_complete",
            staged_complete("t1", "a\nb\n", committed),
        );

        assert!(
            matches!(
                &settled[..],
                [
                    ProviderEvent::CommandOutputDelta { content, .. },
                    ProviderEvent::CommandCompleted { .. },
                ] if content == "b\n"
            ),
            "{settled:?}"
        );
    }

    /// Nothing will ever report a Tool its losing phase left running, so it is left for the store to
    /// settle with the Turn, as any Activity a Turn leaves open is (ADR 0039).
    #[test]
    fn a_tool_a_rejected_phase_left_running_is_left_to_the_turns_settle() {
        let mut correlation = in_route("r");
        let staged = fusion("r", "primary", false);
        project(
            &mut correlation,
            "tool.execution_start",
            staged_start("t1", "bash", json!({ "command": "ls" }), staged),
        );

        let committed = project(
            &mut correlation,
            "session.fusion_commit_started",
            commit_of("r", "repair"),
        );
        assert_eq!(committed, []);
        assert_eq!(
            project(&mut correlation, "assistant.idle", json!({})),
            [ProviderEvent::TurnCompleted]
        );
    }

    #[test]
    fn whichever_copy_comes_first_stands_and_the_other_adds_nothing() {
        let mut correlation = in_route("r");
        let staged = fusion("r", "primary", false);
        let committed = fusion("r", "primary", true);
        let args = json!({ "command": "ls" });

        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                staged_message("m1", "Done", committed.clone())
            )
            .len(),
            4
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                staged_message("m1", "Done", staged.clone())
            ),
            []
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_start",
                staged_start("t1", "bash", args.clone(), committed.clone())
            )
            .len(),
            1
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_complete",
                staged_complete("t1", "x", committed)
            )
            .len(),
            2
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_start",
                staged_start("t1", "bash", args, staged.clone())
            ),
            []
        );
        assert_eq!(
            project(
                &mut correlation,
                "tool.execution_complete",
                staged_complete("t1", "x", staged)
            ),
            []
        );
    }

    /// Messages arrive whole here, and both copies share one `messageId`, so no copy leaves a Message
    /// open for the next to trip on.
    #[test]
    fn a_routes_messages_arrive_whole_and_leave_none_open() {
        let mut correlation = in_route("r");
        let staged = fusion("r", "primary", false);

        let first = project(
            &mut correlation,
            "assistant.message",
            staged_message("m1", "One", staged.clone()),
        );
        assert_eq!(
            first,
            [
                ProviderEvent::AgentMessageStarted,
                ProviderEvent::AgentMessageIdentified {
                    message: ProviderMessageId::new("m1")
                },
                ProviderEvent::AgentMessageDelta {
                    content: "One".to_owned()
                },
                ProviderEvent::AgentMessageCompleted,
            ]
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message_start",
                json!({ "messageId": "m1" })
            ),
            [],
            "a start for a Message a Route already carried opens nothing"
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                staged_message("m2", "Two", staged)
            )
            .len(),
            4
        );
    }

    #[test]
    fn a_message_a_route_streamed_after_all_is_completed_rather_than_repeated() {
        let mut correlation = in_route("r");
        project(
            &mut correlation,
            "assistant.message_start",
            json!({ "messageId": "m1" }),
        );
        project(
            &mut correlation,
            "assistant.message_delta",
            json!({ "messageId": "m1", "deltaContent": "Do" }),
        );

        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                staged_message("m1", "Done", fusion("r", "primary", false))
            ),
            [
                ProviderEvent::AgentMessageIdentified {
                    message: ProviderMessageId::new("m1"),
                },
                ProviderEvent::AgentMessageDelta {
                    content: "ne".to_owned()
                },
                ProviderEvent::AgentMessageCompleted,
            ],
            "named while it is still open, which is what naming it reads"
        );
    }

    #[test]
    fn reasoning_inside_a_routes_message_stands_once_and_an_empty_message_adds_nothing_else() {
        let mut correlation = in_route("r");
        let mut with_reasoning = staged_message("m1", "", fusion("r", "primary", false));
        with_reasoning["reasoningText"] = json!("**Thinking**\n\nAbout it");
        let mut committed = staged_message("m1", "", fusion("r", "primary", true));
        committed["reasoningText"] = json!("**Thinking**\n\nAbout it");

        let shown = project(&mut correlation, "assistant.message", with_reasoning);
        assert!(matches!(
            shown.first(),
            Some(ProviderEvent::ReasoningStarted { .. })
        ));
        assert!(matches!(
            shown.last(),
            Some(ProviderEvent::ReasoningCompleted { .. })
        ));
        assert!(
            !shown
                .iter()
                .any(|event| matches!(event, ProviderEvent::AgentMessageStarted)),
            "{shown:?}"
        );
        assert_eq!(
            project(&mut correlation, "assistant.message", committed),
            []
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.message",
                staged_message("m2", "", fusion("r", "primary", false))
            ),
            [],
            "a Model call with nothing to say and no reasoning adds nothing"
        );
    }

    /// Reasoning a Turn with no Route reports on its own events stays the only copy of it.
    #[test]
    fn a_message_with_no_route_keeps_ignoring_the_reasoning_inside_it() {
        let mut correlation = in_turn();

        let shown = project(
            &mut correlation,
            "assistant.message",
            json!({ "messageId": "m1", "content": "Done", "reasoningText": "Thinking" }),
        );

        assert_eq!(
            shown,
            [
                ProviderEvent::AgentMessageStarted,
                ProviderEvent::AgentMessageDelta {
                    content: "Done".to_owned()
                },
                ProviderEvent::AgentMessageCompleted,
            ]
        );
    }

    /// Superseding is only ever within its own Route: a second Route's commit reaches nothing of the
    /// first's, whose `turnId`s it may well share.
    #[test]
    fn a_commit_supersedes_only_the_other_phases_of_its_own_route() {
        let mut correlation = in_route("first");
        project(
            &mut correlation,
            "assistant.message",
            staged_message("a", "Attempt", fusion("first", "primary", false)),
        );
        project(
            &mut correlation,
            "assistant.message",
            staged_message("empty", "", fusion("first", "primary", false)),
        );
        project(
            &mut correlation,
            "assistant.message",
            staged_message("b", "Repair", fusion("first", "repair", false)),
        );
        let first = project(
            &mut correlation,
            "session.fusion_commit_started",
            commit_of("first", "repair"),
        );
        assert_eq!(
            superseded_messages(&first),
            [ProviderMessageId::new("a")],
            "the winner stands, and a Message that never showed is nothing to mark"
        );

        project(
            &mut correlation,
            "session.fusion_resolved",
            resolved("second", &["primary"]),
        );
        project(
            &mut correlation,
            "assistant.message",
            staged_message("c", "Second", fusion("second", "primary", false)),
        );
        let second = project(
            &mut correlation,
            "session.fusion_commit_started",
            commit_of("second", "primary"),
        );

        assert_eq!(superseded_messages(&second), []);
    }

    #[test]
    fn a_commit_naming_the_first_phase_supersedes_nothing_of_it() {
        let mut correlation = in_route("r");
        project(
            &mut correlation,
            "assistant.message",
            staged_message("a", "Answer", fusion("r", "phase:0", false)),
        );

        let committed = project(
            &mut correlation,
            "session.fusion_commit_started",
            commit_of("r", "phase:0"),
        );

        assert_eq!(superseded_messages(&committed), []);
    }

    /// A Subagent's work is its own Transcript's. A Route that staged it dedupes its copies, and
    /// leaves its Messages unmarked.
    #[test]
    fn a_subagents_routed_work_is_shown_once_and_never_superseded() {
        let mut correlation = in_route("r");
        project_attributed(&mut correlation, spawn_started("agent-1", "spawn-1"));
        let staged = fusion("r", "primary", false);

        let shown = project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message",
                staged_message("m1", "Sub", staged.clone()),
            ),
        );
        assert_eq!(shown.len(), 4);
        assert!(
            shown
                .iter()
                .all(|event| event.attribution == subagent("agent-1"))
        );
        assert_eq!(
            project_attributed(
                &mut correlation,
                agent_event(
                    "agent-1",
                    "assistant.message",
                    staged_message("m1", "Sub", fusion("r", "primary", true))
                ),
            ),
            []
        );

        let committed = project(
            &mut correlation,
            "session.fusion_commit_started",
            commit_of("r", "repair"),
        );
        assert_eq!(superseded_messages(&committed), []);
    }

    #[test]
    fn a_route_reported_with_no_turn_and_nothing_owed_lands_nowhere() {
        let mut correlation = CopilotCorrelation::new();
        for (kind, data) in [
            (
                "session.fusion_route_started",
                json!({
                    "attemptId": "r:route", "turnKind": "user", "syntheticModel": "hydrafusion", "policy": "standard",
                }),
            ),
            ("session.fusion_resolved", resolved("r", &["primary"])),
            ("session.fusion_commit_started", commit_of("r", "primary")),
        ] {
            assert_eq!(project(&mut correlation, kind, data), [], "{kind}");
        }
        assert!(!correlation.is_route_running());
        assert!(!correlation.is_turn_running());
    }

    #[test]
    fn a_route_begun_while_a_continuation_is_owed_opens_it() {
        let mut correlation = CopilotCorrelation::new();
        correlation.late_settle_owes_continuation = true;

        let shown = project(
            &mut correlation,
            "session.fusion_resolved",
            resolved("r", &["primary"]),
        );

        assert_eq!(
            shown,
            [ProviderEvent::RouteDecided {
                decision: RouteDecision::Chosen {
                    steps: vec![RouteStep {
                        model: crate::protocol::ModelId::new("gpt-primary"),
                        action: RouteAction::Works,
                    }],
                },
            }]
        );
        assert!(correlation.is_turn_running());
    }

    #[test]
    fn a_route_is_running_from_its_routing_until_its_commit_or_its_turns_settle() {
        let mut correlation = in_turn();
        assert!(!correlation.is_route_running(), "a Turn alone is no Route");

        project(
            &mut correlation,
            "session.fusion_route_started",
            json!({
                "attemptId": "r:route", "turnKind": "user", "syntheticModel": "hydrafusion", "policy": "standard",
            }),
        );
        assert!(correlation.is_route_running(), "routing is running already");
        project(
            &mut correlation,
            "session.fusion_resolved",
            resolved("r", &["primary"]),
        );
        assert!(correlation.is_route_running());
        project(
            &mut correlation,
            "session.fusion_commit_started",
            commit_of("r", "primary"),
        );
        assert!(
            !correlation.is_route_running(),
            "its commit names the winner, in milliseconds"
        );

        project(
            &mut correlation,
            "session.fusion_resolved",
            resolved("again", &["primary"]),
        );
        assert!(
            correlation.is_route_running(),
            "a second Route runs in the same Turn"
        );
        project(&mut correlation, "assistant.idle", json!({}));
        assert!(!correlation.is_route_running());
    }

    fn discarded() -> ProviderEvent {
        ProviderEvent::RouteDecided {
            decision: RouteDecision::Discarded {
                provider: crate::protocol::ProviderId::new(COPILOT_PROVIDER_ID),
            },
        }
    }

    #[test]
    fn an_interrupted_route_says_its_conversation_is_gone_and_a_committed_one_does_not() {
        let mut interrupted = in_route("r");
        assert_eq!(
            project(
                &mut interrupted,
                "assistant.idle",
                json!({ "aborted": true })
            ),
            [discarded(), ProviderEvent::TurnInterrupted]
        );

        let mut committed = in_route("r");
        project(
            &mut committed,
            "session.fusion_commit_started",
            commit_of("r", "primary"),
        );
        assert_eq!(
            project(&mut committed, "assistant.idle", json!({ "aborted": true })),
            [ProviderEvent::TurnInterrupted]
        );
    }

    /// Stopped while it was still choosing, a Route has done nothing to lose.
    #[test]
    fn a_route_interrupted_before_any_phase_says_nothing_of_a_conversation() {
        let mut correlation = in_turn();
        project(
            &mut correlation,
            "session.fusion_route_started",
            json!({
                "attemptId": "r:route", "turnKind": "user", "syntheticModel": "hydrafusion", "policy": "standard",
            }),
        );
        project(
            &mut correlation,
            "session.fusion_resolved",
            resolved("r", &["primary"]),
        );

        assert_eq!(
            project(
                &mut correlation,
                "assistant.idle",
                json!({ "aborted": true })
            ),
            [ProviderEvent::TurnInterrupted]
        );
    }

    #[test]
    fn a_route_a_failure_ended_says_its_conversation_is_gone() {
        let mut correlation = in_route("r");
        project(
            &mut correlation,
            "session.error",
            json!({
                "errorType": "query", "message": "Execution failed",
            }),
        );

        let settled = project(&mut correlation, "assistant.idle", json!({}));

        assert_eq!(settled[0], discarded());
        assert!(matches!(settled[1], ProviderEvent::TurnFailed { .. }));
    }

    /// A Route ending unexplained leaves its rows as they stand.
    #[test]
    fn a_route_that_idles_with_no_commit_or_interruption_marks_nothing() {
        let mut correlation = in_route("r");
        project(
            &mut correlation,
            "assistant.message",
            staged_message("a", "Answer", fusion("r", "primary", false)),
        );

        assert_eq!(
            project(&mut correlation, "assistant.idle", json!({})),
            [ProviderEvent::TurnCompleted]
        );
    }

    #[test]
    fn a_crash_takes_a_staging_route_with_it_and_says_so_once() {
        let mut correlation = in_route("r");

        let lost = correlation.project_routes_lost();
        assert_eq!(
            lost.iter()
                .map(|lost| lost.event.clone())
                .collect::<Vec<_>>(),
            [discarded()]
        );
        assert_eq!(correlation.project_routes_lost(), []);
    }

    #[test]
    fn a_fallback_runs_live_so_nothing_of_it_is_discarded() {
        let mut correlation = in_turn();
        let shown = project(
            &mut correlation,
            "session.fusion_route_failed",
            json!({
                "attemptId": "r:route",
                "fusionId": "r",
                "syntheticModel": "hydrafusion",
                "policy": "standard",
                "reason": "invalid_response",
                "errorMessage": "no such model",
                "fallbackModel": "gpt-fallback",
            }),
        );
        assert_eq!(
            shown,
            [ProviderEvent::RouteDecided {
                decision: RouteDecision::Fallback {
                    failed: None,
                    stand_in: Some(crate::protocol::ModelId::new("gpt-fallback")),
                    reason: Some("no such model".to_owned()),
                },
            }]
        );
        project(
            &mut correlation,
            "assistant.fusion_phase_started",
            phase_started("r", "primary"),
        );
        project(
            &mut correlation,
            "assistant.message_start",
            json!({ "messageId": "m1" }),
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.idle",
                json!({ "aborted": true })
            ),
            [
                ProviderEvent::AgentMessageCompleted,
                ProviderEvent::TurnInterrupted
            ]
        );
    }

    /// A phase failing on its own says what failed, and the degraded completion that follows it
    /// repeats nothing. A completion is left to say so only where nothing else has.
    #[test]
    fn a_phase_failing_on_its_own_stands_as_a_fallback_and_a_degraded_completion_adds_nothing() {
        let mut correlation = in_route("r");
        let failed = project(
            &mut correlation,
            "assistant.fusion_phase_failed",
            json!({
                "fusionId": "r",
                "phaseId": "r:judge",
                "phaseKind": "judge",
                "role": "judge",
                "conversationScope": "review",
                "model": "gpt-judge",
                "status": "failed",
                "reason": "review_unavailable",
                "durationMs": 1.0,
                "usage": { "requestCount": 0, "inputTokens": 0, "outputTokens": 0, "cachedTokens": 0, "totalNanoAiu": 0 },
            }),
        );
        assert_eq!(
            failed,
            [ProviderEvent::RouteDecided {
                decision: RouteDecision::Fallback {
                    failed: Some(crate::protocol::ModelId::new("gpt-judge")),
                    stand_in: None,
                    reason: Some("review_unavailable".to_owned()),
                },
            }]
        );
        let degraded = |route: &str| {
            json!({
                "fusionId": route,
                "commitId": format!("{route}:commit"),
                "turnId": format!("{route}:turn"),
                "syntheticModel": "hydrafusion",
                "pattern": "cascade",
                "outcome": "degraded",
                "finalSourceModel": "gpt-primary",
                "followUpModel": "gpt-primary",
                "degradedReason": "review_unavailable",
                "phaseCount": 2,
                "requestCount": 1,
                "inputTokens": 1,
                "outputTokens": 1,
                "cachedTokens": 0,
                "totalNanoAiu": 1,
                "durationMs": 1.0,
            })
        };
        assert_eq!(
            project(&mut correlation, "session.fusion_completed", degraded("r")),
            []
        );

        let mut correlation = in_route("quiet");
        assert_eq!(
            project(
                &mut correlation,
                "session.fusion_completed",
                degraded("quiet")
            ),
            [ProviderEvent::RouteDecided {
                decision: RouteDecision::Fallback {
                    failed: None,
                    stand_in: Some(crate::protocol::ModelId::new("gpt-primary")),
                    reason: Some("review_unavailable".to_owned()),
                },
            }]
        );
    }

    #[test]
    fn a_review_it_cannot_read_still_stands_with_no_account_of_itself() {
        let mut correlation = in_route("r");
        let shown = project(
            &mut correlation,
            "assistant.fusion_phase_completed",
            json!({
                "fusionId": "r",
                "phaseId": "r:critic",
                "phaseKind": "critic",
                "role": "critic",
                "conversationScope": "review",
                "model": "gpt-critic",
                "status": "succeeded",
                "content": "not json",
                "durationMs": 1.0,
                "verdict": null,
                "usage": { "requestCount": 0, "inputTokens": 0, "outputTokens": 0, "cachedTokens": 0, "totalNanoAiu": 0 },
            }),
        );
        assert_eq!(
            shown,
            [ProviderEvent::RouteDecided {
                decision: RouteDecision::Verdict {
                    reviewer: crate::protocol::ModelId::new("gpt-critic"),
                    verdict: RouteVerdict::Unread,
                    detail: None,
                },
            }]
        );
    }

    /// A Route's fan of activity reports, and a solver's finishing, say nothing in the Transcript.
    #[test]
    fn a_routes_progress_reports_add_nothing() {
        let mut correlation = in_route("r");
        assert_eq!(
            project(
                &mut correlation,
                "assistant.fusion_phase_activity",
                json!({
                    "activity": "model_output", "fusionId": "r", "phaseId": "r:primary", "phaseKind": "primary",
                    "conversationScope": "root", "pattern": "cascade", "model": "gpt", "role": "solver",
                })
            ),
            []
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.turn_start",
                json!({ "turnId": "0" })
            ),
            []
        );
        assert_eq!(
            project(
                &mut correlation,
                "assistant.turn_end",
                json!({ "turnId": "0" })
            ),
            []
        );
    }

    /// How many rows `events` say a conversation was discarded in.
    fn discards(events: &[AttributedProviderEvent]) -> usize {
        events
            .iter()
            .filter(|projected| {
                matches!(
                    &projected.event,
                    ProviderEvent::RouteDecided {
                        decision: RouteDecision::Discarded { .. }
                    }
                )
            })
            .count()
    }

    /// A Subagent's stages are its own Transcript's. An interrupt of its owner discards the Route
    /// the owner ran, once, however many a Subagent's events named.
    #[test]
    fn an_interrupted_owner_says_nothing_of_a_route_only_its_subagent_staged() {
        let mut correlation = in_route("owner");
        project_attributed(&mut correlation, spawn_started("agent-1", "spawn-1"));
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message",
                staged_message("m1", "Sub", fusion("sub", "primary", false)),
            ),
        );
        let settled = project_attributed(
            &mut correlation,
            event("assistant.idle", json!({ "aborted": true })),
        );
        assert_eq!(discards(&settled), 1, "{settled:?}");

        let mut correlation = in_turn();
        project_attributed(&mut correlation, spawn_started("agent-1", "spawn-1"));
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "tool.execution_start",
                staged_start(
                    "t1",
                    "bash",
                    json!({ "command": "ls" }),
                    fusion("sub", "primary", false),
                ),
            ),
        );
        let settled = project_attributed(
            &mut correlation,
            event("assistant.idle", json!({ "aborted": true })),
        );
        assert_eq!(discards(&settled), 0, "the owner ran no Route: {settled:?}");
    }

    /// A Subagent working past its owner's settle runs nothing of the owner's, so the owner's next
    /// Turn has no Route for its close to stop.
    #[test]
    fn a_subagent_staging_past_its_owners_settle_leaves_the_next_turn_no_route_to_stop() {
        let mut correlation = in_turn();
        project_attributed(&mut correlation, spawn_started("agent-1", "spawn-1"));
        project_attributed(&mut correlation, event("assistant.idle", json!({})));
        assert!(!correlation.is_turn_running());
        project_attributed(
            &mut correlation,
            agent_event(
                "agent-1",
                "assistant.message",
                staged_message("m1", "Sub", fusion("sub", "primary", false)),
            ),
        );

        correlation.begin_turn().expect("open the next Turn");

        assert!(!correlation.is_route_running());
    }
}
