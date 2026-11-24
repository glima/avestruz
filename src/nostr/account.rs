// SPDX-License-Identifier: MIT

//! The signed-in account: reading what someone signs in with, keeping what
//! is secret where it belongs, signing, and encrypting what the account
//! keeps to itself.
//!
//! Signing in takes one of three things:
//!
//! - an `nsec`, the account's secret key: full access, signed here;
//! - an `npub`, its public key: read-only;
//! - a `bunker://` address (NIP-46): full access, signed by a remote signer
//!   that keeps the key.
//!
//! A remote signer knows this client by a key of the client's own. There is
//! one for each signer, made the first time that signer is signed in with
//! and the same ever after, signing out included: a signer that only answers
//! clients it was told about (`nak bunker -k`) is told this key once. When a
//! signer turns a sign-in down, the key's public half is what is shown, to
//! be authorized there. Revoking it is done at the signer.
//!
//! What is secret goes to the system keyring (Secret Service: gnome-keyring,
//! KWallet) and nowhere else: an account's `nsec` or its bunker address,
//! and the client keys. None is ever written to the config, the event store
//! or a log. The config only names the account by its `npub`.
//!
//! Signing with an `nsec` fetches it from the keyring, signs, and lets go of
//! it: the applet runs for as long as the session does, and holds no secret
//! key between one signature and the next. A remote signer is connected to
//! when the first signature is asked for, and stays connected until the
//! account signs out: connecting anew for every signature would have the
//! signer greet this client each time.
//!
//! What an account keeps to itself, such as its private bookmarks, it
//! encrypts to its own key (NIP-44), and only it can read back. That takes
//! the secret key as signing does, and goes the same two ways: here, with
//! the key from the keyring, or at the remote signer.
//!
//! ```mermaid
//! flowchart LR
//!     subgraph keyring["The system keyring, service cosmic-applet-avestruz"]
//!         secret["Under the account's npub: its nsec, or its bunker address"]
//!         client["Under bunker-client and a signer's key: this client's key for that signer"]
//!     end
//!     subgraph config["The config"]
//!         npub["The account's npub, and nothing secret"]
//!     end
//!     signer["Signer"] -->|"an nsec: fetched for one signature, and let go of"| secret
//!     signer -->|"a bunker: fetched once, and the connection kept"| secret
//!     signer --> client
//!     restore["At startup"] --> npub
//!     restore -->|"is anything kept for it?"| secret
//! ```
//!
//! The three ways in, step by step, and what a signature goes through,
//! are in [`crate::guide::flows`].

use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::{Either, select};
use nostr_connect::prelude::{AsyncNip04, AsyncNip44, AuthUrlHandler, Nip04, Nip44, NostrConnect, NostrConnectUri};
use nostr_sdk::prelude::{
    AsyncGetPublicKey, Event, EventBuilder, FinalizeEvent, FinalizeEventAsync, Keys, PublicKey, ToBech32, Url,
};
use tokio::sync::{Mutex, Notify};

use crate::links;

/// The keyring service an account's secret is filed under; the entry's user
/// name is the account's `npub`.
const KEYRING_SERVICE: &str = "cosmic-applet-avestruz";

/// What the keyring user name of a client key starts with; the signer's
/// public key, in hex, follows.
const CLIENT_KEY_PREFIX: &str = "bunker-client:";

/// How bunker addresses start.
const BUNKER_SCHEME: &str = "bunker://";

/// How long a remote signer gets to answer one request. Someone may have to
/// pick up a phone and approve it.
const REMOTE_TIMEOUT: Duration = Duration::from_secs(60);

/// What someone signed in with.
pub enum Credential {
    /// An `nsec`: the account can sign.
    Secret(Keys),
    /// An `npub`: the account can only read.
    Public(PublicKey),
    /// A `bunker://` address: the account signs through a remote signer.
    Bunker(NostrConnectUri),
}

/// Why the text in the sign-in field can't be signed in with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    Empty,
    /// Neither an `nsec`, an `npub` nor a bunker address.
    NotAKey,
    InvalidSecret,
    InvalidPublic,
    /// A bunker address that names no signer, or no relay to reach it by.
    InvalidBunker,
}

/// Whether `input` is meant as a bunker address.
fn is_bunker_address(input: &str) -> bool {
    input.get(..BUNKER_SCHEME.len()).is_some_and(|scheme| scheme.eq_ignore_ascii_case(BUNKER_SCHEME))
}

/// Read what was typed or pasted into the sign-in field.
///
/// Keys are only taken in their bech32 forms. A bare hex key reads the same
/// whether it is secret or public, and guessing wrong would either refuse a
/// sign-in or treat a public key as a secret one.
pub fn parse_credential(input: &str) -> Result<Credential, CredentialError> {
    let input = input.trim();
    if is_bunker_address(input) {
        // A signer is reached through relays: an address without one leads
        // nowhere.
        return NostrConnectUri::parse(input)
            .ok()
            .filter(|uri| uri.is_bunker() && !uri.relays().is_empty())
            .map(Credential::Bunker)
            .ok_or(CredentialError::InvalidBunker);
    }
    let input = input.strip_prefix("nostr:").unwrap_or(input);
    if input.is_empty() {
        Err(CredentialError::Empty)
    } else if input.starts_with("nsec1") {
        Keys::parse(input).map(Credential::Secret).map_err(|_| CredentialError::InvalidSecret)
    } else if input.starts_with("npub1") {
        PublicKey::parse(input).map(Credential::Public).map_err(|_| CredentialError::InvalidPublic)
    } else {
        Err(CredentialError::NotAKey)
    }
}

/// The account in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub public_key: PublicKey,
    /// Whether the keyring holds what it signs with: its secret key, or the
    /// way to its remote signer. Without it the account is read-only.
    pub can_sign: bool,
}

impl Account {
    /// The account's public key as an `npub`, the form the config stores.
    pub fn npub(&self) -> String {
        let Ok(npub) = self.public_key.to_bech32();
        npub
    }
}

/// Why nobody was signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInError {
    /// Signing in was called off while the remote signer was being waited for.
    Cancelled,
    /// The keyring would not keep the secret.
    Keyring(String),
    /// The remote signer could not be reached, or said no.
    Bunker {
        /// What it said, or what went wrong.
        reason: String,
        /// The key it knows this client by, once there is one.
        client: Option<PublicKey>,
        /// The address had a secret in it, which may have been used up.
        with_secret: bool,
    },
}

/// Why an event was not signed, or something not encrypted or decrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignError {
    /// What the account signs with could not be had from the keyring, or
    /// would not sign.
    Keyring(String),
    /// The remote signer could not be reached, or would not do it.
    Remote(String),
    /// The account's key was there, and what it was given could not be
    /// encrypted or decrypted with it: it was not encrypted to this
    /// account, or is not whole.
    Cipher(String),
}

/// Whether `payload` was encrypted the old way (NIP-04) rather than the
/// new (NIP-44): it ends in `?iv=` and the 24 characters of an
/// initialisation vector, which nothing encrypted the new way does. Lists
/// written by older clients are.
fn is_nip04(payload: &str) -> bool {
    const IV_MARK: &[u8] = b"?iv=";
    const IV_CHARS: usize = 24;
    // Some client wrote this after the vector.
    let bytes = payload.strip_suffix("-null").unwrap_or(payload).as_bytes();
    let mark = bytes.len().checked_sub(IV_MARK.len() + IV_CHARS).and_then(|at| bytes.get(at..));
    mark.is_some_and(|mark| mark.starts_with(IV_MARK))
}

/// What the keyring holds for an account: how it signs.
enum StoredSigner {
    /// With its own secret key.
    Local(Keys),
    /// Through a remote signer: its address, without the secret that only
    /// serves the first connection.
    Remote(NostrConnectUri),
}

impl StoredSigner {
    /// As it is written to the keyring: an `nsec`, or a remote signer's
    /// address.
    fn to_secret(&self) -> String {
        match self {
            Self::Local(keys) => nsec(keys),
            Self::Remote(bunker) => bunker.to_string(),
        }
    }

    /// Read back what [`Self::to_secret`] wrote.
    fn parse(secret: &str) -> Result<Self, String> {
        let unreadable = || "what the keyring holds for this account can't be read".to_string();
        let secret = secret.trim();
        if is_bunker_address(secret) {
            NostrConnectUri::parse(secret).ok().filter(NostrConnectUri::is_bunker).map(Self::Remote).ok_or_else(unreadable)
        } else {
            Keys::parse(secret).map(Self::Local).map_err(|_| unreadable())
        }
    }
}

/// `keys`' secret half as an `nsec`: the form the keyring keeps.
fn nsec(keys: &Keys) -> String {
    let Ok(nsec) = keys.secret_key().to_bech32();
    nsec
}

/// The signer a bunker address names.
fn remote_signer(bunker: &NostrConnectUri) -> Result<PublicKey, String> {
    bunker.remote_signer_public_key().copied().ok_or_else(|| "not a bunker address".to_string())
}

/// Where the keyring keeps the key this client is known by to the signer
/// `remote_signer`.
fn client_keys_entry(remote_signer: &PublicKey) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, &format!("{CLIENT_KEY_PREFIX}{}", remote_signer.to_hex())).map_err(|e| e.to_string())
}

/// The key this client is known by to the signer `remote_signer`: the one
/// in the keyring, or, the first time, a new one that is put there.
fn client_keys(remote_signer: &PublicKey) -> Result<Keys, String> {
    let entry = client_keys_entry(remote_signer)?;
    match entry.get_password() {
        Ok(kept) => Keys::parse(kept.trim()).map_err(|_| "the client key in the keyring can't be read".to_string()),
        Err(keyring::Error::NoEntry) => {
            let keys = Keys::generate();
            entry.set_password(&nsec(&keys)).map_err(|e| e.to_string())?;
            Ok(keys)
        }
        Err(e) => Err(e.to_string()),
    }
}

fn keyring_entry(public_key: &PublicKey) -> Result<keyring::Entry, String> {
    let Ok(npub) = public_key.to_bech32();
    keyring::Entry::new(KEYRING_SERVICE, &npub).map_err(|e| e.to_string())
}

/// Put what `public_key` signs with in the keyring, filed under its `npub`.
fn store_signer(public_key: &PublicKey, signer: &StoredSigner) -> Result<(), String> {
    keyring_entry(public_key)?.set_password(&signer.to_secret()).map_err(|e| e.to_string())
}

/// Whether the keyring holds what `public_key` signs with.
fn has_signer(public_key: &PublicKey) -> Result<bool, String> {
    match keyring_entry(public_key)?.get_password() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// What `public_key` signs with, read from the keyring for the purpose.
fn load_signer(public_key: &PublicKey) -> Result<StoredSigner, String> {
    let secret = keyring_entry(public_key)?.get_password().map_err(|e| e.to_string())?;
    let signer = StoredSigner::parse(&secret)?;
    // An entry filed under one account and holding another's key would
    // sign as somebody else.
    match &signer {
        StoredSigner::Local(keys) if keys.public_key() != *public_key => {
            Err("the key in the keyring is not this account's".to_string())
        }
        _ => Ok(signer),
    }
}

/// Take what `public_key` signs with out of the keyring, if it is there.
fn delete_signer(public_key: &PublicKey) -> Result<(), String> {
    match keyring_entry(public_key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Run a keyring call off the async executor: the Secret Service is reached
/// over D-Bus with a blocking client, and may wait on an unlock prompt.
async fn blocking<T: Send + 'static>(call: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(call).await.map_err(|e| e.to_string())?
}

/// Opens the page a remote signer asks to be approved on. Some signers
/// answer a request with an address where the person says yes.
#[derive(Debug)]
struct ApproveInBrowser;

impl AuthUrlHandler for ApproveInBrowser {
    fn on_auth_url(&self, auth_url: Url) -> Pin<Box<dyn Future<Output = Result<(), nostr_connect::error::Error>> + Send + '_>> {
        Box::pin(async move {
            if let Err(e) = links::open_in_browser(auth_url.as_str()) {
                tracing::warn!("the remote signer's approval page was not opened: {e:?}");
            }
            Ok(())
        })
    }
}

/// A connection to an account's remote signer.
struct Session {
    /// The account the signer signs as.
    public_key: PublicKey,
    connect: NostrConnect,
}

impl Session {
    /// Connect to the signer at `bunker` as the client `client_keys`, and
    /// ask whom it signs as. If that should be `expected` and isn't, there
    /// is no session.
    async fn open(bunker: NostrConnectUri, client_keys: Keys, expected: Option<PublicKey>) -> Result<Self, String> {
        let mut connect = NostrConnect::new(bunker, client_keys, REMOTE_TIMEOUT, None).map_err(|e| e.to_string())?;
        connect.auth_url_handler(ApproveInBrowser);
        let public_key = match connect.get_public_key_async().await {
            Ok(public_key) => public_key,
            Err(e) => {
                connect.shutdown().await;
                return Err(e.to_string());
            }
        };
        if expected.is_some_and(|expected| expected != public_key) {
            connect.shutdown().await;
            return Err("the remote signer now signs as another account".to_string());
        }
        Ok(Self { public_key, connect })
    }

    /// Have the signer sign `builder`'s event. What comes back is only
    /// taken if it is signed, and by the account.
    async fn sign(&self, builder: EventBuilder) -> Result<Event, String> {
        let event = builder.finalize_async(&self.connect).await.map_err(|e| e.to_string())?;
        if event.pubkey != self.public_key || event.verify().is_err() {
            return Err("the remote signer did not sign as this account".to_string());
        }
        Ok(event)
    }

    /// Have the signer encrypt `text` to the account itself.
    async fn encrypt(&self, text: &str) -> Result<String, String> {
        self.connect.nip44_encrypt_async(&self.public_key, text).await.map_err(|e| e.to_string())
    }

    /// Have the signer decrypt `payload`, which the account encrypted to
    /// itself, one way or the other.
    async fn decrypt(&self, payload: &str) -> Result<String, String> {
        let decrypted = if is_nip04(payload) {
            self.connect.nip04_decrypt_async(&self.public_key, payload).await
        } else {
            self.connect.nip44_decrypt_async(&self.public_key, payload).await
        };
        decrypted.map_err(|e| e.to_string())
    }

    async fn close(self) {
        self.connect.shutdown().await;
    }
}

/// What signs for the account. Cheap to clone; clones share the connection
/// to a remote signer, when there is one.
#[derive(Clone, Default)]
pub struct Signer {
    /// The connection to the account's remote signer, once a signature was
    /// asked of it. Held for as long as one request takes, so requests go
    /// to the signer one at a time.
    session: Arc<Mutex<Option<Session>>>,
    /// Told when a sign-in that is waiting on a remote signer is called off.
    cancelled: Arc<Notify>,
}

impl Signer {
    /// Sign in with `credential`.
    ///
    /// A secret key is stored in the keyring first; if the keyring refuses,
    /// nobody is signed in and the key is kept nowhere. A bunker address is
    /// connected to first, as the client its signer knows or is about to,
    /// which is where [`Self::cancel_sign_in`] can call it off; what is
    /// then kept for the account is the address.
    pub async fn sign_in(&self, credential: Credential) -> Result<Account, SignInError> {
        // Whoever was signed in before is not any more.
        let mut session = self.session.lock().await;
        if let Some(previous) = session.take() {
            previous.close().await;
        }
        match credential {
            Credential::Public(public_key) => Ok(Account { public_key, can_sign: false }),
            Credential::Secret(keys) => {
                let public_key = keys.public_key();
                blocking(move || store_signer(&public_key, &StoredSigner::Local(keys))).await.map_err(SignInError::Keyring)?;
                Ok(Account { public_key, can_sign: true })
            }
            Credential::Bunker(bunker) => {
                let with_secret = bunker.secret().is_some();
                let signer =
                    remote_signer(&bunker).map_err(|reason| SignInError::Bunker { reason, client: None, with_secret })?;
                let client_keys = blocking(move || client_keys(&signer)).await.map_err(SignInError::Keyring)?;
                let opened = self.connect(bunker, client_keys).await?;

                let public_key = opened.public_key;
                // Without the address's secret, which was for this once.
                let stored = match opened.connect.bunker_uri().await {
                    Ok(bunker) => StoredSigner::Remote(bunker),
                    Err(e) => {
                        opened.close().await;
                        return Err(SignInError::Bunker { reason: e.to_string(), client: None, with_secret });
                    }
                };
                if let Err(e) = blocking(move || store_signer(&public_key, &stored)).await {
                    opened.close().await;
                    return Err(SignInError::Keyring(e));
                }
                *session = Some(opened);
                Ok(Account { public_key, can_sign: true })
            }
        }
    }

    /// Connect to the signer at `bunker` as the client `client_keys`, for
    /// as long as it takes or until [`Self::cancel_sign_in`]. A signer that
    /// says no is reported with the key it would have to know.
    async fn connect(&self, bunker: NostrConnectUri, client_keys: Keys) -> Result<Session, SignInError> {
        let client = Some(client_keys.public_key());
        let with_secret = bunker.secret().is_some();
        let cancelled = pin!(self.cancelled.notified());
        let opening = pin!(Session::open(bunker, client_keys, None));
        match select(opening, cancelled).await {
            Either::Left((opened, _)) => opened.map_err(|reason| SignInError::Bunker { reason, client, with_secret }),
            Either::Right(((), _)) => Err(SignInError::Cancelled),
        }
    }

    /// Call off a sign-in that is waiting on a remote signer. One that is
    /// past waiting, or isn't waiting on anyone, goes on as it was.
    pub fn cancel_sign_in(&self) {
        self.cancelled.notify_waiters();
    }

    /// What `public_key` signs with, ready for one use: its secret key, out
    /// of the keyring, or its remote signer, which `session` is connected
    /// to when this says `None`.
    async fn keys_or_session(session: &mut Option<Session>, public_key: PublicKey) -> Result<Option<Keys>, SignError> {
        if session.as_ref().is_some_and(|session| session.public_key == public_key) {
            return Ok(None);
        }
        match blocking(move || load_signer(&public_key)).await.map_err(SignError::Keyring)? {
            StoredSigner::Local(keys) => Ok(Some(keys)),
            StoredSigner::Remote(bunker) => {
                if let Some(previous) = session.take() {
                    previous.close().await;
                }
                let signer = remote_signer(&bunker).map_err(SignError::Keyring)?;
                let client_keys = blocking(move || client_keys(&signer)).await.map_err(SignError::Keyring)?;
                let opened = Session::open(bunker, client_keys, Some(public_key)).await.map_err(SignError::Remote)?;
                *session = Some(opened);
                Ok(None)
            }
        }
    }

    /// The connection to the remote signer that `session` holds.
    fn connected(session: &Option<Session>) -> Result<&Session, SignError> {
        session.as_ref().ok_or_else(|| SignError::Remote("no connection to the remote signer".to_string()))
    }

    /// Sign the event `builder` describes as `public_key`: with its secret
    /// key from the keyring, or through its remote signer.
    pub async fn sign(&self, public_key: PublicKey, builder: EventBuilder) -> Result<Event, SignError> {
        let mut session = self.session.lock().await;
        match Self::keys_or_session(&mut session, public_key).await? {
            Some(keys) => builder.finalize(&keys).map_err(|e| SignError::Keyring(e.to_string())),
            None => Self::connected(&session)?.sign(builder).await.map_err(SignError::Remote),
        }
    }

    /// Encrypt `text` so that only `public_key` can read it, as
    /// `public_key`: what an account keeps to itself (NIP-44).
    pub async fn encrypt_to_self(&self, public_key: PublicKey, text: &str) -> Result<String, SignError> {
        let mut session = self.session.lock().await;
        match Self::keys_or_session(&mut session, public_key).await? {
            Some(keys) => keys.nip44_encrypt(&public_key, text).map_err(|e| SignError::Cipher(e.to_string())),
            None => Self::connected(&session)?.encrypt(text).await.map_err(SignError::Remote),
        }
    }

    /// Read `payload`, which `public_key` encrypted to itself: the new way
    /// (NIP-44), or the old (NIP-04), which lists written by older clients
    /// still are.
    pub async fn decrypt_from_self(&self, public_key: PublicKey, payload: &str) -> Result<String, SignError> {
        let mut session = self.session.lock().await;
        match Self::keys_or_session(&mut session, public_key).await? {
            Some(keys) => {
                let decrypted = if is_nip04(payload) {
                    keys.nip04_decrypt(&public_key, payload)
                } else {
                    keys.nip44_decrypt(&public_key, payload)
                };
                decrypted.map_err(|e| SignError::Cipher(e.to_string()))
            }
            None => Self::connected(&session)?.decrypt(payload).await.map_err(SignError::Remote),
        }
    }

    /// Sign `public_key` out: what it signs with leaves the keyring, and its
    /// remote signer is let go of. The key the signer knows this client by
    /// stays, as the signer's knowing it does: signing in again is not a
    /// new client to authorize. Forgetting this client is done at the signer.
    pub async fn sign_out(&self, public_key: PublicKey) -> Result<(), String> {
        if let Some(session) = self.session.lock().await.take() {
            session.close().await;
        }
        blocking(move || delete_signer(&public_key)).await
    }
}

/// The account the config names by `npub`, as it stands at startup: able to
/// sign if the keyring holds what it signs with. A keyring that can't be
/// asked leaves the account read-only rather than signed out.
pub async fn restore(npub: String) -> Result<Account, String> {
    let public_key = PublicKey::parse(&npub).map_err(|e| e.to_string())?;
    let can_sign = blocking(move || has_signer(&public_key)).await.unwrap_or_else(|e| {
        tracing::warn!("keyring unavailable, account is read-only for now: {e}");
        false
    });
    Ok(Account { public_key, can_sign })
}

/// A remote signer in this process, for the tests here and for those of
/// what signs or encrypts through one: no test can count on a keyring.
#[cfg(test)]
pub(crate) mod testing {
    use super::{Session, Signer};
    use nostr_connect::prelude::{
        NostrConnectKeys, NostrConnectRemoteSigner, NostrConnectRequest, NostrConnectSignerActions, NostrConnectUri,
    };
    use nostr_sdk::prelude::{Keys, LocalRelay, PublicKey};
    use std::future::Future;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;

    /// A remote signer that says yes to everything, or to everything but
    /// signing, or but decrypting.
    pub(crate) struct Approves {
        pub signing: bool,
        pub decrypting: bool,
    }

    impl NostrConnectSignerActions for Approves {
        fn approve(&self, _client: &PublicKey, request: &NostrConnectRequest) -> bool {
            match request {
                NostrConnectRequest::SignEvent(_) => self.signing,
                NostrConnectRequest::Nip04Decrypt { .. } | NostrConnectRequest::Nip44Decrypt { .. } => self.decrypting,
                _ => true,
            }
        }
    }

    /// Run `test` with a remote signer that keeps the key `user_keys`,
    /// listening on a relay in this process: its bunker address, with
    /// `secret` if any. The relay keeps what it is sent, as any does.
    pub(crate) fn with_remote_signer_for<F: Future<Output = ()>>(
        user_keys: Keys,
        secret: Option<&str>,
        approves: Approves,
        test: impl FnOnce(NostrConnectUri) -> F,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let relay = LocalRelay::builder().build();
            relay.run().await.unwrap();
            let keys = NostrConnectKeys::new(Keys::generate(), user_keys);
            let remote = NostrConnectRemoteSigner::new(keys, [relay.url().await], secret.map(String::from), None).unwrap();
            let bunker = remote.bunker_uri();
            let serving = tokio::spawn(async move { remote.serve(approves).await });
            // Requests are not kept by the relay: the signer has to be
            // listening before one is sent.
            tokio::time::sleep(Duration::from_millis(500)).await;

            tokio::time::timeout(Duration::from_secs(30), test(bunker)).await.unwrap();

            serving.abort();
            relay.shutdown();
        });
    }

    impl Signer {
        /// What signs for the account whose remote signer is at `bunker`,
        /// connected to it already, as a client it has not seen before:
        /// nothing is asked of a keyring.
        pub(crate) async fn connected_to(bunker: NostrConnectUri) -> Self {
            let session = Session::open(bunker, Keys::generate(), None).await.unwrap();
            Self { session: Arc::new(Mutex::new(Some(session))), cancelled: Arc::default() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{Approves, with_remote_signer_for};
    use super::{
        Credential, CredentialError, KEYRING_SERVICE, Session, SignError, SignInError, Signer, StoredSigner, client_keys,
        client_keys_entry, delete_signer, has_signer, is_nip04, keyring_entry, load_signer, parse_credential, remote_signer,
        store_signer,
    };
    use nostr_connect::prelude::{Nip04, Nip44, NostrConnectUri};
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, PublicKey, ToBech32};
    use std::future::Future;
    use std::time::Duration;

    fn error(input: &str) -> Option<CredentialError> {
        parse_credential(input).err()
    }

    fn note(text: &str) -> EventBuilder {
        EventBuilder::new(Kind::TextNote, text)
    }

    /// Run `test` with a remote signer for a made-up account listening on a
    /// relay in this process: its bunker address, with `secret` if any, and
    /// the account it signs as.
    fn with_remote_signer<F: Future<Output = ()>>(
        secret: Option<&str>,
        approves: Approves,
        test: impl FnOnce(NostrConnectUri, PublicKey) -> F,
    ) {
        let user_keys = Keys::generate();
        let user = user_keys.public_key();
        with_remote_signer_for(user_keys, secret, approves, |bunker| test(bunker, user));
    }

    #[test]
    fn what_was_encrypted_the_old_way_is_told_from_the_new() {
        let keys = Keys::generate();
        let me = keys.public_key();
        let old = keys.nip04_encrypt(&me, "kept to myself").unwrap();
        let new = keys.nip44_encrypt(&me, "kept to myself").unwrap();
        assert!(is_nip04(&old), "{old}");
        assert!(is_nip04(&format!("{old}-null")), "as one client left them");
        assert!(!is_nip04(&new), "{new}");
        for neither in ["", "?iv=", "?iv=tooshort", "héllo ?iv= wörld, not at the right place at all"] {
            assert!(!is_nip04(neither), "{neither}");
        }
    }

    #[test]
    fn what_an_account_encrypts_to_itself_only_it_reads_back_here_or_at_its_remote_signer() {
        let user_keys = Keys::generate();
        let user = user_keys.public_key();
        let approves = Approves { signing: true, decrypting: true };
        with_remote_signer_for(user_keys.clone(), None, approves, async |bunker| {
            let signer = Signer::connected_to(bunker).await;

            // Encrypted there, read there and with the key itself.
            let there = signer.encrypt_to_self(user, "kept to myself").await.unwrap();
            assert!(!there.contains("kept"));
            assert!(!is_nip04(&there), "the new way");
            assert_eq!(signer.decrypt_from_self(user, &there).await.as_deref(), Ok("kept to myself"));
            assert_eq!(user_keys.nip44_decrypt(&user, &there).as_deref().ok(), Some("kept to myself"));

            // Encrypted with the key itself, either way, and read there.
            let here = user_keys.nip44_encrypt(&user, "from another client").unwrap();
            assert_eq!(signer.decrypt_from_self(user, &here).await.as_deref(), Ok("from another client"));
            let old = user_keys.nip04_encrypt(&user, "from an old client").unwrap();
            assert_eq!(signer.decrypt_from_self(user, &old).await.as_deref(), Ok("from an old client"));

            // What somebody else encrypted to themselves is not read.
            let stranger = Keys::generate();
            let theirs = stranger.nip44_encrypt(&stranger.public_key(), "not yours").unwrap();
            assert!(matches!(signer.decrypt_from_self(user, &theirs).await, Err(SignError::Remote(_))));
        });
    }

    #[test]
    fn a_remote_signer_that_will_not_decrypt_is_an_error() {
        let user_keys = Keys::generate();
        let user = user_keys.public_key();
        let approves = Approves { signing: true, decrypting: false };
        with_remote_signer_for(user_keys.clone(), None, approves, async |bunker| {
            let signer = Signer::connected_to(bunker).await;
            let kept = user_keys.nip44_encrypt(&user, "kept to myself").unwrap();
            let refused = signer.decrypt_from_self(user, &kept).await;
            assert!(matches!(&refused, Err(SignError::Remote(reason)) if reason.contains("Rejected")), "{refused:?}");
        });
    }

    #[test]
    fn a_remote_signer_signs_as_the_account_whose_key_it_keeps() {
        with_remote_signer(Some("once"), Approves { signing: true, decrypting: true }, async |bunker, user| {
            assert_eq!(bunker.secret(), Some("once"));
            let session = Session::open(bunker, Keys::generate(), None).await.unwrap();
            assert_eq!(session.public_key, user);

            let signed = session.sign(note("signed far away")).await.unwrap();
            assert_eq!(signed.pubkey, user);
            assert_eq!(signed.content, "signed far away");
            assert!(signed.verify().is_ok());

            // What is kept for next time has not the secret, which was for
            // this once.
            assert_eq!(session.connect.bunker_uri().await.unwrap().secret(), None);
            session.close().await;
        });
    }

    #[test]
    fn a_session_is_only_for_the_account_it_is_expected_to_sign_as() {
        with_remote_signer(None, Approves { signing: true, decrypting: true }, async |bunker, user| {
            let client_keys = Keys::generate();
            // As at a later start: the same client, the address without a
            // secret, and the account known.
            let session = Session::open(bunker.clone(), client_keys.clone(), Some(user)).await.unwrap();
            assert!(session.sign(note("again")).await.is_ok());
            session.close().await;

            let somebody_else = Keys::generate().public_key();
            assert!(Session::open(bunker, client_keys, Some(somebody_else)).await.is_err());
        });
    }

    #[test]
    fn a_signature_the_remote_signer_refuses_is_an_error() {
        with_remote_signer(None, Approves { signing: false, decrypting: true }, async |bunker, _user| {
            let session = Session::open(bunker, Keys::generate(), None).await.unwrap();
            let refused = session.sign(note("not this")).await;
            assert!(refused.is_err_and(|reason| reason.contains("Rejected")));
            session.close().await;
        });
    }

    #[test]
    fn a_signer_that_says_no_is_reported_with_the_key_it_would_have_to_know() {
        // The signer wants a secret, and the address has another.
        with_remote_signer(Some("right"), Approves { signing: true, decrypting: true }, async |bunker, _user| {
            let NostrConnectUri::Bunker { remote_signer_public_key, relays, .. } = bunker else {
                panic!("a bunker address");
            };
            let wrong = NostrConnectUri::Bunker { remote_signer_public_key, relays, secret: Some("wrong".into()) };
            let client_keys = Keys::generate();

            let refused = Signer::default().connect(wrong, client_keys.clone()).await;
            match refused {
                Err(SignInError::Bunker { reason, client, with_secret }) => {
                    assert!(!reason.is_empty());
                    assert_eq!(client, Some(client_keys.public_key()));
                    assert!(with_secret);
                }
                _ => panic!("refused"),
            }
        });
    }

    #[test]
    fn a_sign_in_waiting_on_a_remote_signer_can_be_called_off() {
        with_remote_signer(None, Approves { signing: true, decrypting: true }, async |bunker, _user| {
            // The same relay, and a signer nobody is: no answer will come.
            let NostrConnectUri::Bunker { relays, .. } = bunker else {
                panic!("a bunker address");
            };
            let nobody =
                NostrConnectUri::Bunker { remote_signer_public_key: Keys::generate().public_key(), relays, secret: None };

            let signer = Signer::default();
            let waiting = signer.connect(nobody, Keys::generate());
            let calling_off = async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                signer.cancel_sign_in();
            };
            let (result, ()) = tokio::join!(waiting, calling_off);
            assert!(matches!(result, Err(SignInError::Cancelled)));

            // Calling off when nobody is signing in is not kept for later.
            signer.cancel_sign_in();
            let public_key = Keys::generate().public_key();
            assert!(signer.sign_in(Credential::Public(public_key)).await.is_ok());
        });
    }

    /// Whether a system keyring answers here. The tests of it run wherever
    /// one does. Where none does there is nothing for them to hold to: a
    /// machine with no session, which is not one Avestruz runs on.
    fn keyring_answers() -> bool {
        let asked =
            keyring::Entry::new(KEYRING_SERVICE, "a-test-asking-whether-it-answers").and_then(|entry| entry.get_password());
        matches!(asked, Ok(_) | Err(keyring::Error::NoEntry))
    }

    /// What a test put in the real keyring, taken out again when the test
    /// is over, however it ended: one that fails half way leaves no
    /// throwaway key behind.
    struct TakenOutAfter(Vec<keyring::Entry>);

    impl Drop for TakenOutAfter {
        fn drop(&mut self) {
            for entry in &self.0 {
                let _ = entry.delete_credential();
            }
        }
    }

    /// Against the real system keyring, with a throwaway key that is removed
    /// again.
    #[test]
    fn a_secret_key_is_kept_in_the_keyring_until_sign_out() {
        if !keyring_answers() {
            return;
        }
        let keys = Keys::generate();
        let public_key = keys.public_key();
        let _taken_out = TakenOutAfter(vec![keyring_entry(&public_key).unwrap()]);
        assert_eq!(has_signer(&public_key), Ok(false));

        store_signer(&public_key, &StoredSigner::Local(keys.clone())).unwrap();
        assert_eq!(has_signer(&public_key), Ok(true));
        assert!(matches!(load_signer(&public_key), Ok(StoredSigner::Local(_))));

        // While it is there, the account signs with it, and as itself.
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let signer = Signer::default();
        let signed = runtime.block_on(signer.sign(public_key, note("signed from the keyring"))).unwrap();
        assert_eq!(signed.pubkey, public_key);
        assert!(signed.verify().is_ok());

        // And encrypts to itself, and reads it back, either way.
        let kept = runtime.block_on(signer.encrypt_to_self(public_key, "kept to myself")).unwrap();
        assert_eq!(keys.nip44_decrypt(&public_key, &kept).as_deref().ok(), Some("kept to myself"));
        assert_eq!(runtime.block_on(signer.decrypt_from_self(public_key, &kept)).as_deref(), Ok("kept to myself"));
        let old = keys.nip04_encrypt(&public_key, "from an old client").unwrap();
        assert_eq!(runtime.block_on(signer.decrypt_from_self(public_key, &old)).as_deref(), Ok("from an old client"));
        let stranger = Keys::generate();
        let theirs = stranger.nip44_encrypt(&stranger.public_key(), "not yours").unwrap();
        assert!(matches!(runtime.block_on(signer.decrypt_from_self(public_key, &theirs)), Err(SignError::Cipher(_))));

        runtime.block_on(signer.sign_out(public_key)).unwrap();
        assert_eq!(has_signer(&public_key), Ok(false));
        // Signing out of an account whose key is already gone is not an error.
        assert_eq!(delete_signer(&public_key), Ok(()));
        assert!(matches!(runtime.block_on(signer.sign(public_key, note("no key"))), Err(SignError::Keyring(_))));
    }

    /// Against the real system keyring, as the test above.
    #[test]
    fn a_bunker_account_signs_through_what_the_keyring_keeps_until_sign_out() {
        if !keyring_answers() {
            return;
        }
        with_remote_signer(None, Approves { signing: true, decrypting: true }, async |bunker, user| {
            let remote = remote_signer(&bunker).unwrap();
            let _taken_out = TakenOutAfter(vec![keyring_entry(&user).unwrap(), client_keys_entry(&remote).unwrap()]);
            let signer = Signer::default();
            let account = signer.sign_in(Credential::Bunker(bunker.clone())).await.unwrap();
            assert_eq!((account.public_key, account.can_sign), (user, true));
            assert!(matches!(load_signer(&user), Ok(StoredSigner::Remote(_))));
            assert_eq!(signer.sign(user, note("at once")).await.map(|event| event.pubkey), Ok(user));
            let known_as = client_keys(&remote).unwrap().public_key();

            // After a restart there is no connection, only the keyring.
            let restarted = Signer::default();
            assert_eq!(super::restore(account.npub()).await, Ok(account.clone()));
            assert_eq!(restarted.sign(user, note("later")).await.map(|event| event.pubkey), Ok(user));

            restarted.sign_out(user).await.unwrap();
            assert_eq!(has_signer(&user), Ok(false));
            assert!(matches!(restarted.sign(user, note("signed out")).await, Err(SignError::Keyring(_))));

            // Signing in again is as the client the signer already knows.
            assert_eq!(client_keys(&remote).unwrap().public_key(), known_as);
            assert_eq!(restarted.sign_in(Credential::Bunker(bunker)).await, Ok(account));
            restarted.sign_out(user).await.unwrap();
            client_keys_entry(&remote).unwrap().delete_credential().unwrap();
        });
    }

    #[test]
    fn what_the_keyring_keeps_reads_back_as_it_was_written() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();

        // A secret key is kept as its nsec and nothing else.
        assert_eq!(StoredSigner::Local(keys.clone()).to_secret(), nsec);
        assert!(matches!(StoredSigner::parse(&nsec), Ok(StoredSigner::Local(read)) if read == keys));

        // A remote signer, as its address and nothing else.
        let address = format!("bunker://{}?relay=wss://relay.example.com", Keys::generate().public_key().to_hex());
        let bunker = NostrConnectUri::parse(&address).unwrap();
        assert_eq!(StoredSigner::Remote(bunker.clone()).to_secret(), address);
        assert!(matches!(StoredSigner::parse(&address), Ok(StoredSigner::Remote(read)) if read == bunker));

        assert!(StoredSigner::parse("").is_err());
        assert!(StoredSigner::parse("not a key").is_err());
        assert!(StoredSigner::parse("bunker://not-a-signer").is_err());
        assert!(StoredSigner::parse("https://example.com").is_err());
    }

    #[test]
    fn an_nsec_signs_in_with_full_access_and_an_npub_read_only() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let npub = keys.public_key().to_bech32().unwrap();

        match parse_credential(&nsec) {
            Ok(Credential::Secret(parsed)) => assert_eq!(parsed.public_key(), keys.public_key()),
            _ => panic!("an nsec is a secret key"),
        }
        match parse_credential(&npub) {
            Ok(Credential::Public(parsed)) => assert_eq!(parsed, keys.public_key()),
            _ => panic!("an npub is a public key"),
        }
    }

    #[test]
    fn a_bunker_address_signs_in_through_the_signer_and_relays_it_names() {
        let remote = Keys::generate().public_key();
        let address =
            format!("bunker://{}?relay=wss://relay.example.com&relay=wss://other.example.com&secret=abc", remote.to_hex());

        match parse_credential(&format!("  {address}\n")) {
            Ok(Credential::Bunker(bunker)) => {
                assert_eq!(bunker.remote_signer_public_key(), Some(&remote));
                assert_eq!(bunker.relays().len(), 2);
                assert_eq!(bunker.secret(), Some("abc"));
            }
            _ => panic!("a bunker address"),
        }
        assert!(matches!(parse_credential(&address.replace("bunker://", "BUNKER://")), Ok(Credential::Bunker(_))));

        assert_eq!(error(&format!("bunker://{}", remote.to_hex())), Some(CredentialError::InvalidBunker), "no relay");
        assert_eq!(error("bunker://not-a-key?relay=wss://relay.example.com"), Some(CredentialError::InvalidBunker));
        assert_eq!(error("bunker://"), Some(CredentialError::InvalidBunker));
        // The other kind of address is one a client shows, not one it is given.
        let shown = format!("nostrconnect://{}?relay=wss://relay.example.com", remote.to_hex());
        assert_eq!(error(&shown), Some(CredentialError::NotAKey));
    }

    #[test]
    fn a_pasted_key_may_carry_spaces_or_the_nostr_scheme() {
        let npub = Keys::generate().public_key().to_bech32().unwrap();
        assert!(matches!(parse_credential(&format!("  {npub}\n")), Ok(Credential::Public(_))));
        assert!(matches!(parse_credential(&format!("nostr:{npub}")), Ok(Credential::Public(_))));
    }

    #[test]
    fn anything_else_is_refused_with_the_reason() {
        assert_eq!(error("   "), Some(CredentialError::Empty));
        assert_eq!(error("hello"), Some(CredentialError::NotAKey));
        assert_eq!(error("nsec1notakey"), Some(CredentialError::InvalidSecret));
        assert_eq!(error("npub1notakey"), Some(CredentialError::InvalidPublic));
        // A hex key doesn't say which half it is.
        let hex = Keys::generate().public_key().to_hex();
        assert_eq!(error(&hex), Some(CredentialError::NotAKey));
    }
}
