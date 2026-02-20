/**
 * The Hawk SDK client — central orchestrator that owns the event queue,
 * background worker, and transport.
 *
 * Lifecycle:
 * 1. User calls `hawk::init(token)` → creates a `Client` and stores it
 *    in a global `OnceLock`.
 * 2. `hawk::send()` / `hawk::capture_error()` read the global `Client`
 *    and enqueue events.
 * 3. `hawk::init` returns a `Guard`; when the guard is dropped, it calls
 *    `Client::flush()` to drain pending events before the process exits.
 *
 * The client is intentionally **not** `Clone` — there is exactly one
 * instance per process, held in the `OnceLock`.
 */
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crossbeam_channel::{Sender, TrySendError};

use crate::protocol::constants::CATCHER_TYPE;
use crate::protocol::token;
use crate::protocol::types::{EventData, HawkEvent};
use crate::transport::{FlushSignal, Transport, Worker, WorkerMsg};

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

/**
 * Process-wide singleton holding the initialized `Client`.
 *
 * `OnceLock` ensures that `init()` can only succeed once — subsequent calls
 * return an error. All public free functions (`send`, `capture_error`, etc.)
 * access this global via `get_client()`.
 */
static GLOBAL_CLIENT: OnceLock<Client> = OnceLock::new();

/**
 * Returns a reference to the global client, or `None` if `init()` has not
 * been called yet.
 */
pub fn get_client() -> Option<&'static Client> {
    GLOBAL_CLIENT.get()
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/**
 * Configuration options for the Hawk SDK.
 *
 * Passed to `hawk::init()` alongside the integration token.
 * All fields have sensible defaults via `Default`.
 *
 * # Example
 * ```ignore
 * use std::sync::Arc;
 *
 * hawk::init("BASE64_TOKEN", hawk::Options {
 *     before_send: Some(Arc::new(|mut event| {
 *         event.title = format!("[filtered] {}", event.title);
 *         Some(event) // return modified event, or None to drop
 *     })),
 *     ..Default::default()
 * });
 * ```
 */
#[derive(Default)]
pub struct Options {
    /// Optional callback invoked before each event is sent.
    ///
    /// Receives a clone of the event. Return value:
    /// - `None` → drop the event (it will NOT be sent)
    /// - `Some(event)` → send this (possibly modified) event
    ///
    /// If the callback panics, the original event is sent unchanged
    /// and a warning is printed to stderr.
    ///
    /// If not set, events are sent as-is.
    pub before_send: Option<Arc<dyn Fn(EventData) -> Option<EventData> + Send + Sync>>,
}

// ---------------------------------------------------------------------------
// Internal constants
// ---------------------------------------------------------------------------

/// Bounded channel capacity — internal implementation detail.
/// When full, new events are silently dropped (back-pressure).
const QUEUE_CAPACITY: usize = 100;

/// Maximum time that `flush()` will block waiting for the worker
/// to drain pending events before giving up.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/**
 * The core SDK client.
 *
 * Owns:
 * - The raw token string (passed through in every envelope).
 * - The bounded channel sender (events are enqueued here).
 */
pub struct Client {
    /// Raw base64-encoded integration token — included in every `HawkEvent`.
    token: String,

    /// Sender side of the bounded event channel.
    sender: Sender<WorkerMsg>,

    /// Optional before_send callback.
    before_send: Option<Arc<dyn Fn(EventData) -> Option<EventData> + Send + Sync>>,
}

impl Client {
    /**
     * Creates a new `Client` and stores it in the global `OnceLock`.
     *
     * This function should be called exactly once (via `hawk::init()`).
     * Subsequent calls return `Err` because the `OnceLock` is already set.
     *
     * # Steps
     * 1. Decode the integration token to extract `integrationId`.
     * 2. Derive the collector endpoint from the integration ID.
     * 3. Create the bounded channel.
     * 4. Build and spawn the transport + worker.
     * 5. Store the client in `GLOBAL_CLIENT`.
     *
     * # Arguments
     * * `token_str` — The raw base64-encoded integration token.
     * * `options` — SDK configuration (use `Default::default()` for defaults).
     *
     * # Returns
     * `Ok(())` on success, `Err(String)` if the token is invalid or the
     * client has already been initialized.
     */
    pub fn init(token_str: &str, options: Options) -> Result<(), String> {
        /*
         * Early guard: avoid spawning threads/HTTP clients if already initialized.
         */
        if GLOBAL_CLIENT.get().is_some() {
            return Err("Hawk SDK is already initialized".into());
        }

        /*
         * Step 1: Decode the integration token.
         * This validates the token format and extracts the integrationId.
         */
        let decoded = token::decode_token(token_str)?;

        /*
         * Step 2: Derive the collector endpoint from the integration ID.
         * Format: https://{integrationId}.k1.hawk.so/
         */
        let endpoint = token::default_endpoint(&decoded.integration_id);

        /*
         * Step 3: Create the bounded channel.
         * `try_send` on the sender will fail gracefully when the channel
         * is full, causing events to be dropped — which is the intended
         * back-pressure behaviour.
         */
        let (sender, receiver) = crossbeam_channel::bounded(QUEUE_CAPACITY);

        /*
         * Step 4: Create the transport (HTTP client) and spawn the worker.
         */
        let transport = Transport::new()?;
        Worker::spawn(receiver, endpoint, transport)?;

        /*
         * Step 5: Store in the global singleton.
         * `set()` returns `Err(value)` if already initialized.
         */
        let client = Client {
            token: token_str.to_string(),
            sender,
            before_send: options.before_send,
        };

        GLOBAL_CLIENT
            .set(client)
            .map_err(|_| "Hawk SDK is already initialized".to_string())?;

        Ok(())
    }

    /**
     * Enqueues a fully built `EventData` for delivery.
     *
     * This is the internal "send" path used by all public functions.
     * It:
     * 1. Fills in `catcher_version` if empty.
     * 2. Runs the `before_send` callback if configured.
     * 3. Wraps the payload in a `HawkEvent` envelope.
     * 4. Enqueues the envelope on the bounded channel (non-blocking).
     *
     * If the queue is full, the event is silently dropped.
     *
     * # Arguments
     * * `event` — The event data to send.
     */
    pub fn send_event(&self, mut event: EventData) {
        /*
         * Run the before_send callback if configured.
         *
         * Mirrors the Node.js catcher behaviour:
         * - Callback receives a clone of the event (original stays intact).
         * - Returns None  → drop the event.
         * - Returns Some(modified) → send the modified event.
         * - Panics → send the original event unchanged, print a warning.
         */
        if let Some(ref callback) = self.before_send {
            let original = event.clone();

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                callback(original)
            }));

            match result {
                Ok(None) => return,
                Ok(Some(modified)) => event = modified,
                Err(_) => {
                    eprintln!(
                        "[Hawk] before_send panicked — sending original event unchanged"
                    );
                }
            }
        }

        /*
         * Wrap in the HawkEvent envelope — the exact format the backend expects.
         */
        let hawk_event = HawkEvent {
            token: self.token.clone(),
            catcher_type: CATCHER_TYPE.to_string(),
            payload: event,
        };

        /*
         * Non-blocking enqueue. If the channel is full, the event is dropped
         * silently — this is the intended back-pressure behaviour.
         */
        match self.sender.try_send(WorkerMsg::Event(hawk_event)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                eprintln!("[Hawk] Event queue is full — dropping event");
            }
            Err(TrySendError::Disconnected(_)) => {
                eprintln!("[Hawk] Worker thread has shut down — dropping event");
            }
        }
    }

    /**
     * Flushes all pending events, blocking until the worker has drained
     * the queue or the timeout elapses (2 seconds).
     *
     * Called automatically by `Guard::drop()` to ensure events are
     * delivered before the process exits.
     *
     * # Returns
     * `true` if the flush completed within the timeout, `false` otherwise.
     */
    pub fn flush(&self) -> bool {
        let signal = Arc::new(FlushSignal::new());

        /*
         * Send a Flush message into the channel. Because the channel is FIFO,
         * by the time the worker processes this message, all preceding
         * Event messages will have been sent.
         */
        match self.sender.send_timeout(WorkerMsg::Flush(signal.clone()), FLUSH_TIMEOUT) {
            Ok(()) => signal.wait_timeout(FLUSH_TIMEOUT),
            Err(_) => false,
        }
    }
}
