//! Fake data: a Model Catalog as three Providers would publish one, and each Provider's Approval Posture values.
//! Nothing here is read from anywhere, and every price, limit and advisory is made up.

/// One Model Option: a dimension a Model offers, and the values it may take.
pub struct Opt {
    pub label: &'static str,
    pub choices: &'static [&'static str],
    /// What the Provider's own default is.
    pub default: usize,
    /// The value the reading under the composer leaves unsaid, where there is one.
    pub quiet: Option<usize>,
}

pub struct Model {
    pub name: &'static str,
    pub id: &'static str,
    pub about: &'static str,
    /// The Provider's relative price, as the TUI's glyph and word.
    pub price: (&'static str, &'static str),
    /// Per million tokens: input, output, cache read, cache write.
    pub tokens: [&'static str; 4],
    pub context: &'static str,
    pub output: &'static str,
    pub images: bool,
    /// A Model Advisory that is a warning: shown before the Model is chosen, and marked wherever it is listed.
    pub warning: Option<&'static str>,
    /// Advisories told only where the Model is detailed.
    pub notes: &'static [&'static str],
    pub options: &'static [Opt],
    pub default: bool,
}

/// One dimension of a Provider's Approval Posture, with that Provider's native values.
pub struct Dimension {
    pub label: &'static str,
    pub what: &'static str,
    /// Each native value and what it means. The meanings are the prototype's own words: the TUI lists the values bare.
    pub values: &'static [(&'static str, &'static str)],
    /// What the Provider's Setting gives a Session that has not overridden it.
    pub setting: usize,
}

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub models: &'static [Model],
    pub posture: &'static [Dimension],
}

const EFFORT: Opt = Opt {
    label: "Reasoning effort",
    choices: &["Low", "Medium", "High", "Max"],
    default: 2,
    quiet: None,
};
const CONTEXT: Opt = Opt {
    label: "Context",
    choices: &["200K", "1M"],
    default: 0,
    quiet: Some(0),
};
const SPEED: Opt = Opt {
    label: "Speed",
    choices: &["Standard", "Fast"],
    default: 0,
    quiet: Some(0),
};
const CODEX_EFFORT: Opt = Opt {
    label: "Reasoning effort",
    choices: &["Minimal", "Low", "Medium", "High"],
    default: 2,
    quiet: None,
};
const TIER: Opt = Opt {
    label: "Routing tier",
    choices: &["Default", "Economy", "Balanced", "Capable"],
    default: 0,
    quiet: Some(0),
};

pub static PROVIDERS: [Provider; 3] = [
    Provider {
        id: "claude",
        name: "Claude",
        models: &[
            Model {
                name: "Fable 5.2",
                id: "claude-fable-5-2",
                about: "The most capable, for long and hard work",
                price: ("$$$$", "Very high"),
                tokens: ["$15.00", "$75.00", "$1.50", "$18.75"],
                context: "1M",
                output: "128K",
                images: true,
                warning: None,
                notes: &[],
                options: &[EFFORT, CONTEXT, SPEED],
                default: true,
            },
            Model {
                name: "Haiku 5.5",
                id: "claude-haiku-5-5",
                about: "Fast and cheap, for reading and small edits",
                price: ("$", "Low"),
                tokens: ["$1.00", "$5.00", "$0.10", "$1.25"],
                context: "200K",
                output: "64K",
                images: true,
                warning: None,
                notes: &[],
                options: &[],
                default: false,
            },
            Model {
                name: "Opus 5.5",
                id: "claude-opus-5-5",
                about: "Careful over long work",
                price: ("$$$", "High"),
                tokens: ["$5.00", "$25.00", "$0.50", "$6.25"],
                context: "1M",
                output: "64K",
                images: true,
                warning: None,
                notes: &["Half price on cache reads until 2026-12-31."],
                options: &[EFFORT, CONTEXT],
                default: false,
            },
            Model {
                name: "Sonnet 5.5",
                id: "claude-sonnet-5-5",
                about: "The everyday balance of speed and care",
                price: ("$$", "Medium"),
                tokens: ["$3.00", "$15.00", "$0.30", "$3.75"],
                context: "1M",
                output: "64K",
                images: true,
                warning: None,
                notes: &[],
                options: &[EFFORT, CONTEXT],
                default: false,
            },
        ],
        posture: &[Dimension {
            label: "Permission mode",
            what: "When Claude asks before using a tool",
            values: &[
                ("default", "Asks before an edit or a command"),
                ("acceptEdits", "Edits without asking; asks before a command"),
                (
                    "dontAsk",
                    "Asks nothing, and declines what it would have asked",
                ),
                ("bypassPermissions", "Asks nothing and declines nothing"),
                (
                    "auto",
                    "Claude's classifier passes routine requests and asks about the rest",
                ),
            ],
            setting: 4,
        }],
    },
    Provider {
        id: "codex",
        name: "Codex",
        models: &[
            Model {
                name: "GPT-6 Codex",
                id: "gpt-6-codex",
                about: "Tuned for long agentic coding",
                price: ("$$$", "High"),
                tokens: ["$2.50", "$20.00", "$0.25", "—"],
                context: "400K",
                output: "128K",
                images: true,
                warning: None,
                notes: &[],
                options: &[CODEX_EFFORT, SPEED],
                default: true,
            },
            Model {
                name: "GPT-6 Codex Mini",
                id: "gpt-6-codex-mini",
                about: "Small and quick",
                price: ("$", "Low"),
                tokens: ["$0.40", "$3.20", "$0.04", "—"],
                context: "400K",
                output: "64K",
                images: true,
                warning: None,
                notes: &[],
                options: &[CODEX_EFFORT],
                default: false,
            },
            Model {
                name: "GPT-6 Preview",
                id: "gpt-6-preview",
                about: "A research preview of the next general Model",
                price: ("?", "Unknown"),
                tokens: ["—", "—", "—", "—"],
                context: "1M",
                output: "128K",
                images: false,
                warning: Some(
                    "Prompts sent to this preview are kept for 30 days and may be reviewed.",
                ),
                notes: &["Limits may change without notice while in preview."],
                options: &[CODEX_EFFORT],
                default: false,
            },
        ],
        posture: &[
            Dimension {
                label: "Approval policy",
                what: "When Codex asks before running a tool",
                values: &[
                    ("untrusted", "Asks before anything but a known safe read"),
                    ("on-request", "Asks when the agent wants out of its sandbox"),
                    ("never", "Asks nothing"),
                ],
                setting: 1,
            },
            Dimension {
                label: "Sandbox",
                what: "What Codex may access without an approval",
                values: &[
                    ("read-only", "Reads files and changes nothing"),
                    ("workspace-write", "Writes inside the Execution Directory"),
                    (
                        "danger-full-access",
                        "Reads and writes anywhere, network included",
                    ),
                ],
                setting: 1,
            },
        ],
    },
    Provider {
        id: "copilot",
        name: "Copilot",
        models: &[
            Model {
                name: "Auto",
                id: "auto",
                about: "Copilot routes each request for you",
                price: ("$$", "Medium"),
                tokens: ["—", "—", "—", "—"],
                context: "200K",
                output: "64K",
                images: true,
                warning: None,
                notes: &["Billed at the rate of whichever Model each request is routed to."],
                options: &[TIER],
                default: true,
            },
            Model {
                name: "Claude Sonnet 5.5",
                id: "claude-sonnet-5.5",
                about: "Anthropic's Model, through Copilot",
                price: ("$$", "Medium"),
                tokens: ["1× premium", "—", "—", "—"],
                context: "200K",
                output: "64K",
                images: true,
                warning: None,
                notes: &[],
                options: &[],
                default: false,
            },
            Model {
                name: "GPT-6",
                id: "gpt-6",
                about: "OpenAI's Model, through Copilot",
                price: ("$$", "Medium"),
                tokens: ["1× premium", "—", "—", "—"],
                context: "272K",
                output: "64K",
                images: true,
                warning: None,
                notes: &[],
                options: &[CODEX_EFFORT],
                default: false,
            },
        ],
        posture: &[Dimension {
            label: "Permissions",
            what: "When Copilot asks before using a tool",
            values: &[
                ("ask", "Asks before an edit or a command"),
                (
                    "auto",
                    "Copilot's judge passes routine requests and asks about the rest",
                ),
                ("allowAll", "Asks nothing"),
            ],
            setting: 1,
        }],
    },
];

/// Why Codex cannot be used in the scenes that find it unavailable, and what its own check says to do.
pub const NOT_SIGNED_IN: (&str, &str) = (
    "not signed in",
    "Run `codex login` in a terminal, then choose this row to ask Codex again.",
);

/// An Agent Selection: a Provider, one of its Models, and a value for each Model Option that Model offers.
#[derive(Clone, PartialEq)]
pub struct Agent {
    pub provider: usize,
    pub model: usize,
    pub opts: Vec<usize>,
}

impl Agent {
    /// A Model as its Provider would run it with nothing chosen.
    pub fn of(provider: usize, model: usize) -> Agent {
        Agent {
            provider,
            model,
            opts: PROVIDERS[provider].models[model]
                .options
                .iter()
                .map(|o| o.default)
                .collect(),
        }
    }

    pub fn provider(&self) -> &'static Provider {
        &PROVIDERS[self.provider]
    }

    pub fn model(&self) -> &'static Model {
        &self.provider().models[self.model]
    }

    /// The Model Options the reading says: each by its value, but for one left at the value that goes unsaid.
    pub fn said(&self) -> Vec<&'static str> {
        self.model()
            .options
            .iter()
            .zip(&self.opts)
            .filter(|(o, at)| o.quiet != Some(**at))
            .map(|(o, at)| o.choices[*at])
            .collect()
    }

    /// The reading as the TUI's footer says it: `Claude · Fable 5.2 · High`.
    pub fn reading(&self) -> String {
        let mut parts = vec![self.provider().name, self.model().name];
        parts.extend(self.said());
        parts.join(" · ")
    }
}

/// The posture a Provider's Setting gives: one value a dimension.
pub fn setting(provider: usize) -> Vec<usize> {
    PROVIDERS[provider]
        .posture
        .iter()
        .map(|d| d.setting)
        .collect()
}

/// A posture as its Provider's native values: `default`, or `on-request · workspace-write`.
pub fn posture_reading(provider: usize, posture: &[usize]) -> String {
    PROVIDERS[provider]
        .posture
        .iter()
        .zip(posture)
        .map(|(d, at)| d.values[*at].0)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The Models a picker lists under each Provider: by name, narrowed to those whose name or id carries `query`.
pub fn listed(provider: usize, query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    PROVIDERS[provider]
        .models
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            query.is_empty()
                || m.name.to_lowercase().contains(&query)
                || m.id.to_lowercase().contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}
