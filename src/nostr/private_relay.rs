// SPDX-License-Identifier: MIT

//! The relay Avestruz runs itself, for what is the account's alone: what is
//! written and not published, and whatever else is not for any other relay.
//!
//! It is a relay like any other, over the same event store the client
//! keeps, but it listens nowhere: on no port, and on no socket file. A
//! connection to it is a pipe made inside this process, one end handed to
//! the relay and the other kept, so there is nothing for another program
//! to open, and Avestruz is the only client it has.
//!
//! That is the relay, not the store under it: the store is a file, and
//! whoever can read the user's files can read that one. What is written
//! and not published is encrypted in it.
//!
//! The relay takes the events of the account that is signed in, and no
//! others. Avestruz talks to it as a client talks to any relay: an event
//! is sent and its `OK` waited for, a question is asked and its answers
//! read until the relay says that is all.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant client as PrivateRelay: send or ask
//!     participant near as The near end of a pipe
//!     participant far as The far end
//!     participant relay as LocalRelay
//!     participant store as Event store
//!     client->>near: a pipe is made, in memory
//!     client->>relay: take this connection (the far end)
//!     client->>near: EVENT, or REQ, as WebSocket frames
//!     near->>far: bytes
//!     far->>relay: the message
//!     relay->>relay: is the event signed by the account signed in?
//!     relay->>store: kept, or read
//!     relay-->>client: OK, or the events and EOSE, back the same way
//!     client->>near: closed, and the relay's end of it is done
//! ```
//!
//! Each `send` and each `ask` makes a pipe of its own and closes it. The
//! two ends speak WebSocket from the first byte, since that is what the
//! relay library takes a connection as; there is no HTTP request to open
//! it, and nothing listens.
//!
//! What goes to this relay, and why, is in [`crate::guide::relays`].

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr_sdk::prelude::{
    ClientMessage, Event, Filter, LocalRelay, MachineReadablePrefix, NostrDatabase, PublicKey, RelayMessage, SubscriptionId,
    WritePolicy, WritePolicyResult,
};
use tokio::io::DuplexStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

/// How much either end of a connection may have written that the other
/// has not read yet. The writer waits when it is that much.
const PIPE_BYTES: usize = 64 * 1024;

/// How long the relay has to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// The address the relay is told its connections are from: they are all
/// from this process, and a pipe has none.
const PEER: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

/// Takes the events the account signed in wrote, and no others; with
/// nobody signed in, none at all.
#[derive(Debug, Default)]
struct OwnersAlone {
    owner: Arc<Mutex<Option<PublicKey>>>,
}

impl WritePolicy for OwnersAlone {
    fn admit_event<'a>(
        &'a self,
        event: &'a Event,
        _addr: &'a SocketAddr,
    ) -> Pin<Box<dyn Future<Output = WritePolicyResult> + Send + 'a>> {
        let owner = *self.owner.lock().unwrap_or_else(PoisonError::into_inner);
        Box::pin(async move {
            if owner == Some(event.pubkey) {
                WritePolicyResult::Accept
            } else {
                WritePolicyResult::reject(MachineReadablePrefix::Restricted, "this relay keeps its own account's events")
            }
        })
    }
}

/// The relay Avestruz runs itself, and the way to it. Clones are the same
/// relay; it is gone when the last of them is.
#[derive(Debug, Clone)]
pub struct PrivateRelay {
    /// Shared rather than cloned: the relay is large, and this is passed
    /// about with the client it belongs to.
    relay: Arc<LocalRelay>,
    /// Whose events it takes.
    owner: Arc<Mutex<Option<PublicKey>>>,
}

impl PrivateRelay {
    /// A relay over `database`, which is reached from here and from
    /// nowhere else.
    pub fn over(database: Arc<dyn NostrDatabase>) -> Self {
        let owner: Arc<Mutex<Option<PublicKey>>> = Arc::default();
        let relay = LocalRelay::builder().database(database).write_policy(OwnersAlone { owner: owner.clone() }).build();
        Self { relay: Arc::new(relay), owner }
    }

    /// Take the events of `owner` from now on, and no others: the account
    /// that is signed in, or nobody's when none is.
    pub fn set_owner(&self, owner: Option<PublicKey>) {
        *self.owner.lock().unwrap_or_else(PoisonError::into_inner) = owner;
    }

    /// A connection to the relay, as its client: a pipe, with the relay
    /// at the far end of it for as long as this end is held.
    ///
    /// The two ends speak WebSocket from the first byte, without the
    /// request that opens one elsewhere: the relay takes a connection as
    /// one that is already agreed to.
    async fn connect(&self) -> WebSocketStream<DuplexStream> {
        let (near, far) = tokio::io::duplex(PIPE_BYTES);
        let relay = self.relay.clone();
        tokio::spawn(async move {
            if let Err(e) = relay.take_connection(far, PEER).await {
                tracing::debug!("a connection to the relay of this machine ended: {e}");
            }
        });
        WebSocketStream::from_raw_socket(near, Role::Client, None).await
    }

    /// Send `event` to the relay, and wait for it to say that it took it.
    /// What it says otherwise is the error.
    pub async fn send(&self, event: &Event) -> Result<(), String> {
        let sending = async {
            let mut relay = self.connect().await;
            relay.send(Message::text(ClientMessage::event(event.clone()).as_json())).await.map_err(|e| e.to_string())?;
            while let Some(message) = relay.next().await {
                let Message::Text(text) = message.map_err(|e| e.to_string())? else {
                    continue;
                };
                if let Ok(RelayMessage::Ok { event_id, status, message }) = RelayMessage::from_json(text.as_str())
                    && event_id == event.id
                {
                    let _ = relay.close(None).await;
                    return if status { Ok(()) } else { Err(message.into_owned()) };
                }
            }
            Err("the relay hung up".to_string())
        };
        tokio::time::timeout(ANSWER_TIMEOUT, sending).await.map_err(|_| "the relay did not answer".to_string())?
    }

    /// The events the relay has that `filter` matches.
    pub async fn ask(&self, filter: Filter) -> Result<Vec<Event>, String> {
        let asking = async {
            let mut relay = self.connect().await;
            let question = SubscriptionId::generate();
            let asked = ClientMessage::req(question.clone(), filter).as_json();
            relay.send(Message::text(asked)).await.map_err(|e| e.to_string())?;
            let mut events = Vec::new();
            while let Some(message) = relay.next().await {
                let Message::Text(text) = message.map_err(|e| e.to_string())? else {
                    continue;
                };
                match RelayMessage::from_json(text.as_str()) {
                    Ok(RelayMessage::Event { event, .. }) => events.push(event.into_owned()),
                    Ok(RelayMessage::EndOfStoredEvents(_)) => {
                        let _ = relay.send(Message::text(ClientMessage::close(question).as_json())).await;
                        let _ = relay.close(None).await;
                        return Ok(events);
                    }
                    Ok(RelayMessage::Closed { message, .. }) => return Err(message.into_owned()),
                    _ => {}
                }
            }
            Err("the relay hung up".to_string())
        };
        tokio::time::timeout(ANSWER_TIMEOUT, asking).await.map_err(|_| "the relay did not answer".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::PrivateRelay;
    use nostr_lmdb::NostrLmdb;
    use nostr_sdk::prelude::{Event, EventBuilder, Filter, FinalizeEvent, IntoNostrDatabase, Keys, Kind};

    fn note(keys: &Keys, text: &str) -> Event {
        EventBuilder::new(Kind::TextNote, text).finalize(keys).unwrap()
    }

    /// Every file under `directory`, however deep.
    fn files_under(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { files.extend(files_under(&path)) } else { files.push(path) }
        }
        files
    }

    #[test]
    fn the_relay_is_reached_from_here_and_leaves_nothing_to_open() {
        let store = std::env::temp_dir().join(format!("avestruz-test-{}-private-relay", std::process::id()));
        let _ = std::fs::remove_dir_all(&store);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            tokio::fs::create_dir_all(&store).await.unwrap();
            let database = NostrLmdb::open(&store).await.unwrap().into_nostr_database();
            let before = files_under(&store);
            let relay = PrivateRelay::over(database.clone());

            // With nobody signed in it takes nothing; then its account's
            // events, and nobody else's.
            let (me, other) = (Keys::generate(), Keys::generate());
            let mine = note(&me, "mine");
            assert!(relay.send(&mine).await.is_err());
            relay.set_owner(Some(me.public_key()));
            assert_eq!(relay.send(&mine).await, Ok(()));
            let refused = relay.send(&note(&other, "somebody else's")).await.unwrap_err();
            assert!(refused.contains("restricted"), "{refused}");

            // What it has is what the store has, the event it took and
            // one the store got another way.
            let stored = note(&other, "from another relay");
            database.save_event(&stored).await.unwrap();
            let mut found: Vec<_> =
                relay.ask(Filter::new().kind(Kind::TextNote)).await.unwrap().into_iter().map(|event| event.id).collect();
            found.sort();
            let mut expected = vec![mine.id, stored.id];
            expected.sort();
            assert_eq!(found, expected);
            assert_eq!(database.query(Filter::new().id(mine.id)).await.unwrap().len(), 1);

            // An answer longer than the pipe holds at once comes whole.
            let long: Vec<Event> = (0..4).map(|nth| note(&me, &format!("{nth} {}", "a long note ".repeat(2_500)))).collect();
            for note in &long {
                assert_eq!(relay.send(note).await, Ok(()));
            }
            let asked = relay.ask(Filter::new().ids(long.iter().map(|note| note.id))).await.unwrap();
            assert_eq!(asked.len(), 4);
            assert!(asked.iter().map(|note| note.content.len()).sum::<usize>() > super::PIPE_BYTES);

            // A clone is the same relay.
            assert_eq!(relay.clone().ask(Filter::new().id(mine.id)).await.unwrap().len(), 1);

            // Nothing was made beside the store's own files for anything
            // else to connect to.
            assert_eq!(files_under(&store), before);
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&store);
    }
}
