//! What turns an Icon into the glyph it is drawn with, for every Client: the
//! Icon Catalog ([`glyph`], [`entries`], [`names`], [`search`]), the plain
//! glyphs that stand where an Icon is absent ([`FOLDER`], [`MONITOR`]), the
//! glyphs a Checkout State is led by ([`GIT_BRANCH`], [`WORKTREE`],
//! [`GIT_COMMIT`]), the Icon the Sidekick Workspace stands beside
//! ([`SIDEKICK_ICON`]), and a Provider's glyph ([`provider_glyph`]).
//!
//! It holds no I/O and no Shaman type, so the Server, the TUI and a thin
//! Client's core link it alike. Every glyph it resolves is a Nerd Fonts 3.5.1
//! codepoint, and the tests read the Symbols Nerd Font the iOS app bundles to
//! see the font draws each one.

mod catalog;

pub use catalog::{IconEntry, IconGroup, entries, glyph, names, search};

/// Nerd Fonts `nf-cod-folder` (Codicons folder): the plain glyph a Workspace
/// without an Icon is named beside.
pub const FOLDER: char = '\u{ea83}';

/// Nerd Fonts `nf-md-monitor` (Material Design monitor): the plain glyph a
/// Remote without an Icon is named beside.
pub const MONITOR: char = '\u{f0379}';

/// Nerd Fonts `nf-cod-git_branch` (Codicons git-branch): the glyph a Checkout
/// State on a branch of the main Worktree is led by.
pub const GIT_BRANCH: char = '\u{ec6f}';

/// Nerd Fonts `nf-cod-worktree` (Codicons worktree): the glyph a Checkout
/// State on a branch of a linked Worktree is led by.
pub const WORKTREE: char = '\u{ec7e}';

/// Nerd Fonts `nf-cod-git_commit` (Codicons git-commit): the glyph a Checkout
/// State on a detached head is led by.
pub const GIT_COMMIT: char = '\u{eafc}';

/// The Catalog name of the Icon the Sidekick Workspace is named beside
/// wherever it has none of its own: a standing default rather than a stored
/// one, so it is never derived and a user's own choice still replaces it.
pub const SIDEKICK_ICON: &str = "md-robot";

/// Each Provider Shaman knows a glyph for, by its identifier, with the glyph
/// drawn beside its name and the Nerd Fonts name it is pinned to.
const PROVIDER_GLYPHS: &[ProviderGlyph] = &[
    ProviderGlyph {
        provider: "claude",
        nerd_font_name: "cod-claude",
        glyph: '\u{ec82}',
    },
    ProviderGlyph {
        provider: "codex",
        nerd_font_name: "cod-openai",
        glyph: '\u{ec81}',
    },
    ProviderGlyph {
        provider: "copilot",
        nerd_font_name: "cod-copilot",
        glyph: '\u{ec1e}',
    },
];

struct ProviderGlyph {
    /// The Provider's identifier, as a `ProviderId` carries it.
    provider: &'static str,
    /// The Nerd Fonts name `glyph` is pinned to in the tests, which is all
    /// it is read for.
    #[cfg_attr(not(test), allow(dead_code))]
    nerd_font_name: &'static str,
    glyph: char,
}

/// The glyph drawn beside the Provider `provider` identifies, or `None` for a
/// Provider this build knows no glyph for, which keeps its text alone — so a
/// Provider added later is presented whole before it chooses a glyph.
pub fn provider_glyph(provider: &str) -> Option<char> {
    PROVIDER_GLYPHS
        .iter()
        .find(|known| known.provider == provider)
        .map(|known| known.glyph)
}

/// What Nerd Fonts says of a glyph, read from copies Shaman keeps rather than
/// the network: the names it publishes, in this crate's
/// `tests/fixtures/glyphnames.json`, and the Symbols Nerd Font the iOS app
/// bundles. Both are found from `CARGO_MANIFEST_DIR`, so the paths hold
/// whatever directory a test runner starts in, and joined component by
/// component, so they resolve on Windows too.
#[cfg(test)]
pub(crate) mod nerd_fonts {
    use std::{
        path::{Path, PathBuf},
        sync::LazyLock,
    };

    use serde_json::Value;

    fn from_manifest(components: &[&str]) -> PathBuf {
        components.iter().fold(
            Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf(),
            |path, component| path.join(component),
        )
    }

    static GLYPH_NAMES: LazyLock<Value> = LazyLock::new(|| {
        let path = from_manifest(&["tests", "fixtures", "glyphnames.json"]);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        serde_json::from_str(&raw).expect("glyphnames.json fixture is valid JSON")
    });

    static FONT: LazyLock<Vec<u8>> = LazyLock::new(|| {
        let path = from_manifest(&[
            "..",
            "..",
            "ios",
            "ShamanUI",
            "Sources",
            "ShamanUI",
            "Resources",
            "Fonts",
            "SymbolsNerdFont-Regular.ttf",
        ]);
        std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    });

    /// The glyph Nerd Fonts publishes under `name`, panicking where it
    /// publishes none.
    pub(crate) fn published(name: &str) -> char {
        let record = GLYPH_NAMES
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` is not a Nerd Font glyph name"));
        let code = record["code"]
            .as_str()
            .unwrap_or_else(|| panic!("`{name}` has no `code` in the fixture"));
        let codepoint = u32::from_str_radix(code, 16)
            .unwrap_or_else(|_| panic!("`{name}` has a non-hex `code`: {code}"));
        char::from_u32(codepoint)
            .unwrap_or_else(|| panic!("`{name}` publishes an invalid codepoint"))
    }

    /// Whether the bundled font has a glyph for `glyph`.
    pub(crate) fn draws(glyph: char) -> bool {
        let face = ttf_parser::Face::parse(&FONT, 0).expect("the bundled font parses");
        face.glyph_index(glyph).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every fixed glyph, with the Nerd Fonts name it is pinned to.
    fn fixed_glyphs() -> impl Iterator<Item = (&'static str, char)> {
        [
            ("cod-folder", FOLDER),
            ("md-monitor", MONITOR),
            ("cod-git_branch", GIT_BRANCH),
            ("cod-worktree", WORKTREE),
            ("cod-git_commit", GIT_COMMIT),
        ]
        .into_iter()
        .chain(
            PROVIDER_GLYPHS
                .iter()
                .map(|known| (known.nerd_font_name, known.glyph)),
        )
    }

    #[test]
    fn a_known_provider_resolves_to_its_glyph() {
        assert_eq!(provider_glyph("claude"), Some('\u{ec82}'));
        assert_eq!(provider_glyph("codex"), Some('\u{ec81}'));
        assert_eq!(provider_glyph("copilot"), Some('\u{ec1e}'));
    }

    #[test]
    fn a_provider_this_build_does_not_know_resolves_to_nothing() {
        assert_eq!(provider_glyph("controlled"), None);
        assert_eq!(provider_glyph(""), None);
    }

    #[test]
    fn the_sidekick_icon_is_one_the_catalog_carries() {
        assert_eq!(glyph(SIDEKICK_ICON), Some('\u{f06a9}'));
    }

    #[test]
    fn every_fixed_glyph_is_pinned_to_the_nerd_fonts_fixture() {
        for (name, glyph) in fixed_glyphs() {
            assert_eq!(
                glyph,
                nerd_fonts::published(name),
                "`{name}` is pinned to the wrong codepoint"
            );
        }
    }

    #[test]
    fn the_bundled_font_draws_every_fixed_glyph() {
        for (name, glyph) in fixed_glyphs() {
            assert!(
                nerd_fonts::draws(glyph),
                "the bundled font has no glyph for `{name}` (U+{:X})",
                glyph as u32
            );
        }
    }

    /// The checks of the font could pass on one they failed to read: a
    /// codepoint no Nerd Font carries is reported missing.
    #[test]
    fn the_bundled_font_is_read_and_not_taken_as_drawing_everything() {
        assert!(!nerd_fonts::draws('\u{10fffd}'));
    }
}
