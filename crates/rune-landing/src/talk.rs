//! Fake data: one open Session's Transcript, in the units both Clients share (a Message, an Activity, a Group and
//! its members, a Turn Fold). Nothing here is read from anywhere.

/// How an Activity or a Turn stands.
#[derive(Clone, Copy, PartialEq)]
pub enum Outcome {
    Working,
    Worked,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// A shell command and what it printed.
    Command,
    /// A Tool Call that read a file.
    Read,
    /// Any other Tool Call.
    Tool,
    /// A File Change, drawn as a diff.
    Edit,
}

pub struct Activity {
    pub id: &'static str,
    pub kind: Kind,
    /// The command, the Tool and its input, or the path.
    pub head: &'static str,
    /// The right slot: a duration, a line range, or what an edit added and removed.
    pub slot: &'static str,
    pub outcome: Outcome,
    /// What it printed or read; for an edit, the text before.
    pub body: &'static str,
    /// For an edit, the text after.
    pub after: &'static str,
}

pub enum Unit {
    /// An agent Message that is not its Turn's last.
    Message(&'static str),
    /// A work Group: adjacent commands, Tool Calls and File Changes.
    Group(&'static str, &'static [Activity]),
    /// One stretch of a Subagent's work.
    Subagent {
        id: &'static str,
        name: &'static str,
        title: &'static str,
        slot: &'static str,
        outcome: Outcome,
    },
    /// A user Message that steered the Turn.
    Steer(&'static str),
    /// An Approval already decided, as the Transcript keeps it.
    Decided(&'static str, &'static str),
}

pub struct Turn {
    pub id: &'static str,
    pub prompt: &'static str,
    /// Whether the Prompt carries the one fake Attachment.
    pub attachment: bool,
    pub work: &'static [Unit],
    /// The Turn's final agent Message.
    pub answer: &'static str,
    /// Whether the answer shows the one fake image.
    pub picture: bool,
    pub outcome: Outcome,
    pub duration: &'static str,
    /// What the work Groups count, as a Turn Fold's marker could say it.
    pub counts: &'static str,
}

const READ_PARTS: &str = "\
/// A stand-in for an open Session's Transcript and composer, or for the Landing where none is open.
pub fn main_view(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let Some(x) = s.open.and_then(data::session) else {
        return landing(s, k);
    };
    let para = |t: &str| text(t.to_owned()).font(k.ty.body).color(c.text_base).wrap();";

const CHECK_OUT: &str = "\
    Checking shaman-client-model v0.4.2 (crates/shaman-client-model)
    Checking shaman-desktop v0.4.2 (crates/shaman-desktop)
warning: unused variable: `overscan`
  --> crates/shaman-desktop/src/transcript.rs:41:9
   |
41 |     let overscan = 4;
   |         ^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_overscan`
   |
   = note: `#[warn(unused_variables)]` on by default

warning: `shaman-desktop` (lib) generated 1 warning
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.21s";

const TEST_OUT: &str = "\
    Starting 14 tests across 2 binaries
        PASS [   0.004s] shaman-client-model transcript::tests::a_unit_keeps_its_key_across_updates
        PASS [   0.004s] shaman-client-model transcript::tests::a_group_stands_where_its_first_member_began
        PASS [   0.005s] shaman-client-model transcript::tests::a_turn_fold_hides_the_work_between_prompt_and_answer
        PASS [   0.006s] shaman-client-model transcript::tests::a_steer_stays_outside_the_fold
        PASS [   0.007s] shaman-desktop transcript::tests::rows_off_screen_are_never_built
        PASS [   0.009s] shaman-desktop transcript::tests::the_foot_stays_in_view_while_a_message_streams
        PASS [   0.011s] shaman-desktop transcript::tests::a_reader_scrolled_back_is_left_where_they_are
        PASS [   0.012s] shaman-desktop transcript::tests::a_row_is_rebuilt_only_when_its_revision_moves
------------
     Summary [   0.214s] 14 tests run: 14 passed, 0 skipped";

pub const EDIT_BEFORE: &str = "\
pub fn transcript(s: &State, k: &Kit) -> El {
    let Some(open) = &s.open else {
        return landing(s, k);
    };
    let mut list = col().key(\"rows\").w_full().gap(14.0);
    for unit in &open.units {
        list = list.child(draw(open, unit, k));
    }
    col()
        .key(\"transcript\")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .child(list)
}
";

pub const EDIT_AFTER: &str = "\
pub fn transcript(s: &State, k: &Kit) -> El {
    let Some(open) = &s.open else {
        return landing(s, k);
    };
    // One row per unit, keyed by the unit, so a row off screen is never built.
    let list = col()
        .key(\"rows\")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            open.units.len(),
            ROW_ESTIMATE,
            |s: &State, at| Cow::Borrowed(s.unit(at).key.as_str()),
            move |s: &State, at| memo(s.unit(at).key.as_str(), s.unit(at).revision, || draw(s, at)),
        )
        .virtual_rows_append_only();
    col()
        .key(s.viewport())
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .child(list)
}
";

const MODEL_BEFORE: &str = "\
pub struct Unit {
    pub item: Item,
}
";

const MODEL_AFTER: &str = "\
pub struct Unit {
    /// Stable for as long as the unit stands, so a row keeps its place and its disclosure.
    pub key: String,
    /// Moves whenever what the unit draws from does.
    pub revision: u64,
    pub item: Item,
}
";

pub const FOLLOW_BEFORE: &str = "\
    fn landed(&mut self, tx: &mut Tx) {
        tx.scroll_to_end(self.viewport());
    }
";

pub const FOLLOW_AFTER: &str = "\
    fn landed(&mut self, tx: &mut Tx) {
        // Follow the end only while the reader is already there.
        if self.at_foot {
            tx.scroll_to_end(self.viewport());
        } else {
            self.unseen += 1;
        }
    }
";

const fn act(
    id: &'static str,
    kind: Kind,
    head: &'static str,
    slot: &'static str,
    body: &'static str,
    after: &'static str,
) -> Activity {
    Activity {
        id,
        kind,
        head,
        slot,
        outcome: Outcome::Worked,
        body,
        after,
    }
}

static G1: [Activity; 4] = [
    act(
        "g1a",
        Kind::Read,
        "crates/rune-layout/src/parts.rs",
        "lines 272–332",
        READ_PARTS,
        "",
    ),
    act(
        "g1b",
        Kind::Read,
        "crates/rune-transcript/src/view.rs",
        "lines 299–351",
        READ_PARTS,
        "",
    ),
    act(
        "g1c",
        Kind::Tool,
        "Grep  virtual_rows  crates/",
        "14 matches",
        "crates/rune-transcript/src/view.rs:315:        .virtual_rows(\ncrates/rune-transcript/src/view.rs:341:            .virtual_rows_version(s.opened)\ncrates/rune-transcript/src/view.rs:342:            .virtual_rows_append_only();",
        "",
    ),
    act(
        "g1d",
        Kind::Command,
        "cargo check -p shaman-desktop",
        "6.2s",
        CHECK_OUT,
        "",
    ),
];

static G2: [Activity; 3] = [
    act(
        "g2a",
        Kind::Edit,
        "crates/shaman-desktop/src/transcript.rs",
        "+12 −5",
        EDIT_BEFORE,
        EDIT_AFTER,
    ),
    act(
        "g2b",
        Kind::Edit,
        "crates/shaman-client-model/src/transcript.rs",
        "+4 −0",
        MODEL_BEFORE,
        MODEL_AFTER,
    ),
    act(
        "g2c",
        Kind::Command,
        "cargo nextest run -p shaman-client-model -p shaman-desktop",
        "1.4s",
        TEST_OUT,
        "",
    ),
];

static G3: [Activity; 3] = [
    act(
        "g3a",
        Kind::Read,
        "crates/shaman-desktop/src/transcript.rs",
        "lines 1–96",
        READ_PARTS,
        "",
    ),
    act(
        "g3b",
        Kind::Edit,
        "crates/shaman-desktop/src/state.rs",
        "+6 −1",
        FOLLOW_BEFORE,
        FOLLOW_AFTER,
    ),
    act(
        "g3c",
        Kind::Command,
        "cargo nextest run -p shaman-client-model -p shaman-desktop",
        "1.4s",
        TEST_OUT,
        "",
    ),
];

/// The latest Turn's work where it has only read so far.
static G3R: [Activity; 1] = [act(
    "g3r",
    Kind::Read,
    "crates/shaman-desktop/src/transcript.rs",
    "lines 1–96",
    READ_PARTS,
    "",
)];

/// The latest Turn's Group while its last member still runs: adjacent Activities stand in one Group.
static G3W: [Activity; 4] = [
    act(
        "g3a",
        Kind::Read,
        "crates/shaman-desktop/src/transcript.rs",
        "lines 1–96",
        READ_PARTS,
        "",
    ),
    act(
        "g3b",
        Kind::Edit,
        "crates/shaman-desktop/src/state.rs",
        "+6 −1",
        FOLLOW_BEFORE,
        FOLLOW_AFTER,
    ),
    act(
        "g3c",
        Kind::Command,
        "cargo nextest run -p shaman-client-model -p shaman-desktop",
        "1.4s",
        TEST_OUT,
        "",
    ),
    Activity {
        id: "live",
        kind: Kind::Command,
        head: "cargo nextest run -p shaman-desktop transcript::",
        slot: "12s",
        outcome: Outcome::Working,
        body: "    Starting 6 tests across 1 binary\n        PASS [   0.006s] shaman-desktop transcript::tests::rows_off_screen_are_never_built\n        PASS [   0.009s] shaman-desktop transcript::tests::the_foot_stays_in_view_while_a_message_streams\n        PASS [   0.011s] shaman-desktop transcript::tests::a_reader_scrolled_back_is_left_where_they_are",
        after: "",
    },
];

static W_SETTLED: [Unit; 1] = [Unit::Group("g3", &G3)];
static W_WORKING: [Unit; 1] = [Unit::Group("g3", &G3W)];
static W_ASKING: [Unit; 1] = [Unit::Group("g3r", &G3R)];
static W_SUBAGENTS: [Unit; 5] = [
    Unit::Group("g3", &G3),
    Unit::Message(
        "The rule is in. I'll have two Subagents check it while I read the kit's scroll anchoring.",
    ),
    Unit::Steer("Measure it at 400 Messages, not 100."),
    Unit::Subagent {
        id: "s1",
        name: "explore",
        title: "Read Rune's virtual_rows",
        slot: "1m 40s",
        outcome: Outcome::Worked,
    },
    Unit::Subagent {
        id: "s2",
        name: "bench",
        title: "Measure reflow at 400 Messages",
        slot: "58s",
        outcome: Outcome::Working,
    },
];

pub const ANSWER_1: &str = "\
The Transcript is now a virtual list: one row per unit, keyed by the unit, so a row that is off screen is never built.

- **`transcript.rs`** builds the list with `virtual_rows` on a growth stack inside a `stick_bottom` scroll.
- **`Unit`** carries a `key` and a `revision`, and a row is rebuilt only when its revision moves.
- Both crates pass their tests (14 of 14).

```rust
.virtual_rows(open.units.len(), ROW_ESTIMATE, key_of, row_of)
.virtual_rows_append_only();
```

Here is the list at 1,068 rows, scrolled back while a Message streams at the foot:";

pub const ANSWER_2: &str = "\
The foot now follows the end only while the reader is already there. A reader who has scrolled back is left where they are, and `unseen` counts what has landed since so the window can offer the way back to the latest.";

/// What a streaming Message holds so far.
pub const STREAMING: &str = "\
The rule is in `State::landed`. While the reader is at the foot, every row that lands scrolls the foot back into view. Once they scroll back, the list leaves them where they are and counts";

pub static TURNS: [Turn; 2] = [
    Turn {
        id: "t1",
        prompt: "Draw the Transcript as virtual rows, one per unit, and keep the bottom in view while a Message streams. The sketch is what I mean by a row. [Image 1]",
        attachment: true,
        work: &[
            Unit::Message(
                "I'll read how the layout prototype stands in for the Transcript, then how the measuring prototype built its rows.",
            ),
            Unit::Group("g1", &G1),
            Unit::Message(
                "One warning and nothing else. The list can move onto `virtual_rows` without touching what a row draws.",
            ),
            Unit::Group("g2", &G2),
        ],
        answer: ANSWER_1,
        picture: true,
        outcome: Outcome::Worked,
        duration: "3m 41s",
        counts: "3 tools · 2 commands · 2 edits",
    },
    Turn {
        id: "t2",
        prompt: "Now the stick-to-bottom rule: follow the end only while the reader is already there.",
        attachment: false,
        work: &[Unit::Group("g3", &G3)],
        answer: ANSWER_2,
        picture: false,
        outcome: Outcome::Worked,
        duration: "48s",
        counts: "1 tool · 1 command · 1 edit",
    },
];

/// What the latest Turn is doing, which is what a scene chooses.
#[derive(Clone, Copy, PartialEq)]
pub enum Phase {
    /// Both Turns settled.
    Settled,
    /// The latest Turn works: a command runs.
    Working,
    /// The latest Turn works: its Message arrives.
    Streaming,
    /// The latest Turn waits on Subagents.
    Subagents,
    /// The latest Turn owes an Approval.
    Approval,
    /// The latest Turn owes a Questionnaire.
    Questionnaire,
    /// The latest Turn was stopped.
    Stopped,
}

impl Phase {
    pub fn live(self) -> bool {
        !matches!(self, Phase::Settled | Phase::Stopped)
    }
}

/// The Approval the latest Turn owes: a File Change, named with what it would do.
pub const APPROVAL_PATH: &str = "crates/shaman-desktop/src/state.rs";

pub struct Question {
    pub ask: &'static str,
    pub options: &'static [(&'static str, &'static str)],
    pub many: bool,
}

pub static QUESTIONS: [Question; 2] = [
    Question {
        ask: "When the reader has scrolled back and new rows land, what should the window offer?",
        options: &[
            (
                "A pill at the foot",
                "“3 new” over the Transcript's foot, pressed to return to the latest",
            ),
            (
                "A mark on the scroll bar",
                "Nothing over the Transcript; the bar shows where the foot is",
            ),
            ("Nothing", "The reader scrolls down when they are ready"),
        ],
        many: false,
    },
    Question {
        ask: "Which of these should pull the reader back to the foot?",
        options: &[
            ("Sending a Prompt", "Their own Message lands at the foot"),
            (
                "An Intervention arriving",
                "An Approval or a Questionnaire of this Session",
            ),
            ("The Turn settling", "The final Message has landed"),
        ],
        many: true,
    },
];

/// Prompts waiting behind the working Turn.
pub const QUEUED: [&str; 2] = [
    "Then measure it against the 1,068-row Session and say what a frame costs.",
    "Write the ADR for one row per unit.",
];

/// The files the Session has changed, as a Section of the Aside could list them.
pub const CHANGED: [(&str, &str); 3] = [
    ("crates/shaman-desktop/src/transcript.rs", "+12 −5"),
    ("crates/shaman-client-model/src/transcript.rs", "+4 −0"),
    ("crates/shaman-desktop/src/state.rs", "+6 −1"),
];

/// A long answer for streaming live in a real window: prose, a list, a table of sorts and code, about 520 words.
pub const LONG: &str = "\
The rule is in `State::landed`, and it comes down to one question asked each time a row lands: was the reader at the foot a moment ago? If they were, the list scrolls the foot back into view. If they were not, the list leaves them exactly where they are and counts what they have not seen.

## What changed

- **`State::at_foot`** is set from the scroll view's own report, so it is true only while the last row's foot is inside the viewport. Nothing else writes it.
- **`State::unseen`** counts the rows that landed while the reader was away from the foot, and returns to zero the moment they come back.
- **`landed`** reads the first and writes the second. It never moves a reader who has scrolled back, whatever arrives.

That last point is the whole rule. A Transcript that follows its own foot is pleasant until the reader wants to read something, and then it is the most hostile thing a window can do: the paragraph they were on slides away under them, once a frame, for as long as the agent keeps writing.

## How it behaves

While a Message streams, each delta grows the last row. The growth stack eases the row's drawn height toward its real one, so the foot glides instead of jumping a line at a time. A reader at the foot rides that glide. A reader who scrolled up sees none of it, because the rows above the viewport's top edge do not move.

```rust
fn landed(&mut self, tx: &mut Tx) {
    // Follow the end only while the reader is already there.
    if self.at_foot {
        tx.scroll_to_end(self.viewport());
    } else {
        self.unseen += 1;
    }
}
```

There are three cases worth naming, because each one is a place a simpler rule goes wrong:

1. **The reader sends a Prompt while scrolled back.** Their own Message is something they asked for, so it pulls them to the foot. That is the one arrival that overrides the rule.
2. **An Intervention arrives while they are scrolled back.** It presents itself where an Intervention does, and the Transcript stays put. The reader decides from where they stand.
3. **The window is resized.** Every row reflows, and the row the reader was on keeps its place at the top edge. The foot is followed only if it was being followed before.

## What I checked

I ran the six tests under `transcript::` and all of them pass, including the two new ones: `a_reader_scrolled_back_is_left_where_they_are` and `the_foot_stays_in_view_while_a_message_streams`. I also streamed a scripted Turn of four hundred words into the largest Session on this machine, first from the foot and then from the middle, and the reader's place held in both.

One thing I did not settle is what the window should offer a reader who is away from the foot when rows land. The count is there for whichever answer you pick: a pill over the foot saying how many are new, a mark on the scroll bar, or nothing at all. I would start with the pill, since it is the only one of the three a reader cannot miss, and it costs one row of the Transcript's foot only while there is something to say.

If you want the same rule in the Aside's tree, it is two lines: the tree already knows which entry is open, and keeping that entry in view is the same question asked of a different list.";

/// The long answer, repeated `RC_REPEAT` times over where the environment asks, for measuring a very long Message.
fn long_text() -> &'static str {
    static TEXT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let repeat = std::env::var("RC_REPEAT")
            .ok()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(1);
        if repeat <= 1 {
            LONG
        } else {
            Box::leak(vec![LONG; repeat].join("\n\n").into_boxed_str())
        }
    })
}

/// The first `words` words of the long answer, and whether that is all of it.
pub fn long(words: usize) -> (&'static str, bool) {
    let text = long_text();
    let mut seen = 0;
    let mut inside = false;
    for (at, c) in text.char_indices() {
        let space = c.is_whitespace();
        if !space && !inside {
            seen += 1;
            if seen > words {
                return (text[..at].trim_end(), false);
            }
        }
        inside = !space;
    }
    (text, true)
}

/// A Turn as a phase leaves it standing.
pub struct Shown {
    pub turn: &'static Turn,
    pub work: &'static [Unit],
    pub outcome: Outcome,
    pub duration: &'static str,
    pub counts: &'static str,
    pub answer: Option<&'static str>,
    pub streaming: bool,
}

/// The Turns on show: the first as it settled, and the latest as `phase` has it, or as far as a live stream has come.
pub fn shown(phase: Phase, first_only: bool, streamed: Option<usize>) -> Vec<Shown> {
    let whole = |turn: &'static Turn| Shown {
        turn,
        work: turn.work,
        outcome: turn.outcome,
        duration: turn.duration,
        counts: turn.counts,
        answer: Some(turn.answer),
        streaming: false,
    };
    let first = whole(&TURNS[0]);
    if first_only {
        return vec![first];
    }
    let turn = &TURNS[1];
    let live = |work: &'static [Unit], duration, counts| Shown {
        turn,
        work,
        outcome: Outcome::Working,
        duration,
        counts,
        answer: None,
        streaming: false,
    };
    // A Message streamed live in a real window stands in for the fake answer, as far as it has come.
    if let Some(words) = streamed {
        let (said, all) = long(words);
        let latest = if all {
            Shown {
                answer: Some(said),
                ..whole(turn)
            }
        } else {
            Shown {
                answer: Some(said),
                streaming: true,
                ..live(&W_SETTLED, "48s", "1 tool · 1 command · 1 edit")
            }
        };
        return vec![first, latest];
    }
    let latest = match phase {
        Phase::Settled => whole(turn),
        Phase::Working => live(&W_WORKING, "31s", "1 tool · 2 commands · 1 edit"),
        Phase::Streaming => Shown {
            answer: Some(STREAMING),
            streaming: true,
            ..live(&W_SETTLED, "48s", "1 tool · 1 command · 1 edit")
        },
        Phase::Subagents => live(&W_SUBAGENTS, "2m 06s", "1 tool · 1 command · 1 edit"),
        Phase::Approval | Phase::Questionnaire => live(&W_ASKING, "19s", "1 tool"),
        Phase::Stopped => Shown {
            outcome: Outcome::Stopped,
            ..live(&W_SETTLED, "31s", "1 tool · 1 command · 1 edit")
        },
    };
    vec![first, latest]
}

/// The words the Working Indicator says for a phase, and how long the work has run.
pub fn working(phase: Phase) -> (&'static str, &'static str) {
    match phase {
        Phase::Subagents => ("Waiting on 1 subagent", "2m 06s"),
        Phase::Streaming => ("Working", "48s"),
        Phase::Approval | Phase::Questionnaire => ("Waiting on you", "19s"),
        _ => ("Working", "31s"),
    }
}

pub fn activity(id: &str) -> Option<&'static Activity> {
    [&G3R[..], &G3W[..]]
        .into_iter()
        .flatten()
        .chain(
            TURNS
                .iter()
                .flat_map(|t| t.work.iter())
                .filter_map(|u| match u {
                    Unit::Group(_, members) => Some(members.iter()),
                    _ => None,
                })
                .flatten(),
        )
        .find(|a| a.id == id)
}
