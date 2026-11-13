//! The line above the composer of a Session whose Prompt Cache is about to
//! lapse, or has: what it says, in what colour, and when it stands. Every
//! frame is drawn at a moment fixed on the Session clock, which a test moves
//! by hand.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use ratatui::buffer::Buffer;
use shaman::{
    managed_client::{ManagedEvent, SessionEvent},
    protocol::{
        AsideSettings, AsideVisibility, EffectiveSettings, LapsePenalty, ModelId, PromptCache,
        PromptId, SessionChange, SessionContentWidth, SessionId, SessionRevision, SessionSettings,
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

const PRICED: &str = "Prompt cache lapsed · next prompt costs ~$1.65 more";
const UNPRICED: &str = "Prompt cache lapsed · next prompt re-reads the whole context";

/// An idle Session whose Prompt Cache lapses at `lapses_at`, at
/// `lapse_penalty`, its Cache Lifetime unknown.
fn session(
    workspace: &WorkspaceDir,
    lapses_at: SessionTimestamp,
    lapse_penalty: Option<f64>,
) -> SessionSnapshot {
    session_kept_for(workspace, lapses_at, lapse_penalty, None)
}

/// An idle Session whose Prompt Cache, kept for `lifetime_ms`, lapses at
/// `lapses_at`, at `lapse_penalty`.
fn session_kept_for(
    workspace: &WorkspaceDir,
    lapses_at: SessionTimestamp,
    lapse_penalty: Option<f64>,
    lifetime_ms: Option<u64>,
) -> SessionSnapshot {
    let mut snapshot = failed_session_snapshot(
        SessionId::new(),
        PromptId::new(),
        "Read the whole repository",
        workspace.path(),
    );
    snapshot.session.prompt_cache = Some(PromptCache {
        lapses_at,
        written_in: None,
        written_by: ModelId::new("gpt-fixture"),
        lifetime_ms,
        write_price: shaman::protocol::CacheWritePrice::Default,
        prompt_limit_tokens: None,
        lapse_penalty: lapse_penalty
            .map(|usd| LapsePenalty::from_usd(usd).expect("a representable penalty")),
        compaction_due_at: None,
    });
    snapshot
}

/// A client with `snapshot` open, its Session clock reading `clock`.
fn open(workspace: &WorkspaceDir, snapshot: SessionSnapshot, clock: Arc<AtomicU64>) -> Application {
    let mut application = connected_application(workspace.path())
        .with_session_clock(move || SessionTimestamp(clock.load(Ordering::SeqCst)));
    application
        .handle_event(ApplicationEvent::SessionAttached(snapshot))
        .expect("open the session");
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
fn composer_top_line(application: &Application) -> Option<String> {
    let rows = composer_top_rows(application);
    (!rows.is_empty()).then(|| rows.join(" · "))
}

/// How long the client would sleep before the line above the composer next
/// changes, once a frame has laid it out: a line no frame has drawn wakes
/// nothing.
fn wakeup_after_a_frame(application: &Application) -> Option<std::time::Duration> {
    drawn(application);
    application.composer_top_wakeup()
}

/// The rows the line above the composer stands on, each trimmed: from the
/// first that speaks of the Prompt Cache, or of a Compaction pending, to the
/// last above the composer's border, the first row below that the border
/// draws across the line's column.
fn composer_top_rows(application: &Application) -> Vec<String> {
    let buffer = drawn(application);
    let rows = buffer_rows(&buffer);
    let Some((first, opening)) = rows.iter().enumerate().find_map(|(index, row)| {
        ["Prompt cache", "Compacting in"]
            .into_iter()
            .find(|opening| row.contains(opening))
            .map(|opening| (index, opening))
    }) else {
        return Vec::new();
    };
    let (column, _) = text_position(&buffer, opening);
    let border_at = |row: usize| {
        let row = u16::try_from(row).expect("a terminal row");
        [column, column + 1].into_iter().all(|column| {
            buffer
                .cell((column, row))
                .is_some_and(|cell| matches!(cell.symbol(), "─" | "╭" | "┌"))
        })
    };
    let border = (first + 1..rows.len())
        .find(|row| border_at(*row))
        .unwrap_or(rows.len());
    rows[first..border]
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

    assert_eq!(composer_top_line(&application).as_deref(), Some(PRICED));
    let buffer = drawn(&application);
    let (column, row) = text_position(&buffer, "next prompt costs ~$1.65 more");
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
    // The longest line there is, which the default column breaks in two.
    let snapshot = written_under(
        &workspace,
        claude("opus", "medium"),
        claude("haiku", "medium"),
        SessionTimestamp(NOW.0 + 240_000),
        None,
    );
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
        composer_top_rows(&application),
        [selection_changed(None)],
        "{}",
        screen(&application)
    );
}

#[test]
fn a_cold_session_whose_penalty_is_unknown_says_the_whole_context_is_read_again() {
    let workspace = workspace_dir();
    let snapshot = session(&workspace, NOW, None);
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(composer_top_line(&application).as_deref(), Some(UNPRICED));
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
    assert_eq!(composer_top_line(&small), None, "{}", screen(&small));

    let enough = open(
        &workspace,
        session(&workspace, lapsed, Some(0.05)),
        clock_at(NOW),
    );
    assert_eq!(
        composer_top_line(&enough).as_deref(),
        Some("Prompt cache lapsed · next prompt costs ~$0.05 more"),
    );
}

#[test]
fn a_warm_session_draws_nothing_before_its_final_stretch_and_the_cold_line_at_its_lapse() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 60_001);
    let clock = clock_at(NOW);
    let application = open(
        &workspace,
        session(&workspace, lapses_at, Some(1.65)),
        clock.clone(),
    );
    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );

    clock.store(lapses_at.0, Ordering::SeqCst);
    assert_eq!(composer_top_line(&application).as_deref(), Some(PRICED));
}

#[test]
fn a_working_session_draws_no_line_however_long_ago_its_cache_lapsed() {
    let workspace = workspace_dir();
    let mut snapshot = session(&workspace, SessionTimestamp(NOW.0 - 3_600_000), Some(1.65));
    snapshot.session.status = SessionStatus::Active;
    snapshot.session.working_since = Some(SessionTimestamp(NOW.0 - 1_000));
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
}

#[test]
fn the_line_outlasts_interaction_and_never_holds_back_a_prompt() {
    let workspace = workspace_dir();
    let snapshot = session(&workspace, NOW, Some(1.65));
    let mut application = open(&workspace, snapshot, clock_at(NOW));

    type_terminal_text(&mut application, "One more thing");
    assert_eq!(
        composer_top_line(&application).as_deref(),
        Some(PRICED),
        "typing dismisses nothing, since the line is no notice"
    );
    let transition = application
        .handle_event(ApplicationEvent::Command(CommandId::SubmitSteer))
        .expect("send the prompt");
    assert!(
        matches!(transition, ApplicationTransition::AdmitPrompt { .. }),
        "the prompt is sent at once, unconfirmed: {transition:?}"
    );
}

#[test]
fn the_line_clears_once_the_next_turn_has_written_the_cache_afresh() {
    let workspace = workspace_dir();
    let snapshot = session(&workspace, SessionTimestamp(NOW.0 - 1), Some(1.65));
    let session_id = snapshot.session.id;
    let revision = snapshot.revision;
    let mut application = open(&workspace, snapshot, clock_at(NOW));
    assert_eq!(composer_top_line(&application).as_deref(), Some(PRICED));

    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id,
                revision: SessionRevision(revision.0 + 1),
                changes: vec![SessionChange::PromptCacheChanged {
                    prompt_cache: Some(PromptCache {
                        lapses_at: SessionTimestamp(NOW.0 + 300_000),
                        written_in: None,
                        written_by: ModelId::new("gpt-fixture"),
                        lifetime_ms: None,
                        write_price: shaman::protocol::CacheWritePrice::Default,
                        prompt_limit_tokens: None,
                        lapse_penalty: LapsePenalty::from_usd(1.70),
                        compaction_due_at: None,
                    }),
                }],
            },
        )))
        .expect("the next turn's lapse arrives");
    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
}

#[test]
fn a_subagents_view_draws_no_line_for_its_own_lapse() {
    let workspace = workspace_dir();
    let mut snapshot = session(&workspace, SessionTimestamp(NOW.0 - 1), Some(1.65));
    snapshot.session.parent = Some(SessionId::new());
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
}

#[test]
fn an_idle_client_wakes_when_the_countdown_comes_to_stand_and_not_after_the_lapse() {
    let workspace = workspace_dir();
    let clock = clock_at(NOW);
    let lapses_at = SessionTimestamp(NOW.0 + 105_000);
    let application = open(
        &workspace,
        session(&workspace, lapses_at, Some(1.65)),
        clock.clone(),
    );
    assert_eq!(
        wakeup_after_a_frame(&application),
        Some(std::time::Duration::from_secs(45)),
        "the client first wakes when the countdown comes to stand"
    );

    clock.store(lapses_at.0, Ordering::SeqCst);
    assert_eq!(
        wakeup_after_a_frame(&application),
        None,
        "a lapse already drawn wants no further wakeup"
    );

    let mut subagent = session(&workspace, lapses_at, Some(1.65));
    subagent.session.parent = Some(SessionId::new());
    let viewing_a_subagent = open(&workspace, subagent, clock_at(NOW));
    assert_eq!(wakeup_after_a_frame(&viewing_a_subagent), None);
}

const FIVE_MINUTES: u64 = 300_000;

/// The production lead: a scheduled Compaction falls due two minutes before
/// the lapse.
const LEAD: u64 = 120_000;

/// The countdown as the line above the composer reads it.
fn lapses_in(figure: &str) -> String {
    format!("Prompt cache lapses in {figure}")
}

/// A client with an idle Session open whose Prompt Cache, kept for
/// `lifetime_ms`, lapses at `lapses_at` and costs $1.65 more once it has,
/// and the clock its frames are drawn by.
fn counting_down(
    workspace: &WorkspaceDir,
    lapses_at: SessionTimestamp,
    lifetime_ms: Option<u64>,
) -> (Application, Arc<AtomicU64>) {
    let clock = clock_at(NOW);
    let application = open(
        workspace,
        session_kept_for(workspace, lapses_at, Some(1.65), lifetime_ms),
        clock.clone(),
    );
    (application, clock)
}

/// The line above the composer with the Session clock at `moment`.
fn line_at(application: &Application, clock: &AtomicU64, moment: u64) -> Option<String> {
    clock.store(moment, Ordering::SeqCst);
    composer_top_line(application)
}

#[test]
fn the_last_minute_of_a_five_minute_cache_counts_down_dim_just_above_the_composer() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let (application, clock) = counting_down(&workspace, lapses_at, Some(FIVE_MINUTES));
    let lapse = lapses_at.0;

    assert_eq!(line_at(&application, &clock, lapse - 61_000), None);
    assert_eq!(
        line_at(&application, &clock, lapse - 60_001),
        None,
        "nothing a moment before the final stretch: {}",
        screen(&application)
    );
    assert_eq!(
        line_at(&application, &clock, lapse - 60_000),
        Some(lapses_in("1:00"))
    );
    let buffer = drawn(&application);
    let (column, row) = text_position(&buffer, "Prompt cache lapses in 1:00");
    assert_eq!(
        composer_row(&buffer).checked_sub(row),
        Some(2),
        "the countdown stands directly above the composer's border:\n{}",
        buffer_rows(&buffer).join("\n")
    );
    assert_eq!(
        buffer.cell((column, row)).map(|cell| cell.fg),
        Some(PANDA.muted),
        "it is dim, not the warning colour"
    );
    assert_eq!(
        line_at(&application, &clock, lapse - 59_999),
        Some(lapses_in("1:00")),
        "a second is shown until the whole of it has gone"
    );
    assert_eq!(
        line_at(&application, &clock, lapse - 59_000),
        Some(lapses_in("0:59"))
    );
    assert_eq!(
        line_at(&application, &clock, lapse - 45_000),
        Some(lapses_in("0:45"))
    );
    assert_eq!(
        line_at(&application, &clock, lapse - 9_500),
        Some(lapses_in("0:10"))
    );
    assert_eq!(
        line_at(&application, &clock, lapse - 1_001),
        Some(lapses_in("0:02"))
    );
    assert_eq!(
        line_at(&application, &clock, lapse - 1_000),
        Some(lapses_in("0:01"))
    );
    assert_eq!(
        line_at(&application, &clock, lapse - 1),
        Some(lapses_in("0:01"))
    );
    assert_eq!(
        line_at(&application, &clock, lapse),
        Some(PRICED.to_owned()),
        "at the lapse the cold line takes the countdown's place"
    );
    assert!(
        !screen(&application).contains("lapses in"),
        "the two lines never stand together: {}",
        screen(&application)
    );
}

#[test]
fn a_cache_whose_lifetime_is_short_counts_down_only_its_last_fifth() {
    let workspace = workspace_dir();
    // A fifth of a hundred seconds is twenty, shorter than a minute.
    let lapses_at = SessionTimestamp(NOW.0 + 100_000);
    let (application, clock) = counting_down(&workspace, lapses_at, Some(100_000));
    let lapse = lapses_at.0;

    assert_eq!(line_at(&application, &clock, lapse - 45_000), None);
    assert_eq!(line_at(&application, &clock, lapse - 20_001), None);
    assert_eq!(
        line_at(&application, &clock, lapse - 20_000),
        Some(lapses_in("0:20"))
    );
    assert_eq!(
        line_at(&application, &clock, lapse - 1),
        Some(lapses_in("0:01"))
    );
    assert_eq!(
        line_at(&application, &clock, lapse),
        Some(PRICED.to_owned())
    );
}

#[test]
fn an_hour_long_cache_counts_down_only_its_last_minute() {
    let workspace = workspace_dir();
    // A fifth of an hour is twelve minutes: the minute is the shorter.
    let lapses_at = SessionTimestamp(NOW.0 + 3_600_000);
    let (application, clock) = counting_down(&workspace, lapses_at, Some(3_600_000));
    let lapse = lapses_at.0;

    assert_eq!(line_at(&application, &clock, lapse - 720_000), None);
    assert_eq!(line_at(&application, &clock, lapse - 60_001), None);
    assert_eq!(
        line_at(&application, &clock, lapse - 60_000),
        Some(lapses_in("1:00"))
    );
}

#[test]
fn a_cache_of_unknown_lifetime_counts_down_its_last_minute() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let (application, clock) = counting_down(&workspace, lapses_at, None);
    let lapse = lapses_at.0;

    assert_eq!(line_at(&application, &clock, lapse - 60_001), None);
    assert_eq!(
        line_at(&application, &clock, lapse - 60_000),
        Some(lapses_in("1:00"))
    );
}

#[test]
fn a_countdown_to_a_lapse_under_five_cents_gives_way_to_nothing() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let clock = clock_at(SessionTimestamp(lapses_at.0 - 1));
    let application = open(
        &workspace,
        session_kept_for(&workspace, lapses_at, Some(0.0499), Some(FIVE_MINUTES)),
        clock.clone(),
    );

    assert_eq!(composer_top_line(&application), Some(lapses_in("0:01")));
    assert_eq!(
        line_at(&application, &clock, lapses_at.0),
        None,
        "{}",
        screen(&application)
    );
}

#[test]
fn a_session_first_shown_after_its_lapse_draws_the_cold_line_and_no_countdown() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 - 1);
    let (application, _) = counting_down(&workspace, lapses_at, Some(FIVE_MINUTES));

    assert_eq!(composer_top_line(&application).as_deref(), Some(PRICED));
    assert!(!screen(&application).contains("lapses in"));
}

#[test]
fn a_working_session_draws_no_countdown_and_wants_no_wakeup() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 30_000);
    let mut snapshot = session_kept_for(&workspace, lapses_at, Some(1.65), Some(FIVE_MINUTES));
    snapshot.session.status = SessionStatus::Active;
    snapshot.session.working_since = Some(SessionTimestamp(NOW.0 - 1_000));
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
    assert_eq!(wakeup_after_a_frame(&application), None);
}

#[test]
fn a_session_whose_provider_keeps_no_cache_lifetime_never_counts_down() {
    let workspace = workspace_dir();
    let snapshot = failed_session_snapshot(
        SessionId::new(),
        PromptId::new(),
        "Read the whole repository",
        workspace.path(),
    );
    assert!(snapshot.session.prompt_cache.is_none());
    let clock = clock_at(NOW);
    let application = open(&workspace, snapshot, clock.clone());

    for moment in [NOW.0, NOW.0 + 240_000, NOW.0 + 299_999, NOW.0 + 3_600_000] {
        assert_eq!(line_at(&application, &clock, moment), None);
    }
    assert_eq!(wakeup_after_a_frame(&application), None);
}

#[test]
fn a_subagents_view_draws_no_countdown() {
    let workspace = workspace_dir();
    let mut snapshot = session(&workspace, SessionTimestamp(NOW.0 + 30_000), Some(1.65));
    snapshot.session.parent = Some(SessionId::new());
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
    assert_eq!(wakeup_after_a_frame(&application), None);
}

#[test]
fn a_turn_that_moves_the_lapse_starts_the_countdown_afresh() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 30_000);
    let snapshot = session_kept_for(&workspace, lapses_at, Some(1.65), Some(FIVE_MINUTES));
    let session_id = snapshot.session.id;
    let revision = snapshot.revision;
    let mut application = open(&workspace, snapshot, clock_at(NOW));
    assert_eq!(composer_top_line(&application), Some(lapses_in("0:30")));

    let moved = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id,
                revision: SessionRevision(revision.0 + 1),
                changes: vec![SessionChange::PromptCacheChanged {
                    prompt_cache: Some(PromptCache {
                        lapses_at: moved,
                        written_in: None,
                        written_by: ModelId::new("gpt-fixture"),
                        lifetime_ms: Some(FIVE_MINUTES),
                        write_price: shaman::protocol::CacheWritePrice::Default,
                        prompt_limit_tokens: None,
                        lapse_penalty: LapsePenalty::from_usd(1.70),
                        compaction_due_at: None,
                    }),
                }],
            },
        )))
        .expect("the next turn's lapse arrives");
    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
    assert_eq!(
        wakeup_after_a_frame(&application),
        Some(std::time::Duration::from_secs(240)),
        "nothing wakes the client until the new final stretch begins"
    );
}

#[test]
fn a_client_wakes_when_the_final_stretch_begins_then_each_second_until_the_lapse() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let (application, clock) = counting_down(&workspace, lapses_at, Some(FIVE_MINUTES));
    let lapse = lapses_at.0;
    let wakeup_at = |moment: u64| {
        clock.store(moment, Ordering::SeqCst);
        wakeup_after_a_frame(&application)
    };

    assert_eq!(
        wakeup_at(NOW.0),
        Some(std::time::Duration::from_secs(240)),
        "no wakeup before the final stretch begins"
    );
    assert_eq!(
        wakeup_at(lapse - 60_000),
        Some(std::time::Duration::from_secs(1))
    );
    assert_eq!(
        wakeup_at(lapse - 59_600),
        Some(std::time::Duration::from_millis(600)),
        "each wakeup falls when the figure changes"
    );
    assert_eq!(
        wakeup_at(lapse - 1),
        Some(std::time::Duration::from_millis(1))
    );
    assert_eq!(wakeup_at(lapse), None, "and none once the cache is cold");
}

#[test]
fn a_countdown_no_frame_has_room_for_wants_no_wakeup() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 30_000);
    let (application, _) = counting_down(&workspace, lapses_at, Some(FIVE_MINUTES));
    assert_eq!(
        wakeup_after_a_frame(&application),
        Some(std::time::Duration::from_secs(1))
    );

    rendered_application_buffer(&application, 20, 4);
    assert_eq!(
        application.composer_top_wakeup(),
        None,
        "a terminal too small to draw the session wakes nothing for its line"
    );
}

/// Whether a Selection has left the Prompt Cache is judged against the whole
/// Selection the Turn that wrote it ran under: its Provider, its Model and
/// every Model Option, compared as chosen. Where the writing Turn, or what it
/// ran under, is not known, nothing can be said.
#[test]
fn a_selection_has_left_the_cache_where_anything_it_names_differs_from_its_writers() {
    let workspace = workspace_dir();
    let snapshot = written_under(
        &workspace,
        claude("default", "medium"),
        claude("default", "medium"),
        SessionTimestamp(1_767_225_900_000),
        None,
    );
    let changed = |snapshot: &SessionSnapshot, model: &str, effort: &str| {
        let mut selecting = snapshot.clone();
        selecting.session.agent_selection = Some(claude(model, effort));
        selecting.selection_changed_since_cache_written()
    };
    assert_eq!(changed(&snapshot, "default", "medium"), Some(false));
    assert_eq!(
        changed(&snapshot, "default", "high"),
        Some(true),
        "a model option the writer did not run under leaves the cache"
    );
    assert_eq!(changed(&snapshot, "opus", "medium"), Some(true));

    let mut unbound = snapshot.clone();
    unbound.turns[0].agent = None;
    assert_eq!(
        changed(&unbound, "default", "medium"),
        None,
        "nothing is known of a writer that ran under no agent"
    );
    let mut unwritten = snapshot.clone();
    unwritten
        .session
        .prompt_cache
        .as_mut()
        .expect("the fixture holds a cache")
        .written_in = None;
    assert_eq!(
        changed(&unwritten, "default", "medium"),
        None,
        "nor of a cache no turn is known to have written"
    );
    let mut unselected = snapshot;
    unselected.session.agent_selection = None;
    assert_eq!(
        unselected.selection_changed_since_cache_written(),
        None,
        "nor of a session selecting nothing"
    );
}

/// What the line above the composer opens with where the Selection standing
/// is not the one the Prompt Cache was written under.
const SELECTION_CHANGED: &str = "Prompt cache does not cover this selection";

/// The line for a changed Selection, saying what the next Prompt pays: costs
/// more by `figure`, or re-reads the whole context where `figure` is none.
fn selection_changed(figure: Option<&str>) -> String {
    match figure {
        Some(figure) => format!("{SELECTION_CHANGED} · next prompt costs ~{figure} more"),
        None => format!("{SELECTION_CHANGED} · next prompt re-reads the whole context"),
    }
}

/// An Agent Selection on Claude of `model` at reasoning `effort`.
fn claude(model: &str, effort: &str) -> shaman::protocol::AgentSelection {
    shaman::protocol::AgentSelection {
        provider: shaman::protocol::ProviderId::new("claude"),
        model: ModelId::new(model),
        options: vec![shaman::protocol::ModelOptionSelection {
            id: shaman::protocol::ModelOptionId::new("reasoning_effort"),
            value: shaman::protocol::ModelOptionValue::Select {
                choice: shaman::protocol::ModelOptionChoiceId::new(effort),
            },
        }],
    }
}

/// An idle Session whose only Turn ran under `written_under` and wrote a
/// five-minute Prompt Cache lapsing at `lapses_at`, at `lapse_penalty`, now
/// selecting `selected`.
fn written_under(
    workspace: &WorkspaceDir,
    written_under: shaman::protocol::AgentSelection,
    selected: shaman::protocol::AgentSelection,
    lapses_at: SessionTimestamp,
    lapse_penalty: Option<f64>,
) -> SessionSnapshot {
    let mut snapshot = session_kept_for(workspace, lapses_at, lapse_penalty, Some(FIVE_MINUTES));
    let turn_id = snapshot.turns[0].id;
    let cache = snapshot
        .session
        .prompt_cache
        .as_mut()
        .expect("the fixture holds a cache");
    cache.written_in = Some(turn_id);
    cache.written_by = written_under.model.clone();
    snapshot.turns[0].agent = Some(shaman::protocol::AgentIdentity {
        agent: shaman::protocol::AgentId::new("coding"),
        selection: written_under,
    });
    snapshot.session.agent_selection = Some(selected);
    snapshot
}

/// Has the Server tell `application` the open Session now selects
/// `selection`, at the penalty it forecasts for it.
fn reselect(
    application: &mut Application,
    snapshot: &SessionSnapshot,
    revision: u64,
    selection: shaman::protocol::AgentSelection,
    lapse_penalty: Option<f64>,
) {
    let mut cache = snapshot
        .session
        .prompt_cache
        .clone()
        .expect("the fixture holds a cache");
    cache.lapse_penalty =
        lapse_penalty.map(|usd| LapsePenalty::from_usd(usd).expect("a representable penalty"));
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id: snapshot.session.id,
                revision: SessionRevision(snapshot.revision.0 + revision),
                changes: vec![
                    SessionChange::AgentSelectionChanged { selection },
                    SessionChange::PromptCacheChanged {
                        prompt_cache: Some(cache),
                    },
                ],
            },
        )))
        .expect("the selection lands");
}

#[test]
fn a_warm_session_selecting_another_model_warns_in_the_warning_colour_just_above_the_composer() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 240_000);
    let snapshot = written_under(
        &workspace,
        claude("opus", "medium"),
        claude("haiku", "medium"),
        lapses_at,
        Some(1.65),
    );
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(
        composer_top_line(&application),
        Some(selection_changed(Some("$1.65"))),
        "however recently the cache was written: {}",
        screen(&application)
    );
    let buffer = drawn(&application);
    let (column, row) = text_position(&buffer, SELECTION_CHANGED);
    assert_eq!(
        composer_row(&buffer).checked_sub(row),
        Some(2),
        "the line stands directly above the composer's border:\n{}",
        buffer_rows(&buffer).join("\n")
    );
    assert_eq!(
        buffer.cell((column, row)).map(|cell| cell.fg),
        Some(PANDA.warning)
    );
    let (column, row) = text_position(&buffer, "next prompt costs ~$1.65 more");
    assert_eq!(
        buffer.cell((column, row)).map(|cell| cell.fg),
        Some(PANDA.warning)
    );
}

#[test]
fn a_changed_selection_whose_penalty_is_unknown_says_the_whole_context_is_read_again() {
    let workspace = workspace_dir();
    let snapshot = written_under(
        &workspace,
        claude("opus", "medium"),
        claude("haiku", "medium"),
        SessionTimestamp(NOW.0 + 240_000),
        None,
    );
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(
        composer_top_line(&application),
        Some(selection_changed(None))
    );
    assert_eq!(
        composer_top_rows(&application),
        [SELECTION_CHANGED, "next prompt re-reads the whole context"],
        "a column too narrow for the whole line breaks it between its parts"
    );
}

#[test]
fn a_model_option_changed_since_the_cache_was_written_warns_as_a_model_does() {
    let workspace = workspace_dir();
    let snapshot = written_under(
        &workspace,
        claude("opus", "medium"),
        claude("opus", "high"),
        SessionTimestamp(NOW.0 + 240_000),
        Some(1.65),
    );
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(
        composer_top_line(&application),
        Some(selection_changed(Some("$1.65")))
    );
}

#[test]
fn a_call_that_ran_on_another_model_under_the_same_selection_warns_of_nothing() {
    let workspace = workspace_dir();
    // The CLI fell back, or an alias resolved: the cache names another Model
    // than the one selected, and the user changed nothing.
    let mut snapshot = written_under(
        &workspace,
        claude("default", "medium"),
        claude("default", "medium"),
        SessionTimestamp(NOW.0 + 240_000),
        Some(1.65),
    );
    snapshot
        .session
        .prompt_cache
        .as_mut()
        .expect("the fixture holds a cache")
        .written_by = ModelId::new("claude-sonnet-4-5");
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
}

#[test]
fn selecting_the_original_selection_again_removes_the_warning() {
    let workspace = workspace_dir();
    let snapshot = written_under(
        &workspace,
        claude("opus", "medium"),
        claude("opus", "medium"),
        SessionTimestamp(NOW.0 + 240_000),
        Some(1.65),
    );
    let mut application = open(&workspace, snapshot.clone(), clock_at(NOW));
    assert_eq!(composer_top_line(&application), None);

    reselect(
        &mut application,
        &snapshot,
        1,
        claude("haiku", "medium"),
        Some(0.40),
    );
    assert_eq!(
        composer_top_line(&application),
        Some(selection_changed(Some("$0.40"))),
        "the figure is the one forecast for the selection now standing"
    );

    reselect(
        &mut application,
        &snapshot,
        2,
        claude("opus", "medium"),
        Some(1.65),
    );
    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
}

#[test]
fn a_changed_selection_replaces_the_countdown_until_the_original_is_selected_again() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 30_000);
    let snapshot = written_under(
        &workspace,
        claude("opus", "medium"),
        claude("opus", "medium"),
        lapses_at,
        Some(1.65),
    );
    let mut application = open(&workspace, snapshot.clone(), clock_at(NOW));
    assert_eq!(composer_top_line(&application), Some(lapses_in("0:30")));

    reselect(
        &mut application,
        &snapshot,
        1,
        claude("opus", "high"),
        Some(1.65),
    );
    assert_eq!(
        composer_top_line(&application),
        Some(selection_changed(Some("$1.65")))
    );
    assert!(
        !screen(&application).contains("lapses in"),
        "a lapse that comes to a cache the next prompt will not read is no news: {}",
        screen(&application)
    );

    reselect(
        &mut application,
        &snapshot,
        2,
        claude("opus", "medium"),
        Some(1.65),
    );
    assert_eq!(composer_top_line(&application), Some(lapses_in("0:30")));
}

#[test]
fn at_the_lapse_the_cold_line_takes_over_from_a_changed_selections_warning() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 240_000);
    let clock = clock_at(NOW);
    let snapshot = written_under(
        &workspace,
        claude("opus", "medium"),
        claude("haiku", "medium"),
        lapses_at,
        Some(1.65),
    );
    let application = open(&workspace, snapshot, clock.clone());
    assert_eq!(
        wakeup_after_a_frame(&application),
        Some(std::time::Duration::from_secs(240)),
        "the line counts nothing down, so the client wakes only for the lapse"
    );

    assert_eq!(
        line_at(&application, &clock, lapses_at.0 - 30_000),
        Some(selection_changed(Some("$1.65"))),
        "the final stretch counts down nothing here"
    );
    assert_eq!(
        line_at(&application, &clock, lapses_at.0).as_deref(),
        Some(PRICED),
        "a cold session says it is cold, priced at the selection standing"
    );
    assert_eq!(wakeup_after_a_frame(&application), None);
}

#[test]
fn a_changed_selections_known_penalty_under_five_cents_draws_no_line_and_five_cents_does() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 240_000);
    let small = open(
        &workspace,
        written_under(
            &workspace,
            claude("opus", "medium"),
            claude("haiku", "medium"),
            lapses_at,
            Some(0.0499),
        ),
        clock_at(NOW),
    );
    assert_eq!(composer_top_line(&small), None, "{}", screen(&small));
    assert_eq!(
        wakeup_after_a_frame(&small),
        None,
        "nor does its lapse change anything to wake for"
    );

    let enough = open(
        &workspace,
        written_under(
            &workspace,
            claude("opus", "medium"),
            claude("haiku", "medium"),
            lapses_at,
            Some(0.05),
        ),
        clock_at(NOW),
    );
    assert_eq!(
        composer_top_line(&enough),
        Some(selection_changed(Some("$0.05"))),
    );
}

#[test]
fn a_cold_session_whose_selection_changed_draws_the_cold_line() {
    let workspace = workspace_dir();
    let snapshot = written_under(
        &workspace,
        claude("opus", "medium"),
        claude("haiku", "medium"),
        SessionTimestamp(NOW.0 - 1),
        Some(1.65),
    );
    let application = open(&workspace, snapshot, clock_at(NOW));

    assert_eq!(composer_top_line(&application).as_deref(), Some(PRICED));
}

#[test]
fn a_working_session_or_a_subagents_view_draws_no_warning_for_a_changed_selection() {
    let workspace = workspace_dir();
    let changed = || {
        written_under(
            &workspace,
            claude("opus", "medium"),
            claude("haiku", "medium"),
            SessionTimestamp(NOW.0 + 240_000),
            Some(1.65),
        )
    };
    let mut working = changed();
    working.session.status = SessionStatus::Active;
    working.session.working_since = Some(SessionTimestamp(NOW.0 - 1_000));
    let working = open(&workspace, working, clock_at(NOW));
    assert_eq!(composer_top_line(&working), None, "{}", screen(&working));
    assert_eq!(wakeup_after_a_frame(&working), None);

    let mut subagent = changed();
    subagent.session.parent = Some(SessionId::new());
    let subagent = open(&workspace, subagent, clock_at(NOW));
    assert_eq!(composer_top_line(&subagent), None, "{}", screen(&subagent));
    assert_eq!(wakeup_after_a_frame(&subagent), None);
}

#[test]
fn where_what_wrote_the_cache_is_unknown_no_warning_is_drawn() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + 240_000);
    let changed = || {
        written_under(
            &workspace,
            claude("opus", "medium"),
            claude("haiku", "medium"),
            lapses_at,
            Some(1.65),
        )
    };

    // A cache stored before the Turn that wrote it was.
    let mut unwritten = changed();
    unwritten
        .session
        .prompt_cache
        .as_mut()
        .expect("the fixture holds a cache")
        .written_in = None;
    let clock = clock_at(NOW);
    let unwritten = open(&workspace, unwritten, clock.clone());
    assert_eq!(
        composer_top_line(&unwritten),
        None,
        "{}",
        screen(&unwritten)
    );
    assert_eq!(
        line_at(&unwritten, &clock, lapses_at.0 - 30_000),
        Some(lapses_in("0:30")),
        "the countdown stands as it would have"
    );

    // A writing Turn that ran under no Agent.
    let mut unbound = changed();
    unbound.turns[0].agent = None;
    let unbound = open(&workspace, unbound, clock_at(NOW));
    assert_eq!(composer_top_line(&unbound), None, "{}", screen(&unbound));
}

/// The countdown to a pending scheduled Compaction as the line above the
/// composer reads it.
fn compacting_in(figure: &str) -> String {
    format!("Compacting in {figure}")
}

/// An idle Session whose Prompt Cache, kept five minutes, lapses at
/// `lapses_at` and costs $1.65 more once it has, with a scheduled Compaction
/// published as falling due at `due_at`.
fn compaction_due(
    workspace: &WorkspaceDir,
    lapses_at: SessionTimestamp,
    due_at: SessionTimestamp,
) -> SessionSnapshot {
    let mut snapshot = session_kept_for(workspace, lapses_at, Some(1.65), Some(FIVE_MINUTES));
    snapshot
        .session
        .prompt_cache
        .as_mut()
        .expect("the session has a prompt cache")
        .compaction_due_at = Some(due_at);
    snapshot
}

#[test]
fn a_pending_compaction_counts_down_dim_through_the_two_minutes_before_it_falls_due() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let due = lapses_at.0 - LEAD;
    let clock = clock_at(NOW);
    let application = open(
        &workspace,
        compaction_due(&workspace, lapses_at, SessionTimestamp(due)),
        clock.clone(),
    );

    assert_eq!(line_at(&application, &clock, NOW.0), None);
    assert_eq!(
        line_at(&application, &clock, due - 120_001),
        None,
        "nothing a moment before its two minutes: {}",
        screen(&application)
    );
    assert_eq!(
        line_at(&application, &clock, due - 120_000),
        Some(compacting_in("2:00"))
    );
    let buffer = drawn(&application);
    let (column, row) = text_position(&buffer, "Compacting in 2:00");
    assert_eq!(
        composer_row(&buffer).checked_sub(row),
        Some(2),
        "it stands directly above the composer's border:\n{}",
        buffer_rows(&buffer).join("\n")
    );
    assert_eq!(
        buffer.cell((column, row)).map(|cell| cell.fg),
        Some(PANDA.muted),
        "it is dim, not the warning colour"
    );
    assert_eq!(
        line_at(&application, &clock, due - 119_999),
        Some(compacting_in("2:00"))
    );
    assert_eq!(
        line_at(&application, &clock, due - 119_000),
        Some(compacting_in("1:59"))
    );
    assert_eq!(
        line_at(&application, &clock, due - 60_000),
        Some(compacting_in("1:00"))
    );
    assert_eq!(
        line_at(&application, &clock, due - 1_000),
        Some(compacting_in("0:01"))
    );
    assert_eq!(
        line_at(&application, &clock, due - 1),
        Some(compacting_in("0:01"))
    );
    assert_eq!(
        line_at(&application, &clock, due),
        None,
        "at the moment it counts to it reads neither 0:00 nor on past it: {}",
        screen(&application)
    );
    assert_eq!(line_at(&application, &clock, due + 30_000), None);
}

/// A Compaction that falls due and is skipped late, so nothing begins and the
/// Server has yet to withdraw it, leaves the line as it would be with none
/// pending: the lapse's own countdown, then the cold line.
#[test]
fn past_the_moment_with_nothing_begun_the_lapse_counts_down_as_ever() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let due = lapses_at.0 - LEAD;
    let clock = clock_at(NOW);
    let application = open(
        &workspace,
        compaction_due(&workspace, lapses_at, SessionTimestamp(due)),
        clock.clone(),
    );

    assert_eq!(line_at(&application, &clock, due + 1), None);
    assert_eq!(
        line_at(&application, &clock, lapses_at.0 - 60_000),
        Some(lapses_in("1:00"))
    );
    assert_eq!(
        line_at(&application, &clock, lapses_at.0),
        Some(PRICED.to_owned())
    );
}

/// Where the countdown to a Compaction and the lapse's final stretch would
/// both stand — a short lead, as tests inject — the Compaction's takes the
/// place of the lapse's, and the two never stand together.
#[test]
fn a_pending_compaction_replaces_the_lapse_countdown_wherever_the_two_would_meet() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let due = lapses_at.0 - 30_000;
    let clock = clock_at(NOW);
    let application = open(
        &workspace,
        compaction_due(&workspace, lapses_at, SessionTimestamp(due)),
        clock.clone(),
    );

    for (moment, line) in [
        (lapses_at.0 - 150_000, compacting_in("2:00")),
        (lapses_at.0 - 60_000, compacting_in("0:30")),
        (lapses_at.0 - 45_000, compacting_in("0:15")),
        (due - 1, compacting_in("0:01")),
        (due, lapses_in("0:30")),
    ] {
        assert_eq!(line_at(&application, &clock, moment), Some(line));
        let screen = screen(&application);
        assert!(
            !(screen.contains("Compacting in") && screen.contains("lapses in")),
            "the two never stand together: {screen}"
        );
    }
}

/// A Session the Server publishes nothing pending for — one whose scheduled
/// Compaction would be skipped, as below its minimum, Settled under active Sessions alone, or
/// with the Setting off — counts its lapse down as ever.
#[test]
fn a_session_with_nothing_pending_counts_its_lapse_down_as_ever() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let clock = clock_at(NOW);
    let application = open(
        &workspace,
        session_kept_for(&workspace, lapses_at, Some(1.65), Some(FIVE_MINUTES)),
        clock.clone(),
    );

    for moment in [lapses_at.0 - 240_000, lapses_at.0 - LEAD - 1] {
        assert_eq!(line_at(&application, &clock, moment), None);
    }
    assert_eq!(
        line_at(&application, &clock, lapses_at.0 - 30_000),
        Some(lapses_in("0:30"))
    );
}

#[test]
fn a_working_session_or_a_subagents_view_draws_no_countdown_to_a_compaction() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let due = SessionTimestamp(lapses_at.0 - LEAD);
    let in_its_window = SessionTimestamp(due.0 - 30_000);

    let mut working = compaction_due(&workspace, lapses_at, due);
    working.session.status = SessionStatus::Active;
    working.session.working_since = Some(SessionTimestamp(in_its_window.0 - 1_000));
    let application = open(&workspace, working, clock_at(in_its_window));
    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
    assert_eq!(wakeup_after_a_frame(&application), None);

    let mut subagent = compaction_due(&workspace, lapses_at, due);
    subagent.session.parent = Some(SessionId::new());
    let application = open(&workspace, subagent, clock_at(in_its_window));
    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
    assert_eq!(wakeup_after_a_frame(&application), None);
}

/// The Compaction begins at its moment: the update saying the Session is
/// Working in it takes the line down for the Working Indicator, and the one
/// withdrawing it leaves nothing pending to come back to.
#[test]
fn once_the_compaction_begins_the_line_gives_way_to_the_working_indicator() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let due = SessionTimestamp(lapses_at.0 - LEAD);
    let snapshot = compaction_due(&workspace, lapses_at, due);
    let session_id = snapshot.session.id;
    let revision = snapshot.revision;
    let cache = snapshot.session.prompt_cache.clone();
    let clock = clock_at(SessionTimestamp(due.0 - 1_000));
    let mut application = open(&workspace, snapshot, clock.clone());
    assert_eq!(composer_top_line(&application), Some(compacting_in("0:01")));

    clock.store(due.0, Ordering::SeqCst);
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id,
                revision: SessionRevision(revision.0 + 1),
                changes: vec![
                    SessionChange::SessionStatusChanged {
                        status: SessionStatus::Active,
                    },
                    SessionChange::SessionWorkingChanged {
                        working_since: Some(due),
                    },
                    SessionChange::PromptCacheChanged {
                        prompt_cache: cache.map(|cache| PromptCache {
                            compaction_due_at: None,
                            ..cache
                        }),
                    },
                ],
            },
        )))
        .expect("the compaction's turn begins");
    assert_eq!(
        composer_top_line(&application),
        None,
        "{}",
        screen(&application)
    );
    assert!(
        screen(&application).contains("Working"),
        "the working indicator stands: {}",
        screen(&application)
    );
}

/// The countdown to a Compaction wakes an idle client once, when its two
/// minutes begin, then once a second until the moment it counts to, and
/// then for the lapse's own final stretch.
#[test]
fn a_client_wakes_for_a_pending_compaction_when_its_countdown_begins_then_each_second() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let due = lapses_at.0 - LEAD;
    let clock = clock_at(NOW);
    let application = open(
        &workspace,
        compaction_due(&workspace, lapses_at, SessionTimestamp(due)),
        clock.clone(),
    );
    let wakeup_at = |moment: u64| {
        clock.store(moment, Ordering::SeqCst);
        wakeup_after_a_frame(&application)
    };

    assert_eq!(
        wakeup_at(NOW.0),
        Some(std::time::Duration::from_secs(60)),
        "no wakeup before its two minutes begin"
    );
    assert_eq!(
        wakeup_at(due - 120_000),
        Some(std::time::Duration::from_secs(1))
    );
    assert_eq!(
        wakeup_at(due - 1),
        Some(std::time::Duration::from_millis(1)),
        "the last wakeup falls on the moment it counts to"
    );
    assert_eq!(
        wakeup_at(due),
        Some(std::time::Duration::from_secs(60)),
        "past it, the lapse's final stretch is what comes next"
    );
}

/// The Server publishes no Compaction pending under a changed selection,
/// which it skips; were one to arrive, the changed selection's warning
/// stands all the same.
#[test]
fn a_changed_selections_warning_stands_over_a_pending_compaction() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let due = SessionTimestamp(lapses_at.0 - LEAD);
    let mut snapshot = written_under(
        &workspace,
        claude("claude-opus-4-7", "high"),
        claude("claude-sonnet-4-6", "high"),
        lapses_at,
        Some(1.65),
    );
    snapshot
        .session
        .prompt_cache
        .as_mut()
        .expect("the session has a prompt cache")
        .compaction_due_at = Some(due);
    let application = open(
        &workspace,
        snapshot,
        clock_at(SessionTimestamp(due.0 - 30_000)),
    );
    assert_eq!(
        composer_top_line(&application),
        Some(selection_changed(Some("$1.65"))),
        "{}",
        screen(&application)
    );
}

/// The countdown to a Compaction is held to no floor: the user's own
/// minimum chose it, and it warns of nothing, so a penalty under five cents
/// still draws it, where the cold line it would otherwise lead to would not.
#[test]
fn a_pending_compaction_counts_down_whatever_the_penalty() {
    let workspace = workspace_dir();
    let lapses_at = SessionTimestamp(NOW.0 + FIVE_MINUTES);
    let due = SessionTimestamp(lapses_at.0 - LEAD);
    let mut snapshot = session_kept_for(&workspace, lapses_at, Some(0.04), Some(FIVE_MINUTES));
    snapshot
        .session
        .prompt_cache
        .as_mut()
        .expect("the session has a prompt cache")
        .compaction_due_at = Some(due);
    let clock = clock_at(NOW);
    let application = open(&workspace, snapshot, clock.clone());

    assert_eq!(
        line_at(&application, &clock, due.0 - 30_000),
        Some(compacting_in("0:30"))
    );
    assert_eq!(
        line_at(&application, &clock, lapses_at.0),
        None,
        "the cold line under five cents is not drawn"
    );
}
