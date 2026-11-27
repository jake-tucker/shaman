//! Throwaway (#797): how the composer works in a window.
//!
//! What every form shares is what Rune's editor was asked to do. A draft is one `TextEdit`. An Attachment, a
//! Collapsed Paste and a Skill Invocation each stand in it as a pill: Rune's atomic run, whose text is the label
//! (`[Image 1]`, `[Paste 1: 14 lines]`, `$review`) and whose id leads to what is bound. The field's own hooks take a
//! paste (a picture, files, text long enough to collapse), the keys the editor must not have (Return, Esc, Tab, the
//! arrows while completions are open or the caret is at an end of the draft), and a press on a pill. A draft is
//! captured as text and spans, which is what the shared model holds, and put back into an editor from them.
//!
//! The three forms disagree about what is drawn around that:
//!
//! - a, the TUI's: labels in the text and a line for each Attachment under it, completions docked over the composer
//!   as wide as it with what the row means beside the list, queued Prompts in one Pending box with their actions
//!   always on show.
//! - b, a chat's: a tray of pictures at the head of the box, completions in a list that floats from the composer's
//!   corner, each queued Prompt a card whose actions come with the pointer, and the whole main view takes a drop.
//! - c, all in the text: a pill carries its picture and a press on one opens what it stands for, completions open
//!   inside the box over the field, and queued Prompts fold to one line.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

use rune::{
    Align, ClipboardContent, El, Image, Key, KeyEvent, Mark, MarkRange, PillStyle, Pixels,
    PointerPress, Side, TextEdit, Tx, col, el, row, text,
};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, MenuItem, PopupItem,
    PopupList, Spinner, TextArea, context_menu,
};

use crate::parts::glyph;
use crate::talk::Phase;
use crate::{App, Platform, bits, settings};

/// The Server's limits on Attachments, which the Client checks too.
const MAX_IMAGES: usize = 10;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// How many completions are offered at once, as in the TUI.
const LIMIT: usize = 10;
/// How long the fake Server takes to store an Attachment, and to apply a posture.
const UPLOADS: f64 = 0.9;
const APPLIES: f64 = 0.8;

#[derive(Clone, Copy, PartialEq)]
pub enum Form {
    A,
    B,
    C,
}

impl Form {
    pub const ALL: [Form; 3] = [Form::A, Form::B, Form::C];

    pub fn name(self) -> &'static str {
        match self {
            Form::A => "A (the TUI's: labels and lines, completions docked, a Pending box)",
            Form::B => "B (a chat's: a tray of pictures, a floating list, cards)",
            Form::C => {
                "C (all in the text: pills with pictures, a list in the box, a folded queue)"
            }
        }
    }

    pub fn step(self, by: usize) -> Form {
        let at = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(at + by) % Self::ALL.len()]
    }
}

/// What a pill's id leads to.
pub enum Bound {
    Image {
        n: u32,
        format: &'static str,
        bytes: usize,
        /// The picture's own bytes, kept until there is a cache to decode them with.
        raw: Option<Vec<u8>>,
        image: Option<Image>,
        /// Whether the Server has yet to answer for it.
        uploading: bool,
        since: Option<f64>,
    },
    Paste {
        n: u32,
        text: String,
    },
    Skill,
}

/// A draft as the shared model holds one: its text, and the spans bound into it.
#[derive(Clone, Default, PartialEq)]
pub struct Draft {
    pub text: String,
    pub spans: Vec<(Range<usize>, String)>,
}

impl Draft {
    pub fn of(edit: &TextEdit) -> Draft {
        Draft {
            text: edit.text.clone(),
            spans: edit.pills().map(|(r, id)| (r, id.to_owned())).collect(),
        }
    }

    /// An editor holding this draft, the keys in it and the caret at one end.
    pub fn editor(&self, at_end: bool) -> TextEdit {
        let mut edit = TextEdit::new(self.text.clone());
        edit.set_marks(
            self.spans
                .iter()
                .map(|(r, id)| MarkRange::new(r.clone(), Mark::Pill(id.clone())))
                .collect(),
        );
        let at = if at_end { self.text.len() } else { 0 };
        edit.anchor = at;
        edit.focus = at;
        edit.focused = true;
        edit
    }

    /// The Prompt this draft sends: each Collapsed Paste's words back where its label stood, and every other span
    /// moved to suit.
    pub fn unfolded(&self, bound: &HashMap<String, Bound>) -> Draft {
        let mut out = Draft::default();
        let mut at = 0;
        for (range, id) in &self.spans {
            out.text.push_str(&self.text[at..range.start]);
            match bound.get(id) {
                Some(Bound::Paste { text, .. }) => out.text.push_str(text),
                _ => {
                    let start = out.text.len();
                    out.text.push_str(&self.text[range.clone()]);
                    out.spans.push((start..out.text.len(), id.clone()));
                }
            }
            at = range.end;
        }
        out.text.push_str(&self.text[at..]);
        out
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum How {
    Began,
    Steered,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Completing {
    Commands,
    Skills,
}

pub struct Popup {
    pub kind: Completing,
    /// Where the token being completed begins.
    pub start: usize,
    pub active: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Catalog {
    Ready,
    Loading,
}

/// How a change to the Approval Posture stands, as the protocol's `ApprovalPostureApplication` says it.
#[derive(Clone, Copy, PartialEq)]
pub enum Applying {
    Applied,
    Applying,
    NextTurn,
    Failed,
}

pub struct State {
    pub form: Form,
    pub bound: HashMap<String, Bound>,
    uid: u32,
    /// The highest label number this draft has given out; none is reused until the draft is sent or cleared.
    images: u32,
    pastes: u32,
    pub popup: Option<Popup>,
    /// The completion the reader shut, which stays shut while the caret stays in its token.
    shut: Option<(Completing, usize)>,
    pub skills: Catalog,
    pub queue: Vec<Draft>,
    pub sent: Vec<(Draft, How)>,
    history: Vec<Draft>,
    walk: Option<usize>,
    scratch: Option<Draft>,
    /// Whether Esc was pressed once while a Turn works, so that the next one interrupts.
    pub armed: bool,
    /// Whether files are being dragged over what takes them.
    pub over: bool,
    /// What the composer says of something it would not take.
    pub note: Option<String>,
    /// a: the queued Prompt the keys are on, while the reader manages the queue.
    pub managing: Option<usize>,
    /// b: the queued Prompt the pointer is on.
    pub hover: Option<usize>,
    /// c: whether the folded queue is open.
    pub queue_open: bool,
    pub applying: Applying,
    applying_since: Option<f64>,
    /// The pill a secondary press landed on, and where.
    pub menu: Option<(String, (f32, f32))>,
    /// c: the pill whose content is on show.
    pub preview: Option<String>,
    /// What the composer last did, for the strip under the window.
    pub last: String,
}

impl State {
    pub fn new(form: Form) -> State {
        State {
            form,
            bound: HashMap::new(),
            uid: 0,
            images: 0,
            pastes: 0,
            popup: None,
            shut: None,
            skills: Catalog::Ready,
            queue: Vec::new(),
            sent: Vec::new(),
            history: Vec::new(),
            walk: None,
            scratch: None,
            armed: false,
            over: false,
            note: None,
            managing: None,
            hover: None,
            queue_open: false,
            applying: Applying::Applied,
            applying_since: None,
            menu: None,
            preview: None,
            last: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The catalogs.

pub struct Command {
    pub name: &'static str,
    pub about: &'static str,
    /// Whether the command takes the rest of the line, and so is completed into the draft and not run.
    pub takes_text: bool,
}

const fn cmd(name: &'static str, about: &'static str) -> Command {
    Command {
        name,
        about,
        takes_text: false,
    }
}

/// The TUI's slash commands a first-cut window would list, by the TUI's names.
pub const COMMANDS: &[Command] = &[
    cmd("new", "Begin a new session"),
    cmd("sessions", "Search every session"),
    cmd("models", "Choose the model and its options"),
    cmd("posture", "Choose how the agent asks before it acts"),
    Command {
        name: "compact",
        about: "Compact this session's context, with what to keep in mind",
        takes_text: true,
    },
    Command {
        name: "move",
        about: "Move this session to another worktree",
        takes_text: true,
    },
    cmd("rename", "Rename this session"),
    cmd("settle", "Settle this session"),
    cmd("unsettle", "Bring a settled session back"),
    cmd("context", "Show what fills this session's context"),
    cmd("worktree", "Choose the worktree a new session begins in"),
    cmd("workspace", "Choose the workspace a new session begins in"),
    cmd("sidebar", "Show or hide the sidebar"),
    cmd("aside", "Show or hide the aside"),
    cmd("settings", "Open settings"),
];

pub struct Skill {
    pub name: &'static str,
    pub about: &'static str,
    /// Whether the Provider's limit on distinct Skills a Prompt is already met without it.
    pub over_limit: bool,
}

const fn skill(name: &'static str, about: &'static str) -> Skill {
    Skill {
        name,
        about,
        over_limit: false,
    }
}

pub const SKILLS: &[Skill] = &[
    skill(
        "review",
        "Review the changes since a fixed point against standards and spec",
    ),
    skill("research", "Investigate a question against primary sources"),
    skill("release-notes", "Draft release notes from merged changes"),
    skill(
        "prototype",
        "Build a throwaway prototype to answer a design question",
    ),
    skill("tdd", "Build features or fix bugs test-first"),
    skill("diagnosing-bugs", "A diagnosis loop for hard bugs"),
    skill(
        "domain-modeling",
        "Build and sharpen the project's domain model",
    ),
    skill("grilling", "Stress-test a plan, a decision or an idea"),
    skill(
        "simplify",
        "Review changed code for reuse and simplification",
    ),
    skill(
        "security-review",
        "Review pending changes for security problems",
    ),
    skill(
        "codebase-design",
        "Find where a module's interface could be deeper",
    ),
    skill("wizard", "Generate a wizard for steps only a person can do"),
];

pub struct Item {
    pub id: String,
    pub label: String,
    pub about: &'static str,
    pub note: Option<&'static str>,
    pub disabled: bool,
}

/// What the open completion offers for what is typed: names that begin with it first, then names that carry it,
/// then anything whose description does.
pub fn matches(s: &App) -> Vec<Item> {
    let Some(p) = &s.c.popup else {
        return Vec::new();
    };
    let caret = s.draft.focus.min(s.draft.text.len());
    let query = s
        .draft
        .text
        .get(p.start + 1..caret)
        .unwrap_or("")
        .to_lowercase();
    let rank = |name: &str, about: &str| {
        if name.starts_with(&query) {
            Some(0)
        } else if name.contains(&query) {
            Some(1)
        } else if about.to_lowercase().contains(&query) {
            Some(2)
        } else {
            None
        }
    };
    let mut found: Vec<(u8, Item)> = match p.kind {
        Completing::Commands => COMMANDS
            .iter()
            .filter_map(|c| {
                Some((
                    rank(c.name, c.about)?,
                    Item {
                        id: c.name.to_owned(),
                        label: format!("/{}", c.name),
                        about: c.about,
                        note: c.takes_text.then_some("takes the rest of the line"),
                        disabled: false,
                    },
                ))
            })
            .collect(),
        Completing::Skills if s.c.skills == Catalog::Loading => Vec::new(),
        Completing::Skills => SKILLS
            .iter()
            .filter_map(|k| {
                Some((
                    rank(k.name, k.about)?,
                    Item {
                        id: k.name.to_owned(),
                        label: format!("${}", k.name),
                        about: k.about,
                        note: k
                            .over_limit
                            .then_some("this prompt already names as many skills as it may"),
                        disabled: k.over_limit,
                    },
                ))
            })
            .collect(),
    };
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, i)| i).take(LIMIT).collect()
}

/// The completion the caret stands in, and where its token begins: `/word` as the whole draft, or `$word` at the
/// start of a token.
fn trigger(edit: &TextEdit) -> Option<(Completing, usize)> {
    if edit.anchor != edit.focus {
        return None;
    }
    let t = &edit.text;
    let caret = edit.focus.min(t.len());
    if t.starts_with('/') && caret == t.len() && !t.contains(char::is_whitespace) {
        return Some((Completing::Commands, 0));
    }
    let before = &t[..caret];
    let name = before
        .rfind(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .map_or(0, |i| {
            i + before[i..].chars().next().map_or(1, char::len_utf8)
        });
    let start = name.checked_sub(1)?;
    if !before[start..].starts_with('$') {
        return None;
    }
    let token_start = start == 0 || before[..start].ends_with(char::is_whitespace);
    // A `$` that already belongs to a Skill Invocation completes nothing.
    let bound = edit.pills().any(|(r, _)| r.contains(&start));
    (token_start && !bound).then_some((Completing::Skills, start))
}

/// Opens, keeps or shuts the completion to suit where the caret now stands.
pub fn retrigger(s: &mut App) {
    match trigger(&s.draft) {
        Some(found) if s.c.shut != Some(found) => {
            let active = match &s.c.popup {
                Some(p) if (p.kind, p.start) == found => p.active,
                _ => 0,
            };
            s.c.popup = Some(Popup {
                kind: found.0,
                start: found.1,
                active,
            });
            let n = matches(s).len();
            if let Some(p) = &mut s.c.popup {
                p.active = p.active.min(n.saturating_sub(1));
            }
        }
        Some(_) => s.c.popup = None,
        None => {
            s.c.popup = None;
            s.c.shut = None;
        }
    }
}

fn changed(s: &mut App) {
    s.c.walk = None;
    s.c.armed = false;
    retrigger(s);
}

fn accept(s: &mut App, id: &str) {
    let Some(p) = s.c.popup.take() else {
        return;
    };
    match p.kind {
        Completing::Commands => {
            let Some(c) = COMMANDS.iter().find(|c| c.name == id) else {
                return;
            };
            if c.takes_text {
                s.draft = Draft {
                    text: format!("/{} ", c.name),
                    spans: Vec::new(),
                }
                .editor(true);
                s.c.last = format!("completed /{} into the draft", c.name);
            } else {
                clear(s);
                invoke(s, c);
            }
        }
        Completing::Skills => {
            let caret = s.draft.focus;
            let pill = format!("skill:{id}");
            s.c.bound.insert(pill.clone(), Bound::Skill);
            let label = format!("${id}");
            s.draft.undo_group(|e| {
                e.replace_with_pill(p.start..caret, pill, &label);
                e.insert(" ");
            });
            s.c.last = format!("bound ${id}");
        }
    }
    s.draft.focused = true;
}

fn accept_active(s: &mut App) -> bool {
    let found = matches(s);
    let at = s.c.popup.as_ref().map_or(0, |p| p.active);
    match found.get(at) {
        Some(item) if !item.disabled => {
            let id = item.id.clone();
            accept(s, &id);
            true
        }
        _ => false,
    }
}

fn invoke(s: &mut App, c: &Command) {
    s.c.last = format!("ran /{}", c.name);
    match c.name {
        "settings" => s.open_settings(),
        "new" => {
            s.landed();
            s.land();
        }
        "sidebar" => s.sidebar.toggle(),
        "aside" => s.toggle_aside(),
        "models" | "posture" => s.choose(crate::Choosing::All),
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------------------------
// What a draft takes in.

/// Empties the draft, as sending or running a command does, and lets its labels count from one again.
fn clear(s: &mut App) {
    s.compose("");
    s.c.images = 0;
    s.c.pastes = 0;
    s.c.popup = None;
    s.c.shut = None;
    s.c.walk = None;
    s.c.scratch = None;
    s.c.preview = None;
}

fn next_id(s: &mut App, kind: &str) -> String {
    s.c.uid += 1;
    format!("{kind}:{}", s.c.uid)
}

/// The Attachments standing in the draft, in its order.
pub fn attached(s: &App) -> Vec<(String, Range<usize>)> {
    s.draft
        .pills()
        .filter(|(_, id)| id.starts_with("image:"))
        .map(|(r, id)| (id.to_owned(), r))
        .collect()
}

fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG") {
        Some("PNG")
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        Some("JPEG")
    } else if bytes.starts_with(b"GIF8") {
        Some("GIF")
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("WebP")
    } else {
        None
    }
}

/// Places a picture on the draft: its label goes in at the caret at once and reads as uploading until the Server
/// has it.
pub fn attach(s: &mut App, bytes: Vec<u8>) {
    let Some(format) = sniff(&bytes) else {
        s.c.note = Some("Only PNG, JPEG, GIF and WebP pictures can be attached.".into());
        return;
    };
    if attached(s).len() >= MAX_IMAGES {
        s.c.note = Some(format!("A prompt carries at most {MAX_IMAGES} pictures."));
        return;
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        s.c.note = Some(format!(
            "That picture is {}; a prompt carries pictures up to 5 MiB.",
            size(bytes.len())
        ));
        return;
    }
    s.c.note = None;
    s.c.images += 1;
    let n = s.c.images;
    let id = next_id(s, "image");
    let len = bytes.len();
    let image = s.images.as_ref().map(|p| p.place(bytes.clone()));
    s.c.bound.insert(
        id.clone(),
        Bound::Image {
            n,
            format,
            bytes: len,
            raw: image.is_none().then_some(bytes),
            image,
            uploading: true,
            since: None,
        },
    );
    s.draft.undo_group(|e| {
        e.insert_pill(id, &format!("[Image {n}]"));
        e.insert(" ");
    });
    s.draft.focused = true;
    s.c.last = format!("attached Image {n} ({format}, {})", size(len));
    s.tick();
}

/// Text long enough to collapse, as the TUI measures it: three written lines, or more than 400 characters.
fn collapses(t: &str) -> bool {
    !t.trim().is_empty()
        && (t.trim_end_matches('\n').lines().count() >= 3 || t.chars().count() > 400)
}

pub fn paste_text(s: &mut App, t: &str) -> bool {
    let on = s.settings.value(settings::COLLAPSE) == "true";
    if !on || !collapses(t) {
        return false;
    }
    s.c.pastes += 1;
    let n = s.c.pastes;
    let lines = t.trim_end_matches('\n').lines().count();
    let label = if lines >= 3 {
        format!("[Paste {n}: {lines} lines]")
    } else {
        format!("[Paste {n}: {} chars]", t.chars().count())
    };
    let id = next_id(s, "paste");
    s.c.bound.insert(
        id.clone(),
        Bound::Paste {
            n,
            text: t.to_owned(),
        },
    );
    s.draft.undo_group(|e| {
        e.insert_pill(id, &label);
        e.insert(" ");
    });
    s.c.last = format!("collapsed a paste of {lines} lines");
    true
}

/// Files handed to the composer, dragged in or pasted: a picture becomes an Attachment, and anything else is named
/// by its path, which only means something to an Agent on this machine.
pub fn take_files(s: &mut App, paths: &[PathBuf]) {
    for path in paths {
        let picture = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp"
            )
        });
        if picture {
            match std::fs::read(path) {
                Ok(bytes) => attach(s, bytes),
                Err(e) => s.c.note = Some(format!("{}: {e}", path.display())),
            }
        } else if s.outlook.is_some() {
            s.c.note = Some(
                "Only pictures travel to a remote. A file's path on this machine would mean nothing there.".into(),
            );
        } else {
            let shown = path.display().to_string();
            let shown = if shown.contains(' ') {
                format!("\"{shown}\" ")
            } else {
                format!("{shown} ")
            };
            s.draft.insert(&shown);
            s.c.last = "wrote a dropped file's path into the draft".into();
        }
    }
    s.draft.focused = true;
    s.c.over = false;
    changed(s);
}

fn paste(s: &mut App, content: &ClipboardContent) -> bool {
    if let Some(image) = content.image() {
        attach(s, image.png().to_vec());
        return true;
    }
    if !content.files().is_empty() {
        take_files(s, &content.files().to_vec());
        return true;
    }
    let took = paste_text(s, content.text());
    if took {
        changed(s);
    }
    took
}

fn span_of(s: &App, id: &str) -> Option<Range<usize>> {
    s.draft.pills().find(|(_, i)| *i == id).map(|(r, _)| r)
}

/// Takes a label out of the draft whole, with the space that followed it.
pub fn remove(s: &mut App, id: &str) {
    let Some(mut range) = span_of(s, id) else {
        return;
    };
    if s.draft.text[range.end..].starts_with(' ') {
        range.end += 1;
    }
    s.draft.set_selection(range.start, range.end);
    s.draft.insert("");
    s.draft.focused = true;
    s.c.preview = None;
    s.c.menu = None;
    s.c.last = "removed a label".into();
    changed(s);
}

/// Turns a Collapsed Paste back into the words it stands for.
pub fn expand(s: &mut App, id: &str) {
    let (Some(range), Some(Bound::Paste { text, .. })) = (span_of(s, id), s.c.bound.get(id)) else {
        return;
    };
    let text = text.clone();
    s.draft.set_selection(range.start, range.end);
    s.draft.insert(&text);
    s.draft.focused = true;
    s.c.preview = None;
    s.c.menu = None;
    s.c.last = "expanded a paste".into();
    changed(s);
}

// ---------------------------------------------------------------------------------------------------------------
// Sending.

fn uploading(s: &App) -> bool {
    attached(s).iter().any(|(id, _)| {
        matches!(
            s.c.bound.get(id),
            Some(Bound::Image {
                uploading: true,
                ..
            })
        )
    })
}

/// Return, or Alt+Return to queue: the first Prompt on the Landing, a Prompt that begins a Turn, one that steers the
/// Turn at work, or one that waits its turn.
pub fn submit(s: &mut App, queue: bool) {
    s.c.note = None;
    s.c.armed = false;
    let draft = Draft::of(&s.draft);
    if draft.text.trim().is_empty() {
        // An empty Return still tries a failed start again.
        if s.open.is_none() {
            s.submit();
        }
        return;
    }
    if uploading(s) {
        s.c.note = Some("Waiting for a picture to finish uploading.".into());
        return;
    }
    let word = draft.text.trim();
    if let Some(name) = word.strip_prefix('/') {
        let (name, rest) = name.split_once(' ').unwrap_or((name, ""));
        if let Some(c) = COMMANDS.iter().find(|c| c.name == name) {
            if c.takes_text || rest.is_empty() {
                clear(s);
                invoke(s, c);
                return;
            }
        } else if rest.is_empty() && !name.contains('/') {
            // `/tmp/x` is a path and is sent; `/tpyo` is a command nobody has.
            s.c.note = Some(format!("There is no command /{name}."));
            return;
        }
    }
    s.c.history.push(draft.clone());
    let sends = draft.unfolded(&s.c.bound);
    if s.open.is_none() {
        s.c.last = "sent the first prompt".into();
        s.draft = TextEdit::new(sends.text);
        s.submit();
        clear(s);
        return;
    }
    clear(s);
    if s.phase.live() && queue {
        s.c.queue.push(draft);
        s.c.last = "queued a prompt".into();
    } else if s.phase.live() {
        s.c.sent.push((sends, How::Steered));
        s.c.last = "steered the turn".into();
    } else {
        s.c.sent.push((sends, How::Began));
        s.phase = Phase::Working;
        s.c.last = "sent a prompt, which began a turn".into();
    }
}

pub fn stop(s: &mut App) {
    s.phase = Phase::Stopped;
    s.c.armed = false;
    s.draft.focused = true;
    s.c.last = "interrupted the turn".into();
}

pub fn steer_now(s: &mut App, at: usize) {
    if at < s.c.queue.len() {
        let draft = s.c.queue.remove(at);
        s.c.sent.push((draft.unfolded(&s.c.bound), How::Steered));
        s.c.last = "promoted a queued prompt to steer".into();
    }
    after_queue(s);
}

pub fn cancel(s: &mut App, at: usize) {
    if at < s.c.queue.len() {
        s.c.queue.remove(at);
        s.c.last = "cancelled a queued prompt".into();
    }
    after_queue(s);
}

/// b and c: cancels a queued Prompt and puts it back in the composer, which the TUI has no way to do.
pub fn edit(s: &mut App, at: usize) {
    if at < s.c.queue.len() {
        let draft = s.c.queue.remove(at);
        let held = Draft::of(&s.draft);
        if !held.text.trim().is_empty() {
            s.c.history.push(held);
        }
        s.draft = draft.editor(true);
        s.c.last = "took a queued prompt back to edit".into();
    }
    after_queue(s);
}

fn after_queue(s: &mut App) {
    s.c.hover = None;
    s.c.managing = match s.c.managing {
        Some(at) if !s.c.queue.is_empty() => Some(at.min(s.c.queue.len() - 1)),
        _ => None,
    };
    if s.c.queue.is_empty() {
        s.c.queue_open = false;
    }
    s.draft.focused = true;
}

fn recall(s: &mut App, back: bool) -> bool {
    let n = s.c.history.len();
    let at = match (s.c.walk, back) {
        (None, true) if n > 0 => {
            s.c.scratch = Some(Draft::of(&s.draft));
            n - 1
        }
        (Some(at), true) if at > 0 => at - 1,
        (Some(at), false) if at + 1 < n => at + 1,
        (Some(_), false) => {
            // Past the newest: what was being typed before the walk began.
            s.draft = s.c.scratch.take().unwrap_or_default().editor(true);
            s.c.walk = None;
            return true;
        }
        _ => return false,
    };
    s.draft = s.c.history[at].editor(!back);
    s.c.walk = Some(at);
    s.c.popup = None;
    s.c.last = format!("recalled prompt {} of {n}", at + 1);
    true
}

// ---------------------------------------------------------------------------------------------------------------
// The keys.

/// Whether `ev` is Ctrl with `c`. Off macOS that is the command chord, the same event as ⌘ with `c` on a Mac.
fn ctrl(ev: &KeyEvent, c: char) -> bool {
    ev.mods.ctrl
        && match &ev.key {
            Key::Text(t) => t.eq_ignore_ascii_case(&c.to_string()),
            Key::Command(k) => *k == c,
            _ => false,
        }
}

/// The composer's key hook, which Rune runs before its editor sees a key.
pub fn key(s: &mut App, ev: &KeyEvent, _: &mut Tx) -> bool {
    let bare = !ev.mods.shift && !ev.mods.alt && !ev.mods.ctrl && !ev.mods.cmd;
    // A menu or a preview shuts on Esc before anything else hears it.
    if ev.key == Key::Escape && (s.c.menu.is_some() || s.c.preview.is_some()) {
        s.c.menu = None;
        s.c.preview = None;
        return true;
    }
    if s.c.popup.is_some() {
        let n = matches(s).len();
        let step = |s: &mut App, by: usize| {
            if let (Some(p), true) = (&mut s.c.popup, n > 0) {
                p.active = (p.active + by) % n;
            }
        };
        if (ev.key == Key::Up && bare) || ctrl(ev, 'p') {
            step(s, n.saturating_sub(1));
            return true;
        }
        if (ev.key == Key::Down && bare) || ctrl(ev, 'n') {
            step(s, 1);
            return true;
        }
        if matches!(ev.key, Key::Enter | Key::Tab) && bare && accept_active(s) {
            return true;
        }
        if ev.key == Key::Escape {
            let p = s.c.popup.take();
            s.c.shut = p.map(|p| (p.kind, p.start));
            return true;
        }
    }
    if let Some(at) = s.c.managing {
        let n = s.c.queue.len();
        match ev.key {
            Key::Up if n > 0 => s.c.managing = Some((at + n - 1) % n),
            Key::Down if n > 0 => s.c.managing = Some((at + 1) % n),
            Key::Enter => steer_now(s, at),
            Key::Escape => s.c.managing = None,
            _ if ctrl(ev, 'd') => cancel(s, at),
            _ => return false,
        }
        return true;
    }
    if ev.key == Key::Enter && !ev.mods.shift {
        // Ctrl+Return is a new line in the TUI, as Shift+Return is; off macOS that is the command chord.
        if ev.mods.ctrl || ev.mods.cmd {
            s.draft.insert("\n");
            changed(s);
        } else {
            submit(s, ev.mods.alt);
        }
        return true;
    }
    if ctrl(ev, 'j') {
        s.draft.insert("\n");
        changed(s);
        return true;
    }
    if ev.key == Key::Escape {
        if crate::field_key(s, ev) {
            return true;
        }
        if s.open.is_some() && s.phase.live() {
            if s.c.armed {
                stop(s);
            } else {
                s.c.armed = true;
            }
        }
        // Esc never clears a draft, and the field must not give the keys up to it.
        return true;
    }
    s.c.armed = false;
    let caret = s.draft.focus;
    let no_selection = s.draft.anchor == s.draft.focus;
    if ev.key == Key::Up && bare && no_selection && caret == 0 && recall(s, true) {
        return true;
    }
    if ev.key == Key::Down
        && bare
        && no_selection
        && caret == s.draft.text.len()
        && s.c.walk.is_some()
        && recall(s, false)
    {
        return true;
    }
    crate::field_key(s, ev)
}

/// Moves what the clock moves: an Attachment the Server has stored, a posture it has applied.
pub fn poll(s: &mut App, now: f64) -> (bool, bool) {
    let mut moved = false;
    let mut waiting = false;
    // A scene poses its draft before there is a cache to decode a picture with.
    if let Some(pictures) = &s.images {
        for b in s.c.bound.values_mut() {
            if let Bound::Image { raw, image, .. } = b
                && let Some(bytes) = raw.take()
            {
                *image = Some(pictures.place(bytes));
                moved = true;
            }
        }
    }
    if s.scene_free() {
        for b in s.c.bound.values_mut() {
            if let Bound::Image {
                uploading, since, ..
            } = b
                && *uploading
            {
                let began = *since.get_or_insert(now);
                if now - began > UPLOADS {
                    *uploading = false;
                    moved = true;
                } else {
                    waiting = true;
                }
            }
        }
        if s.c.applying == Applying::Applying {
            let began = *s.c.applying_since.get_or_insert(now);
            if now - began > APPLIES {
                // A posture changed under a working Turn holds from the next one.
                s.c.applying = if s.phase.live() {
                    Applying::NextTurn
                } else {
                    Applying::Applied
                };
                s.c.applying_since = None;
                moved = true;
            } else {
                waiting = true;
            }
        }
        if s.c.applying == Applying::NextTurn && !s.phase.live() {
            s.c.applying = Applying::Applied;
            moved = true;
        }
    }
    (moved, waiting)
}

/// A change of posture is on its way to the Server.
pub fn posture_changed(s: &mut App) {
    s.c.applying = Applying::Applying;
    s.c.applying_since = None;
    s.tick();
}

// ---------------------------------------------------------------------------------------------------------------
// What is drawn.

fn size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KiB", bytes.div_ceil(1024))
    }
}

/// An Attachment in words: `Image 1 · PNG · 1280×720 · 312 KiB`.
fn described(b: &Bound) -> String {
    let Bound::Image {
        n,
        format,
        bytes,
        image,
        ..
    } = b
    else {
        return String::new();
    };
    let dims = image
        .as_ref()
        .and_then(|i| i.size())
        .map(|[w, h]| format!(" · {w}×{h}"))
        .unwrap_or_default();
    format!("Image {n} · {format}{dims} · {}", size(*bytes))
}

fn pixels_of(b: &Bound) -> Option<Pixels> {
    match b {
        Bound::Image {
            image: Some(image), ..
        } if image.is_ready() => Some(image.pixels().clone()),
        _ => None,
    }
}

fn is_mac(s: &App) -> bool {
    s.platform == Platform::Mac
}

fn queue_keys(s: &App) -> &'static str {
    if is_mac(s) { "⌥⏎" } else { "Alt+⏎" }
}

/// The composer's field: Rune's multiline editor, with every hook the composer needs on it.
pub fn field(s: &App, placeholder: &str, min: usize, max: usize, k: &Kit) -> El {
    let c = &k.c;
    let pastes: HashMap<String, String> =
        s.c.bound
            .iter()
            .filter_map(|(id, b)| match b {
                Bound::Paste { text, .. } => Some((id.clone(), text.clone())),
                _ => None,
            })
            .collect();
    let pending: HashSet<String> =
        s.c.bound
            .iter()
            .filter(|(_, b)| {
                matches!(
                    b,
                    Bound::Image {
                        uploading: true,
                        ..
                    }
                )
            })
            .map(|(id, _)| id.clone())
            .collect();
    let (chip, ink, faint, skill_bg, skill_ink) =
        (c.layer_03, c.text_base, c.text_faint, c.info.bg, c.info.fg);
    let form = s.c.form;
    let placeholder = if s.c.over {
        "Drop to attach"
    } else {
        placeholder
    };
    let mut area = TextArea::new(&s.draft)
        .label("Prompt")
        .placeholder(placeholder.to_owned())
        .plain()
        .width(None)
        .auto_grow(min, max)
        .on_edit(|s: &mut App| &mut s.draft)
        .on_change(|s: &mut App, _: &mut Tx| changed(s))
        .on_input_selection(|s: &mut App, _: &mut Tx| retrigger(s))
        .on_key(key)
        .on_input_paste(|s: &mut App, content: &ClipboardContent, _: &mut Tx| paste(s, content))
        .on_pill_press(move |s: &mut App, id: &str, _: PointerPress, _: &mut Tx| {
            // c: a press on a label opens what it stands for. a and b leave the press to the caret.
            if form != Form::C || id.starts_with("skill:") {
                return false;
            }
            s.c.preview = (s.c.preview.as_deref() != Some(id)).then(|| id.to_owned());
            true
        })
        .on_pill_secondary(|s: &mut App, id: &str, p: PointerPress, _: &mut Tx| {
            s.c.menu = Some((id.to_owned(), (p.x, p.y)));
            true
        })
        // Copying or cutting a Collapsed Paste takes its words, not its label.
        .pill_copy_text(move |id| pastes.get(id).cloned())
        .pill_style(move |id| {
            if id.starts_with("skill:") {
                PillStyle::new().bg(skill_bg).fg(skill_ink)
            } else if pending.contains(id) {
                PillStyle::new().bg(chip).fg(faint)
            } else {
                PillStyle::new().bg(chip).fg(ink)
            }
        });
    if form == Form::C {
        let thumbs: HashMap<String, Pixels> =
            s.c.bound
                .iter()
                .filter_map(|(id, b)| Some((id.clone(), pixels_of(b)?)))
                .collect();
        area = area.pill_image(move |id| thumbs.get(id).cloned());
    }
    area.build(k).key("composer-field")
}

fn mini(label: &str, variant: ButtonVariant) -> Button {
    Button::new(label.to_owned())
        .variant(variant)
        .control_size(ControlSize::Mini)
}

/// The composer's buttons in an open Session: what Return does now.
pub fn acts(s: &App, k: &Kit) -> El {
    let typed = !s.draft.text.trim().is_empty();
    let mut acts = row().gap(6.0).center_y().preserve_focus_on_press(true);
    if s.phase.live() && !typed {
        let (label, keys, variant) = if s.c.armed {
            ("Stop", "esc again", ButtonVariant::Contrast)
        } else {
            ("Stop", "esc", ButtonVariant::Outline)
        };
        acts = acts.child(
            bits::small_button(label, variant, keys, k)
                .on_press(stop)
                .build(k)
                .key("stop"),
        );
    } else if s.phase.live() {
        acts = acts
            .child(
                bits::small_button("Queue", ButtonVariant::Ghost, queue_keys(s), k)
                    .on_press(|s: &mut App| submit(s, true))
                    .build(k)
                    .key("queue"),
            )
            .child(
                bits::small_button("Steer", ButtonVariant::Contrast, "⏎", k)
                    .on_press(|s: &mut App| submit(s, false))
                    .build(k)
                    .key("send"),
            );
    } else {
        acts = acts.child(
            bits::small_button("Send", ButtonVariant::Contrast, "⏎", k)
                .disabled(!typed)
                .on_press(|s: &mut App| submit(s, false))
                .build(k)
                .key("send"),
        );
    }
    acts
}

/// The whole composer: what stands over the box, and the box, as the form has them.
pub fn composer(
    s: &App,
    readings: El,
    acts: El,
    placeholder: &str,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
    let c = &k.c;
    let form = s.c.form;
    let foot = row()
        .w_full()
        .justify_between()
        .center_y()
        .child(readings)
        .child(acts);
    let mut inner = col()
        .key("composer")
        .w_full()
        .padding(10.0)
        .gap(8.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(
            1.0,
            if s.c.over && form != Form::B {
                c.info.border
            } else {
                c.border_base
            },
        )
        .on_drop_files(|s: &mut App, files: &[PathBuf], _: &mut Tx| take_files(s, files))
        .on_drop_hover(|s: &mut App, over: bool, _: &mut Tx| s.c.over = over);
    let field = field(s, placeholder, 1, 8, k);
    inner = match form {
        Form::A => inner.child(field).children(lines(s, k)).child(foot),
        Form::B => inner.children(tray(s, k)).child(field).child(foot),
        Form::C => {
            let list = (s.c.popup.is_some()).then(|| embedded(s, k));
            inner.children(list).child(field).child(foot)
        }
    };
    let boxed = pill_menu(s, inner, k, layer);
    if form == Form::B && s.c.popup.is_some() {
        floating(s, k, layer);
    }
    let mut stack = col().key("composer-stack").w_full().gap(8.0);
    if !s.c.queue.is_empty() && s.open.is_some() {
        stack = stack.child(match form {
            Form::A => pending(s, k),
            Form::B => cards(s, k),
            Form::C => folded(s, k),
        });
    }
    if form == Form::A && s.c.popup.is_some() {
        stack = stack.child(docked(s, k));
    }
    if let Some(id) = s.c.preview.as_deref().filter(|_| form == Form::C) {
        stack = stack.children(preview(s, id, k));
    }
    if let Some(note) = &s.c.note {
        stack = stack.child(refusal(note, k));
    }
    stack.child(boxed)
}

fn refusal(note: &str, k: &Kit) -> El {
    let c = &k.c;
    row()
        .key("composer-note")
        .w_full()
        .px(10.0)
        .py(6.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .fill(c.warning.bg)
        .child(
            text(note.to_owned())
                .font(k.ty.ui)
                .color(c.text_base)
                .wrap(),
        )
        .child(el().grow(1.0))
        .child(
            IconButton::new(k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .label("Dismiss")
                .on_press(|s: &mut App| {
                    s.c.note = None;
                    s.draft.focused = true;
                })
                .build(k),
        )
}

/// The menu a secondary press on a label opens.
fn pill_menu(s: &App, trigger: El, k: &Kit, layer: &mut Vec<El>) -> El {
    let id =
        s.c.menu
            .as_ref()
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
    let point =
        s.c.menu
            .as_ref()
            .map(|(_, p)| PointerPress { x: p.0, y: p.1 });
    let close = |s: &mut App| {
        s.c.menu = None;
        s.draft.focused = true;
    };
    let mut menu = context_menu("pill-menu", trigger, point)
        .label("Label")
        .min_width(200.0);
    if id.starts_with("paste:") {
        let of = id.clone();
        menu =
            menu.item(MenuItem::new("Expand paste").on_select(move |s: &mut App| expand(s, &of)));
    }
    let of = id.clone();
    menu = menu.item(
        MenuItem::new(if id.starts_with("image:") {
            "Remove attachment"
        } else {
            "Remove"
        })
        .on_select(move |s: &mut App| remove(s, &of)),
    );
    menu.on_close(close).build(k, layer)
}

// -- Attachments ------------------------------------------------------------------------------------------------

/// a: a line for each Attachment under the text, as the TUI has them.
fn lines(s: &App, k: &Kit) -> Option<El> {
    let c = &k.c;
    let found = attached(s);
    if found.is_empty() {
        return None;
    }
    let mut list = col()
        .key("attachment-lines")
        .w_full()
        .gap(1.0)
        .preserve_focus_on_press(true);
    for (id, _) in found {
        let Some(b) = s.c.bound.get(&id) else {
            continue;
        };
        let waits = matches!(
            b,
            Bound::Image {
                uploading: true,
                ..
            }
        );
        let of = id.clone();
        list = list.child(
            row()
                .w_full()
                .h(22.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .child(if waits {
                    Spinner::new().size(11.0).build(k)
                } else {
                    glyph('\u{f03e}', 12.0, c.icon_muted)
                })
                .child(
                    text(if waits {
                        format!("{} · uploading…", described(b))
                    } else {
                        described(b)
                    })
                    .font(k.ty.small)
                    .color(c.text_muted)
                    .truncate(),
                )
                .child(el().grow(1.0))
                .child(
                    IconButton::new(k.glyph.close)
                        .variant(IconButtonVariant::GhostMuted)
                        .label("Remove attachment")
                        .on_press(move |s: &mut App| remove(s, &of))
                        .build(k),
                ),
        );
    }
    Some(list)
}

/// b: the pictures themselves at the head of the box. Each still has its label in the text, which is where a
/// Prompt's words point at it.
fn tray(s: &App, k: &Kit) -> Option<El> {
    let c = &k.c;
    let found = attached(s);
    if found.is_empty() {
        return None;
    }
    let mut tray = row()
        .key("attachment-tray")
        .w_full()
        .px(8.0)
        .pt(4.0)
        .gap(10.0)
        .wrap()
        .preserve_focus_on_press(true);
    for (id, _) in found {
        let Some(b) = s.c.bound.get(&id) else {
            continue;
        };
        let Bound::Image { n, uploading, .. } = b else {
            continue;
        };
        let mut tile = el()
            .w(88.0)
            .h(60.0)
            .radius(k.radius.md)
            .border(1.0, c.border_base)
            .fill(c.layer_02)
            .center();
        if let Some(pixels) = pixels_of(b) {
            tile = tile.pixels_fit(&pixels, rune::SurfaceFit::Fill);
        }
        if *uploading {
            tile = tile.opacity(0.5).child(Spinner::new().size(14.0).build(k));
        }
        let of = id.clone();
        tray = tray.child(
            col().gap(3.0).child(tile).child(
                row()
                    .w(88.0)
                    .center_y()
                    .child(
                        text(format!("Image {n}"))
                            .font(k.ty.small)
                            .color(c.text_muted),
                    )
                    .child(el().grow(1.0))
                    .child(
                        IconButton::new(k.glyph.close)
                            .variant(IconButtonVariant::GhostMuted)
                            .label(format!("Remove image {n}"))
                            .on_press(move |s: &mut App| remove(s, &of))
                            .build(k),
                    ),
            ),
        );
    }
    Some(tray)
}

/// c: what a pressed label stands for, over the composer: the picture, or the first lines of a paste.
fn preview(s: &App, id: &str, k: &Kit) -> Option<El> {
    let c = &k.c;
    let b = s.c.bound.get(id)?;
    span_of(s, id)?;
    let of = id.to_owned();
    let also = id.to_owned();
    let mut card = col()
        .key("label-preview")
        .w_full()
        .padding(10.0)
        .gap(8.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_base)
        .preserve_focus_on_press(true);
    let mut actions = row().w_full().gap(6.0).center_y();
    match b {
        Bound::Image { .. } => {
            let mut picture = el()
                .w(320.0)
                .h(200.0)
                .radius(k.radius.md)
                .border(1.0, c.border_base)
                .fill(c.layer_02);
            if let Some(pixels) = pixels_of(b) {
                picture = picture.pixels_fit(&pixels, rune::SurfaceFit::Fit);
            }
            card = card.child(picture);
            actions = actions.child(text(described(b)).font(k.ty.small).color(c.text_muted));
        }
        Bound::Paste { text: words, n } => {
            let total = words.lines().count();
            let head: Vec<&str> = words.lines().take(8).collect();
            card = card.child(
                text(head.join("\n"))
                    .font(k.ty.mono)
                    .color(c.text_base)
                    .wrap(),
            );
            actions = actions.child(
                text(if total > 8 {
                    format!("Paste {n} · the first 8 of {total} lines")
                } else {
                    format!("Paste {n} · {total} lines")
                })
                .font(k.ty.small)
                .color(c.text_muted),
            );
        }
        Bound::Skill => return None,
    }
    actions = actions.child(el().grow(1.0));
    if matches!(b, Bound::Paste { .. }) {
        actions = actions.child(
            mini("Expand into the draft", ButtonVariant::Neutral)
                .on_press(move |s: &mut App| expand(s, &also))
                .build(k)
                .key("preview-expand"),
        );
    }
    actions = actions
        .child(
            mini("Remove", ButtonVariant::Ghost)
                .on_press(move |s: &mut App| remove(s, &of))
                .build(k)
                .key("preview-remove"),
        )
        .child(
            mini("Close", ButtonVariant::Ghost)
                .on_press(|s: &mut App| {
                    s.c.preview = None;
                    s.draft.focused = true;
                })
                .build(k),
        );
    Some(card.child(actions))
}

// -- Completions ------------------------------------------------------------------------------------------------

fn popup_title(s: &App) -> &'static str {
    match s.c.popup.as_ref().map(|p| p.kind) {
        Some(Completing::Skills) => "Skills",
        _ => "Commands",
    }
}

fn empty_words(s: &App) -> &'static str {
    match s.c.popup.as_ref().map(|p| p.kind) {
        Some(Completing::Skills) if s.c.skills == Catalog::Loading => "Loading skills…",
        Some(Completing::Skills) => "No skill by that name",
        _ => "No command by that name",
    }
}

/// a: the completions docked over the composer, as wide as it, with what the row the keys are on means beside the
/// list where there is room.
fn docked(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let found = matches(s);
    let active = s.c.popup.as_ref().map_or(0, |p| p.active);
    let mut list = col().grow(1.0).min_w(0.0).gap(1.0);
    for (i, item) in found.iter().enumerate() {
        let id = item.id.clone();
        let mut line = row()
            .key(format!("completion-{}", item.id))
            .w_full()
            .h(26.0)
            .px(8.0)
            .gap(10.0)
            .center_y()
            .radius(k.radius.sm)
            .child(
                text(item.label.clone())
                    .font(k.ty.mono)
                    .color(if item.disabled {
                        c.text_faint
                    } else {
                        c.text_base
                    }),
            )
            .child(
                row().grow(1.0).min_w(0.0).child(
                    text(item.about)
                        .font(k.ty.small)
                        .color(c.text_muted)
                        .truncate(),
                ),
            );
        if i == active {
            line = line.fill(c.layer_03);
        }
        list = list.child(if item.disabled {
            line
        } else {
            k.interactive(line)
                .role("option")
                .on_press(move |s: &mut App| accept(s, &id))
        });
    }
    if found.is_empty() {
        list = list.child(
            row()
                .h(26.0)
                .px(8.0)
                .center_y()
                .child(text(empty_words(s)).font(k.ty.small).color(c.text_muted)),
        );
    }
    let mut body = row().w_full().gap(10.0).child(list);
    if let (Some(item), true) = (found.get(active), s.win_w >= 900.0) {
        let mut about = col()
            .w(240.0)
            .padding(8.0)
            .gap(6.0)
            .radius(k.radius.md)
            .fill(c.layer_02)
            .child(text(item.label.clone()).font(k.ty.mono).color(c.text_base))
            .child(text(item.about).font(k.ty.small).color(c.text_muted).wrap());
        if let Some(note) = item.note {
            about = about.child(text(note).font(k.ty.small).color(c.text_faint).wrap());
        }
        body = body.child(about);
    }
    col()
        .key("completions")
        .w_full()
        .padding(8.0)
        .gap(4.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_base)
        .preserve_focus_on_press(true)
        .child(
            row()
                .w_full()
                .px(4.0)
                .justify_between()
                .child(
                    text(popup_title(s))
                        .font(k.ty.small_medium)
                        .color(c.text_muted),
                )
                .child(
                    text("↑↓ move · ⏎ or ⇥ take · esc close")
                        .font(k.ty.small)
                        .color(c.text_faint),
                ),
        )
        .child(body)
}

fn popup_items(s: &App, k: &Kit) -> Vec<PopupItem> {
    matches(s)
        .into_iter()
        .map(|item| {
            let mut row = PopupItem::new(item.id, item.label)
                .description(item.about)
                .disabled(item.disabled);
            if let Some(note) = item.note.filter(|_| !item.disabled) {
                let _ = note;
                row = row.trailing(text("…").font(k.ty.small).color(k.c.text_faint));
            }
            row
        })
        .collect()
}

fn popup_list(s: &App, k: &Kit, anchor: &str) -> PopupList<App> {
    PopupList::new("completions", anchor)
        .label(popup_title(s))
        .items(popup_items(s, k))
        .active(s.c.popup.as_ref().map(|p| p.active))
        .empty_message(empty_words(s))
        .on_active(|s: &mut App, i| {
            if let Some(p) = &mut s.c.popup {
                p.active = i;
            }
        })
        .on_choose(|s: &mut App, id: &str, _: &mut Tx| accept(s, id))
        .on_close(|s: &mut App| {
            let p = s.c.popup.take();
            s.c.shut = p.map(|p| (p.kind, p.start));
        })
}

/// b: Rune's own popup list, floating from the composer's leading corner. It takes no focus.
fn floating(s: &App, k: &Kit, layer: &mut Vec<El>) {
    popup_list(s, k, "composer")
        .side(Side::Top)
        .align(Align::Start)
        .offset(6.0)
        .width(420.0)
        .max_height(280.0)
        .build(k, layer);
}

/// c: the same list inside the composer's box, over the field.
fn embedded(s: &App, k: &Kit) -> El {
    col()
        .key("completions")
        .w_full()
        .gap(6.0)
        .child(popup_list(s, k, "").max_height(224.0).build_list(k))
        .child(bits::rule(k))
}

// -- Queued Prompts ---------------------------------------------------------------------------------------------

/// A draft as one line of words, its labels kept.
fn one_line(d: &Draft) -> String {
    d.text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// a: the TUI's Pending box. Every row's actions are on show, and the keys manage the queue as they do there.
fn pending(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let managing = s.c.managing;
    let mut list = col()
        .key("pending")
        .w_full()
        .padding(8.0)
        .gap(2.0)
        .radius(k.radius.lg)
        .border(1.0, c.border_muted)
        .preserve_focus_on_press(true)
        .child(
            row()
                .w_full()
                .px(4.0)
                .pb(2.0)
                .justify_between()
                .center_y()
                .child(text("Pending").font(k.ty.small_medium).color(c.text_muted))
                .child(if managing.is_some() {
                    text("↑↓ move · ⏎ steer now · ⌃D cancel · esc done")
                        .font(k.ty.small)
                        .color(c.text_faint)
                } else {
                    mini("Manage", ButtonVariant::Ghost)
                        .on_press(|s: &mut App| {
                            s.c.managing = Some(0);
                            s.draft.focused = true;
                        })
                        .build(k)
                        .key("pending-manage")
                }),
        );
    for (i, d) in s.c.queue.iter().enumerate() {
        let mut line = row()
            .key(format!("pending-{i}"))
            .w_full()
            .h(28.0)
            .px(6.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.sm)
            .child(
                text(format!("{}", i + 1))
                    .font(k.ty.small)
                    .color(c.text_faint),
            )
            .child(
                row().grow(1.0).min_w(0.0).child(
                    text(one_line(d))
                        .font(k.ty.ui)
                        .color(c.text_base)
                        .truncate(),
                ),
            )
            .child(
                mini("Steer now", ButtonVariant::Neutral)
                    .on_press(move |s: &mut App| steer_now(s, i))
                    .build(k)
                    .key(format!("pending-steer-{i}")),
            )
            .child(
                mini("Cancel", ButtonVariant::Ghost)
                    .on_press(move |s: &mut App| cancel(s, i))
                    .build(k)
                    .key(format!("pending-cancel-{i}")),
            );
        if managing == Some(i) {
            line = line.fill(c.layer_03);
        }
        list = list.child(line);
    }
    list
}

/// b: each queued Prompt a card of its own, whose actions come with the pointer.
fn cards(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut list = col()
        .key("pending")
        .w_full()
        .gap(6.0)
        .items_end()
        .preserve_focus_on_press(true);
    for (i, d) in s.c.queue.iter().enumerate() {
        let over = s.c.hover == Some(i);
        let mut card = row()
            .key(format!("pending-{i}"))
            .max_w(560.0)
            .px(12.0)
            .py(8.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.lg)
            .border(1.0, c.border_muted)
            .child(glyph('\u{f017}', 12.0, c.icon_faint))
            .child(
                row().shrink().min_w(0.0).child(
                    text(one_line(d))
                        .font(k.ty.ui)
                        .color(c.text_muted)
                        .truncate(),
                ),
            );
        if over {
            card = card
                .fill(c.layer_01)
                .child(
                    mini("Steer now", ButtonVariant::Neutral)
                        .on_press(move |s: &mut App| steer_now(s, i))
                        .build(k)
                        .key(format!("pending-steer-{i}")),
                )
                .child(
                    mini("Edit", ButtonVariant::Ghost)
                        .on_press(move |s: &mut App| edit(s, i))
                        .build(k)
                        .key(format!("pending-edit-{i}")),
                )
                .child(
                    IconButton::new(k.glyph.close)
                        .variant(IconButtonVariant::GhostMuted)
                        .label("Cancel this prompt")
                        .on_press(move |s: &mut App| cancel(s, i))
                        .build(k)
                        .key(format!("pending-cancel-{i}")),
                );
        }
        list = list.child(
            card.on_hover_change(move |s: &mut App, on: bool, _: &mut Tx| {
                if on {
                    s.c.hover = Some(i);
                } else if s.c.hover == Some(i) {
                    s.c.hover = None;
                }
            }),
        );
    }
    list
}

/// c: the queue folded to one line that says how many wait and what is next, opening onto the rest.
fn folded(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let n = s.c.queue.len();
    let open = s.c.queue_open;
    let head = row()
        .key("pending-head")
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .child(
            rune::icon(
                if open {
                    k.glyph.chevron_down
                } else {
                    k.glyph.caret_right
                },
                11.0,
            )
            .color(c.icon_faint),
        )
        .child(
            text(format!("{n} queued"))
                .font(k.ty.small_medium)
                .color(c.text_muted),
        )
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(if open {
                    String::new()
                } else {
                    format!("next: {}", one_line(&s.c.queue[0]))
                })
                .font(k.ty.small)
                .color(c.text_faint)
                .truncate(),
            ),
        );
    let mut list = col()
        .key("pending")
        .w_full()
        .gap(1.0)
        .preserve_focus_on_press(true)
        .child(
            k.interactive(head)
                .role("button")
                .expanded(open)
                .on_press(|s: &mut App| {
                    s.c.queue_open = !s.c.queue_open;
                    s.draft.focused = true;
                }),
        );
    if open {
        for (i, d) in s.c.queue.iter().enumerate() {
            list = list.child(
                row()
                    .key(format!("pending-{i}"))
                    .w_full()
                    .h(26.0)
                    .pl(27.0)
                    .pr(8.0)
                    .gap(6.0)
                    .center_y()
                    .child(
                        row().grow(1.0).min_w(0.0).child(
                            text(one_line(d))
                                .font(k.ty.ui)
                                .color(c.text_base)
                                .truncate(),
                        ),
                    )
                    .child(
                        mini("Steer now", ButtonVariant::Ghost)
                            .on_press(move |s: &mut App| steer_now(s, i))
                            .build(k)
                            .key(format!("pending-steer-{i}")),
                    )
                    .child(
                        mini("Edit", ButtonVariant::Ghost)
                            .on_press(move |s: &mut App| edit(s, i))
                            .build(k)
                            .key(format!("pending-edit-{i}")),
                    )
                    .child(
                        IconButton::new(k.glyph.close)
                            .variant(IconButtonVariant::GhostMuted)
                            .label("Cancel this prompt")
                            .on_press(move |s: &mut App| cancel(s, i))
                            .build(k)
                            .key(format!("pending-cancel-{i}")),
                    ),
            );
        }
    }
    list
}

// -- The Transcript, the reading and the strip ------------------------------------------------------------------

/// The Prompts sent from this composer, after the fake Transcript's own.
pub fn sent_rows(s: &App, k: &Kit) -> Vec<El> {
    let c = &k.c;
    s.c.sent
        .iter()
        .enumerate()
        .map(|(i, (d, how))| {
            let pills = d.spans.iter().map(|(r, id)| {
                let style = if id.starts_with("skill:") {
                    PillStyle::new().bg(c.info.bg).fg(c.info.fg)
                } else {
                    PillStyle::new().bg(c.layer_03).fg(c.text_base)
                };
                (r.clone(), style)
            });
            let mut bubble = col()
                .max_w(520.0)
                .shrink()
                .min_w(0.0)
                .padding(12.0)
                .gap(10.0)
                .radius(k.radius.lg)
                .fill(c.layer_02);
            let mut pictures = row().gap(8.0).wrap();
            let mut any = false;
            for (_, id) in &d.spans {
                if let Some(pixels) = s.c.bound.get(id).and_then(pixels_of) {
                    any = true;
                    pictures = pictures.child(
                        el().w(150.0)
                            .h(96.0)
                            .radius(k.radius.md)
                            .border(1.0, c.border_base)
                            .pixels_fit(&pixels, rune::SurfaceFit::Fill),
                    );
                }
            }
            if any {
                bubble = bubble.child(pictures);
            }
            bubble = bubble.child(
                text(d.text.trim_end().to_owned())
                    .font(k.ty.body)
                    .color(c.text_base)
                    .wrap()
                    .pills(pills),
            );
            let mut cell = col().items_end().gap(3.0).child(bubble);
            if *how == How::Steered {
                cell = cell.child(
                    text("steered the turn")
                        .font(k.ty.small)
                        .color(c.text_faint),
                );
            }
            row()
                .key(format!("sent-{i}"))
                .w_full()
                .justify_end()
                .child(cell)
        })
        .collect()
}

/// What the reading under the field says of a posture on its way.
pub fn posture_state(s: &App, k: &Kit) -> Option<El> {
    let c = &k.c;
    match s.c.applying {
        Applying::Applied => None,
        Applying::Applying => Some(
            row()
                .key("posture-state")
                .gap(5.0)
                .center_y()
                .child(Spinner::new().size(10.0).build(k))
                .child(text("applying…").font(k.ty.small).color(c.text_faint)),
        ),
        Applying::NextTurn => Some(
            text("posture from the next turn")
                .key("posture-state")
                .font(k.ty.small)
                .color(c.info.icon),
        ),
        Applying::Failed => Some(
            row()
                .key("posture-state")
                .gap(6.0)
                .center_y()
                .preserve_focus_on_press(true)
                .child(
                    text("posture not applied")
                        .font(k.ty.small)
                        .color(c.danger.fg),
                )
                .child(
                    mini("Try again", ButtonVariant::Ghost)
                        .on_press(|s: &mut App| {
                            posture_changed(s);
                            s.draft.focused = true;
                        })
                        .build(k)
                        .key("posture-retry"),
                ),
        ),
    }
}

/// b: what the main view shows while files are dragged over it.
pub fn drop_veil(k: &Kit) -> El {
    let c = &k.c;
    col()
        .key("drop-veil")
        .pin(Some(8.0), Some(8.0), Some(8.0), Some(8.0))
        .center()
        .gap(6.0)
        .radius(k.radius.lg)
        .fill(c.layer_01.alpha(0.92))
        .border(2.0, c.info.border)
        .child(glyph('\u{f03e}', 28.0, c.info.icon))
        .child(
            text("Drop to attach")
                .font(k.ty.body_medium)
                .color(c.text_base),
        )
        .child(
            text("Pictures are attached. Any other file is named by its path.")
                .font(k.ty.small)
                .color(c.text_muted),
        )
}

/// The draft as the shared model would hold it, and what the composer last did. Not part of any design.
pub fn strip(s: &App) -> String {
    let d = Draft::of(&s.draft);
    let mut spans = Vec::new();
    for (r, id) in &d.spans {
        let what = match s.c.bound.get(id) {
            Some(b @ Bound::Image { uploading, .. }) => {
                format!(
                    "{}{}",
                    described(b),
                    if *uploading { " (uploading)" } else { "" }
                )
            }
            Some(Bound::Paste { n, text }) => format!("Paste {n}, {} chars", text.chars().count()),
            Some(Bound::Skill) => format!("Skill {}", &d.text[r.clone()]),
            None => "?".into(),
        };
        spans.push(format!("{}..{} {what}", r.start, r.end));
    }
    let sends = d.unfolded(&s.c.bound);
    format!(
        "draft {} bytes, caret {}, sends {} chars │ bound: {} │ {}",
        d.text.len(),
        s.draft.focus,
        sends.text.chars().count(),
        if spans.is_empty() {
            "nothing".into()
        } else {
            spans.join(" · ")
        },
        if s.c.last.is_empty() {
            "—"
        } else {
            &s.c.last
        },
    )
}

// ---------------------------------------------------------------------------------------------------------------
// Poses, for the scenes and for the strip's buttons.

pub const FAKE_PASTE: &str = "error[E0502]: cannot borrow `self.rows` as mutable because it is also borrowed as immutable\n   --> crates/shaman-desktop/src/transcript.rs:412:17\n    |\n409 |         let first = self.rows.first();\n    |                     --------- immutable borrow occurs here\n...\n412 |                 self.rows.push(row);\n    |                 ^^^^^^^^^^^^^^^^^^^ mutable borrow occurs here\n413 |             }\n414 |             first\n    |             ----- immutable borrow later used here\n\nerror: could not compile `shaman-desktop` (lib) due to 1 previous error\nwarning: build failed, waiting for other jobs to finish...\n";

/// A picture on the draft, as a paste of one would leave it. `stored` has the Server already holding it.
pub fn fake_picture(s: &mut App, shot: bool, stored: bool) {
    let bytes = if shot { bits::SHOT } else { bits::SKETCH };
    let before: HashSet<String> = s.c.bound.keys().cloned().collect();
    attach(s, bytes.to_vec());
    if stored {
        for (id, b) in s.c.bound.iter_mut() {
            if let (false, Bound::Image { uploading, .. }) = (before.contains(id), b) {
                *uploading = false;
            }
        }
    }
}

fn type_in(s: &mut App, words: &str) {
    s.draft.insert(words);
    changed(s);
}

fn bind_skill(s: &mut App, name: &str) {
    type_in(s, &format!("${name}"));
    accept(s, name);
}

/// A draft holding one of each thing that can be bound into it.
pub fn pose_draft(s: &mut App) {
    bind_skill(s, "review");
    type_in(s, "the layout in ");
    fake_picture(s, false, true);
    type_in(s, "against what the build printed ");
    paste_text(s, FAKE_PASTE);
    type_in(s, "and say what differs");
    s.c.popup = None;
    s.c.last.clear();
}

pub fn pose_queue(s: &mut App) {
    s.phase = Phase::Working;
    for words in [
        "Then run the whole suite on the ramdisk and tell me what is slow",
        "After that, write the ADR for the pin",
    ] {
        s.c.queue.push(Draft {
            text: words.into(),
            spans: Vec::new(),
        });
    }
    let mut third = TextEdit::new("");
    std::mem::swap(&mut s.draft, &mut third);
    type_in(s, "And compare it with ");
    fake_picture(s, true, true);
    let d = Draft::of(&s.draft);
    s.c.queue.push(d);
    clear(s);
}

pub fn pose_sent(s: &mut App) {
    pose_draft(s);
    submit(s, false);
    type_in(s, "Measure it at 400 Messages too");
    submit(s, false);
}
