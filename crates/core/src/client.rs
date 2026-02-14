/**
 * The Hawk SDK client — central orchestrator that owns the event queue,
 * background worker, and transport.
 *
 * Lifecycle:
 * 1. User calls `hawk::init(token, options)` → creates a `Client` and
 *    stores it in a global `OnceLock`.
 * 2. All `hawk::capture_*` calls read the global `Client`.
 * 3. `hawk::init` returns a `Guard`; when the guard is dropped, it calls
 *    `Client::flush()` to drain pending events before the process exits.
 *
 * The client is intentionally **not** `Clone` — there is exactly one
 * instance per process, held in the `OnceLock`.
 */
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crossbeam_channel::{Sender, TrySendError};

use crate::token;
use crate::transport::Transport;
use crate::types::{
    BeforeSendResult, EventData, HawkEvent, CATCHER_TYPE, CATCHER_VERSION,
};
use crate::worker::{FlushSignal, Worker, WorkerMsg};

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

/**
 * Process-wide singleton holding the initialized `Client`.
 *
 * `OnceLock` ensures that `init()` can only succeed once — subsequent calls
 * are no-ops. All public free functions (`capture_message`, etc.)
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
 * hawk::init("BASE64_TOKEN", hawk::Options {
 *     queue_capacity: 200,
 *     ..Default::default()
 * });
 * ```
 */
pub struct Options {
    /// Custom collector endpoint URL. If `None`, the SDK derives the
    /// endpoint from the integration token:
    /// `https://{integrationId}.k1.hawk.so/`
    pub collector_endpoint: Option<String>,

    /// Bounded channel capacity. When the queue is full, new events are
    /// dropped silently (back-pressure).
    /// Default: `100`.
    pub queue_capacity: usize,

    /// Maximum time (in milliseconds) that `flush()` will block waiting
    /// for the worker to drain pending events.
    /// Default: `2000` (2 seconds).
    pub flush_timeout_ms: u64,

    /// Optional callback invoked before each event is sent.
    ///
    /// Allows the user to:
    /// - Inspect / modify the event (`BeforeSendResult::Send(modified)`)
    /// - Drop the event entirely (`BeforeSendResult::Drop`)
    ///
    /// If not set, events are sent as-is.
    pub before_send: Option<Arc<dyn Fn(EventData) -> BeforeSendResult + Send + Sync>>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            collector_endpoint: None,
            queue_capacity: 100,
            flush_timeout_ms: 2000,
            before_send: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/**
 * The core SDK client.
 *
 * Owns:
 * - The raw token string (passed through in every envelope).
 * - The bounded channel sender (events are enqueued here).
 * - Snapshot of `Options` fields needed at send-time.
 */
pub struct Client {
    /// Raw base64-encoded integration token — included in every `HawkEvent`.
    token: String,

    /// Sender side of the bounded event channel.
    sender: Sender<WorkerMsg>,

    /// Optional before_send callback.
    before_send: Option<Arc<dyn Fn(EventData) -> BeforeSendResult + Send + Sync>>,

    /// Flush timeout duration.
    flush_timeout: Duration,
}

/**
 * `Client` is `Send + Sync` because:
 * - `Sender<WorkerMsg>` is `Send + Sync`.
 * - `Arc<dyn Fn + Send + Sync>` is `Send + Sync`.
 * - All other fields are plain data.
 */
unsafe impl Send for Client {}
unsafe impl Sync for Client {}

impl Client {
    /**
     * Creates a new `Client` and stores it in the global `OnceLock`.
     *
     * This function should be called exactly once (via `hawk::init()`).
     * Subsequent calls return `Err` because the `OnceLock` is already set.
     *
     * # Steps
     * 1. Decode the integration token to extract `integrationId`.
     * 2. Resolve the collector endpoint (custom or default).
     * 3. Create the bounded channel.
     * 4. Build and spawn the transport + worker.
     * 5. Store the client in `GLOBAL_CLIENT`.
     *
     * # Arguments
     * * `token` — The raw base64-encoded integration token.
     * * `options` — SDK configuration options.
     *
     * # Returns
     * `Ok(())` on success, `Err(String)` if the token is invalid or the
     * client has already been initialized.
     */
    pub fn init(token_str: &str, options: Options) -> Result<(), String> {
        /*
         * Step 1: Decode the integration token.
         * This validates the token format and extracts the integrationId.
         */
        let decoded = token::decode_token(token_str)?;

        /*
         * Step 2: Resolve the collector endpoint.
         * If the user provided a custom endpoint, use it; otherwise derive
         * from the integration ID — matching Node.js catcher behaviour.
         */
        let endpoint = options
            .collector_endpoint
            .clone()
            .unwrap_or_else(|| token::default_endpoint(&decoded.integration_id));

        /*
         * Step 3: Create the bounded channel.
         * `try_send` on the sender will fail gracefully when the channel
         * is full, causing events to be dropped — which is the intended
         * back-pressure behaviour.
         */
        let (sender, receiver) = crossbeam_channel::bounded(options.queue_capacity);

        /*
         * Step 4: Create the transport (HTTP client) and spawn the worker.
         */
        let transport = Transport::new()?;
        let _worker = Worker::spawn(receiver, endpoint, transport);

        /*
         * Build the client with snapshots of relevant options.
         */
        let client = Client {
            token: token_str.to_string(),
            sender,
            before_send: options.before_send,
            flush_timeout: Duration::from_millis(options.flush_timeout_ms),
        };

        /*
         * Step 5: Store in the global singleton.
         * `set()` returns `Err(value)` if already initialized.
         */
        GLOBAL_CLIENT
            .set(client)
            .map_err(|_| "Hawk SDK is already initialized".to_string())?;

        Ok(())
    }

    /**
     * Enqueues a fully built `EventData` for delivery.
     *
     * This is the internal "send" path used by all public `capture_*` functions.
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
         * Fill in the catcher version if the caller didn't set it.
         */
        if event.catcher_version.is_empty() {
            event.catcher_version = CATCHER_VERSION.to_string();
        }

        /*
         * Run the before_send callback if configured.
         * This lets the user filter sensitive data or drop events entirely.
         */
        if let Some(ref callback) = self.before_send {
            match callback(event) {
                BeforeSendResult::Drop => return,
                BeforeSendResult::Send(modified) => event = modified,
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
     * the queue or the configured timeout elapses.
     *
     * This is called automatically by `Guard::drop()` to ensure events
     * are delivered before the process exits.
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
        match self.sender.try_send(WorkerMsg::Flush(signal.clone())) {
            Ok(()) => signal.wait_timeout(self.flush_timeout),
            Err(_) => false,
        }
    }
}
