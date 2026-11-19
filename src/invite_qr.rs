//! An Invite drawn as a QR code, for a phone to scan (issue #677).
//!
//! The code holds the Invite as it is, `shaman-v1-…` and nothing else, at the
//! lowest error-correction level so it is as small as the Invite allows. It is
//! drawn in half-blocks — each character cell holds two modules, one above the
//! other — inside a margin of [`MARGIN`] light modules, and always dark on light
//! in colours no theme or terminal palette moves ([`DARK`] and [`LIGHT`]): a
//! scanner reads dark modules on a light ground, and a code drawn the other way
//! round, light on a dark theme, is read by none.
//!
//! `/serve` and `shaman serve invite --qr` both draw it from here, each only when
//! asked, so it says the same in both.

use qrcode::{Color, EcLevel, QrCode};

/// The light modules drawn around the code on every side. The standard asks for
/// four; two is what scanners need in practice, and keeps the code inside
/// terminals that would not fit four.
pub(crate) const MARGIN: usize = 2;

/// The 256-colour palette's black, `#000000`, in the colour cube no theme sets.
pub(crate) const DARK: u8 = 16;

/// The 256-colour palette's white, `#ffffff`, in the colour cube no theme sets.
pub(crate) const LIGHT: u8 = 231;

/// An Invite encoded, its margin included, ready to be drawn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InviteQrCode {
    /// Modules on a side, the margin included.
    side: usize,
    /// Whether each module is dark, row by row.
    dark: Vec<bool>,
}

impl InviteQrCode {
    /// Encodes `invite` as it is, or says why it cannot be: only an Invite longer
    /// than the largest QR code holds.
    pub(crate) fn of(invite: &str) -> Result<Self, String> {
        let code = QrCode::with_error_correction_level(invite.as_bytes(), EcLevel::L)
            .map_err(|error| format!("the invite cannot be drawn as a QR code: {error}"))?;
        let width = code.width();
        let side = width + 2 * MARGIN;
        let colors = code.into_colors();
        let dark = (0..side * side)
            .map(|index| {
                let (row, column) = (index / side, index % side);
                let inside = |at: usize| (MARGIN..MARGIN + width).contains(&at);
                inside(row)
                    && inside(column)
                    && colors[(row - MARGIN) * width + (column - MARGIN)] == Color::Dark
            })
            .collect();
        Ok(Self { side, dark })
    }

    /// The character cells the code takes across.
    pub(crate) fn columns(&self) -> usize {
        self.side
    }

    /// The character cells the code takes down, two modules to each. An odd
    /// module left over is drawn above light.
    pub(crate) fn rows(&self) -> usize {
        self.side.div_ceil(2)
    }

    fn is_dark(&self, row: usize, column: usize) -> bool {
        row < self.side && self.dark[row * self.side + column]
    }

    /// The code as text, one line to each row of cells: a dark module is ink
    /// and a light one is ground, so it reads right only drawn [`DARK`] on
    /// [`LIGHT`].
    pub(crate) fn lines(&self) -> Vec<String> {
        (0..self.rows())
            .map(|row| {
                (0..self.side)
                    .map(|column| {
                        match (
                            self.is_dark(2 * row, column),
                            self.is_dark(2 * row + 1, column),
                        ) {
                            (true, true) => '█',
                            (true, false) => '▀',
                            (false, true) => '▄',
                            (false, false) => ' ',
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// [`Self::lines`] as a terminal draws them, each set [`DARK`] on [`LIGHT`]
    /// and put back as it was at its end, so nothing after it is coloured.
    pub(crate) fn ansi_lines(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .map(|line| format!("\x1b[38;5;{DARK};48;5;{LIGHT}m{line}\x1b[0m"))
            .collect()
    }
}

/// What the tests of a drawn code share with `tests/`: an Invite as a Server
/// issues one, and a reader of a drawn code.
#[cfg(test)]
#[path = "../tests/support/qr_code.rs"]
pub(crate) mod reading;

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    use super::{
        reading::{LONG_INVITE as INVITE, read_back},
        *,
    };
    use crate::{pairing::InvitePayload, protocol::Way};

    /// An Invite offering `ways`, shaped as a Server issues one: its identity key
    /// is a P-256 public key's 91 bytes, and its token 32.
    fn an_invite(ways: &[&str]) -> String {
        let payload = InvitePayload {
            ways: ways
                .iter()
                .map(|way| match way.parse() {
                    Ok(address) => Way::Direct(address),
                    Err(_) => Way::Relay((*way).to_owned()),
                })
                .collect(),
            server_key: URL_SAFE_NO_PAD.encode([0xa5_u8; 91]),
            token: URL_SAFE_NO_PAD.encode([0x5a_u8; 32]),
            hostname: "jakes-macbook-pro".to_owned(),
        };
        format!(
            "shaman-v1-{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
        )
    }

    /// The sizes "How a phone pairs" (#638) estimated hold for Invites shaped as
    /// a Server issues them, and each reads back.
    #[test]
    fn invites_as_issued_fit_the_sizes_estimated_and_read_back() {
        for (ways, (columns, rows)) in [
            (vec!["192.168.1.20:7777"], (69, 35)),
            (
                vec![
                    "192.168.1.20:7777",
                    "100.101.102.103:7777",
                    "https://relay.example.com",
                ],
                (77, 39),
            ),
            (
                vec![
                    "192.168.1.20:7777",
                    "100.101.102.103:7777",
                    "10.0.0.8:7777",
                    "[fd7a:115c:a1e0::1234:5678]:7777",
                    "https://relay.example.com",
                ],
                (85, 43),
            ),
        ] {
            let invite = an_invite(&ways);
            let code = InviteQrCode::of(&invite).unwrap();
            assert!(
                code.columns() <= columns && code.rows() <= rows,
                "{ways:?}: {} by {}",
                code.columns(),
                code.rows()
            );
            assert_eq!(read_back(&code.lines()).as_deref(), Some(invite.as_str()));
        }
    }

    #[test]
    fn the_code_has_its_margin_and_two_modules_to_a_row_of_cells() {
        let code = InviteQrCode::of(INVITE).unwrap();
        let lines = code.lines();
        assert_eq!(lines.len(), code.rows());
        assert_eq!(code.rows(), code.columns().div_ceil(2));
        for line in &lines {
            assert_eq!(line.chars().count(), code.columns());
            assert!(line.starts_with("  ") && line.ends_with("  "), "{line:?}");
        }
        assert!(lines[0].trim().is_empty(), "the top margin is a whole row");
        assert!(
            lines[1].starts_with("  ▄") || lines[1].starts_with("  █"),
            "{:?}",
            lines[1]
        );
    }

    #[test]
    fn the_code_is_at_the_lowest_error_correction_level() {
        let code = QrCode::with_error_correction_level(INVITE.as_bytes(), EcLevel::L).unwrap();
        assert_eq!(code.error_correction_level(), EcLevel::L);
        assert_eq!(
            InviteQrCode::of(INVITE).unwrap().columns(),
            code.width() + 2 * MARGIN
        );
    }

    #[test]
    fn a_drawn_code_reads_back_to_the_invite() {
        let code = InviteQrCode::of(INVITE).unwrap();
        assert_eq!(read_back(&code.lines()).as_deref(), Some(INVITE));
    }

    #[test]
    fn each_line_is_drawn_dark_on_light_and_leaves_no_colour_behind() {
        for line in InviteQrCode::of(INVITE).unwrap().ansi_lines() {
            assert!(line.starts_with("\x1b[38;5;16;48;5;231m"), "{line:?}");
            assert!(line.ends_with("\x1b[0m"), "{line:?}");
        }
    }

    #[test]
    fn an_invite_too_long_for_any_code_says_so() {
        let error = InviteQrCode::of(&"x".repeat(4000)).unwrap_err();
        assert!(error.contains("cannot be drawn as a QR code"), "{error}");
    }
}
