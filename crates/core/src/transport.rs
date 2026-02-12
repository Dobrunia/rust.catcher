/**
 * HTTP transport layer for sending events to the Hawk collector.
 *
 * This module wraps a `reqwest::blocking::Client` and provides a single
 * `send` method that POSTs a serialized `HawkEvent` envelope to the
 * collector endpoint.
 *
 * Design decisions:
 * - **Blocking HTTP** — the worker thread is already a dedicated background
 *   thread, so blocking I/O is perfectly fine and avoids pulling in a full
 *   async runtime.
 * - **Best-effort delivery** — errors are logged to stderr but never
 *   propagated to the caller. The SDK must never crash the host application.
 * - **Single attempt** — no retries, no exponential backoff. This keeps the
 *   MVP simple. The backend is designed to be highly available; transient
 *   failures are acceptable to drop.
 * - **No `Authorization` header** — the Node.js catcher sends the token
 *   inside the JSON body, not as a header. We match that behaviour exactly.
 */
use crate::types::HawkEvent;

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/**
 * Thin wrapper around `reqwest::blocking::Client` responsible for delivering
 * serialized events to the Hawk collector.
 *
 * A single `Transport` instance is created during `Client::new()` and shared
 * (moved into) the background worker thread.
 */
pub struct Transport {
    /// The underlying HTTP client. Reused across all requests to benefit
    /// from connection pooling and keep-alive.
    http: reqwest::blocking::Client,
}

impl Transport {
    /**
     * Creates a new `Transport` with a default `reqwest::blocking::Client`.
     *
     * The client is configured with sensible defaults:
     * - 10-second connect timeout
     * - 30-second total request timeout
     *
     * Returns `Err` only if reqwest fails to build the client (extremely
     * rare — e.g. TLS backend unavailable).
     */
    pub fn new() -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

        Ok(Self { http })
    }

    /**
     * Sends a `HawkEvent` to the given collector endpoint.
     *
     * The event is serialized to JSON and POSTed with
     * `Content-Type: application/json`.
     *
     * # Arguments
     * * `endpoint` — The full collector URL, e.g.
     *   `https://{integrationId}.k1.hawk.so/`.
     * * `event` — The fully assembled `HawkEvent` envelope containing the
     *   token, catcher type, and event payload.
     *
     * # Error handling
     * This method is **best-effort**: any network or serialization error is
     * printed to stderr and then swallowed. The SDK must never crash the
     * host application.
     */
    pub fn send(&self, endpoint: &str, event: &HawkEvent) {
        /*
         * Attempt the POST request. We use `.json(event)` which handles
         * serialization and sets the Content-Type header automatically.
         *
         * This mirrors the Node.js catcher's:
         *   axios.post(this.collectorEndpoint, eventFormatted)
         */
        let result = self.http
            .post(endpoint)
            .json(event)
            .send();

        /*
         * Best-effort: log failures to stderr but never propagate them.
         * This matches the Node.js catcher's `.catch(err => console.error(...))`.
         */
        match result {
            Ok(response) => {
                if !response.status().is_success() {
                    eprintln!(
                        "[Hawk] Collector responded with HTTP {}: {}",
                        response.status(),
                        response
                            .text()
                            .unwrap_or_else(|_| "<unreadable body>".into())
                    );
                }
            }
            Err(err) => {
                eprintln!("[Hawk] Failed to send event: {err}");
            }
        }
    }
}
