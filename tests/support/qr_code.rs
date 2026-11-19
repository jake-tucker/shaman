//! What the tests of an Invite's QR code share, wherever they draw it: an Invite
//! shaped as a Server issues one, and a reader that reads a drawn code back as a
//! phone's scanner would. `src/invite_qr.rs` includes this file for its own tests
//! and those of `src/cli.rs`, as `tests/integration.rs` does for the rest.

/// An Invite shaped as a Server issues one offering one LAN address: its
/// identity key is a P-256 public key's 91 bytes, and its token 32. Its code is
/// the smallest a real one has.
pub const LONG_INVITE: &str = "shaman-v1-eyJ3IjpbeyJkaXJlY3QiOiIxOTIuMTY4LjEuMjA6Nzc3NyJ9XSwiayI6IkFBRUNBd1FGQm\
     djSUNRb0xEQTBPRHhBUkVoTVVGUllYR0JrYUd4d2RIaDhnSVNJakpDVW1KeWdwS2lzc0xTNHZNREV5TX\
     pRMU5qYzRPVG83UEQwLVAwQkJRa05FUlVaSFNFbEtTMHhOVGs5UVVWSlRWRlZXVjFoWldnIiwidCI6Il\
     pHVm1aMmhwYW10c2JXNXZjSEZ5YzNSMWRuZDRlWHA3ZkgxLWY0Q0Jnb00iLCJoIjoid29ya3N0YXRpb2\
     4ifQ";

/// Reads drawn half-block `lines` back to the text they hold, as a phone's
/// scanner would, so a test sees a drawn code says what it was drawn for. Only
/// the code's own margin is around it: nothing is added for the reader.
pub fn read_back<S: AsRef<str>>(lines: &[S]) -> Option<String> {
    /// Pixels to a module on a side.
    const SCALE: usize = 4;
    let cells = lines
        .iter()
        .map(|line| line.as_ref().chars().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let columns = cells.iter().map(Vec::len).max()?;
    let dark = |x: usize, y: usize| {
        let (column, module_row) = (x / SCALE, y / SCALE);
        let cell = cells[module_row / 2].get(column).copied().unwrap_or(' ');
        matches!((cell, module_row % 2), ('█', _) | ('▀', 0) | ('▄', 1))
    };
    let mut image = rqrr::PreparedImage::prepare_from_greyscale(
        columns * SCALE,
        cells.len() * 2 * SCALE,
        |x, y| if dark(x, y) { 0 } else { 255 },
    );
    let grids = image.detect_grids();
    grids.first()?.decode().ok().map(|(_, text)| text)
}
