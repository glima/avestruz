// SPDX-License-Identifier: MIT

//! What views ask for that has to be fetched: the asking, apart from what
//! is asked for.
//!
//! A view is built from a model it can't change, and is built again and
//! again. What it finds missing it asks for through [`Requests::ask`], which
//! sends the request down a channel once however often it is asked. A
//! subscription hands what the channel carries to the app, which starts the
//! fetch if [`Requests::begin`] lets it: so many are on their way at once,
//! and no more. Only what is on screen is ever built, so only what is on
//! screen is ever asked for.
//!
//! ```mermaid
//! stateDiagram-v2
//!     [*] --> Asked: a view finds it missing, and asks
//!     Asked --> Asked: built again, the view asks again. Nothing is sent twice
//!     Asked --> InFlight: the app takes it up, and fewer than so many are on their way
//!     Asked --> [*]: the app takes it up, but it is had already, or too many are on their way
//!     InFlight --> [*]: fetched, or failed
//! ```
//!
//! One that was turned away because too many were on their way is asked
//! for again by the next view that still needs it, which is the next
//! frame if it is still on screen and never if it scrolled away. So
//! nothing is fetched for a row that was only passed by.
//!
//! The whole road a picture takes, from a row to the web and back, is in
//! [`crate::guide::flows`].

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use cosmic::iced::Subscription;
use futures_util::SinkExt;
use tokio::sync::{Mutex, mpsc};

/// The receiving end of some requests, as the app model keeps it for the
/// subscription that drains it.
pub type Inbox<K> = Arc<Mutex<mpsc::Receiver<K>>>;

/// What views asked for through the [`Requests`] that `inbox` receives
/// from, each as the message `message` makes of it. The app decides which
/// are actually fetched.
pub fn subscription<K, M>(inbox: &Inbox<K>, message: fn(K) -> M) -> Subscription<M>
where
    K: Send + 'static,
    M: Send + 'static,
{
    /// An inbox and what to make of what it carries, hashed by where they
    /// are so that they can key a `run_with` subscription.
    struct Drain<K, M>(Inbox<K>, fn(K) -> M);

    impl<K, M> Hash for Drain<K, M> {
        fn hash<H: Hasher>(&self, state: &mut H) {
            Arc::as_ptr(&self.0).hash(state);
        }
    }

    Subscription::run_with(Drain(inbox.clone(), message), |drain: &Drain<K, M>| {
        let (requests, message) = (drain.0.clone(), drain.1);
        cosmic::iced::stream::channel(64, async move |mut channel| {
            let mut requests = requests.lock().await;
            while let Some(request) = requests.recv().await {
                if channel.send(message(request)).await.is_err() {
                    break;
                }
            }
            futures_util::future::pending().await
        })
    })
}

/// The requests for one kind of thing, each named by a `K`.
pub struct Requests<K> {
    /// On their way.
    in_flight: HashSet<K>,
    /// Sent down `sender` and not taken up by the app yet, so that a view
    /// built many times asks once.
    asked: RefCell<HashSet<K>>,
    sender: mpsc::Sender<K>,
    /// How many may be on their way at once.
    max_in_flight: usize,
}

impl<K: Clone + Eq + Hash> Requests<K> {
    /// Requests of which `max_in_flight` may be on their way at once and
    /// `queue` may wait between a view asking and the app answering, and
    /// the receiving end of them. A request that doesn't fit the queue is
    /// dropped: what is still on screen is asked for again the next time
    /// the view is built.
    pub fn new(queue: usize, max_in_flight: usize) -> (Self, mpsc::Receiver<K>) {
        let (sender, receiver) = mpsc::channel(queue);
        (Self { in_flight: HashSet::new(), asked: RefCell::new(HashSet::new()), sender, max_in_flight }, receiver)
    }

    /// Ask for `key`, unless it was asked for or is on its way already.
    pub fn ask(&self, key: &K) {
        if !self.in_flight.contains(key) && !self.asked.borrow().contains(key) && self.sender.try_send(key.clone()).is_ok() {
            self.asked.borrow_mut().insert(key.clone());
        }
    }

    /// The request for `key` reached the app; `wanted` says whether there
    /// is still a use for what it asks. Returns whether to start fetching:
    /// not what is on its way already, and nothing while as many as may be
    /// are. A request turned away for that last reason is simply asked
    /// again by the next view that needs it.
    pub fn begin(&mut self, key: &K, wanted: bool) -> bool {
        self.asked.get_mut().remove(key);
        wanted && self.in_flight.len() < self.max_in_flight && self.in_flight.insert(key.clone())
    }

    /// The fetch of `key` ended, one way or the other.
    pub fn end(&mut self, key: &K) {
        self.in_flight.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::Requests;

    #[test]
    fn what_is_asked_for_often_is_asked_for_once_and_fetched_once() {
        let (mut requests, mut receiver) = Requests::new(8, 2);

        // A view is built several times before the app gets to the request.
        for _ in 0..3 {
            requests.ask(&"a");
        }
        assert_eq!(receiver.try_recv().ok(), Some("a"));
        assert!(receiver.try_recv().is_err(), "asked once");

        // On its way: not asked for again, and not begun again.
        assert!(requests.begin(&"a", true));
        requests.ask(&"a");
        assert!(receiver.try_recv().is_err());
        assert!(!requests.begin(&"a", true));

        // Once it has ended, whoever keeps what came decides what is asked.
        requests.end(&"a");
        requests.ask(&"a");
        assert_eq!(receiver.try_recv().ok(), Some("a"));
        assert!(!requests.begin(&"a", false), "it is here already: nothing to fetch");
    }

    #[test]
    fn only_so_many_are_on_their_way_at_once_and_the_rest_ask_again() {
        let (mut requests, mut receiver) = Requests::new(8, 2);
        for key in ["a", "b", "c"] {
            requests.ask(&key);
        }
        assert!(requests.begin(&"a", true) && requests.begin(&"b", true));
        assert!(!requests.begin(&"c", true), "two are on their way");
        while receiver.try_recv().is_ok() {}

        // Turned away, it is asked for again by the next view that needs it.
        requests.ask(&"c");
        assert_eq!(receiver.try_recv().ok(), Some("c"));
        requests.end(&"a");
        assert!(requests.begin(&"c", true));
    }

    #[test]
    fn a_request_that_does_not_fit_the_queue_is_asked_again_later() {
        let (requests, mut receiver) = Requests::new(1, 2);
        requests.ask(&"a");
        requests.ask(&"b");
        assert_eq!(receiver.try_recv().ok(), Some("a"));
        assert!(receiver.try_recv().is_err(), "no room for the second");

        requests.ask(&"b");
        assert_eq!(receiver.try_recv().ok(), Some("b"));
    }
}
