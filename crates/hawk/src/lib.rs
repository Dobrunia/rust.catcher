/**
 * Hawk — Rust error tracking SDK.
 *
 * This is the main crate users should depend on. It re-exports the core
 * SDK API and wires up addons (panic hook, etc.) through a single `init` call.
 *
 * # Quick start
 *
 * ```ignore
 * fn main() {
 *     let _guard = hawk::init("YOUR_BASE64_TOKEN");
 *
 *     hawk::send("Application started");
 *
 *     // panics are automatically captured (catch_panics defaults to true)
 *     // _guard is dropped here → flush() is called automatically
 * }
 * ```
 *
 * # With options
 *
 * ```ignore
 * use std::sync::Arc;
 *
 * fn main() {
 *     let _guard = hawk::init(hawk::Options {
 *         token: "YOUR_TOKEN".into(),
 *         catch_panics: false,
 *         before_send: Some(Arc::new(|mut event| {
 *             event.title = format!("[filtered] {}", event.title);
 *             hawk::BeforeSendResult::Send(event)
 *         })),
 *     });
 *
 *     hawk::send("something happened");
 * }
 * ```
 */

use std::sync::Arc;

// ---------------------------------------------------------------------------
// Re-exports from hawk_core — the public surface area
// ---------------------------------------------------------------------------

pub use hawk_core::{
    BacktraceFrame, BeforeSendResult, EventData, Guard, HawkEvent, CATCHER_VERSION,
    send, capture_event, flush, get_backtrace, convert_backtrace,
};

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/**
 * Configuration for the Hawk SDK.
 *
 * Implements `From<&str>` so you can pass just a token string to `init()`.
 * All optional fields have sensible defaults:
 * - `catch_panics` = `true`
 * - `before_send` = `None`
 */
pub struct Options {
    /// The base64-encoded integration token from your Hawk project settings.
    pub token: String,

    /// Whether to install a panic hook that auto-captures panics.
    /// Defaults to `true`.
    pub catch_panics: bool,

    /// Optional callback invoked before each event is sent.
    /// Return `BeforeSendResult::Send(event)` to send (possibly modified),
    /// or `BeforeSendResult::Drop` to discard the event.
    pub before_send: Option<Arc<dyn Fn(EventData) -> BeforeSendResult + Send + Sync>>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            token: String::new(),
            catch_panics: true,
            before_send: None,
        }
    }
}

/**
 * Allows `hawk::init("TOKEN")` — converts a token string into
 * `Options` with all defaults.
 */
impl From<&str> for Options {
    fn from(token: &str) -> Self {
        Self {
            token: token.to_string(),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

/**
 * Initializes the Hawk SDK.
 *
 * Accepts either a bare token string or a full `Options` struct:
 *
 * ```ignore
 * // Simple — just a token (panics caught by default)
 * let _guard = hawk::init("TOKEN");
 *
 * // Full control
 * let _guard = hawk::init(hawk::Options {
 *     token: "TOKEN".into(),
 *     catch_panics: false,
 *     before_send: Some(Arc::new(|e| hawk::BeforeSendResult::Send(e))),
 * });
 * ```
 *
 * # Panics
 * Panics if the token is malformed or `init` is called more than once.
 * These are configuration bugs that should be caught at startup.
 *
 * # Returns
 * A `Guard` — keep it alive for the duration of your app.
 * When it drops, all pending events are flushed.
 */
pub fn init(options: impl Into<Options>) -> Guard {
    let opts = options.into();

    /*
     * Split Options into the core part (before_send) and addon flags.
     */
    let core_options = hawk_core::Options {
        before_send: opts.before_send,
    };

    let guard = hawk_core::init(&opts.token, core_options)
        .expect("[Hawk] Failed to initialize SDK");

    /*
     * Install addons based on the options.
     * Panic hook is opt-out (enabled by default) — most users want it.
     */
    if opts.catch_panics {
        hawk_panic::install();
    }

    guard
}
