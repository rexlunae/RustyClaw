//! Which search backend a `web_search` call goes to.
//!
//! Kept out of both provider modules so the choice is one readable function
//! with its own tests, rather than a chain of `if let Ok(_) = env::var(..)`
//! threaded through the Brave path (issue #140).

use serde_json::Value;

/// A `web_search` backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provider {
    /// Keyword search. Cheap, fast, the default when its key is present.
    Brave,
    /// Semantic search with page text. Costs more per query.
    Exa,
}

/// Whether a key is configured, as a value so the choice can be tested
/// without setting process-wide environment variables and racing every other
/// test in the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Keys {
    pub brave: bool,
    pub exa: bool,
}

impl Keys {
    fn from_env() -> Self {
        Self {
            brave: std::env::var("BRAVE_API_KEY").is_ok_and(|v| !v.trim().is_empty()),
            exa: std::env::var("EXA_API_KEY").is_ok_and(|v| !v.trim().is_empty()),
        }
    }
}

/// Pick a provider from the caller's argument and the configured keys.
pub(crate) fn choose(args: &Value) -> Result<Provider, String> {
    decide(
        args.get("provider").and_then(|v| v.as_str()),
        Keys::from_env(),
    )
}

/// The decision itself.
///
/// An explicit `provider` is obeyed even when its key is missing, so the
/// error names the key the caller actually needs rather than quietly
/// searching somewhere else and returning results from a service they did not
/// ask for.
pub(crate) fn decide(requested: Option<&str>, keys: Keys) -> Result<Provider, String> {
    match requested.map(str::trim) {
        Some("brave") => Ok(Provider::Brave),
        Some("exa") => Ok(Provider::Exa),
        Some("auto") | None => auto(keys),
        Some(other) => Err(format!(
            "Unsupported search provider `{other}`. Use auto, brave or exa."
        )),
    }
}

/// Brave first: it is the cheaper of the two and was the only one before this,
/// so a configuration that worked yesterday keeps behaving the same today.
/// Falling through to Exa is the new part — `web_search` used to fail outright
/// with "BRAVE_API_KEY not set" even for someone who had configured Exa.
fn auto(keys: Keys) -> Result<Provider, String> {
    match (keys.brave, keys.exa) {
        (true, _) => Ok(Provider::Brave),
        (false, true) => Ok(Provider::Exa),
        (false, false) => Err("No search provider is configured. Set BRAVE_API_KEY \
             (https://brave.com/search/api/) or EXA_API_KEY (https://exa.ai/)."
            .to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOTH: Keys = Keys {
        brave: true,
        exa: true,
    };
    const ONLY_EXA: Keys = Keys {
        brave: false,
        exa: true,
    };
    const NEITHER: Keys = Keys {
        brave: false,
        exa: false,
    };

    /// The behaviour that existed before Exa, unchanged for anyone with a
    /// Brave key.
    #[test]
    fn auto_prefers_brave_when_both_are_configured() {
        assert_eq!(decide(None, BOTH), Ok(Provider::Brave));
        assert_eq!(decide(Some("auto"), BOTH), Ok(Provider::Brave));
    }

    /// The new part. `web_search` used to fail with "BRAVE_API_KEY not set"
    /// even when the caller had configured Exa.
    #[test]
    fn auto_falls_through_to_exa_when_brave_is_not_configured() {
        assert_eq!(decide(None, ONLY_EXA), Ok(Provider::Exa));
    }

    /// An explicit choice is obeyed even with no key, so the error names the
    /// key the caller needs. Silently using the other provider would return
    /// results from a service they did not ask for.
    #[test]
    fn an_explicit_provider_is_obeyed_even_without_its_key() {
        assert_eq!(decide(Some("exa"), NEITHER), Ok(Provider::Exa));
        assert_eq!(decide(Some("brave"), ONLY_EXA), Ok(Provider::Brave));
    }

    /// With nothing configured, the message names both keys — the previous
    /// error named only Brave.
    #[test]
    fn with_no_keys_the_error_names_both_options() {
        let err = decide(None, NEITHER).expect_err("nothing to search with");
        assert!(err.contains("BRAVE_API_KEY"), "got: {err}");
        assert!(err.contains("EXA_API_KEY"), "got: {err}");
    }

    /// A typo becomes a named refusal rather than a silent fall back to Brave,
    /// which would look like it worked.
    #[test]
    fn an_unknown_provider_is_refused_by_name() {
        let err = decide(Some("google"), BOTH).expect_err("must be refused");
        assert!(err.contains("google"), "got: {err}");
        assert!(
            err.contains("exa"),
            "the message should list what works: {err}"
        );
    }

    #[test]
    fn the_argument_is_read_from_the_tool_call() {
        assert_eq!(
            decide(
                json!({"provider": "exa"})
                    .get("provider")
                    .and_then(|v| v.as_str()),
                BOTH
            ),
            Ok(Provider::Exa)
        );
    }
}
