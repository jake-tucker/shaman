//! Text as Shaman takes it in from elsewhere and shows it: the escape
//! sequences in what a Provider or another machine sends ([`ansi`]), and what
//! of another machine's text may reach a reader ([`shown_text`]). It does no
//! I/O, so a thin Client links it on a platform the Server never runs on
//! (iOS).

pub mod ansi;
pub mod shown_text;
