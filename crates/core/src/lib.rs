/**
 * Hawk Core — the internal SDK engine.
 *
 * This crate provides the transport, event queue, and worker thread.
 * End users should depend on the `hawk` facade crate instead, which
 * re-exports everything and wires up addons (panic hook, etc.).
 *
 * # Architecture
 *
 * - `init()` creates a global `Client` (stored in `OnceLock`) and spawns
 *   a background worker thread that drains events from a bounded channel.
 * - `send()` builds an `EventData` and enqueues it (non-blocking).
 * - The worker POSTs each event as a `HawkEvent` envelope to the Hawk
 *   collector via `reqwest` (blocking HTTP in the dedicated thread).
 * - `Guard::drop()` calls `flush()` to ensure pending events are delivered
 *   before the process exits.
 */

// ---------------------------------------------------------------------------
// Module declarations (all private — public surface is re-exports only)
// ---------------------------------------------------------------------------

mod client;
mod guard;
mod token;
mod transport;
mod types;
mod worker;

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

pub use client::Options;
pub use guard::Guard;
pub use types::{BacktraceFrame, BeforeSendResult, EventData, HawkEvent, CATCHER_VERSION};

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/**
 * Initializes the SDK with the given token and options.
 *
 * Returns `Ok(Guard)` on success. The `Guard` flushes pending events
 * when dropped — keep it alive for the duration of your app.
 *
 * Returns `Err` if the token is malformed or `init` was already called.
 */
pub fn init(token: &str, options: Options) -> Result<Guard, String> {
    client::Client::init(token, options)?;
    Ok(Guard::new())
}

/**
 * Sends an event to Hawk.
 *
 * Accepts anything that implements `Display` — strings, errors, formatted
 * messages. A backtrace is captured at the call site so the Hawk dashboard
 * shows exactly where `hawk::send(...)` was called from.
 *
 * Silent no-op if the SDK has not been initialized.
 */
pub fn send(message: &(impl std::fmt::Display + ?Sized)) {
    if let Some(client) = client::get_client() {
        let event = EventData {
            title: message.to_string(),
            event_type: Some("error".to_string()),
            backtrace: get_backtrace(),
            catcher_version: CATCHER_VERSION.to_string(),
        };
        client.send_event(event);
    }
}

/**
 * Sends a pre-built `EventData` directly to Hawk.
 *
 * Low-level API used by addons (e.g. `hawk_panic`) to send events
 * with custom backtrace data. Silent no-op if not initialized.
 */
pub fn capture_event(event: EventData) {
    if let Some(client) = client::get_client() {
        client.send_event(event);
    }
}

/**
 * Manually flushes all pending events, blocking until drained or timeout.
 *
 * Normally you don't need this — the `Guard` handles it on drop.
 */
pub fn flush() -> bool {
    if let Some(client) = client::get_client() {
        client.flush()
    } else {
        true
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/**
 * Captures a backtrace at the current call site.
 * Returns `None` if no useful frames were resolved.
 */
pub fn get_backtrace() -> Option<Vec<BacktraceFrame>> {
    let bt = backtrace::Backtrace::new();
    let frames = convert_backtrace(&bt);
    if frames.is_empty() { None } else { Some(frames) }
}

/**
 * Converts a `backtrace::Backtrace` into `Vec<BacktraceFrame>`.
 * Filters out frames with no useful info (no file AND no function).
 */
pub fn convert_backtrace(bt: &backtrace::Backtrace) -> Vec<BacktraceFrame> {
    let mut frames = Vec::new();

    for frame in bt.frames() {
        for symbol in frame.symbols() {
            let function = symbol.name().map(|n| n.to_string());
            let file = symbol.filename().map(|p| p.display().to_string());
            let line = symbol.lineno();

            if function.is_none() && file.is_none() {
                continue;
            }

            frames.push(BacktraceFrame {
                file,
                line,
                column: symbol.colno(),
                function,
            });
        }
    }

    frames
}
