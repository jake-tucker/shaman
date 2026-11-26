//! Rune's side: the state the view reads, how what the link says lands in it, and the view.

use std::{
    borrow::Cow,
    rc::Rc,
    sync::{Arc, Mutex, OnceLock, mpsc::Receiver},
    time::Instant,
};

use rune::tok::{self, type_};
use rune::{
    Cx, El, GrowthSpec, ScrollSpec, TextEdit, Tx, Waker, col, el, memo, row, text, text_input,
};
use rune_kit::{Kit, Markdown};
use shaman::protocol::{Activity, MessageRole, MessageStatus, SessionId};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    link::{Ask, Listed, Said},
    model::{Item, Open, Row},
};

/// What a run is trying, read once from the environment.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Whether each row is wrapped in a `memo` versioned by its revision.
    pub memo: bool,
    /// How many of an Activity's last lines a row draws; 0 draws all of them.
    pub activity_lines: usize,
    /// How many times over the Transcript is tiled.
    pub repeat: usize,
    /// Whether the list is told its keys by a version, so a commit does not ask every row for its key.
    pub versioned_keys: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let number = |name: &str, default: usize| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(default)
        };
        Self {
            memo: number("RT_MEMO", 1) != 0,
            activity_lines: number("RT_ACTIVITY_LINES", 12),
            repeat: number("RT_REPEAT", 1),
            versioned_keys: number("RT_VERSIONED_KEYS", 1) != 0,
        }
    }
}

/// What the frame hooks and the state both write, since a hook is handed no state.
#[derive(Default)]
pub struct Timings {
    /// When the Session now opening was asked for, until the first frame that shows it.
    pub opening: Option<Instant>,
    pub open_to_encoded_ms: Option<f64>,
    pub open_to_presented_ms: Option<f64>,
    pub awaiting_present: Option<(u64, Instant)>,
    /// Each drawn frame's update and encode, in milliseconds, since last cleared.
    pub frames: Vec<(f64, f64)>,
    /// How many times something crossed from the link, and how many updates that carried.
    pub crossings: u64,
    pub updates: u64,
}

pub struct State {
    pub config: Config,
    pub asks: Option<UnboundedSender<Ask>>,
    /// Where the link's sayings wait when they cross by `poll` and not by `AppHandle::update`.
    pub inbox: Option<Receiver<Said>>,
    pub waker: Arc<OnceLock<Waker>>,
    pub sessions: Vec<Listed>,
    pub open: Option<Open>,
    /// Counts the Sessions opened, so each gets a scroll viewport and a key version of its own.
    pub opened: u64,
    pub draft: TextEdit,
    pub note: String,
    pub timings: Arc<Mutex<Timings>>,
}

impl State {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            asks: None,
            inbox: None,
            waker: Arc::new(OnceLock::new()),
            sessions: Vec::new(),
            open: None,
            opened: 0,
            draft: TextEdit::new(""),
            note: "Connecting to the scratch Server…".to_owned(),
            timings: Arc::new(Mutex::new(Timings::default())),
        }
    }

    pub fn viewport(&self) -> String {
        format!("transcript{}", self.opened)
    }

    pub fn ask_open(&mut self, session_id: SessionId) {
        if let Some(asks) = &self.asks {
            let _ = asks.send(Ask::Open(session_id, Instant::now()));
        }
    }

    pub fn show(&mut self, open: Open) {
        self.open = Some(open);
        self.opened += 1;
    }

    /// Lands one thing the link said. Runs on the UI thread, whichever way it crossed.
    pub fn take(&mut self, said: Said, tx: &mut Tx) {
        let mut timings = self.timings.lock().expect("timings");
        timings.crossings += 1;
        match said {
            Said::Sessions(sessions) => {
                self.note = format!("{} Sessions on the scratch Server", sessions.len());
                self.sessions = sessions;
            }
            Said::Snapshot {
                snapshot,
                asked,
                arrived,
            } => {
                let crossed = arrived.elapsed();
                let indexing = Instant::now();
                let open = Open::new(*snapshot, self.config.repeat);
                self.note = format!(
                    "{} rows ({} Messages, {} Activities) · asked→snapshot decoded {:.1} ms · crossed to UI {:.1} ms · \
                     indexed {:.1} ms",
                    open.rows.len(),
                    open.snapshot.messages.len(),
                    open.snapshot.activities.len(),
                    (arrived - asked).as_secs_f64() * 1e3,
                    crossed.as_secs_f64() * 1e3,
                    indexing.elapsed().as_secs_f64() * 1e3,
                );
                timings.opening = Some(asked);
                timings.open_to_encoded_ms = None;
                timings.open_to_presented_ms = None;
                drop(timings);
                self.show(open);
                // A Session opens at its foot with nothing animating in.
                tx.settle();
            }
            Said::Updates(updates) => {
                timings.updates += updates.len() as u64;
                drop(timings);
                let Some(open) = &mut self.open else { return };
                for update in &updates {
                    if let Err(error) = open.apply(update) {
                        self.note = format!("update refused: {error:#}");
                    }
                }
            }
            Said::Problem(problem) => self.note = problem,
        }
    }

    /// `poll`: takes everything waiting in the inbox, and says whether anything was.
    pub fn drain(&mut self, tx: &mut Tx) -> bool {
        let mut any = false;
        loop {
            let Some(said) = self.inbox.as_ref().and_then(|inbox| inbox.try_recv().ok()) else {
                return any;
            };
            self.take(said, tx);
            any = true;
        }
    }

    /// Says what the frames drawn since this was last asked cost, where a person scrolling can read it.
    fn note_frames(&mut self) {
        let mut timings = self.timings.lock().expect("timings");
        self.note = format!(
            "{}; {} crossings carrying {} updates",
            crate::drive::frames(&timings.frames),
            timings.crossings,
            timings.updates
        );
        eprintln!("- by hand: {}", self.note);
        timings.frames.clear();
        timings.crossings = 0;
        timings.updates = 0;
    }

    fn submit(&mut self) {
        let mut numbers = self
            .draft
            .text
            .split_whitespace()
            .filter_map(|word| word.parse::<f64>().ok());
        let words = numbers.next().unwrap_or(400.0) as u64;
        let per_second = numbers.next().unwrap_or(60.0);
        let chunk = numbers.next().unwrap_or(1.0) as u64;
        self.draft.clear();
        if let (Some(open), Some(asks)) = (&self.open, &self.asks) {
            let _ = asks.send(Ask::Stream {
                session_id: open.id,
                words,
                per_second,
                chunk,
            });
        }
    }
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let dark = cx.system_dark();
    let t = tok::theme(dark);
    let mut sessions = col()
        .key("sessions")
        .w(260.0)
        .h_full()
        .padding(8.0)
        .gap(2.0)
        .fill(t.card);
    for listed in &s.sessions {
        let id = listed.id;
        let shown = s.open.as_ref().is_some_and(|open| open.id == id);
        let title = if listed.title.is_empty() {
            id.to_string()
        } else {
            listed.title.clone()
        };
        let mut item = el()
            .key(format!("s{id}"))
            .w_full()
            .padding(8.0)
            .radius(6.0)
            .on_press(move |s: &mut State| s.ask_open(id))
            .child(text(title).font(type_::LABEL).color(t.text).truncate());
        if shown {
            item = item.fill(t.control_off);
        }
        sessions = sessions.child(item);
    }
    let composer = text_input(&s.draft, type_::BODY)
        .key("composer")
        .label("Stream")
        .placeholder(
            "Return streams a Turn into this Session: words, words a second, words a delta (400 60 1)",
            t.text_secondary,
        )
        .color(t.text)
        .w_full()
        .padding(12.0)
        .radius(10.0)
        .fill(t.card)
        .on_edit(|s: &mut State| &mut s.draft)
        .on_submit(|s: &mut State, _| s.submit());
    let main = col()
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        // A flex item is at least as wide as its content, and a paragraph's content is one long line.
        .min_w(0.0)
        .padding(12.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .child(
                    el().key("frames")
                        .px(10.0)
                        .py(4.0)
                        .radius(6.0)
                        .fill(t.control_off)
                        .on_press(|s: &mut State| s.note_frames())
                        .child(
                            text("Frames since last pressed")
                                .font(type_::LABEL)
                                .color(t.text),
                        ),
                )
                .child(
                    text(s.note.clone())
                        .key("note")
                        .font(type_::LABEL)
                        .color(t.text_secondary)
                        .grow(1.0)
                        .min_w(0.0),
                ),
        )
        .child(transcript(s, dark))
        .child(composer);
    row()
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(sessions)
        .child(main)
}

fn transcript(s: &State, dark: bool) -> El {
    let t = tok::theme(dark);
    let Some(open) = &s.open else {
        return col().w_full().grow(1.0).center().child(
            text("Choose a Session at the side")
                .font(type_::BODY)
                .color(t.text_secondary),
        );
    };
    let kit = Rc::new(Kit::neutral(dark));
    let config = s.config;
    let mut rows = col()
        .key("rows")
        .w_full()
        .growth(GrowthSpec::default())
        .selection_group(true)
        .virtual_rows(
            open.rows.len(),
            96.0,
            |s: &State, at| {
                Cow::Borrowed(
                    s.open.as_ref().expect("a Session is open").rows[at]
                        .key
                        .as_str(),
                )
            },
            move |s: &State, at| {
                let open = s.open.as_ref().expect("a Session is open");
                let row = &open.rows[at];
                if config.memo {
                    memo(
                        row.key.as_str(),
                        (row.revision, dark, config.activity_lines),
                        || draw(open, row, &kit, dark, config.activity_lines),
                    )
                } else {
                    draw(open, row, &kit, dark, config.activity_lines)
                }
            },
        );
    if config.versioned_keys {
        rows = rows
            .virtual_rows_version(s.opened)
            .virtual_rows_append_only();
    }
    col()
        .key(s.viewport())
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .child(rows)
}

/// One Transcript item as a row: a Message as Markdown, an Activity as a line of what it was and the foot of what
/// it printed. Every text joins the list's one selection.
fn draw(open: &Open, row: &Row, kit: &Kit, dark: bool, activity_lines: usize) -> El {
    let t = tok::theme(dark);
    let body = match row.item {
        Item::Message(at) => {
            let message = &open.snapshot.messages[at];
            let markdown = Markdown::new(message.content.as_str())
                .selectable(true)
                .streaming(message.status == MessageStatus::Streaming)
                .build(kit)
                // As built, a Markdown is a selection group of its own and a drag stops at its end (#768).
                .selection_group(false)
                .key("md");
            if message.role == MessageRole::User {
                col()
                    .w_full()
                    .padding(10.0)
                    .radius(8.0)
                    .fill(t.card)
                    .child(markdown)
            } else {
                markdown
            }
        }
        Item::Activity(at) => {
            let (head, printed) = activity(open.activity(at), activity_lines);
            let mut lines = col().w_full().gap(2.0).child(
                text(head)
                    .key("head")
                    .font(type_::LABEL.mono())
                    .color(t.text_secondary)
                    .selectable(true)
                    .truncate(),
            );
            if !printed.is_empty() {
                lines = lines.child(
                    text(printed)
                        .key("printed")
                        .font(type_::LABEL.mono())
                        .color(t.text_secondary)
                        .selectable(true)
                        .w_full(),
                );
            }
            lines
        }
        Item::Unknown => text("(an item this build does not know)")
            .font(type_::LABEL)
            .color(t.text_secondary),
    };
    col()
        .key(row.key.clone())
        .w_full()
        .px(20.0)
        .py(6.0)
        .child(body)
}

fn activity(activity: &Activity, lines: usize) -> (String, String) {
    let (head, printed) = match activity {
        Activity::Command {
            command,
            output,
            status,
            exit_status,
            ..
        } => (
            format!("$ {command}  [{status:?} {exit_status:?}]"),
            foot(output, lines),
        ),
        Activity::ToolCall {
            name,
            input,
            output,
            status,
            ..
        } => (format!("{name} {input}  [{status:?}]"), foot(output, lines)),
        Activity::Reasoning { title, content, .. } => (
            format!("Reasoning: {}", title.as_deref().unwrap_or("")),
            foot(content, lines),
        ),
        Activity::Status { text, .. } => (text.clone(), String::new()),
        Activity::Error { text, .. } => (format!("Error: {text}"), String::new()),
        Activity::Subagent {
            name, description, ..
        } => (format!("Subagent {name}"), foot(description, lines)),
        other => {
            let whole = serde_json::to_string_pretty(other).unwrap_or_default();
            let kind = serde_json::to_value(other)
                .ok()
                .and_then(|value| value["type"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "activity".to_owned());
            (kind, foot(&whole, lines))
        }
    };
    let head: String = head
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(240)
        .collect();
    (head, printed)
}

/// The last `lines` lines of `text`, or all of it where `lines` is 0.
fn foot(text: &str, lines: usize) -> String {
    let text = text.trim_end();
    if lines == 0 {
        return text.to_owned();
    }
    let all: Vec<&str> = text.lines().collect();
    if all.len() <= lines {
        return text.to_owned();
    }
    format!(
        "… {} earlier lines\n{}",
        all.len() - lines,
        all[all.len() - lines..].join("\n")
    )
}
