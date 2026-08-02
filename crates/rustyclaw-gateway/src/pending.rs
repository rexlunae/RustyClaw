//! Routing client responses back to the call that asked for them.
//!
//! Tool approvals, `ask_user` answers, credential prompts and DOM queries all
//! travel the same way: the gateway sends a request carrying a call id, and
//! the client eventually sends a response carrying that same id back.
//!
//! These used to be one mpsc channel per connection, shared by whoever held
//! its lock. That works only while a single turn can be in flight, and even
//! then it works badly: a waiter that receives an id it does not recognise
//! has already *consumed* it, so the answer to another call is destroyed on
//! arrival — and at the approval site, an unrecognised id is read as a
//! denial, so a tool the user never saw is refused in their name.
//!
//! Here each call registers its own id and gets its own one-shot. The reader
//! delivers by id, so two turns can wait on the same kind of response at once
//! without either taking the other's answer. An answer to a call nobody is
//! waiting on is logged and dropped, not mistaken for something else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// The set of calls currently waiting for a response of one kind.
///
/// Locking is a plain `std::sync::Mutex`: every critical section here is a
/// single map operation with no `await` inside it, which is also what keeps
/// the guard from ever crossing a suspension point.
#[derive(Debug)]
pub struct PendingResponses<T> {
    waiters: Mutex<HashMap<String, oneshot::Sender<T>>>,
}

impl<T> Default for PendingResponses<T> {
    fn default() -> Self {
        Self {
            waiters: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> PendingResponses<T> {
    /// Claim `id` and get back the handle to wait on.
    ///
    /// Registering an id that is already claimed replaces the old waiter,
    /// whose receiver then resolves to a cancellation. Call ids come from the
    /// model and are expected to be unique; if one repeats, the newer call is
    /// the one that can still be answered.
    pub fn register(self: &Arc<Self>, id: impl Into<String>) -> Pending<T> {
        let id = id.into();
        let (tx, rx) = oneshot::channel();
        self.waiters
            .lock()
            .expect("pending responses mutex poisoned")
            .insert(id.clone(), tx);
        Pending {
            registry: Arc::clone(self),
            id,
            rx,
        }
    }

    /// Hand a response to whoever is waiting for it.
    ///
    /// Returns whether anyone was. `false` means the call was abandoned —
    /// cancelled by Stop, or timed out — and the answer has nowhere to go.
    /// It is dropped, and no other call is disturbed by it.
    pub fn deliver(&self, id: &str, value: T) -> bool {
        let waiter = self
            .waiters
            .lock()
            .expect("pending responses mutex poisoned")
            .remove(id);
        match waiter {
            // The receiver is gone if the waiting side stopped between the
            // lookup and now. Same outcome as never having been there.
            Some(tx) => tx.send(value).is_ok(),
            None => false,
        }
    }

    /// Give up on `id` without answering it.
    fn forget(&self, id: &str) {
        self.waiters
            .lock()
            .expect("pending responses mutex poisoned")
            .remove(id);
    }

    /// How many calls are outstanding.
    #[cfg(test)]
    pub fn outstanding(&self) -> usize {
        self.waiters
            .lock()
            .expect("pending responses mutex poisoned")
            .len()
    }
}

/// A claim on one call id, and the handle to await its response.
///
/// Dropping this releases the claim, so a call abandoned by Stop or a timeout
/// leaves nothing behind in the registry. That matters more than it looks:
/// without it, every cancelled `ask_user` would leak an entry for the life of
/// the connection, and a client answering late would find a waiter that no
/// longer has anywhere to deliver.
#[derive(Debug)]
pub struct Pending<T> {
    registry: Arc<PendingResponses<T>>,
    id: String,
    rx: oneshot::Receiver<T>,
}

impl<T> Pending<T> {
    /// The handle to poll. Borrowed rather than consumed so a caller can
    /// wait in a loop — watching a cancel flag between polls, say — without
    /// giving up its claim on the id.
    pub fn rx(&mut self) -> &mut oneshot::Receiver<T> {
        &mut self.rx
    }
}

impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        self.registry.forget(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer reaches the call that asked for it, and only that one.
    #[tokio::test]
    async fn a_response_goes_to_the_call_that_asked() {
        let registry: Arc<PendingResponses<bool>> = Arc::new(PendingResponses::default());
        let mut first = registry.register("call-1");
        let mut second = registry.register("call-2");

        assert!(registry.deliver("call-2", true));

        assert_eq!(second.rx().await, Ok(true));
        // The other call is untouched and still waiting.
        assert!(first.rx().try_recv().is_err());
        assert_eq!(registry.outstanding(), 1);
    }

    /// Two turns can wait on the same kind of response at once.
    ///
    /// With one shared channel this was impossible: whichever turn held the
    /// lock consumed the other's answer and, at the approval site, read the
    /// unrecognised id as a denial — refusing a tool the user had just
    /// approved, in their name. This is the reason concurrent turns were not
    /// allowed, and removing it is what allows them.
    #[tokio::test]
    async fn one_turns_answer_is_not_eaten_by_another() {
        let registry: Arc<PendingResponses<bool>> = Arc::new(PendingResponses::default());
        let mut turn_a = registry.register("a-tool");
        let mut turn_b = registry.register("b-tool");

        // The user answers B first — out of order, which is normal.
        assert!(registry.deliver("b-tool", true));
        assert!(registry.deliver("a-tool", false));

        assert_eq!(turn_b.rx().await, Ok(true));
        assert_eq!(turn_a.rx().await, Ok(false));
        assert_eq!(registry.outstanding(), 0);
    }

    /// An answer nobody is waiting for is dropped, not misdelivered.
    #[tokio::test]
    async fn an_answer_to_an_abandoned_call_disturbs_nothing() {
        let registry: Arc<PendingResponses<bool>> = Arc::new(PendingResponses::default());
        let mut live = registry.register("live");

        assert!(
            !registry.deliver("never-asked", true),
            "nobody claimed that id"
        );
        assert!(
            live.rx().try_recv().is_err(),
            "the live call must not receive somebody else's answer"
        );
    }

    /// Giving up on a call releases its id.
    ///
    /// Stop ends an `ask_user` wait without an answer. If the claim outlived
    /// the wait, every cancelled question would leak an entry for the life of
    /// the connection.
    #[tokio::test]
    async fn abandoning_a_call_releases_its_id() {
        let registry: Arc<PendingResponses<bool>> = Arc::new(PendingResponses::default());
        {
            let _claim = registry.register("call-1");
            assert_eq!(registry.outstanding(), 1);
        }
        assert_eq!(registry.outstanding(), 0);
        assert!(
            !registry.deliver("call-1", true),
            "a late answer to an abandoned call has nowhere to go"
        );
    }
}
