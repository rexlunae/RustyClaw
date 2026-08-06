//! Discarding a `Result` on purpose, visibly.
//!
//! Most fallible calls in this workspace matter: a failed write loses data, a
//! failed send loses a user's command. Those get handled. But a real minority
//! genuinely do not — a notification to a receiver that has already hung up
//! during shutdown, an unlink of a temporary file on a path that is already
//! cleaning up after an error. For those, the failure is not information.
//!
//! `let _ = …` says that badly. It is the same three characters whether the
//! author weighed the failure and dismissed it or never noticed the call could
//! fail at all, and the audit in `CODE_QUALITY_PASS.md` found plenty of the
//! second kind hiding among the first. Worse, it is invisible to the lint that
//! would catch new ones: `let _ =` is an explicit discard, so `unused_must_use`
//! stays quiet, and `clippy::let_underscore_must_use` is not in the `all` group
//! this workspace denies.
//!
//! Calling [`Ignore::ignore`] is a decision written down. It costs a word at
//! each site, it greps, and it lets the lint be turned on everywhere else — so
//! the next `let _ =` on a fallible call is a build error rather than a thing
//! someone has to notice in review.
//!
//! ```
//! # use rustyclaw_core::ignore::Ignore;
//! # let (tx, rx) = std::sync::mpsc::channel::<u8>();
//! # drop(rx);
//! // The receiver is gone because we are shutting down. That is the
//! // expected ending, not a failure to report.
//! tx.send(1).ignore();
//! ```
//!
//! It is deliberately only implemented for `Result`. A `#[must_use]` value of
//! some other shape is rarer and usually wants a sentence about why it is being
//! dropped, which an `#[allow(clippy::let_underscore_must_use)]` and a comment
//! give room for.
//!
//! Reach for it only where the failure truly carries nothing. If the answer to
//! "what happens when this fails?" is anything other than "nothing anyone can
//! act on", the call wants `?` or a `tracing::warn!`, not this.

/// Discard a `Result` whose failure needs no handling.
pub trait Ignore {
    /// Drop the result, having decided its error carries nothing to act on.
    fn ignore(self);
}

impl<T, E> Ignore for Result<T, E> {
    #[inline]
    fn ignore(self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both ends compile and neither warns — which is the entire point, since
    /// the value of this trait is that it satisfies the lint that `let _ =`
    /// silently sidesteps.
    #[test]
    fn a_result_can_be_ignored_either_way() {
        let ok: Result<u8, String> = Ok(1);
        ok.ignore();

        let err: Result<u8, String> = Err("no receiver".into());
        err.ignore();
    }

    /// The value is dropped, not leaked — an ignored error still runs whatever
    /// its own `Drop` does.
    #[test]
    fn ignoring_drops_the_value() {
        use std::sync::Arc;

        let counted = Arc::new(());
        let result: Result<Arc<()>, ()> = Ok(Arc::clone(&counted));
        assert_eq!(Arc::strong_count(&counted), 2);

        result.ignore();

        assert_eq!(
            Arc::strong_count(&counted),
            1,
            "the ignored value should have been dropped, not held"
        );
    }
}
