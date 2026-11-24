// SPDX-License-Identifier: MIT

//! Runtime-adjustable log level.
//!
//! Avestruz logs to one place: stderr, which cosmic-panel forwards to the
//! systemd journal, and which is the terminal when the applet is run from
//! one.
//!
//! `main` builds the tracing subscriber with its filter wrapped in a
//! [`tracing_subscriber::reload`] layer, then installs a reload hook here
//! with [`install_reload_hook`]. The settings view calls
//! [`set_console_level`] to change how much is logged **live**, without a
//! restart: an applet has no command line to set `RUST_LOG` on.

use std::sync::OnceLock;

use crate::config::LogLevel;

/// Rebuilds the filter for a level and swaps it into the live subscriber.
/// Installed once by `main`.
type ReloadHook = Box<dyn Fn(LogLevel) + Send + Sync>;

static RELOAD: OnceLock<ReloadHook> = OnceLock::new();

/// Register the reload hook. Called once from `main` after the tracing
/// subscriber is initialized. Later calls are ignored.
pub fn install_reload_hook(hook: ReloadHook) {
    let _ = RELOAD.set(hook);
}

/// Apply a new log level at runtime.
///
/// Does nothing if the hook hasn't been installed (in tests, which never
/// initialize tracing), so callers don't need to special-case that.
pub fn set_console_level(level: LogLevel) {
    if let Some(hook) = RELOAD.get() {
        hook(level);
    }
}

/// Whether `RUST_LOG` says how much to log. If it does, it is what `main`
/// went by, and it keeps precedence over the setting.
pub fn level_set_by_environment() -> bool {
    std::env::var_os("RUST_LOG").is_some()
}
