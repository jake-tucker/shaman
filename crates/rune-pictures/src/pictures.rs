//! Throwaway (#800): how pictures are shown in a Transcript.
//!
//! Three looks at the same fake Session, in the window the earlier prototypes chose. Each answers the ticket's three
//! open questions its own way: what a press on a picture does, how much of the Transcript a picture may take, and
//! what stands in for one the app cannot read.
//!
//! - **a, a viewer over the window.** A Message's picture is Rune's Markdown image as it comes: its own size up to
//!   the column's width, however tall. A press opens Rune's `Preview` over the whole window, which zooms and pans,
//!   and ← → step through the Session's pictures. What cannot be read keeps a frame where the picture would stand.
//! - **b, the sheet a diff opens in.** A Message's picture is a band at most 240 points tall with a line under it
//!   that names it, and an Attachment is a line with a small picture at its head. A press opens the sheet over the
//!   window, the Session's pictures down its side and the picture on Rune's `Stage`. What cannot be read is one row,
//!   as an Activity is.
//! - **c, in place.** Nothing opens over the window. A tall picture is cut at 480 points until a press shows all of
//!   it, a pressed Attachment grows to its Prompt's width, and a line under an opened picture offers to copy it,
//!   save it or open it outside the app. What cannot be read is Rune's own broken-image glyph and its words.
//!
//! The pictures come from four places, which is the ticket's first question and which no look disagrees about: an
//! Attachment on a Prompt, a path an agent wrote in a Message, a URL it wrote, and a picture a Tool Call returned.

use std::collections::HashSet;

use rune::{El, Image, ImageCache, Key, KeyEvent, SurfaceFit, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, ImageZoom, Kit, Markdown,
    Preview, Spinner, Stage, StageZoom,
};

use crate::parts::glyph;
use crate::{App, bits, over};

#[derive(Clone, Copy, PartialEq)]
pub enum Look {
    A,
    B,
    C,
}

impl Look {
    pub const ALL: [Look; 3] = [Look::A, Look::B, Look::C];

    pub fn name(self) -> &'static str {
        match self {
            Look::A => "A (a viewer over the window; a frame holds a lost picture's place)",
            Look::B => "B (bands and lines; the sheet a diff opens in; a lost picture is a row)",
            Look::C => "C (in place: a press shows all of it; a lost picture is its words)",
        }
    }

    pub fn step(self, by: usize) -> Look {
        let at = Self::ALL.iter().position(|v| *v == self).unwrap_or(0);
        Self::ALL[(at + by) % Self::ALL.len()]
    }
}

/// Where a picture comes from.
#[derive(Clone, Copy, PartialEq)]
pub enum From {
    /// An Attachment: the Server holds its bytes and its size is known before they arrive.
    Attachment,
    /// A path an agent wrote in a Message, on the machine its Server runs on.
    Path,
    /// A URL an agent wrote in a Message.
    Url,
    /// A picture a Tool Call returned.
    Tool,
}

pub struct Pic {
    /// The label, the alt text or the Tool.
    pub name: &'static str,
    /// What stands in the text: a label, a path or a URL.
    pub src: &'static str,
    pub from: From,
    pub bytes: &'static [u8],
    pub detail: &'static str,
    /// Why the Server cannot hand it over, where it cannot.
    pub lost: Option<&'static str>,
}

const COMMANDS: &[u8] = include_bytes!("../assets/commands.png");
const DARK: &[u8] = include_bytes!("../assets/dark.png");
const LANDING: &[u8] = include_bytes!("../assets/landing.png");
const NARROW: &[u8] = include_bytes!("../assets/narrow.png");
const TALL: &[u8] = include_bytes!("../assets/tall.png");
const SMALL: &[u8] = include_bytes!("../assets/small.png");

pub const PICS: &[Pic] = &[
    Pic {
        name: "Image 1",
        src: "[Image 1]",
        from: From::Attachment,
        bytes: COMMANDS,
        detail: "PNG · 1280×800 · 233 KiB",
        lost: None,
    },
    Pic {
        name: "The Sidebar at 960 pt with compact rows",
        src: "shots/sidebar-compact.png",
        from: From::Path,
        bytes: DARK,
        detail: "PNG · 1280×800 · 233 KiB",
        lost: None,
    },
    Pic {
        name: "Image 1",
        src: "[Image 1]",
        from: From::Attachment,
        bytes: LANDING,
        detail: "PNG · 1280×800 · 100 KiB",
        lost: None,
    },
    Pic {
        name: "Image 2",
        src: "[Image 2]",
        from: From::Attachment,
        bytes: NARROW,
        detail: "PNG · 1120×1040 · 107 KiB",
        lost: None,
    },
    Pic {
        name: "Image 3",
        src: "[Image 3]",
        from: From::Attachment,
        bytes: COMMANDS,
        detail: "JPEG · 1280×800 · 233 KiB",
        lost: None,
    },
    Pic {
        name: "Image 4",
        src: "[Image 4]",
        from: From::Attachment,
        bytes: DARK,
        detail: "PNG · 1280×800 · 233 KiB",
        lost: None,
    },
    Pic {
        name: "browser · screenshot",
        src: "http://localhost:5173/settings",
        from: From::Tool,
        bytes: LANDING,
        detail: "PNG · 1280×800 · 100 KiB",
        lost: None,
    },
    Pic {
        name: "The whole Sidebar",
        src: "shots/sidebar-whole.png",
        from: From::Path,
        bytes: TALL,
        detail: "PNG · 560×1600 · 190 KiB",
        lost: None,
    },
    Pic {
        name: "The glyph in question",
        src: "assets/glyph.png",
        from: From::Path,
        bytes: SMALL,
        detail: "PNG · 48×48 · 2 KiB",
        lost: None,
    },
    Pic {
        name: "Build status",
        src: "https://ci.example.com/shaman/main/badge.png",
        from: From::Url,
        bytes: SMALL,
        detail: "PNG · 48×48 · 2 KiB",
        lost: None,
    },
    Pic {
        name: "The sheet after the fix",
        src: "shots/after-fix.png",
        from: From::Path,
        bytes: SMALL,
        detail: "",
        lost: Some("No such file"),
    },
];

/// How a picture stands for the reader.
pub enum Status<'a> {
    Ready(&'a Image),
    Loading,
    /// The Server holds no such picture, or did not answer.
    Lost(&'static str),
    /// A URL's picture, which nothing fetches until the reader asks.
    Unfetched,
    /// `transcript.imagePreviews` is off.
    Off,
}

pub struct State {
    pub look: Look,
    cache: Option<ImageCache>,
    images: Vec<Image>,
    /// The picture on show over the window, in a and b.
    pub viewing: Option<usize>,
    pub zoom: ImageZoom,
    pub stage: StageZoom,
    /// c: the pictures a press has grown.
    pub grown: HashSet<usize>,
    /// Whether the reader asked for the URL's picture.
    pub fetched: bool,
    /// Whether every picture is still on its way.
    pub slow: bool,
    /// Whether the Server that holds the pictures has stopped answering, as a Remote may.
    pub cut: bool,
    /// Whether `transcript.imagePreviews` is off.
    pub off: bool,
    /// Whether the Tool Call that returned a picture is open.
    pub tool_open: bool,
}

impl State {
    pub fn new(look: Look) -> Self {
        Self {
            look,
            cache: None,
            images: Vec::new(),
            viewing: None,
            zoom: ImageZoom::new(),
            stage: StageZoom::default(),
            grown: HashSet::new(),
            fetched: false,
            slow: false,
            cut: false,
            off: false,
            tool_open: true,
        }
    }

    /// Starts over in the same look, keeping what was decoded.
    pub fn reset(&mut self) {
        let mut fresh = State::new(self.look);
        fresh.cache = self.cache.take();
        fresh.images = std::mem::take(&mut self.images);
        *self = fresh;
    }

    pub fn load(&mut self, cache: ImageCache) {
        self.images = PICS.iter().map(|p| cache.load(p.bytes)).collect();
        // A picture taken headless is of the first frame, so it waits for the pixels.
        if rune::is_headless() {
            for image in &self.images {
                let _ = image.wait();
            }
        }
        self.cache = Some(cache);
    }

    pub fn status(&self, at: usize) -> Status<'_> {
        let pic = &PICS[at];
        if self.off {
            return Status::Off;
        }
        if pic.from == From::Url && !self.fetched {
            return Status::Unfetched;
        }
        if self.cut {
            return Status::Lost("jake-mini is not answering");
        }
        if let Some(why) = pic.lost {
            return Status::Lost(why);
        }
        if self.slow {
            return Status::Loading;
        }
        match self.images.get(at) {
            Some(image) if image.is_ready() => Status::Ready(image),
            _ => Status::Loading,
        }
    }

    fn ready(&self, at: usize) -> Option<&Image> {
        match self.status(at) {
            Status::Ready(image) => Some(image),
            _ => None,
        }
    }

    pub fn view(&mut self, at: usize) {
        self.viewing = Some(at);
        self.zoom = ImageZoom::new();
        self.stage = StageZoom::default();
    }

    /// Steps the picture on show to the next one that can be read.
    fn step(&mut self, forward: bool) {
        let Some(from) = self.viewing else { return };
        let n = PICS.len();
        let mut at = from;
        for _ in 0..n {
            at = if forward {
                (at + 1) % n
            } else {
                (at + n - 1) % n
            };
            if self.ready(at).is_some() {
                self.view(at);
                return;
            }
        }
    }

    pub fn toggle(&mut self, at: usize) {
        if !self.grown.remove(&at) {
            self.grown.insert(at);
        }
    }
}

pub fn poll(s: &mut App, now: f64, tx: &mut Tx) -> bool {
    s.p.zoom.poll(now, tx)
}

/// What the strip under the window says of the pictures.
pub fn strip(s: &App) -> String {
    let p = &s.p;
    let on = |b: bool| if b { "yes" } else { "no" };
    format!(
        "viewing: {}  ·  grown: {}  ·  URL fetched: {}  ·  slow: {}  ·  Server answering: {}  ·  previews: {}",
        p.viewing
            .map_or("nothing".to_owned(), |at| PICS[at].src.to_owned()),
        p.grown.len(),
        on(p.fetched),
        on(p.slow),
        on(!p.cut),
        if p.off { "off" } else { "on" },
    )
}

const FILE: char = '\u{eaea}';
const LINK: char = '\u{eb15}';
const WARN: char = '\u{ea6c}';

fn origin(from: From) -> &'static str {
    match from {
        From::Attachment => "Attachment",
        From::Path => "A file in the Worktree",
        From::Url => "A URL",
        From::Tool => "A Tool Call's result",
    }
}

fn natural(image: &Image) -> [f32; 2] {
    image
        .size()
        .map_or([1.0, 1.0], |[w, h]| [w as f32, h as f32])
}

fn small(label: &str, k: &Kit) -> Button {
    Button::new(label.to_owned())
        .variant(ButtonVariant::Neutral)
        .control_size(ControlSize::Mini)
}

// ---------------------------------------------------------------------------------------------------------------
// The Transcript.

const P1: &str = "[Image 1] The Sidebar's rows look cramped at 960 pt. What is taking the room?";
const A1_HEAD: &str = "The rows keep **Detailed**'s three lines whatever the width, so at 960 pt the Sidebar \
shows nine Sessions where it could show nineteen. With `sidebar.rowDensity` on `compact` it reads like this:";
const A1_FOOT: &str = "Nothing else in the Sidebar gives way: the search box, the selector and the shelf keep \
their heights.";
const P2: &str = "Compare [Image 1] [Image 2] [Image 3] [Image 4] and say which the settings sheet should follow.";
const A2_HEAD: &str = "The second is the one to follow: it keeps one scrolling list where the others split \
the Settings over tabs. I took the Sidebar whole, to see the strip at its foot:";
const A2_SMALL: &str =
    "The gear is this glyph, which is drawn at its own size and not stretched to the column:";
const A2_URL: &str = "CI's badge for `main`, which is a URL and not a file on this machine:";
const A2_LOST: &str = "And the sheet after the fix, which I named but never wrote:";
const A2_FOOT: &str = "So: follow the second, and move the gear to the strip's leading edge.";

pub fn rows(s: &App, mut rows: El, k: &Kit) -> El {
    rows = rows.child(prompt(s, P1, &[0], k));
    rows = rows.child(marker("Worked for 48s", "3 reads · 1 command", k));
    rows = rows.child(bits::md(A1_HEAD, false, k).key("a1-head"));
    rows = rows.child(picture(s, 1, k));
    rows = rows.child(bits::md(A1_FOOT, false, k).key("a1-foot"));

    rows = rows.child(prompt(s, P2, &[2, 3, 4, 5], k));
    rows = rows.child(marker("Worked for 1m 12s", "2 reads · 1 tool call", k));
    rows = rows.child(tool(s, k));
    rows = rows.child(bits::md(A2_HEAD, false, k).key("a2-head"));
    rows = rows.child(picture(s, 7, k));
    rows = rows.child(bits::md(A2_SMALL, false, k).key("a2-small"));
    rows = rows.child(picture(s, 8, k));
    rows = rows.child(bits::md(A2_URL, false, k).key("a2-url"));
    rows = rows.child(picture(s, 9, k));
    rows = rows.child(bits::md(A2_LOST, false, k).key("a2-lost"));
    rows = rows.child(picture(s, 10, k));
    rows.child(bits::md(A2_FOOT, false, k).key("a2-foot"))
}

fn marker(word: &str, counts: &str, k: &Kit) -> El {
    let c = &k.c;
    row()
        .w_full()
        .h(26.0)
        .px(6.0)
        .gap(8.0)
        .center_y()
        .child(rune::icon(k.glyph.chevron_down, 12.0).color(c.icon_muted))
        .child(
            text(word.to_owned())
                .font(k.ty.small_medium)
                .color(c.text_muted),
        )
        .child(text(counts.to_owned()).font(k.ty.small).color(c.text_faint))
        .child(el().grow(1.0).h(1.0).fill(c.border_muted))
}

/// A Prompt and its Attachments.
fn prompt(s: &App, said: &str, attached: &[usize], k: &Kit) -> El {
    let c = &k.c;
    let bubble = col()
        .max_w(520.0)
        .shrink()
        .min_w(0.0)
        .padding(12.0)
        .gap(10.0)
        .radius(k.radius.lg)
        .fill(c.layer_02)
        .child(attachments(s, attached, k))
        .child(
            text(said.to_owned())
                .font(k.ty.body)
                .color(c.text_base)
                .wrap(),
        );
    row().w_full().justify_end().child(bubble)
}

/// The Tool Call that returned a picture, opened under its row as an Activity opens.
fn tool(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let pic = &PICS[6];
    let open = s.p.tool_open;
    let line = k
        .interactive(
            row()
                .key("tool-row")
                .w_full()
                .h(28.0)
                .px(6.0)
                .gap(8.0)
                .center_y()
                .radius(k.radius.sm)
                .child(glyph(shaman_icons::activity::TOOL_CALL, 13.0, c.icon_muted))
                .child(text(pic.name).font(k.ty.mono).color(c.text_base))
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(pic.src).font(k.ty.mono).color(c.text_faint).truncate()),
                )
                .child(
                    text("1 picture · 1.2s")
                        .font(k.ty.small)
                        .color(c.text_faint),
                ),
        )
        .role("button")
        .expanded(open)
        .on_press(|s: &mut App| s.p.tool_open = !s.p.tool_open);
    let mut work = col().w_full().gap(8.0).child(line);
    if open {
        work = work.child(col().w_full().pl(27.0).child(picture(s, 6, k)));
    }
    row()
        .w_full()
        .gap(12.0)
        .child(el().w(2.0).self_stretch().radius(1.0).fill(c.border_base))
        .child(col().grow(1.0).min_w(0.0).child(work))
}

// ---------------------------------------------------------------------------------------------------------------
// An Attachment in its Prompt.

fn attachments(s: &App, attached: &[usize], k: &Kit) -> El {
    match s.p.look {
        Look::B => {
            let mut lines = col().w_full().gap(4.0);
            for &at in attached {
                lines = lines.child(attachment_line(s, at, k));
            }
            lines
        }
        look => {
            let alone = attached.len() == 1;
            let mut tiles = row().w_full().gap(8.0).wrap();
            let mut grown = col().w_full().gap(8.0);
            for &at in attached {
                if look == Look::C && s.p.grown.contains(&at) {
                    if let Some(image) = s.p.ready(at) {
                        grown = grown.child(
                            el().key(format!("grown-{at}"))
                                .role("image")
                                .label(PICS[at].name)
                                .w_full()
                                .radius(k.radius.md)
                                .border(1.0, k.c.border_base)
                                .pixels(image.pixels())
                                .cursor(rune::Cursor::ZoomOut)
                                .on_press(move |s: &mut App| s.p.toggle(at)),
                        );
                        continue;
                    }
                }
                tiles = tiles.child(tile(s, at, alone, k));
            }
            col().w_full().gap(8.0).child(tiles).child(grown)
        }
    }
}

/// a and c: an Attachment as a thumbnail.
fn tile(s: &App, at: usize, alone: bool, k: &Kit) -> El {
    let c = &k.c;
    let (w, h) = if alone { (220.0, 138.0) } else { (116.0, 76.0) };
    let frame = el()
        .key(format!("tile-{at}"))
        .w(w)
        .h(h)
        .radius(k.radius.md)
        .border(1.0, c.border_base)
        .fill(c.layer_03)
        .center();
    match s.p.status(at) {
        Status::Ready(image) => frame
            .role("image")
            .label(PICS[at].name)
            .pixels_fit(image.pixels(), SurfaceFit::Fill)
            .cursor(rune::Cursor::ZoomIn)
            .on_press(move |s: &mut App| match s.p.look {
                Look::C => s.p.toggle(at),
                _ => s.p.view(at),
            }),
        Status::Loading => frame.child(Spinner::new().size(14.0).build(k)),
        // c: a lost Attachment is named, as one is where previews are off.
        Status::Lost(_) | Status::Off if s.p.look == Look::C => attachment_line(s, at, k),
        Status::Off => attachment_line(s, at, k),
        Status::Lost(why) => frame.help(why).child(
            col()
                .items_center()
                .gap(2.0)
                .child(glyph(WARN, 14.0, c.icon_muted))
                .child(text(PICS[at].name).font(k.ty.small).color(c.text_muted)),
        ),
        Status::Unfetched => frame,
    }
}

/// b: an Attachment as a line, a small picture at its head. Also how every look names one it does not draw.
fn attachment_line(s: &App, at: usize, k: &Kit) -> El {
    let c = &k.c;
    let pic = &PICS[at];
    let head = el()
        .size(28.0, 28.0)
        .radius(k.radius.sm)
        .border(1.0, c.border_base)
        .fill(c.layer_03)
        .center();
    let (head, said) = match s.p.status(at) {
        Status::Ready(image) if s.p.look == Look::B => (
            head.pixels_fit(image.pixels(), SurfaceFit::Fill),
            pic.detail.to_owned(),
        ),
        Status::Loading if s.p.look == Look::B => (
            head.child(Spinner::new().size(11.0).build(k)),
            pic.detail.to_owned(),
        ),
        Status::Lost(why) => (
            head.child(glyph(WARN, 12.0, c.icon_muted)),
            format!("{} · {why}", pic.detail),
        ),
        _ => (
            head.child(glyph(FILE, 12.0, c.icon_muted)),
            pic.detail.to_owned(),
        ),
    };
    let line = row()
        .key(format!("attachment-{at}"))
        .w_full()
        .gap(8.0)
        .center_y()
        .radius(k.radius.sm)
        .child(head)
        .child(text(pic.name).font(k.ty.small_medium).color(c.text_base))
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .child(text(said).font(k.ty.small).color(c.text_muted).truncate()),
        );
    if s.p.look == Look::B && s.p.ready(at).is_some() {
        k.interactive(line)
            .role("button")
            .on_press(move |s: &mut App| s.p.view(at))
    } else {
        line
    }
}

// ---------------------------------------------------------------------------------------------------------------
// A picture in a Message, or under the Tool Call that returned it.

/// How tall b lets a picture stand in the Transcript, and c until it is pressed.
const BAND: f32 = 240.0;
const CUT: f32 = 480.0;
/// The widest a picture is drawn: the Session Content Column.
const COLUMN: f32 = 760.0;

fn picture(s: &App, at: usize, k: &Kit) -> El {
    let pic = &PICS[at];
    match s.p.status(at) {
        Status::Ready(image) => match s.p.look {
            Look::A => whole(at, image, k),
            Look::B => band(at, image, k),
            Look::C => in_place(s, at, image, k),
        },
        Status::Off => named(at, "Previews are off", None, k),
        status => match s.p.look {
            Look::A => frame(s, at, &status, k),
            Look::B => match status {
                Status::Loading => named(at, "Reading…", None, k),
                Status::Unfetched => named(
                    at,
                    "ci.example.com · not fetched",
                    Some(("Load picture", true)),
                    k,
                ),
                Status::Lost(why) => named(at, why, Some(("Try again", false)), k),
                _ => el(),
            },
            Look::C => match status {
                // The TUI's: a URL's picture stays a link, and nothing fetches it.
                Status::Unfetched => {
                    bits::md(&format!("[image: {}]({})", pic.name, pic.src), false, k)
                        .key(format!("link-{at}"))
                }
                // Rune's own stand-in for an image with no picture: the broken-image glyph and the alt text.
                Status::Lost(why) => Markdown::new(format!("![{}]({})", pic.name, pic.src))
                    .build(k)
                    .key(format!("broken-{at}"))
                    .help(format!("{} · {why}", pic.src)),
                _ => Markdown::new(format!("![{}]({})", pic.name, pic.src))
                    .build(k)
                    .key(format!("broken-{at}")),
            },
        },
    }
}

/// a: Rune's Markdown image as it comes, its own size up to the column's width, and a press on it opens the viewer.
fn whole(at: usize, image: &Image, k: &Kit) -> El {
    let pic = &PICS[at];
    let image = image.clone();
    Markdown::new(format!("![{}]({})", pic.name, pic.src))
        .images(move |_| Some(image.clone()))
        .image_frame(move |shown, _| {
            shown
                .key(format!("picture-{at}"))
                .on_press(move |s: &mut App| s.p.view(at))
        })
        .build(k)
}

/// b: a band no taller than `BAND`, cut from the picture's head, over a line that names it.
fn band(at: usize, image: &Image, k: &Kit) -> El {
    let c = &k.c;
    let pic = &PICS[at];
    let [w, h] = natural(image);
    let drawn_w = w.min(COLUMN);
    let cut = h * drawn_w / w > BAND;
    let mut shown = el()
        .key(format!("picture-{at}"))
        .role("image")
        .label(pic.name)
        .w_full()
        .max_w(w)
        .radius(k.radius.md)
        .border(1.0, c.border_base)
        .cursor(rune::Cursor::ZoomIn)
        .on_press(move |s: &mut App| s.p.view(at));
    shown = if cut {
        shown
            .h(BAND)
            .pixels_fit(image.pixels(), SurfaceFit::Fill)
            .surface_anchor(rune::Align::Center, rune::Align::Start)
    } else {
        shown.pixels(image.pixels())
    };
    let line = k
        .interactive(
            row()
                .w_full()
                .h(24.0)
                .px(4.0)
                .gap(8.0)
                .center_y()
                .radius(k.radius.sm)
                .child(glyph(FILE, 12.0, c.icon_muted))
                .child(text(pic.name).font(k.ty.small_medium).color(c.text_muted))
                .child(
                    row().grow(1.0).min_w(0.0).child(
                        text(format!("{} · {}", pic.src, pic.detail))
                            .font(k.ty.small)
                            .color(c.text_faint)
                            .truncate(),
                    ),
                )
                .children(cut.then(|| {
                    text("cut · open to see all")
                        .font(k.ty.small)
                        .color(c.text_faint)
                })),
        )
        .role("button")
        .on_press(move |s: &mut App| s.p.view(at));
    col().w_full().gap(4.0).child(shown).child(line)
}

/// c: the picture where it stands, cut at `CUT` until a press shows all of it.
fn in_place(s: &App, at: usize, image: &Image, k: &Kit) -> El {
    let c = &k.c;
    let pic = &PICS[at];
    let [w, h] = natural(image);
    let drawn_w = w.min(COLUMN);
    let tall = h * drawn_w / w > CUT;
    let grown = s.p.grown.contains(&at);
    let mut shown = el()
        .key(format!("picture-{at}"))
        .role("image")
        .label(pic.name)
        .w_full()
        .max_w(w)
        .radius(k.radius.md)
        .border(1.0, c.border_base);
    if tall {
        shown = shown
            .cursor(if grown {
                rune::Cursor::ZoomOut
            } else {
                rune::Cursor::ZoomIn
            })
            .on_press(move |s: &mut App| s.p.toggle(at));
    }
    shown = if tall && !grown {
        shown
            .h(CUT)
            .pixels_fit(image.pixels(), SurfaceFit::Fill)
            .surface_anchor(rune::Align::Center, rune::Align::Start)
    } else {
        shown.pixels(image.pixels())
    };
    let act = |label: &str| text(label.to_owned()).font(k.ty.small).color(c.text_muted);
    let mut line = row().w_full().h(22.0).px(4.0).gap(12.0).center_y();
    if tall {
        line = line.child(
            text(if grown {
                "Show less".to_owned()
            } else {
                format!("Show all {h:.0} px")
            })
            .font(k.ty.small_medium)
            .color(c.text_muted),
        );
    }
    line = line
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(pic.detail)
                    .font(k.ty.small)
                    .color(c.text_faint)
                    .truncate(),
            ),
        )
        .child(act("Copy"))
        .child(act("Save as…"))
        .child(act("Open outside"));
    col()
        .w_full()
        .gap(4.0)
        .child(shown)
        .child(line)
        .fold_glide(bits::FOLD)
}

/// a: a frame where the picture would stand, saying why it does not.
fn frame(s: &App, at: usize, status: &Status<'_>, k: &Kit) -> El {
    let c = &k.c;
    let pic = &PICS[at];
    // An Attachment's size and a Tool Call's are known before their bytes arrive; a path's is not.
    let ratio = match pic.from {
        From::Attachment | From::Tool => 1.6,
        _ => 2.4,
    };
    let said = |words: String| text(words).font(k.ty.small).color(c.text_muted);
    let mut inner = col().items_center().gap(6.0);
    inner = match status {
        Status::Loading => inner
            .child(Spinner::new().size(16.0).build(k))
            .child(said(format!("Reading {}", pic.src))),
        Status::Unfetched => inner
            .child(glyph(LINK, 16.0, c.icon_muted))
            .child(said(format!(
                "{} · a picture from ci.example.com, not fetched",
                pic.name
            )))
            .child(
                small("Load picture", k)
                    .on_press(|s: &mut App| s.p.fetched = true)
                    .build(k)
                    .key(format!("load-{at}")),
            ),
        Status::Lost(why) => inner
            .child(glyph(WARN, 16.0, c.icon_muted))
            .child(said(format!("{} · {}", pic.name, pic.src)))
            .child(said((*why).to_owned()))
            .child(
                small("Try again", k)
                    .on_press(|_: &mut App| {})
                    .build(k)
                    .key(format!("retry-{at}")),
            ),
        _ => inner,
    };
    let _ = s;
    el().key(format!("frame-{at}"))
        .w_full()
        .max_w(420.0)
        .aspect_ratio(ratio)
        .radius(k.radius.md)
        .border(1.0, c.border_base)
        .fill(c.layer_02)
        .center()
        .child(inner)
}

/// b, and every look where previews are off: one row that names a picture and says why it is not drawn.
fn named(at: usize, why: &str, action: Option<(&str, bool)>, k: &Kit) -> El {
    let c = &k.c;
    let pic = &PICS[at];
    let mut line = row()
        .key(format!("named-{at}"))
        .w_full()
        .h(30.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.sm)
        .border(1.0, c.border_muted)
        .child(glyph(
            if pic.from == From::Url { LINK } else { FILE },
            13.0,
            c.icon_muted,
        ))
        .child(text(pic.name).font(k.ty.small_medium).color(c.text_base))
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(format!("{} · {why}", pic.src))
                    .font(k.ty.small)
                    .color(c.text_faint)
                    .truncate(),
            ),
        );
    if let Some((label, fetches)) = action {
        line = line.child(
            small(label, k)
                .on_press(move |s: &mut App| {
                    if fetches {
                        s.p.fetched = true;
                    }
                })
                .build(k)
                .key(format!("act-{at}")),
        );
    }
    line
}

// ---------------------------------------------------------------------------------------------------------------
// Over the window.

pub fn overlays(s: &App, k: &Kit, layer: &mut Vec<El>) {
    let Some(at) = s.p.viewing else { return };
    let Some(image) = s.p.ready(at) else { return };
    let close = |s: &mut App| s.p.viewing = None;
    match s.p.look {
        Look::A => {
            layer.push(rune_kit::scrim("viewer-scrim", close).fill(k.c.scrim));
            let shown = el().key("viewer-picture").pixels(image.pixels());
            layer.push(
                Preview::new(|s: &mut App| &mut s.p.zoom, natural(image), shown)
                    .on_close(|s: &mut App, _| s.p.viewing = None)
                    .on_key(|s: &mut App, ev: &KeyEvent, _| match ev.key {
                        Key::Left => {
                            s.p.step(false);
                            true
                        }
                        Key::Right => {
                            s.p.step(true);
                            true
                        }
                        _ => false,
                    })
                    .build(&s.p.zoom, k, [s.win_w, s.win_h], false),
            );
            layer.push(caption(s, at, k));
        }
        Look::B => {
            layer.push(rune_kit::scrim("sheet-scrim", close).fill(k.c.scrim));
            layer.push(sheet(s, at, image, k));
        }
        Look::C => {}
    }
}

/// a: what the viewer says over the picture, which Rune's preview has no place for.
fn caption(s: &App, at: usize, k: &Kit) -> El {
    let pic = &PICS[at];
    let readable: Vec<usize> = (0..PICS.len())
        .filter(|i| s.p.ready(*i).is_some())
        .collect();
    let place = readable.iter().position(|i| *i == at).map_or(0, |i| i + 1);
    row()
        .key("viewer-caption")
        .pin(Some(0.0), Some(10.0), Some(0.0), None)
        .justify_center()
        .child(
            row()
                .h(26.0)
                .px(10.0)
                .gap(10.0)
                .center_y()
                .radius(13.0)
                .fill(k.c.bg_inverse)
                .child(
                    text(pic.name)
                        .font(k.ty.small_medium)
                        .color(k.c.text_inverse),
                )
                .child(
                    text(format!(
                        "{} · {} of {} · ← →",
                        pic.detail,
                        place,
                        readable.len()
                    ))
                    .font(k.ty.small)
                    .color(k.c.text_inverse),
                ),
        )
}

/// b: the sheet a diff opens in, the Session's pictures down its side and this one on a stage that zooms.
fn sheet(s: &App, at: usize, image: &Image, k: &Kit) -> El {
    let c = &k.c;
    let pic = &PICS[at];
    let width = (s.win_w - 200.0).min(1040.0);
    let height = (s.win_h - 120.0).min(720.0);
    let mut side = col()
        .w(260.0)
        .h_full()
        .padding(8.0)
        .gap(2.0)
        .fill(c.layer_01)
        .border_sides([0.0, 1.0, 0.0, 0.0], c.border_muted)
        .child(
            row().px(6.0).pb(6.0).child(
                text("This session's pictures")
                    .font(k.ty.small_medium)
                    .color(c.text_muted),
            ),
        );
    for (i, other) in PICS.iter().enumerate() {
        let Some(thumb) = s.p.ready(i) else { continue };
        let mut line = row()
            .key(format!("sheet-pic-{i}"))
            .w_full()
            .h(40.0)
            .px(6.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.sm)
            .child(
                el().size(44.0, 30.0)
                    .radius(k.radius.sm)
                    .border(1.0, c.border_base)
                    .pixels_fit(thumb.pixels(), SurfaceFit::Fill),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        text(other.name)
                            .font(k.ty.small_medium)
                            .color(c.text_base)
                            .truncate(),
                    )
                    .child(
                        text(origin(other.from))
                            .font(k.ty.small)
                            .color(c.text_faint)
                            .truncate(),
                    ),
            );
        if i == at {
            line = line.fill(c.layer_03);
        }
        side = side.child(
            k.interactive(line)
                .role("button")
                .on_press(move |s: &mut App| s.p.view(i)),
        );
    }
    let head = row()
        .w_full()
        .h(48.0)
        .px(16.0)
        .gap(10.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], c.border_muted)
        .child(glyph(FILE, 14.0, c.icon_muted))
        .child(
            row().grow(1.0).min_w(0.0).gap(10.0).center_y().child(
                text(format!("{} · {}", pic.name, pic.src))
                    .font(k.ty.body)
                    .color(c.text_base)
                    .truncate(),
            ),
        )
        .child(text(pic.detail).font(k.ty.small).color(c.text_faint))
        .child(small("Copy", k).on_press(|_: &mut App| {}).build(k))
        .child(small("Save as…", k).on_press(|_: &mut App| {}).build(k))
        .child(
            IconButton::new(k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .label("Close")
                .shortcut("esc")
                .on_press(|s: &mut App| s.p.viewing = None)
                .build(k),
        );
    let room = [width - 260.0, height - 48.0];
    let shown = el().key("stage-picture").pixels(image.pixels());
    let body = col().w_full().grow(1.0).min_h(0.0).fill(c.bg_deep).child(
        Stage::new(|s: &mut App| &mut s.p.stage, natural(image), shown).build(&s.p.stage, k, room),
    );
    over::centred(
        over::sheet("picture-sheet", width, k).h(height).child(
            row()
                .w_full()
                .h_full()
                .child(side)
                .child(col().grow(1.0).min_w(0.0).h_full().child(head).child(body)),
        ),
    )
}
