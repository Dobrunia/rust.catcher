/**
 * Core type definitions for the Hawk Rust SDK.
 *
 * These structures mirror the protocol expected by the Hawk backend,
 * matching the Node.js catcher's JSON format 1:1.
 *
 * The outermost envelope is `HawkEvent`, which wraps an `EventData` payload.
 * The backend receives: { token, catcherType, payload: EventData }.
 */
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Envelope — the top-level structure POSTed to the collector
// ---------------------------------------------------------------------------

/**
 * The outer envelope sent to the Hawk collector via HTTP POST.
 *
 * This matches the Node.js `HawkEvent` interface exactly:
 * ```json
 * {
 *   "token": "<base64-encoded-integration-token>",
 *   "catcherType": "errors/rust",
 *   "payload": { ...EventData... }
 * }
 * ```
 *
 * `token` is the raw base64-encoded integration token (passed through as-is).
 * `catcherType` identifies the SDK family — we use `"errors/rust"`.
 * `payload` carries the actual event data.
 */
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HawkEvent {
    /// The raw base64-encoded integration token provided by the user.
    pub token: String,

    /// Identifies the catcher family. Always `"errors/rust"` for this SDK.
    pub catcher_type: String,

    /// The event payload conforming to the `EventData` schema.
    pub payload: EventData,
}

// ---------------------------------------------------------------------------
// EventData — the actual error / message payload
// ---------------------------------------------------------------------------

/**
 * Core event payload matching the backend's `EventData<Addons>` interface.
 *
 * MVP sends only `title`, `type`, `backtrace`, and `catcherVersion`.
 * Fields like `release`, `user`, `context` are omitted for now and will
 * be added in future iterations.
 */
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventData {
    /// Human-readable title, e.g. `"Error: something broke"` or `"panic: index out of bounds"`.
    pub title: String,

    /// Error type name — equivalent to `error.name` in Node.js.
    /// Examples: `"Error"`, `"TypeError"`, `"panic"`, `"message"`.
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,

    /// Stack trace frames, from the most recent call to the earliest.
    /// `None` when no backtrace is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backtrace: Option<Vec<BacktraceFrame>>,

    /// SDK version string, e.g. `"hawk-rust/0.1.0"`.
    pub catcher_version: String,
}

// ---------------------------------------------------------------------------
// BacktraceFrame
// ---------------------------------------------------------------------------

/**
 * A single frame in the backtrace, matching the backend's `BacktraceFrame`.
 *
 * In the MVP we populate what we can from `backtrace::BacktraceFrame`:
 * - `file` — source file path (if resolved)
 * - `line` — line number
 * - `column` — column number (often unavailable)
 * - `function` — demangled function name
 *
 * The `sourceCode` field from the Node.js version is omitted in the MVP
 * because Rust binaries typically don't ship source alongside.
 */
#[derive(Clone, Serialize, Deserialize)]
pub struct BacktraceFrame {
    /// Source file path, if debug info is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,

    /// Line number within the source file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,

    /// Column number within the source line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,

    /// Demangled function / symbol name.
    #[serde(rename = "function")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
}


