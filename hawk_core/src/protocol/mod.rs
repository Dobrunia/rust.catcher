/*!
 * Protocol layer — data structures, constants, and token handling.
 *
 * Everything related to *what* we send to the Hawk backend:
 * - `types` — HawkEvent envelope, EventData payload, BacktraceFrame
 * - `constants` — CATCHER_TYPE, CATCHER_VERSION
 * - `token` — base64 token decoding and endpoint derivation
 */

pub mod constants;
pub mod token;
pub mod types;
