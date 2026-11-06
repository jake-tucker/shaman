//! Which paths will still be there later: the binary a registration names, after
//! an upgrade, and the entries of the `PATH` it keeps, after a reboot. Heuristics
//! all, and only ever a reason to warn.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// Why a path a registration would name may not outlast an upgrade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShortLived {
    Build,
    Versioned,
    Temporary,
}

impl ShortLived {
    pub(super) fn says(self) -> &'static str {
        match self {
            Self::Build => "it is in a build directory",
            Self::Versioned => "it is in a directory named for one version",
            Self::Temporary => "it is in a temporary directory",
        }
    }
}

/// Why `path` may not outlast an upgrade, if there is a reason to think so.
/// Only ever a reason to warn: a path that looks short-lived may be the one the
/// user means to keep.
pub(super) fn short_lived(path: &Path, temporary: &Path) -> Option<ShortLived> {
    let is_named = |names: &[&str]| {
        path.components()
            .any(|component| names.iter().any(|name| component.as_os_str() == *name))
    };
    let profile_directory = path.parent().filter(|parent| {
        parent
            .file_name()
            .is_some_and(|name| name == "debug" || name == "release")
    });
    // Cargo marks the top of every target directory, however it was named.
    let built = profile_directory.is_some_and(|profile| {
        is_named(&["target"])
            || profile
                .parent()
                .is_some_and(|target| target.join("CACHEDIR.TAG").is_file())
    });
    if built {
        return Some(ShortLived::Build);
    }
    if is_temporary(path, temporary) {
        return Some(ShortLived::Temporary);
    }
    let versioned = path
        .components()
        .any(|component| component.as_os_str().to_str().is_some_and(names_a_version));
    if versioned || is_named(&["Cellar", "Caskroom", "nix"]) {
        return Some(ShortLived::Versioned);
    }
    None
}

/// Directories whose contents do not last, wherever the platform keeps its own
/// temporary directory. On macOS that is under `/var/folders`, and `/tmp` is
/// still a place an archive is unpacked to.
const TEMPORARY_DIRECTORIES: [&str; 3] = ["/tmp", "/private/tmp", "/var/tmp"];

fn is_temporary(path: &Path, temporary: &Path) -> bool {
    TEMPORARY_DIRECTORIES
        .iter()
        .any(|directory| path.starts_with(directory))
        || [
            Some(temporary.to_path_buf()),
            crate::paths::canonical(temporary).ok(),
        ]
        .iter()
        .flatten()
        .any(|temporary| path.starts_with(temporary))
}

/// Whether a path's `component` names a version anywhere in it: digits and a dot
/// and digits, standing apart from what is around them, so that `suru-v0.4.1-x`
/// and `app-1.2.3` do and `python3.12`, `jake2` and `x86_64` do not.
fn names_a_version(component: &str) -> bool {
    let is_number = |part: Option<&str>| {
        part.is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    };
    component
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '.'))
        .any(|token| {
            let mut parts = token.strip_prefix(['v', 'V']).unwrap_or(token).split('.');
            is_number(parts.next()) && is_number(parts.next())
        })
}

/// The entries of the `PATH` in `path_variable` that a reboot takes away: those
/// under a temporary directory or under `/run/user`, and the per-shell directories
/// fnm makes wherever it makes them. A Provider found only through one is found
/// by the shell that made it, and not by a Server started at the next login.
pub(super) fn fleeting_path_entries(path_variable: &OsStr, temporary: &Path) -> Vec<String> {
    std::env::split_paths(path_variable)
        .filter(|entry| {
            is_temporary(entry, temporary)
                || entry.starts_with("/run/user")
                || entry
                    .components()
                    .any(|component| component.as_os_str() == "fnm_multishells")
        })
        .map(|entry| entry.display().to_string())
        .collect()
}

/// The path a registration names for the binary at `current`, and why it may
/// not outlast an upgrade where it may not.
///
/// What this process knows itself as may be the far end of a link: on Linux
/// `current_exe` resolves through them, so Homebrew's `bin/suru` reads as a
/// Cellar path naming one version, which the next upgrade removes while the
/// link goes on naming whichever is current. So where `current` looks
/// short-lived, a `PATH` entry that leads to this same binary and does not is
/// named instead.
pub(super) fn registered_executable(
    current: &Path,
    path_variable: Option<&OsStr>,
    temporary: &Path,
) -> (PathBuf, Option<ShortLived>) {
    let Some(why) = short_lived(current, temporary) else {
        return (current.to_path_buf(), None);
    };
    let (Some(name), Ok(binary), Some(path_variable)) = (
        current.file_name(),
        crate::paths::canonical(current),
        path_variable,
    ) else {
        return (current.to_path_buf(), Some(why));
    };
    let lasting = std::env::split_paths(path_variable)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(name))
        .find(|candidate| {
            crate::paths::canonical(candidate).is_ok_and(|resolved| resolved == binary)
                && short_lived(candidate, temporary).is_none()
        });
    match lasting {
        Some(lasting) => (lasting, None),
        None => (current.to_path_buf(), Some(why)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rooted(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        } else {
            PathBuf::from(path)
        }
    }

    #[test]
    fn paths_that_will_not_outlast_an_upgrade_are_told_from_those_that_will() {
        let temporary = rooted("/scratch/tmp");
        for (path, expected) in [
            ("/home/jo/.local/bin/suru", None),
            ("/usr/local/bin/suru", None),
            ("/opt/homebrew/bin/suru", None),
            ("/home/jo/python3.12/suru", None),
            ("/home/jo/suru/target/debug/suru", Some(ShortLived::Build)),
            ("/home/jo/suru/target/release/suru", Some(ShortLived::Build)),
            (
                "/home/jo/suru/target/x86_64-unknown-linux-gnu/release/suru",
                Some(ShortLived::Build),
            ),
            (
                "/opt/homebrew/Cellar/suru/1.2.3/bin/suru",
                Some(ShortLived::Versioned),
            ),
            (
                "/opt/homebrew/Cellar/suru/HEAD/bin/suru",
                Some(ShortLived::Versioned),
            ),
            (
                "/home/jo/.asdf/installs/suru/v0.4.1/suru",
                Some(ShortLived::Versioned),
            ),
            ("/nix/store/abc-suru/bin/suru", Some(ShortLived::Versioned)),
            ("/scratch/tmp/unpacked/suru", Some(ShortLived::Temporary)),
            (
                "/Users/jo/Downloads/suru-v0.4.1-aarch64-apple-darwin/suru",
                Some(ShortLived::Versioned),
            ),
            ("/opt/app-1.2.3/suru", Some(ShortLived::Versioned)),
            ("/opt/suru@1.2/bin/suru", Some(ShortLived::Versioned)),
            ("/opt/suru_0.4.1-rc1/suru", Some(ShortLived::Versioned)),
            ("/home/jo/bin/suru-0.4.1", Some(ShortLived::Versioned)),
            ("/home/jake2/bin/suru", None),
            ("/home/jo/.cargo/bin/suru", None),
            ("/usr/local/lib/python3.12/suru", None),
            ("/home/jo/v2/suru", None),
            ("/home/jo/x86_64/suru", None),
            ("/home/jo/1.notes/suru", None),
        ] {
            assert_eq!(short_lived(&rooted(path), &temporary), expected, "{path}");
        }
    }

    /// `$TMPDIR` on a Mac is under `/var/folders`, and `/tmp` is no less temporary
    /// for not being it.
    #[test]
    fn the_usual_temporary_directories_count_whatever_the_temporary_directory_is() {
        let temporary = rooted("/scratch/tmp");
        for path in ["/tmp/suru", "/private/tmp/x/suru", "/var/tmp/suru"] {
            assert_eq!(
                short_lived(Path::new(path), &temporary),
                Some(ShortLived::Temporary),
                "{path}"
            );
        }
        assert_eq!(short_lived(Path::new("/tmpfiles/suru"), &temporary), None);
    }

    #[test]
    fn path_entries_that_a_reboot_takes_away_are_named() {
        let temporary = rooted("/scratch/tmp");
        let path_variable = std::env::join_paths([
            rooted("/usr/bin"),
            PathBuf::from("/run/user/1000/fnm_multishells/123_456/bin"),
            rooted("/home/jo/.local/state/fnm_multishells/9_8/bin"),
            PathBuf::from("/tmp/scratch/bin"),
            rooted("/scratch/tmp/node/bin"),
            rooted("/home/jo/.local/bin"),
        ])
        .unwrap();
        assert_eq!(
            fleeting_path_entries(&path_variable, &temporary),
            [
                "/run/user/1000/fnm_multishells/123_456/bin".to_owned(),
                rooted("/home/jo/.local/state/fnm_multishells/9_8/bin")
                    .display()
                    .to_string(),
                "/tmp/scratch/bin".to_owned(),
                rooted("/scratch/tmp/node/bin").display().to_string(),
            ]
        );
    }

    /// Cargo's target directory may go by any name, and is known by its mark.
    #[test]
    fn a_target_directory_under_another_name_is_a_build_directory() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let target = root.path().join("cargo-target");
        std::fs::create_dir_all(target.join("debug")).unwrap();
        std::fs::write(target.join("CACHEDIR.TAG"), "").unwrap();
        assert_eq!(
            short_lived(&target.join("debug").join("suru"), elsewhere.path()),
            Some(ShortLived::Build)
        );
    }

    #[test]
    fn a_lasting_path_is_named_as_it_is() {
        let temporary = rooted("/scratch/tmp");
        let current = rooted("/home/jo/.local/bin/suru");
        assert_eq!(
            registered_executable(&current, None, &temporary),
            (current, None)
        );
    }

    /// Homebrew's link is what lasts; the Cellar path it leads to is gone
    /// after the next upgrade. No `PATH` entry leading elsewhere is taken for it.
    #[cfg(unix)]
    #[test]
    fn a_link_on_path_is_named_rather_than_the_version_it_leads_to() {
        let root = tempfile::tempdir().unwrap();
        let root = crate::paths::canonical(root.path()).unwrap();
        let temporary = rooted("/scratch/tmp");
        let cellar = root.join("Cellar").join("suru").join("1.2.3").join("bin");
        let links = root.join("links");
        let other = root.join("other");
        for directory in [&cellar, &links, &other] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let binary = cellar.join("suru");
        std::fs::write(&binary, "").unwrap();
        std::fs::write(other.join("suru"), "another binary").unwrap();
        std::os::unix::fs::symlink(&binary, links.join("suru")).unwrap();

        let path_variable = std::env::join_paths([&other, &links]).unwrap();
        assert_eq!(
            registered_executable(&binary, Some(&path_variable), &temporary),
            (links.join("suru"), None)
        );

        let no_link = std::env::join_paths([&other]).unwrap();
        assert_eq!(
            registered_executable(&binary, Some(&no_link), &temporary),
            (binary, Some(ShortLived::Versioned))
        );
    }
}
