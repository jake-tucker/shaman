//! Paths a Server wrote, shown from the directory a Session works in.
//!
//! A path in a Transcript is the text its Server wrote, on whatever platform
//! that Server runs, and a reader may run on another: a phone reading a
//! Windows Server, or a TUI on Windows reading a Remote on Linux. So a path is
//! read here as text, never through this machine's [`std::path::Path`]: a
//! Windows path by its drive, share or verbatim prefix with either separator,
//! and a POSIX path by its leading `/`, whichever platform reads it. Nothing
//! touches the filesystem, so a deleted file and a symlink's spelling read as
//! they were written.

use std::borrow::Cow;

/// `path` as it is shown to a reader of a Session working in `directory`:
/// from `directory` where it lies within it, `.` where it is `directory`
/// itself, and whole where it lies elsewhere, climbs out of it, or is not
/// absolute. What remains is joined by the separator `path` was written with.
pub fn within<'a>(path: &'a str, directory: &str) -> Cow<'a, str> {
    let (Some(spelled), Some(base)) = (Spelled::of(path), Spelled::of(directory)) else {
        return Cow::Borrowed(path);
    };
    if !spelled.root.same(&base.root)
        || spelled.parts.len() < base.parts.len()
        || spelled.parts[..base.parts.len()] != base.parts[..]
    {
        return Cow::Borrowed(path);
    }
    let rest = &spelled.parts[base.parts.len()..];
    // A remainder that climbs above `directory` does not lie within it.
    let mut depth = 0_usize;
    for part in rest {
        if *part == ".." {
            let Some(up) = depth.checked_sub(1) else {
                return Cow::Borrowed(path);
            };
            depth = up;
        } else {
            depth += 1;
        }
    }
    if rest.is_empty() {
        return Cow::Borrowed(".");
    }
    Cow::Owned(rest.join(if spelled.windows { "\\" } else { "/" }))
}

/// An absolute path, as its text spells it.
struct Spelled<'a> {
    root: Root<'a>,
    /// Its components after the root, `.` and empty ones left out.
    parts: Vec<&'a str>,
    /// Whether it is spelled as Windows spells a path.
    windows: bool,
}

/// What an absolute path is rooted in.
#[derive(Debug)]
enum Root<'a> {
    /// `/`.
    Posix,
    /// A drive, `C:\` or `\\?\C:\`, by its letter.
    Disk(u8),
    /// A share, `\\server\share` or `\\?\UNC\server\share`.
    Share { server: &'a str, share: &'a str },
    /// Any other verbatim or device prefix, `\\?\Volume{…}` or `\\.\pipe`,
    /// by its text.
    Device(&'a str),
}

impl Root<'_> {
    /// Whether two roots are the same, a verbatim prefix the same as the
    /// ordinary one it spells, and a drive or a share the same whatever case
    /// it is written in, as Windows reads them.
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Posix, Self::Posix) => true,
            (Self::Disk(left), Self::Disk(right)) => left.eq_ignore_ascii_case(right),
            (
                Self::Share { server, share },
                Self::Share {
                    server: other_server,
                    share: other_share,
                },
            ) => {
                server.eq_ignore_ascii_case(other_server) && share.eq_ignore_ascii_case(other_share)
            }
            (Self::Device(left), Self::Device(right)) => left.eq_ignore_ascii_case(right),
            _ => false,
        }
    }
}

impl<'a> Spelled<'a> {
    /// `text` read as an absolute path, or `None` where it is not one.
    fn of(text: &'a str) -> Option<Self> {
        let windows = |root, rest: &'a str, separators: &[char]| Self {
            root,
            parts: rest
                .split(separators)
                .filter(|part| !part.is_empty() && *part != ".")
                .collect(),
            windows: true,
        };
        const EITHER: &[char] = &['\\', '/'];
        // A verbatim path is taken as written: only `\` separates it.
        if let Some(verbatim) = text.strip_prefix(r"\\?\") {
            if let Some(unc) = verbatim.strip_prefix(r"UNC\") {
                let (root, rest) = share(unc, &['\\'])?;
                return Some(windows(root, rest, &['\\']));
            }
            if let Some((letter, rest)) = drive(verbatim) {
                return Some(windows(Root::Disk(letter), rest, &['\\']));
            }
            let (device, rest) = verbatim.split_once('\\').unwrap_or((verbatim, ""));
            return Some(windows(Root::Device(device), rest, &['\\']));
        }
        if let Some(device) = text.strip_prefix(r"\\.\") {
            let (device, rest) = device.split_once(EITHER).unwrap_or((device, ""));
            return Some(windows(Root::Device(device), rest, EITHER));
        }
        if let Some(unc) = text.strip_prefix(r"\\") {
            let (root, rest) = share(unc, EITHER)?;
            return Some(windows(root, rest, EITHER));
        }
        if let Some((letter, rest)) = drive(text) {
            return Some(windows(Root::Disk(letter), rest, EITHER));
        }
        let rest = text.strip_prefix('/')?;
        Some(Self {
            root: Root::Posix,
            // A `\` is an ordinary character in a POSIX name.
            parts: rest
                .split('/')
                .filter(|part| !part.is_empty() && *part != ".")
                .collect(),
            windows: false,
        })
    }
}

/// The drive an absolute Windows path is rooted in, `C:\` or `C:/`, and what
/// follows it; none for `C:` alone followed by a name, which is relative.
fn drive(text: &str) -> Option<(u8, &str)> {
    let bytes = text.as_bytes();
    let letter = *bytes.first()?;
    if !letter.is_ascii_alphabetic() || bytes.get(1) != Some(&b':') {
        return None;
    }
    match bytes.get(2) {
        None => Some((letter, "")),
        Some(b'\\' | b'/') => Some((letter, &text[3..])),
        Some(_) => None,
    }
}

/// The share `server\share` begins with, and what follows it.
fn share<'a>(text: &'a str, separators: &[char]) -> Option<(Root<'a>, &'a str)> {
    let mut parts = text.splitn(3, separators);
    let server = parts.next().filter(|server| !server.is_empty())?;
    let share = parts.next().filter(|share| !share.is_empty())?;
    Some((Root::Share { server, share }, parts.next().unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use super::within;

    #[test]
    fn a_posix_path_within_the_directory_is_shown_from_it() {
        let directory = "/work/shaman";
        for (path, shown) in [
            ("/work/shaman/src/tui/qr.rs", "src/tui/qr.rs"),
            ("/work/shaman", "."),
            ("/work/shaman/", "."),
            ("/work//shaman/./src/a.rs", "src/a.rs"),
            ("/work/shaman/a/../b.rs", "a/../b.rs"),
        ] {
            assert_eq!(within(path, directory), shown, "{path}");
        }
        assert_eq!(within("/work/shaman/src", "/work/shaman/"), "src");
    }

    #[test]
    fn a_path_elsewhere_climbing_out_or_not_absolute_is_shown_whole() {
        for path in [
            "/work/other/external.rs",
            "/work/shaman-other/sibling.rs",
            "/work/shaman/../escaped.rs",
            "relative.rs",
            r"\rooted-on-no-drive.rs",
            "C:relative-to-a-drive.rs",
        ] {
            assert_eq!(within(path, "/work/shaman"), path, "{path}");
        }
        assert_eq!(within("/work/shaman/a.rs", "relative"), "/work/shaman/a.rs");
    }

    #[test]
    fn a_windows_path_is_shown_from_its_directory_whatever_this_machine_is() {
        let directory = r"C:\work\shaman";
        for (path, shown) in [
            (r"C:\work\shaman\src\tui\qr.rs", r"src\tui\qr.rs"),
            ("C:/work/shaman/src/tui/qr.rs", r"src\tui\qr.rs"),
            (r"c:\work\shaman\src\a.rs", r"src\a.rs"),
            (r"\\?\C:\work\shaman\src\a.rs", r"src\a.rs"),
            (r"C:\work\shaman", "."),
        ] {
            assert_eq!(within(path, directory), shown, "{path}");
        }
        assert_eq!(
            within(r"C:\work\shaman\src\a.rs", r"\\?\C:\work\shaman"),
            r"src\a.rs",
            "a verbatim directory holds what its ordinary spelling does"
        );
        for elsewhere in [
            r"D:\work\shaman\src\a.rs",
            r"C:\work\shaman-other\a.rs",
            r"C:\work\shaman\..\escaped.rs",
            "/work/shaman/src/a.rs",
        ] {
            assert_eq!(within(elsewhere, directory), elsewhere, "{elsewhere}");
        }
        assert_eq!(
            within(r"C:\work\shaman\a.rs", "/work/shaman"),
            r"C:\work\shaman\a.rs",
            "a Windows path never lies in a POSIX directory"
        );
    }

    #[test]
    fn a_path_on_a_share_is_shown_from_its_directory_however_the_share_is_spelled() {
        let directory = r"\\desk\code\shaman";
        assert_eq!(
            within(r"\\desk\code\shaman\src\a.rs", directory),
            r"src\a.rs"
        );
        assert_eq!(
            within(r"\\DESK\Code\shaman\src\a.rs", directory),
            r"src\a.rs"
        );
        assert_eq!(
            within(r"\\?\UNC\desk\code\shaman\src\a.rs", directory),
            r"src\a.rs"
        );
        assert_eq!(
            within(r"\\desk\other\shaman\src\a.rs", directory),
            r"\\desk\other\shaman\src\a.rs"
        );
    }

    #[test]
    fn a_backslash_in_a_posix_name_is_part_of_the_name() {
        assert_eq!(
            within(r"/work/shaman/odd\name.rs", "/work/shaman"),
            r"odd\name.rs"
        );
        assert_eq!(
            within(r"/work/shaman\other/a.rs", "/work/shaman"),
            r"/work/shaman\other/a.rs"
        );
    }
}
