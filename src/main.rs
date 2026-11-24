// SPDX-License-Identifier: MIT

//! Avestruz — a Nostr client that lives in the COSMIC™ desktop panel.
//!
//! The panel button opens a popup holding the whole client: sign-in, and the
//! feed in a scroller.

use cosmic_applet_avestruz::{app, button, i18n, logging};
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// Crates that say more at `info` than is worth reading in the journal;
/// and what passes between this and a relay, which is logged word for
/// word below `info`: every request, a search and its words among them,
/// and those are written nowhere.
const QUIET: [&str; 13] = [
    "nostr_sdk::relay=info",
    "iced_futures::subscription::tracker=error",
    "iced_winit=warn",
    "sctk=warn",
    "sctk_adwaita=error",
    "i18n_embed=warn",
    "cosmic::app=warn",
    "cosmic_config=warn",
    "winit=warn",
    "cosmic_text=warn",
    "rustls=warn",
    "tungstenite=warn",
    "async_wsocket=warn",
];

/// `filter`, with the crates in [`QUIET`] turned down.
fn quietened(mut filter: EnvFilter) -> EnvFilter {
    for directive in QUIET {
        if let Ok(parsed) = directive.parse() {
            filter = filter.add_directive(parsed);
        }
    }
    filter
}

fn main() -> cosmic::iced::Result {
    // The TLS crypto provider for every connection, relays and pictures
    // alike. Named here rather than left to rustls to work out from which
    // crate features happen to be on: with two providers compiled in it
    // can't, and panics at the first connection instead.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // `RUST_LOG` if it is set, `info` until the settings say otherwise if
    // not: the app applies the level it keeps as soon as it has read it.
    let filter = quietened(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    // The filter is in a reload layer, so that the settings view can change
    // how much is logged while the applet runs (see `logging`).
    //
    // To **stderr**: cosmic-panel forwards an applet's stderr to the journal,
    // but not its stdout.
    let (filter, reload) = tracing_subscriber::reload::Layer::new(filter);
    let console = fmt::layer().with_writer(std::io::stderr).with_timer(ChronoLocal::rfc_3339()).with_filter(filter);
    tracing_subscriber::registry().with(console).init();

    // A level chosen in the settings gets the same quiet crates as the one
    // at startup.
    logging::install_reload_hook(Box::new(move |level| {
        let _ = reload.reload(quietened(EnvFilter::new(level.as_filter_str())));
    }));

    // Started to say toggle to the window that is running, as a keyboard
    // shortcut of the desktop's does: said, and done.
    if std::env::args().any(|said| said == button::TOGGLE_FLAG) {
        let said = button::socket_path().ok_or_else(|| "the session has no runtime directory".to_string());
        if let Err(e) = said.and_then(|socket| button::say_toggle(&socket).map_err(|e| e.to_string())) {
            tracing::error!("the popup was not toggled: no window is listening ({e})");
            std::process::exit(1);
        }
        return Ok(());
    }

    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();
    i18n::init(&requested_languages);

    // The panel starts the button, and the button the window: see `button`.
    if std::env::args().any(|said| said == button::WINDOW_FLAG) { app::run() } else { button::run() }
}
