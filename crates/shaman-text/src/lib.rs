//! Text as Shaman takes it in from elsewhere and shows it: the escape
//! sequences in what a Provider or another machine sends ([`ansi`]), what of
//! another machine's text may reach a reader ([`shown_text`]), and the paths
//! another machine writes, shown from where a Session works ([`paths`]). It
//! does no I/O, so a thin Client links it on a platform the Server never runs
//! on (iOS).

pub mod ansi;
pub mod paths;
pub mod shown_text;
