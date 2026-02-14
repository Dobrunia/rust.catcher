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
 *     let _guard = hawk::init("YOUR_BASE64_TOKEN", hawk::Options {
 *         release: Some("1.0.0".into()),
 *         environment: Some("production".into()),
 *         ..Default::default()
 *     }).expect("Failed to init Hawk");
 *
 *     hawk::capture_message("Application started");
 *
 *     hawk::set_tag("region", "eu");
 *     hawk::set_extra("build", "release");
 *     hawk::set_user(hawk::User {
 *         id: Some("42".into()),
 *         ..Default::default()
 *     });
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
 * - `capture_*` functions build an `EventData`, attach global context,
 *   and enqueue it on the channel (non-blocking).
 * - The worker POSTs each event as a `HawkEvent` envelope to the Hawk
 *   collector via `reqwest` (blocking HTTP in the dedicated thread).
 * - `Guard::drop()` calls `flush()` to ensure pending events are delivered
 *   before the process exits.
 */

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

pub mod client;
pub mod context;
pub mod guard;
pub mod token;
pub mod transport;
pub mod types;
pub mod worker;

// ---------------------------------------------------------------------------
// Re-exports — the public surface area
// ---------------------------------------------------------------------------

pub use client::Options;
pub use guard::Guard;
pub use types::{
    BacktraceFrame, BeforeSendResult, EventData, HawkEvent, User,
    CATCHER_VERSION,
};

// ---------------------------------------------------------------------------
// Public free functions
// ---------------------------------------------------------------------------

/**
 * Initializes the Hawk SDK with the given integration token and options.
 *
 * This function MUST be called exactly once, typically at the very beginning
 * of `main()`. It:
 *
 * 1. Decodes and validates the integration token.
 * 2. Resolves the collector endpoint (custom or default).
 * 3. Creates a bounded event channel and spawns the background worker.
 * 4. Stores the `Client` in a process-wide `OnceLock`.
 * 5. Returns a `Guard` that flushes pending events when dropped.
 *
 * # Arguments
 * * `token` — The base64-encoded integration token from the Hawk project
 *   settings page.
 * * `options` — SDK configuration. Use `Default::default()` for sensible
 *   defaults.
 *
 * # Returns
 * * `Ok(Guard)` — Hold this value alive for the duration of your app.
 * * `Err(String)` — If the token is invalid or the SDK is already initialized.
 *
 * # Example
 * ```ignore
 * let _guard = hawk_core::init("eyJpbnRl...", Default::default())?;
 * ```
 */
pub fn init(token: &str, options: Options) -> Result<Guard, String> {
    client::Client::init(token, options)?;
    Ok(Guard::new())
}

/**
 * Captures a text message and sends it as an event to Hawk.
 *
 * This is the simplest way to send a custom event. Useful for logging
 * significant application milestones (e.g. "Deployment complete").
 *
 * The `type` field is set to `"message"` — matching the convention that
 * `type` is the error class name (like `error.name` in Node.js), and
 * for plain messages there is no error class.
 *
 * If the SDK has not been initialized (no prior `init()` call), this
 * is a silent no-op.
 *
 * # Arguments
 * * `message` — The human-readable message to send as the event title.
 *
 * # Example
 * ```ignore
 * hawk::capture_message("User logged in");
 * hawk::capture_message("Disk usage > 90%");
 * ```
 */
pub fn capture_message(message: &str) {
    if let Some(client) = client::get_client() {
        let event = EventData {
            title: message.to_string(),
            event_type: Some("message".to_string()),
            backtrace: None,
            release: None,
            user: None,
            context: None,
            catcher_version: CATCHER_VERSION.to_string(),
        };
        client.send_event(event);
    }
}

/**
 * Captures a Rust `dyn Error` and sends it as an event to Hawk.
 *
 * The error's `Display` output becomes the event title, and the error type
 * is set to `"error"`. A backtrace is captured at the point of this call
 * to help with debugging.
 *
 * If the SDK has not been initialized, this is a silent no-op.
 *
 * # Arguments
 * * `error` — Any type implementing `std::error::Error`.
 *
 * # Example
 * ```ignore
 * match std::fs::read_to_string("config.toml") {
 *     Ok(_) => {},
 *     Err(e) => hawk::capture_error(&e),
 * }
 * ```
 */
pub fn capture_error(error: &dyn std::error::Error) {
    if let Some(client) = client::get_client() {
        /*
         * Capture a backtrace at the call site.
         * We use the `backtrace` crate to get structured frames, then
         * convert them to our `BacktraceFrame` format.
         */
        let bt = backtrace::Backtrace::new();
        let frames = convert_backtrace(&bt);

        let event = EventData {
            title: error.to_string(),
            event_type: Some("error".to_string()),
            backtrace: if frames.is_empty() {
                None
            } else {
                Some(frames)
            },
            release: None,
            user: None,
            context: None,
            catcher_version: CATCHER_VERSION.to_string(),
        };
        client.send_event(event);
    }
}

/**
 * Sends a pre-built `EventData` directly to Hawk.
 *
 * This is the low-level internal API used by `hawk_panic` to send
 * panic events with custom backtrace and context.
 *
 * Also available for advanced users who want full control over the
 * event payload.
 *
 * If the SDK has not been initialized, this is a silent no-op.
 *
 * # Arguments
 * * `event` — A fully or partially constructed `EventData`. The client
 *   will fill in missing fields (release, user, context) from global state.
 */
pub fn capture_event(event: EventData) {
    if let Some(client) = client::get_client() {
        client.send_event(event);
    }
}

/**
 * Sets a global tag that will be attached to all subsequent events.
 *
 * Tags are string key-value pairs useful for filtering and grouping
 * events in the Hawk dashboard.
 *
 * Overwrites any existing tag with the same key.
 *
 * # Arguments
 * * `key` — Tag name (e.g. `"region"`, `"deployment"`).
 * * `value` — Tag value (e.g. `"eu-west-1"`, `"canary"`).
 */
pub fn set_tag(key: &str, value: &str) {
    if let Some(client) = client::get_client() {
        client.context.set_tag(key, value);
    }
}

/**
 * Sets a global extra that will be attached to all subsequent events.
 *
 * Extras are free-form key-value pairs for additional debugging context.
 *
 * Overwrites any existing extra with the same key.
 *
 * # Arguments
 * * `key` — Extra key (e.g. `"request_id"`, `"correlation_id"`).
 * * `value` — Extra value.
 */
pub fn set_extra(key: &str, value: &str) {
    if let Some(client) = client::get_client() {
        client.context.set_extra(key, value);
    }
}

/**
 * Sets the current user that will be attached to all subsequent events.
 *
 * Replaces any previously set user.
 *
 * # Arguments
 * * `user` — The affected user. At minimum, `id` should be set.
 *
 * # Example
 * ```ignore
 * hawk::set_user(hawk::User {
 *     id: Some("user_42".into()),
 *     name: Some("Alice".into()),
 *     ..Default::default()
 * });
 * ```
 */
pub fn set_user(user: User) {
    if let Some(client) = client::get_client() {
        client.context.set_user(user);
    }
}

/**
 * Manually triggers a flush of all pending events.
 *
 * Blocks the calling thread until the background worker has drained
 * the queue or the configured timeout elapses.
 *
 * Normally you don't need to call this — the `Guard` handles it
 * automatically on drop. Use this if you need to ensure delivery
 * at a specific point in your code.
 *
 * # Returns
 * `true` if all pending events were sent within the timeout,
 * `false` if the timeout expired.
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
 * Converts a `backtrace::Backtrace` into a `Vec<BacktraceFrame>` suitable
 * for the Hawk event payload.
 *
 * Iterates over resolved backtrace frames and extracts:
 * - Function name (demangled symbol)
 * - File path
 * - Line number
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
