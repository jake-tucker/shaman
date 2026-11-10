//! The line above the composer of a Session whose Prompt Cache has lapsed:
//! what it says, in what colour, and when it stands. Every frame is drawn at
//! a moment fixed on the Session clock, which a test moves by hand.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use ratatui::buffer::Buffer;
use suru::{
    managed_client::{ManagedEvent, SessionEvent},
    protocol::{
        AsideSettings, AsideVisibility, EffectiveSettings, LapsePenalty, PromptCache, PromptId,
        SessionChange, SessionContentWidth, SessionId, SessionRevision, SessionSettings,
        SessionSnapshot, SessionStatus, SessionTimestamp, SessionUpdate, SettingsSnapshot,
        SidebarSettings, SidebarVisibility,
    },
    tui::{Application, ApplicationEvent, ApplicationTransition, CommandId},
};

use crate::tui_rendering::support::{
    PANDA, WorkspaceDir, buffer_rows, connected_application, failed_session_snapshot,
    rendered_application_buffer, text_position, type_terminal_text, workspace_dir,
};

const WIDTH: u16 = 160;
const HEIGHT: u16 = 20;

/// When every frame here is drawn, unless a test moves the clock on.
const NOW: SessionTimestamp = SessionTimestamp(1_800_000_000_000);

const PRICED: &str =
    "Prompt cache lapsed · next prompt costs ~$1.65 more · /compact first to shrink it";
const UNPRICED: &str =
    "Prompt cache lapsed · next prompt re-reads the whole context · /compact first to shrink it";

/// An idle Session whose Prompt Cache lapses at `lapses_at`, at
/// `lapse_penalty`.
fn session(
    workspace: &WorkspaceDir,
    lapses_at: SessionTimestamp,
    lapse_penalty: Option<f64>,
) -> SessionSnapshot {
    let mut snapshot = failed_session_snapshot(
        SessionId::new(),
        PromptId::new(),
        "Read the whole repository",
        workspace.path(),
    );
    snapshot.session.prompt_cache = Some(PromptCache {
        lapses_at,
        lapse_penalty: lapse_penalty
            .map(|usd| LapsePenalty::from_usd(usd).expect("a representable penalty")),
    });
    snapshot
}

/// A client with `snapshot` open, its Session clock reading `clock`.
fn open(workspace: &WorkspaceDir, snapshot: SessionSnapshot, clock: Arc<AtomicU64>) -> Application {
    let mut application = connected_application(workspace.path())
        .with_session_clock(move || SessionTimestamp(clock.load(Ordering::SeqCst)));
    application
        .handle_event(ApplicationEvent::SessionAttached(snapshot))
        .expect("open the Session");
    application
}

fn clock_at(now: SessionTimestamp) -> Arc<AtomicU64> {
    Arc::new(AtomicU64::new(now.0))
}

fn drawn(application: &Application) -> Buffer {
    rendered_application_buffer(application, WIDTH, HEIGHT)
}

fn screen(application: &Application) -> String {
    buffer_rows(&drawn(application)).join("\n")
}

/// What the line above the composer says, its rows read back as one: a row
/// breaks only between its parts, so they are joined as they were parted.
fn lapsed_line(application: &Application) -> Option<String> {
    let rows = lapsed_rows(application);
    (!rows.is_empty()).then(|| rows.join(" · "))
}

/// The rows the line above the composer stands on, each trimmed.
fn lapsed_rows(application: &Application) -> Vec<String> {
    let rows = buffer_rows(&drawn(application));
    let Some(first) = rows.iter().position(|row| row.contains("Prompt cache")) else {
        return Vec::new();
    };
    let last = rows[first..]
        .iter()
        .position(|row| row.contains("/compact first to shrink it"))
        .map_or(first, |offset| first + offset);
    rows[first..=last]
        .iter()
        .map(|row| row.trim().to_owned())
        .collect()
}

/// The row the composer's input stands on.
fn composer_row(buffer: &Buffer) -> u16 {
    text_position(buffer, "Type a prompt").1
}

#[test]
fn a_cold_session_states_its_lapse_penalty_in_the_warning_colour_just_above_the_composer() {
    let workspace = workspace_dir();
    let snapshot = session(&workspace, SessionTimestamp(NOW.0 - 1), Some(1.65));
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(lapsed_line(&application).as_deref(), Some(PRICED));
    assert_eq!(
        lapsed_rows(&application),
        [
            "Prompt cache lapsed · next prompt costs ~$1.65 more",
            "/compact first to shrink it",
        ],
        "a column too narrow for the whole line breaks it between its parts, cutting none"
    );
    let buffer = drawn(&application);
    let (column, row) = text_position(&buffer, "/compact first to shrink it");
    assert_eq!(
        composer_row(&buffer).checked_sub(row),
        Some(2),
        "the line stands directly above the composer's border:\n{}",
        buffer_rows(&buffer).join("\n")
    );
    assert_eq!(
        buffer.cell((column, row)).map(|cell| cell.fg),
        Some(PANDA.warning),
        "it is drawn in the warning colour"
    );
    let (column, row) = text_position(&buffer, "Prompt cache lapsed");
    assert_eq!(
        buffer.cell((column, row)).map(|cell| cell.fg),
        Some(PANDA.warning)
    );
}

#[test]
fn a_column_wide_enough_holds_the_whole_line_on_one_row() {
    let workspace = workspace_dir();
    let snapshot = session(&workspace, SessionTimestamp(NOW.0 - 1), Some(1.65));
    let mut application = open(&workspace, snapshot, clock_at(NOW));
    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::SettingsSnapshot(
            SettingsSnapshot {
                settings: EffectiveSettings {
                    session: SessionSettings {
                        content_width: SessionContentWidth::Fill,
                    },
                    // Neither column stands beside the Session's to narrow it.
                    sidebar: SidebarSettings {
                        initial_visibility: SidebarVisibility::Hidden,
                        ..SidebarSettings::default()
                    },
                    aside: AsideSettings {
                        initial_visibility: AsideVisibility::Hidden,
                        ..AsideSettings::default()
                    },
                    ..EffectiveSettings::default()
                },
                pinned: [("session.contentWidth".to_owned(), std::path::PathBuf::new())].into(),
                diagnostics: Vec::new(),
            },
        )))
        .expect("fill the terminal's width");

    assert_eq!(
        lapsed_rows(&application),
        [PRICED],
        "{}",
        screen(&application)
    );
}

#[test]
fn a_cold_session_whose_penalty_is_unknown_says_the_whole_context_is_read_again() {
    let workspace = workspace_dir();
    let snapshot = session(&workspace, NOW, None);
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(lapsed_line(&application).as_deref(), Some(UNPRICED));
}

#[test]
fn a_known_penalty_under_five_cents_draws_no_line_and_five_cents_does() {
    let workspace = workspace_dir();
    let lapsed = SessionTimestamp(NOW.0 - 60_000);
    let small = open(
        &workspace,
        session(&workspace, lapsed, Some(0.0499)),
        clock_at(NOW),
    );
    assert_eq!(lapsed_line(&small), None, "{}", screen(&small));

    let enough = open(
        &workspace,
        session(&workspace, lapsed, Some(0.05)),
        clock_at(NOW),
    );
    assert_eq!(
        lapsed_line(&enough).as_deref(),
        Some("Prompt cache lapsed · next prompt costs ~$0.05 more · /compact first to shrink it"),
    );
}

#[test]
fn a_warm_session_draws_nothing_until_its_clock_passes_the_lapse() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 1);
    let clock = clock_at(NOW);
    let application = open(
        &workspace,
        session(&workspace, lapses_at, Some(1.65)),
        clock.clone(),
    );
    assert_eq!(lapsed_line(&application), None, "{}", screen(&application));

    clock.store(lapses_at.0, Ordering::SeqCst);
    assert_eq!(lapsed_line(&application).as_deref(), Some(PRICED));
}

#[test]
fn a_working_session_draws_no_line_however_long_ago_its_cache_lapsed() {
    let workspace = workspace_dir();
    let mut snapshot = session(&workspace, SessionTimestamp(NOW.0 - 3_600_000), Some(1.65));
    snapshot.session.status = SessionStatus::Active;
    snapshot.session.working_since = Some(SessionTimestamp(NOW.0 - 1_000));
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(lapsed_line(&application), None, "{}", screen(&application));
}

#[test]
fn the_line_outlasts_interaction_and_never_holds_back_a_prompt() {
    let workspace = workspace_dir();
    let snapshot = session(&workspace, NOW, Some(1.65));
    let mut application = open(&workspace, snapshot, clock_at(NOW));

    type_terminal_text(&mut application, "One more thing");
    assert_eq!(
        lapsed_line(&application).as_deref(),
        Some(PRICED),
        "typing dismisses nothing, since the line is no Notice"
    );
    let transition = application
        .handle_event(ApplicationEvent::Command(CommandId::SubmitSteer))
        .expect("send the Prompt");
    assert!(
        matches!(transition, ApplicationTransition::AdmitPrompt { .. }),
        "the Prompt is sent at once, unconfirmed: {transition:?}"
    );
}

#[test]
fn the_line_clears_once_the_next_turn_has_written_the_cache_afresh() {
    let workspace = workspace_dir();
    let snapshot = session(&workspace, SessionTimestamp(NOW.0 - 1), Some(1.65));
    let session_id = snapshot.session.id;
    let revision = snapshot.revision;
    let mut application = open(&workspace, snapshot, clock_at(NOW));
    assert_eq!(lapsed_line(&application).as_deref(), Some(PRICED));

    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id,
                revision: SessionRevision(revision.0 + 1),
                changes: vec![SessionChange::PromptCacheChanged {
                    prompt_cache: Some(PromptCache {
                        lapses_at: SessionTimestamp(NOW.0 + 300_000),
                        lapse_penalty: LapsePenalty::from_usd(1.70),
                    }),
                }],
            },
        )))
        .expect("the next Turn's lapse arrives");
    assert_eq!(lapsed_line(&application), None, "{}", screen(&application));
}

#[test]
fn a_subagents_view_draws_no_line_for_its_own_lapse() {
    let workspace = workspace_dir();
    let mut snapshot = session(&workspace, SessionTimestamp(NOW.0 - 1), Some(1.65));
    snapshot.session.parent = Some(SessionId::new());
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(lapsed_line(&application), None, "{}", screen(&application));
}

#[test]
fn an_idle_client_wakes_to_draw_the_line_at_the_moment_the_cache_lapses_and_not_after() {
    let workspace = workspace_dir();
    let clock = clock_at(NOW);
    let lapses_at = SessionTimestamp(NOW.0 + 45_000);
    let application = open(
        &workspace,
        session(&workspace, lapses_at, Some(1.65)),
        clock.clone(),
    );
    assert_eq!(
        application.prompt_cache_lapse_wakeup(),
        Some(std::time::Duration::from_secs(45)),
        "the client wakes once, when the line comes to stand"
    );

    clock.store(lapses_at.0, Ordering::SeqCst);
    assert_eq!(
        application.prompt_cache_lapse_wakeup(),
        None,
        "a lapse already drawn wants no further wakeup"
    );

    let mut subagent = session(&workspace, lapses_at, Some(1.65));
    subagent.session.parent = Some(SessionId::new());
    let viewing_a_subagent = open(&workspace, subagent, clock_at(NOW));
    assert_eq!(viewing_a_subagent.prompt_cache_lapse_wakeup(), None);
}
