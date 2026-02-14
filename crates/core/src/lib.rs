/**
 * Hawk Rust SDK — Core crate.
 *
 * This is the main entry point for the Hawk error tracking SDK.
 * It provides a simple, thread-safe API for capturing errors and messages
 * and sending them to the Hawk backend.
 *
 * # Quick start
 *
 * ```ignore
 * use hawk_core as hawk;
 *
 * fn main() {
 *     let _guard = hawk::init("YOUR_BASE64_TOKEN", Default::default())
 *         .expect("Failed to init Hawk");
 *
 *     hawk::send("Application started");
 *
 *     // ... your application logic ...
 *
 *     // _guard is dropped here → flush() is called automatically
 * }
 * ```
 *
 * # Architecture
 *
 * - `init()` creates a global `Client` (stored in `OnceLock`) and spawns
 *   a background worker thread that drains events from a bounded channel.
 * - `send()` / `capture_error()` build an `EventData` and enqueue it on
 *   the channel (non-blocking).
 * - The worker POSTs each event as a `HawkEvent` envelope to the Hawk
 *   collector via `reqwest` (blocking HTTP in the dedicated thread).
 * - `Guard::drop()` calls `flush()` to ensure pending events are delivered
 *   before the process exits.
 */

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

mod client;
mod guard;
mod token;
mod transport;
mod types;
mod worker;

// ---------------------------------------------------------------------------
// Re-exports — the public surface area
// ---------------------------------------------------------------------------

use client::Options;
use guard::Guard;
pub use types::{BacktraceFrame, EventData, CATCHER_VERSION};

// ---------------------------------------------------------------------------
// Public free functions
// ---------------------------------------------------------------------------

/**
 * Initializes the Hawk SDK with the given integration token.
 *
 * This function MUST be called exactly once, typically at the very beginning
 * of `main()`. It:
 *
 * 1. Decodes and validates the integration token.
 * 2. Derives the collector endpoint from the token.
 * 3. Creates a bounded event channel and spawns the background worker.
 * 4. Stores the `Client` in a process-wide `OnceLock`.
 * 5. Returns a `Guard` that flushes pending events when dropped.
 *
 * # Arguments
 * * `token` — The base64-encoded integration token from the Hawk project
 *   settings page.
 * * `options` — SDK configuration. Use `Default::default()` for sensible
 *   defaults, or configure `before_send` to filter / modify events.
 *
 * # Returns
 * * `Ok(Guard)` — Hold this value alive for the duration of your app.
 * * `Err(String)` — If the token is invalid or the SDK is already initialized.
 *
 * # Example
 * ```ignore
 * let _guard = hawk::init("eyJpbnRl...", Default::default())?;
 * ```
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
 * Analogous to `HawkCatcher.send(error)` in the Node.js SDK.
 *
 * If the SDK has not been initialized (no prior `init()` call), this
 * is a silent no-op.
 *
 * # Arguments
 * * `message` — Anything implementing `Display`: `&str`, `String`,
 *   `&dyn Error`, `io::Error`, `anyhow::Error`, etc.
 *
 * # Examples
 * ```ignore
 * hawk::send("User session expired");
 *
 * match std::fs::read_to_string("config.toml") {
 *     Ok(_) => {},
 *     Err(e) => hawk::send(&e),
 * }
 * ```
 */
pub fn send(message: &impl std::fmt::Display) {
    if let Some(client) = client::get_client() {
        let event: EventData = EventData {
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
 * This is the low-level API used by `hawk_panic` to send panic events
 * with custom backtrace data. Also available for advanced users who want
 * full control over the event payload.
 *
 * If the SDK has not been initialized, this is a silent no-op.
 *
 * # Arguments
 * * `event` — A fully or partially constructed `EventData`. The client
 *   will fill in `catcher_version` if missing.
 */
pub fn capture_event(event: EventData) {
    if let Some(client) = client::get_client() {
        client.send_event(event);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/**
 * Captures a backtrace at the current call site and converts it into
 * a `Vec<BacktraceFrame>` suitable for the Hawk event payload.
 *
 * Returns `None` if no useful frames were resolved (e.g. no debug info).
 *
 * Used by both `send()` and `capture_error()` to attach stack traces
 * that point back to the exact location where the SDK function was called.
 */
pub fn get_backtrace() -> Option<Vec<BacktraceFrame>> {
    let bt: backtrace::Backtrace = backtrace::Backtrace::new();
    let frames: Vec<BacktraceFrame> = convert_backtrace(&bt);
    if frames.is_empty() { None } else { Some(frames) }
}

/**
 * Converts a `backtrace::Backtrace` into a `Vec<BacktraceFrame>`.
 *
 * Iterates over resolved frames and extracts:
 * - Function name (demangled symbol)
 * - File path
 * - Line number
 * - Column number
 *
 * Filters out frames with no useful information (no file AND no function).
 *
 * # Arguments
 * * `bt` — A captured `backtrace::Backtrace` (must already be resolved).
 */
pub fn convert_backtrace(bt: &backtrace::Backtrace) -> Vec<BacktraceFrame> {
    let mut frames = Vec::new();

    for frame in bt.frames() {
        for symbol in frame.symbols() {
            let function = symbol.name().map(|n| n.to_string());
            let file = symbol.filename().map(|p| p.display().to_string());
            let line = symbol.lineno();

            /*
             * Skip frames with no useful info — typically internal
             * runtime / linker frames that aren't helpful for debugging.
             */
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
