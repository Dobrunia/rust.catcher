/*!
 * HTTP transport layer for sending events to the Hawk collector.
 *
 * Uses `ureq` — a pure-Rust blocking HTTP client with no async runtime.
 * This avoids pulling in tokio (which `reqwest::blocking` does under the hood),
 * cutting compile time and binary size significantly.
 *
 * Design decisions:
 * - **Blocking HTTP** — the worker thread is already a dedicated background
 *   thread, so blocking I/O is perfectly fine.
 * - **Best-effort delivery** — errors are logged to stderr but never
 *   propagated. The SDK must never crash the host application.
 * - **Single attempt** — no retries. The backend is designed to be highly
 *   available; transient failures are acceptable to drop.
 */

use std::time::Duration;

use ureq::Agent;

use crate::protocol::types::HawkEvent;

/**
 * Thin wrapper around `ureq::Agent` responsible for delivering
 * serialized events to the Hawk collector.
 *
 * A single `Transport` instance is created during `Client::init()` and
 * moved into the background worker thread.
 */
pub struct Transport {
    agent: Agent,
}

impl Transport {
    /**
     * Creates a new `Transport` with a configured `ureq::Agent`.
     *
     * Timeouts:
     * - 10 s connect
     * - 30 s total per request
     *
     * Connection pooling and keep-alive are handled by the agent internally.
     */
    pub fn new() -> Result<Self, String> {
        let agent: Agent = Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();

        Ok(Self { agent })
    }

    /**
     * Sends a `HawkEvent` to the given collector endpoint.
     *
     * The event is serialized to JSON and POSTed with
     * `Content-Type: application/json`.
     *
     * Best-effort: any error is printed to stderr and swallowed.
     */
    pub fn send(&self, endpoint: &str, event: &HawkEvent) {
        let result = self.agent
            .post(endpoint)
            .send_json(event);

        match result {
            Ok(response) => {
                let status = response.status().as_u16();
                if !(200..300).contains(&status) {
                    let body = response
                        .into_body()
                        .read_to_string()
                        .unwrap_or_else(|_| "<unreadable body>".into());
                    eprintln!("[Hawk] Collector responded with HTTP {status}: {body}");
                }
            }
            Err(err) => {
                eprintln!("[Hawk] Failed to send event: {err}");
            }
        }
    }
}
