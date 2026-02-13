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
#[derive(Debug, Clone, Serialize)]
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
 * Fields marked `Option` are serialized as `null` when `None` (via serde).
 * `skip_serializing_if` is used for truly optional fields that the backend
 * tolerates being absent.
 *
 * Key invariants (matching Node.js behaviour):
 * - `context` is a shallow merge of global context + per-event context.
 * - `catcher_version` is always present.
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventData {
    /// Human-readable title, e.g. `"Error: something broke"` or `"panic: index out of bounds"`.
    pub title: String,

    /// Severity / error type name. Maps to `Level` enum stringified.
    /// Examples: `"error"`, `"fatal"`, `"info"`.
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,

    /// Stack trace frames, from the most recent call to the earliest.
    /// `None` when no backtrace is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backtrace: Option<Vec<BacktraceFrame>>,

    /// Application release / version string set during `init()`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,

    /// The affected user at the time of the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,

    /// Arbitrary key-value context. Shallow merge of global + per-event context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

// ---------------------------------------------------------------------------
// User
// ---------------------------------------------------------------------------

/**
 * Represents the affected user at the time of the event.
 *
 * Matches the backend's `AffectedUser` interface.
 * All fields are optional except conceptually at least `id` should be set.
 */
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct User {
    /// Internal application user identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// User's display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// URL to the user's profile / details page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// URL to the user's avatar / profile picture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

// ---------------------------------------------------------------------------
// Level (severity)
// ---------------------------------------------------------------------------

/**
 * Severity level for events.
 *
 * Serialized as lowercase strings to match the backend's `type` field:
 * `"debug"`, `"info"`, `"warn"`, `"error"`, `"fatal"`.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl Level {
    /**
     * Returns the string representation used in the backend protocol.
     * E.g. `Level::Fatal` -> `"fatal"`.
     */
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
            Level::Fatal => "fatal",
        }
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// BeforeSendResult
// ---------------------------------------------------------------------------

/**
 * Return type for the `before_send` callback.
 *
 * Allows the user to:
 * - `Drop` — silently discard the event (it will NOT be sent).
 * - `Send(EventData)` — send a potentially modified event.
 *
 * If the callback is not set, the event is sent as-is.
 */
pub enum BeforeSendResult {
    /// Discard the event entirely — do not send it.
    Drop,

    /// Send this (possibly modified) event data.
    Send(EventData),
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The catcher type identifier sent in every `HawkEvent` envelope.
pub const CATCHER_TYPE: &str = "errors/rust";

/// SDK version string included in every event payload.
pub const CATCHER_VERSION: &str = "hawk-rust/0.1.0";
