//! What of a shell's environment a Login start carries to the Server it starts,
//! and what it names as left behind.

use std::ffi::OsString;

use anyhow::Result;

use crate::provider::ProviderEnvironment;

/// What of the environment a Login start carries for every Provider alike: the
/// `PATH` each is found by, and where any of them may read its configuration
/// from. Each Provider declares the rest of what it needs carried
/// ([`ProviderEnvironment`](crate::provider::ProviderEnvironment)). All of it
/// names a place and holds no secret, and it is all a registration captures: a
/// file under the user's home is no place for the rest of a shell's
/// environment.
const CARRIED_FOR_EVERY_PROVIDER: [&str; 2] = ["PATH", "XDG_CONFIG_HOME"];

/// What of the variables `variable` reads a registration carries, of those every
/// Provider needs and those each of `providers` declares.
pub(super) fn carried_environment(
    providers: &[ProviderEnvironment],
    variable: impl Fn(&str) -> Option<OsString>,
) -> Result<Vec<(String, String)>> {
    CARRIED_FOR_EVERY_PROVIDER
        .iter()
        .chain(providers.iter().flat_map(|provider| provider.carried))
        .filter_map(|name| Some((*name, variable(name).filter(|value| !value.is_empty())?)))
        .map(|(name, value)| {
            let value = value
                .into_string()
                .map_err(|_| anyhow::anyhow!("cannot carry {name}: it is not valid Unicode"))?;
            Ok((name.to_owned(), value))
        })
        .collect()
}

/// The shapes of variable names, upper-cased, that a Server started from a shell
/// would use and that any Provider or Relay may read, beyond what each Provider
/// declares of its own.
const UNCARRIED_SUFFIXES: [&str; 4] = ["_API_KEY", "_TOKEN", "_SECRET", "_PROXY"];
const UNCARRIED_PREFIXES: [&str; 1] = ["SSL_CERT_"];
const UNCARRIED_NAMES: [&str; 3] = [
    "NODE_EXTRA_CA_CERTS",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
];

/// Which of the variables `names` a registration will not carry, because they may
/// hold a secret or belong to this shell, and which would make a Server started
/// from it behave differently from one started at login: their names only, so
/// that a Provider signing in by one, or a Relay reached through a proxy, does
/// not fail unexplained. By shape rather than by name, since no list keeps up
/// with what a Provider reads.
pub(super) fn uncarried_variables(
    providers: &[ProviderEnvironment],
    names: impl Iterator<Item = OsString>,
) -> Vec<String> {
    let carried = CARRIED_FOR_EVERY_PROVIDER
        .iter()
        .chain(providers.iter().flat_map(|provider| provider.carried));
    let carried = carried.copied().collect::<Vec<_>>();
    let mut uncarried = names
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| {
            let upper = name.to_ascii_uppercase();
            !carried.contains(&upper.as_str())
                && (UNCARRIED_SUFFIXES
                    .iter()
                    .any(|suffix| upper.ends_with(suffix))
                    || UNCARRIED_NAMES.contains(&upper.as_str())
                    || UNCARRIED_PREFIXES
                        .iter()
                        .chain(
                            providers
                                .iter()
                                .flat_map(|provider| provider.uncarried_prefixes),
                        )
                        .any(|prefix| upper.starts_with(prefix)))
        })
        .collect::<Vec<_>>();
    uncarried.sort();
    uncarried.dedup();
    uncarried
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEX_LIKE: ProviderEnvironment = ProviderEnvironment {
        carried: &["SURU_CODEX_PATH", "CODEX_HOME"],
        uncarried_prefixes: &["OPENAI_"],
    };

    /// The secrets a shell holds go nowhere near a registration: only what
    /// names a place is carried.
    #[test]
    fn only_what_finds_a_provider_is_carried_and_never_a_secret() {
        let shell = |name: &str| {
            let value = match name {
                "PATH" => "/opt/bin",
                "SURU_CODEX_PATH" => "/opt/codex",
                "CODEX_HOME" => "",
                "ANTHROPIC_API_KEY" => "secret",
                "AWS_SECRET_ACCESS_KEY" => "secret",
                "HTTPS_PROXY" => "http://me:secret@proxy",
                _ => return None,
            };
            Some(OsString::from(value))
        };
        assert_eq!(
            carried_environment(&[CODEX_LIKE], shell).unwrap(),
            [
                ("PATH".to_owned(), "/opt/bin".to_owned()),
                ("SURU_CODEX_PATH".to_owned(), "/opt/codex".to_owned()),
            ],
        );
    }

    /// A Provider is carried what it declares: with none declared here, its
    /// variables go no further than the shell.
    #[test]
    fn what_each_provider_declares_is_what_is_carried_for_it() {
        let shell = |name: &str| (name == "SURU_CODEX_PATH").then(|| OsString::from("/codex"));
        assert!(carried_environment(&[], shell).unwrap().is_empty());
        assert_eq!(carried_environment(&[CODEX_LIKE], shell).unwrap().len(), 1);
    }

    /// A list of exact names is what let `https_proxy`, `CLAUDE_CODE_OAUTH_TOKEN`
    /// and the rest slip through: by shape they cannot, in either case, and only
    /// their names ever go anywhere.
    #[test]
    fn variables_are_named_by_their_shape_and_those_carried_are_not_among_them() {
        let names = [
            "PATH",
            "HOME",
            "TERM",
            "SSH_AUTH_SOCK",
            "XDG_CONFIG_HOME",
            "CODEX_HOME",
            "SURU_CODEX_PATH",
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "CODEX_API_KEY",
            "SOME_API_KEY",
            "SOME_TOKEN",
            "some_token",
            "https_proxy",
            "NO_PROXY",
            "NODE_EXTRA_CA_CERTS",
            "SSL_CERT_FILE",
        ];
        assert_eq!(
            uncarried_variables(&[CODEX_LIKE], names.into_iter().map(OsString::from)),
            [
                "CODEX_API_KEY",
                "NODE_EXTRA_CA_CERTS",
                "NO_PROXY",
                "OPENAI_API_KEY",
                "OPENAI_BASE_URL",
                "SOME_API_KEY",
                "SOME_TOKEN",
                "SSL_CERT_FILE",
                "https_proxy",
                "some_token",
            ]
        );
    }

    /// What the review found slipping through the list of names, against the
    /// Providers' own declarations.
    #[test]
    fn what_the_built_in_providers_read_is_named_when_set() {
        let providers = crate::provider::built_in_environments();
        for name in [
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_USE_BEDROCK",
            "ANTHROPIC_BASE_URL",
            "AWS_PROFILE",
            "OPENAI_ORG_ID",
            "GH_ENTERPRISE_HOST",
            "GITHUB_ENTERPRISE",
            "COPILOT_MODEL",
        ] {
            assert_eq!(
                uncarried_variables(&providers, std::iter::once(OsString::from(name))),
                [name]
            );
        }
        for carried in [
            "SURU_CLAUDE_PATH",
            "CLAUDE_CONFIG_DIR",
            "COPILOT_HOME",
            "CODEX_HOME",
        ] {
            assert!(
                uncarried_variables(&providers, std::iter::once(OsString::from(carried)))
                    .is_empty(),
                "{carried}"
            );
        }
    }
}
