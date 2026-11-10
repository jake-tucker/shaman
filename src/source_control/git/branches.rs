//! Which branch names are free for a Managed Worktree to take.
use super::*;
use crate::source_control::naming;

impl GitSourceControl {
    /// Every local branch's short name, read in one listing so a whole run of
    /// numbered names is judged against the same state.
    pub(super) async fn branch_names(&self, directory: &Path) -> Result<Vec<String>, String> {
        let output = self
            .command(
                directory,
                &["for-each-ref", "--format=%(refname)", "refs/heads/"],
            )
            .await?;
        if !output.status.success() {
            return Err(format!(
                "Git could not list branches: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|reference| reference.strip_prefix("refs/heads/"))
            .map(str::to_owned)
            .collect())
    }
}

/// The branch a Managed Worktree called `name` asks for beside the `existing`
/// branches: `name` behind [`naming::BRANCH_PREFIX`]. A branch named as the
/// prefix itself stands where Git would need a directory and would refuse every
/// name behind it, so there the prefix is joined on with `-` instead.
pub(super) fn managed_branch(existing: &[String], name: &str) -> String {
    let directory = naming::BRANCH_PREFIX.trim_end_matches('/');
    if existing
        .iter()
        .any(|branch| branch.eq_ignore_ascii_case(directory))
    {
        format!("{directory}-{name}")
    } else {
        format!("{}{name}", naming::BRANCH_PREFIX)
    }
}

/// Whether `branch` may be created beside the `existing` branches. Git refuses
/// a branch whose name another uses as a directory (`feature/fix` beside
/// `feature/fix/old`) or that needs an existing branch to be one (`feature/fix/old`
/// beside `feature/fix`), and on a case-insensitive filesystem a loose ref that
/// differs only in case is the same file. All of them count as taken on every
/// platform, so a name never depends on where its Repository happens to live.
pub(super) fn branch_available(existing: &[String], branch: &str) -> bool {
    let wanted = branch.to_lowercase();
    !existing.iter().any(|name| {
        let name = name.to_lowercase();
        let nested = |outer: &str, inner: &str| {
            inner
                .strip_prefix(outer)
                .is_some_and(|rest| rest.starts_with('/'))
        };
        name == wanted || nested(&wanted, &name) || nested(&name, &wanted)
    })
}

#[cfg(test)]
mod tests {
    use super::{branch_available, managed_branch};

    #[test]
    fn a_name_is_taken_by_its_case_variants_and_by_directory_conflicts() {
        let existing = [
            "main".to_owned(),
            "feature/Fix-Flicker".to_owned(),
            "feature/ship-picker/old".to_owned(),
            "feature/notes".to_owned(),
        ];
        for (branch, available) in [
            ("feature/fix-flicker", false),
            ("FEATURE/FIX-FLICKER", false),
            ("feature/ship-picker", false),
            ("feature/notes/today", false),
            ("MAIN", false),
            ("feature/fix-flicker-2", true),
            ("feature/ship", true),
            ("feature/ship-picker-2", true),
            ("feature/note", true),
        ] {
            assert_eq!(branch_available(&existing, branch), available, "{branch}");
        }
    }

    #[test]
    fn a_branch_named_as_the_prefix_moves_the_name_out_from_under_it() {
        let named = |existing: &[&str]| {
            let existing = existing.iter().map(|&b| b.to_owned()).collect::<Vec<_>>();
            managed_branch(&existing, "fix-parser")
        };
        assert_eq!(named(&["main"]), "feature/fix-parser");
        assert_eq!(named(&["main", "features"]), "feature/fix-parser");
        assert_eq!(named(&["main", "feature/other"]), "feature/fix-parser");
        assert_eq!(named(&["main", "feature"]), "feature-fix-parser");
        assert_eq!(named(&["main", "Feature"]), "feature-fix-parser");
    }
}
