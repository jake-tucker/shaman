//! The Model view: whether the Overview's Model breakdown files work under
//! the Model each Turn's Agent Selection named or under the Models that
//! actually ran it, a choice held for the TUI's run and offered only where
//! the breakdown is by Model.

use super::{
    HEIGHT, NARROW, NEW_YORK, WIDTH, afternoon, answer, ask, by_provider, copilot, empty_report,
    expect_read, focused_row, legend_row_dimmed, many_workspaces, model, narrow_screen, press,
    press_at, priced_report, reader_in, rendered_row, report, row, screen, show_tokens, space,
    swatch, thirty_days_with_errands, ticks, top_session, with_errands,
};
use crate::tui_rendering::support::{
    invoke, key, rendered_application_buffer, rendered_application_rows_at, text_position,
};
use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use shaman::{
    protocol::{
        UsageMetricsBreakdown, UsageMetricsKey, UsageMetricsModelView, UsageMetricsQuery,
        UsageMetricsReport,
    },
    tui::{Application, ApplicationTransition, CommandId, SemanticCommandId},
};

/// The row under the controls, where the Model view stands beneath the
/// breakdown it refines: four under the title bar.
fn model_view_row(rows: &[String]) -> &str {
    &rows[rendered_row(rows, "esc ✕") + 4]
}

/// The control as it is drawn at `width`, or nothing where it is not.
fn model_view_control(application: &Application, width: u16) -> Option<String> {
    let rows = rendered_application_rows_at(application, width, HEIGHT);
    let line = model_view_row(&rows);
    line.find("Model:")
        .map(|at| line[at..].trim_end().to_owned())
}

/// Clicks the word `label` where it first stands at `width`.
fn click_at_width(application: &mut Application, width: u16, label: &str) -> ApplicationTransition {
    let buffer = rendered_application_buffer(application, width, HEIGHT);
    let at = text_position(&buffer, label);
    press_at(application, at)
}

/// `report` with `key` filed under `to` and labelled `label` in its place.
fn rekeyed(
    mut report: UsageMetricsReport,
    key: &UsageMetricsKey,
    to: UsageMetricsKey,
    label: &str,
) -> UsageMetricsReport {
    for keyed in &mut report.keys {
        if keyed.key == *key {
            keyed.key = to.clone();
            keyed.label = label.to_owned();
        }
    }
    for bucket in report
        .buckets
        .iter_mut()
        .chain([&mut report.across_buckets])
    {
        for keyed in &mut bucket.by_key {
            if keyed.key == *key {
                keyed.key = to.clone();
            }
        }
    }
    report
}

/// [`report`]'s thirty days as the Actual view files them: Opus and Codex
/// ran as chosen, and Copilot's work ran on a Model the catalog does not
/// know, so the Server labels it by the Provider's id. Its most expensive
/// Session is named by that Model too.
fn actual_report(query: &UsageMetricsQuery) -> UsageMetricsReport {
    let mut report = rekeyed(
        report(query),
        &copilot(),
        model("copilot", "gpt-5.5-mini-2026-10"),
        "gpt-5.5-mini-2026-10",
    );
    report.top_sessions = vec![top_session(
        "Route the planner",
        4.20,
        Some("gpt-5.5-mini-2026-10"),
        "shaman",
    )];
    report
}

/// Thirty days of `keys` Actual Models, the first using a million tokens a
/// day and each after it a hundred thousand fewer: [`many_workspaces`]
/// filed by Model.
fn many_models(query: &UsageMetricsQuery, keys: u64) -> UsageMetricsReport {
    (0..keys).fold(many_workspaces(query, keys), |report, index| {
        rekeyed(
            report,
            &UsageMetricsKey::Workspace {
                workspace: shaman::protocol::WorkspaceId(format!("directory:{index}")),
            },
            model("copilot", &format!("model-{index}")),
            &format!("model-{index}"),
        )
    })
}

/// A view of thirty days by Model, read and answered with `answered` in
/// the Actual view, shown under Tokens.
fn thirty_days_actual(
    answered: impl FnOnce(&UsageMetricsQuery) -> UsageMetricsReport,
) -> Application {
    let mut application = reader_in(NEW_YORK, afternoon());
    ask(&mut application);
    let (_, request_id, query) = expect_read(press(&mut application, 'm'));
    assert_eq!(query.model_view, UsageMetricsModelView::Actual);
    answer(&mut application, request_id, Ok(answered(&query)));
    show_tokens(&mut application);
    application
}

#[test]
fn the_control_is_drawn_only_under_the_model_breakdown() {
    let mut application = reader_in(NEW_YORK, afternoon());
    let (_, request_id, query) = ask(&mut application);
    assert_eq!(query.model_view, UsageMetricsModelView::Selected);
    // Before the first answer as after it, beneath the breakdown.
    assert_eq!(
        model_view_control(&application, WIDTH).as_deref(),
        Some("Model: ▐Selected▌  Actual")
    );
    answer(&mut application, request_id, Ok(report(&query)));
    assert_eq!(
        model_view_control(&application, WIDTH).as_deref(),
        Some("Model: ▐Selected▌  Actual")
    );
    let rows = screen(&application);
    let (by_x, _) = text_position(
        &rendered_application_buffer(&application, WIDTH, HEIGHT),
        "by ▐Model▌",
    );
    let model_x = model_view_row(&rows).find("Model:").unwrap();
    assert_eq!(usize::from(by_x), model_x, "it stands under the breakdown");

    for breakdown in ["Provider", "Workspace"] {
        let (_, request_id, query) = expect_read(press(&mut application, 'b'));
        assert_eq!(
            model_view_control(&application, WIDTH),
            None,
            "{breakdown} while reading"
        );
        answer(&mut application, request_id, Ok(empty_report(&query, None)));
        let shown = screen(&application).join("\n");
        assert!(!shown.contains("Selected"), "{breakdown}: {shown}");
        assert!(!shown.contains("Actual"), "{breakdown}: {shown}");
    }

    expect_read(press(&mut application, 'b'));
    assert_eq!(
        model_view_control(&application, WIDTH).as_deref(),
        Some("Model: ▐Selected▌  Actual")
    );
    // Insights draws no Model view.
    press(&mut application, 'i');
    assert!(
        !screen(&application).join("\n").contains("Selected"),
        "{:#?}",
        screen(&application)
    );
}

/// Answers whether `transition` read the buckets `opened` read by Model
/// anew in `expected`, showing nothing until that read answers and the
/// control its choice, then answers it.
fn reads_anew(
    application: &mut Application,
    transition: ApplicationTransition,
    expected: UsageMetricsModelView,
    opened: &UsageMetricsQuery,
) {
    let (_, request_id, query) = expect_read(transition);
    assert_eq!(query.model_view, expected);
    assert_eq!(query.breakdown, UsageMetricsBreakdown::Model);
    assert_eq!(query.buckets, opened.buckets);
    // Figures filed the other way are not shown under the new choice.
    assert!(
        screen(application)
            .iter()
            .any(|row| row.contains("Reading usage metrics")),
        "{expected:?}"
    );
    let chosen = match expected {
        UsageMetricsModelView::Selected => "Model: ▐Selected▌  Actual",
        UsageMetricsModelView::Actual => "Model:  Selected  ▐Actual▌",
    };
    assert_eq!(
        model_view_control(application, WIDTH).as_deref(),
        Some(chosen)
    );
    answer(application, request_id, Ok(report(&query)));
}

#[test]
fn m_and_clicks_choose_the_model_view_and_read_it_anew() {
    assert_eq!(
        [
            SemanticCommandId::MetricsModelViewSelect(UsageMetricsModelView::Selected),
            SemanticCommandId::MetricsModelViewSelect(UsageMetricsModelView::Actual),
            SemanticCommandId::MetricsModelViewToggle,
        ]
        .map(SemanticCommandId::as_str),
        [
            "metrics.model-view.selected",
            "metrics.model-view.actual",
            "metrics.model-view.toggle",
        ]
    );
    let mut application = reader_in(NEW_YORK, afternoon());
    let (_, request_id, opened) = ask(&mut application);
    answer(&mut application, request_id, Ok(report(&opened)));

    let transition = press(&mut application, 'm');
    reads_anew(
        &mut application,
        transition,
        UsageMetricsModelView::Actual,
        &opened,
    );
    let transition = press(&mut application, 'm');
    reads_anew(
        &mut application,
        transition,
        UsageMetricsModelView::Selected,
        &opened,
    );
    let transition = click_at_width(&mut application, WIDTH, "Actual");
    reads_anew(
        &mut application,
        transition,
        UsageMetricsModelView::Actual,
        &opened,
    );
    let transition = click_at_width(&mut application, WIDTH, "Selected");
    reads_anew(
        &mut application,
        transition,
        UsageMetricsModelView::Selected,
        &opened,
    );
    let transition = invoke(
        &mut application,
        SemanticCommandId::MetricsModelViewSelect(UsageMetricsModelView::Actual),
    );
    reads_anew(
        &mut application,
        transition,
        UsageMetricsModelView::Actual,
        &opened,
    );
    let transition = invoke(&mut application, SemanticCommandId::MetricsModelViewToggle);
    reads_anew(
        &mut application,
        transition,
        UsageMetricsModelView::Selected,
        &opened,
    );

    // Choosing the view already shown reads nothing, by click or command.
    assert_eq!(
        click_at_width(&mut application, WIDTH, "Selected"),
        ApplicationTransition::Continue
    );
    assert_eq!(
        invoke(
            &mut application,
            SemanticCommandId::MetricsModelViewSelect(UsageMetricsModelView::Selected),
        ),
        ApplicationTransition::Continue
    );

    // The key names the very command the invocation does.
    assert_eq!(
        application.command_for_terminal_input(InputEvent::Key(KeyEvent::new(
            KeyCode::Char('m'),
            KeyModifiers::NONE
        ))),
        Some(CommandId::InvokeSemantic(
            SemanticCommandId::MetricsModelViewToggle
        ))
    );
}

#[test]
fn an_answer_for_the_view_before_never_lands_after_the_change() {
    let mut application = reader_in(NEW_YORK, afternoon());
    let (_, selected_id, selected) = ask(&mut application);
    let (_, actual_id, actual) = expect_read(press(&mut application, 'm'));

    // The Selected read, asked first, answers late.
    answer(&mut application, selected_id, Ok(report(&selected)));
    let shown = screen(&application).join("\n");
    assert!(shown.contains("Reading usage metrics"), "{shown}");
    assert!(!shown.contains("copilot · GPT-5.5"), "{shown}");

    answer(&mut application, actual_id, Ok(actual_report(&actual)));
    let shown = screen(&application).join("\n");
    assert!(shown.contains("gpt-5.5-mini-2026-10"), "{shown}");
    assert!(!shown.contains("copilot · GPT-5.5"), "{shown}");
}

#[test]
fn under_another_breakdown_m_does_nothing_and_the_model_breakdown_brings_back_the_held_view() {
    let mut application = reader_in(NEW_YORK, afternoon());
    ask(&mut application);
    let (_, request_id, query) = expect_read(press(&mut application, 'm'));
    answer(&mut application, request_id, Ok(actual_report(&query)));

    // The query says the view held, which the Server ignores under the
    // Provider breakdown.
    let (_, request_id, query) = expect_read(press(&mut application, 'b'));
    assert_eq!(query.breakdown, UsageMetricsBreakdown::Provider);
    assert_eq!(query.model_view, UsageMetricsModelView::Actual);
    answer(&mut application, request_id, Ok(by_provider(&query)));
    let before = screen(&application);

    let transitions = [
        press(&mut application, 'm'),
        invoke(
            &mut application,
            SemanticCommandId::MetricsModelViewSelect(UsageMetricsModelView::Selected),
        ),
        invoke(&mut application, SemanticCommandId::MetricsModelViewToggle),
    ];
    assert!(
        transitions
            .iter()
            .all(|transition| *transition == ApplicationTransition::Continue),
        "{transitions:?}"
    );
    assert_eq!(screen(&application), before, "nothing changes on screen");
    // Nor did it change what the next read asks.
    let (_, request_id, query) = expect_read(press(&mut application, 'r'));
    assert_eq!(query.model_view, UsageMetricsModelView::Actual);
    answer(&mut application, request_id, Ok(by_provider(&query)));

    expect_read(press(&mut application, 'b'));
    let (_, _, query) = expect_read(press(&mut application, 'b'));
    assert_eq!(query.breakdown, UsageMetricsBreakdown::Model);
    assert_eq!(query.model_view, UsageMetricsModelView::Actual);
    assert_eq!(
        model_view_control(&application, WIDTH).as_deref(),
        Some("Model:  Selected  ▐Actual▌")
    );
}

#[test]
fn the_view_chosen_outlives_a_range_a_measure_a_tab_and_the_view_closing() {
    let mut application = reader_in(NEW_YORK, afternoon());
    ask(&mut application);
    let (_, request_id, query) = expect_read(press(&mut application, 'm'));
    answer(&mut application, request_id, Ok(actual_report(&query)));
    let actual = Some("Model:  Selected  ▐Actual▌");

    let (_, request_id, query) = expect_read(press(&mut application, '2'));
    assert_eq!(query.model_view, UsageMetricsModelView::Actual);
    assert_eq!(model_view_control(&application, WIDTH).as_deref(), actual);
    answer(&mut application, request_id, Ok(actual_report(&query)));

    assert_eq!(
        press(&mut application, 's'),
        ApplicationTransition::Continue
    );
    assert_eq!(model_view_control(&application, WIDTH).as_deref(), actual);

    press(&mut application, 'i');
    assert_eq!(
        press(&mut application, 'g'),
        ApplicationTransition::Continue
    );
    assert_eq!(model_view_control(&application, WIDTH).as_deref(), actual);
    let (_, _, query) = expect_read(press(&mut application, 'r'));
    assert_eq!(query.model_view, UsageMetricsModelView::Actual);

    key(&mut application, KeyCode::Esc);
    let (_, _, query) = ask(&mut application);
    assert_eq!(query.model_view, UsageMetricsModelView::Actual);
    assert_eq!(model_view_control(&application, WIDTH).as_deref(), actual);
}

#[test]
fn actual_keys_are_drawn_with_their_report_labels_and_name_the_top_session_s_model() {
    let application = thirty_days_actual(actual_report);

    let rows = screen(&application);
    for label in ["Claude Opus 5.5", "GPT-5.5 Codex", "gpt-5.5-mini-2026-10"] {
        row(&rows, &format!("█ {label}"));
    }
    assert!(!rows.join("\n").contains("copilot · GPT-5.5"));
    let session = row(&rows, "Route the planner");
    assert!(session.contains("gpt-5.5-mini-2026-10"), "{session:?}");
}

#[test]
fn hiding_a_series_and_the_other_fold_work_on_actual_keys() {
    let mut application = thirty_days_actual(|query| many_models(query, 9));

    let rows = screen(&application);
    let shown = rows.join("\n");
    for index in 0..6 {
        row(&rows, &format!("model-{index}"));
    }
    for index in 6..9 {
        assert!(!shown.contains(&format!("model-{index}")), "{shown}");
    }
    // The seventh to ninth use 400K, 300K and 200K a day: 27M over thirty.
    assert!(row(&rows, "█ Other").contains("27M"), "{shown}");

    // Other alone, then hidden, as any series.
    key(&mut application, KeyCode::Up);
    assert_eq!(focused_row(&application).as_deref(), Some("Other"));
    press(&mut application, 'o');
    assert_eq!(ticks(&application).last().map(String::as_str), Some("900K"));
    for index in 0..6 {
        assert_eq!(
            legend_row_dimmed(&application, &format!("model-{index}")),
            Some(true)
        );
    }
    space(&mut application, KeyModifiers::NONE);
    assert_eq!(legend_row_dimmed(&application, "Other"), Some(true));

    // An Actual key shows alone with Shift+Space, and hides with Space.
    for _ in 0..6 {
        key(&mut application, KeyCode::Up);
    }
    assert_eq!(focused_row(&application).as_deref(), Some("model-0"));
    space(&mut application, KeyModifiers::SHIFT);
    assert_eq!(legend_row_dimmed(&application, "model-0"), Some(false));
    assert_eq!(legend_row_dimmed(&application, "model-1"), Some(true));
    assert_eq!(legend_row_dimmed(&application, "Other"), Some(true));
    space(&mut application, KeyModifiers::NONE);
    assert_eq!(legend_row_dimmed(&application, "model-0"), Some(true));
}

#[test]
fn a_series_hidden_in_one_view_is_shown_again_in_the_other() {
    let mut application = reader_in(NEW_YORK, afternoon());
    let (_, request_id, query) = ask(&mut application);
    answer(&mut application, request_id, Ok(report(&query)));
    show_tokens(&mut application);
    invoke(
        &mut application,
        SemanticCommandId::MetricsSeriesToggle { series: 0 },
    );
    assert_eq!(
        legend_row_dimmed(&application, "Claude Opus 5.5"),
        Some(true)
    );

    // Opus is a key of the Actual view too, but its series is another's.
    let (_, request_id, query) = expect_read(press(&mut application, 'm'));
    answer(&mut application, request_id, Ok(actual_report(&query)));
    assert_eq!(
        legend_row_dimmed(&application, "Claude Opus 5.5"),
        Some(false)
    );
    assert_eq!(focused_row(&application), None);
}

#[test]
fn at_eighty_columns_the_control_collapses_to_its_choice_as_the_breakdown_does() {
    let mut application = reader_in(NEW_YORK, afternoon());
    let (_, request_id, query) = ask(&mut application);
    answer(&mut application, request_id, Ok(priced_report(&query)));
    let rows = narrow_screen(&application);
    let controls = &rows[rendered_row(&rows, "esc ✕") + 3];
    assert!(
        controls.trim_end().ends_with("by ▐Model▌"),
        "the breakdown collapses: {controls:?}"
    );
    assert_eq!(
        model_view_control(&application, NARROW).as_deref(),
        Some("Model: ▐Selected▌"),
        "and the model view with it"
    );
    // Wherever the breakdown stands whole, so does the Model view.
    assert_eq!(
        model_view_control(&application, 100).as_deref(),
        Some("Model: ▐Selected▌  Actual")
    );

    // Collapsed to its choice, the control takes the other on a click, as
    // `m` does, its brackets as well as its word.
    let (_, request_id, query) = expect_read(click_at_width(&mut application, NARROW, "Selected"));
    assert_eq!(query.model_view, UsageMetricsModelView::Actual);
    answer(&mut application, request_id, Ok(actual_report(&query)));
    assert_eq!(
        model_view_control(&application, NARROW).as_deref(),
        Some("Model: ▐Actual▌")
    );
    let (_, _, query) = expect_read(click_at_width(&mut application, NARROW, "▐Actual▌"));
    assert_eq!(query.model_view, UsageMetricsModelView::Selected);
}

#[test]
fn errands_keep_their_reserved_colour_and_never_fold_in_the_actual_view() {
    let application = thirty_days_actual(|query| with_errands(many_models(query, 9), 1_000, 0, 0));

    let rows = screen(&application);
    assert_eq!(
        rendered_row(&rows, "█ Other"),
        rendered_row(&rows, "Errands") + 1,
        "Errands takes a place among the series kept"
    );
    let selected = thirty_days_with_errands();
    assert_eq!(
        swatch(&application, "Errands"),
        swatch(&selected, "Errands")
    );
}
