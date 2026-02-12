/**
 * RAII guard returned by `hawk::init()`.
 *
 * The `Guard` ensures that all pending events are flushed to the collector
 * before the process exits. It works via Rust's `Drop` trait:
 *
 * ```ignore
 * fn main() {
 *     let _guard = hawk::init("TOKEN", Default::default()).unwrap();
 *
 *     // ... application logic ...
 *     // events are captured here
 *
 * }   // <-- _guard is dropped here, triggering flush()
 * ```
 *
 * The underscore prefix in `_guard` is idiomatic Rust — it tells the reader
 * "I don't use this variable, I only hold it for its Drop behaviour."
 *
 * If the flush times out (default 2 seconds), the guard drops silently
 * without blocking further. Best-effort delivery is the contract.
 */
use crate::client;

// ---------------------------------------------------------------------------
// Guard
// ---------------------------------------------------------------------------

/**
 * Flush-on-drop guard for the Hawk SDK.
 *
 * Created by `hawk::init()` and should be held alive for the entire
 * duration of the application. When dropped, it calls `Client::flush()`
 * to drain any pending events in the background queue.
 *
 * The guard does NOT own the `Client` — the client lives in a
 * `static OnceLock` and outlives the guard. The guard merely triggers
 * the flush on scope exit.
 */
pub struct Guard {
    /// Intentionally private and zero-sized — the guard is just a token
    /// whose only purpose is to trigger `Drop`.
    _private: (),
}

impl Guard {
    /**
     * Creates a new `Guard`.
     *
     * This is `pub(crate)` because only `hawk::init()` should create guards.
     */
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }
}

impl Drop for Guard {
    /**
     * Called automatically when the guard goes out of scope.
     *
     * Triggers `Client::flush()` which sends a `Flush` message through
     * the channel and waits (with timeout) for the background worker to
     * drain all pending events.
     *
     * If the client is not initialized (shouldn't happen in normal usage),
     * this is a no-op.
     */
    fn drop(&mut self) {
        if let Some(client) = client::get_client() {
            let flushed = client.flush();
            if !flushed {
                eprintln!("[Hawk] Flush timed out — some events may not have been sent");
            }
        }
    }
}
