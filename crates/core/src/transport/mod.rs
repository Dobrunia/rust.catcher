/**
 * Transport layer — HTTP delivery and background worker thread.
 *
 * Everything related to *how* we deliver events to the Hawk backend:
 * - `http` — reqwest-based HTTP client wrapper
 * - `worker` — background thread, bounded channel, flush signaling
 */

pub mod http;
pub mod worker;

pub use http::Transport;
pub use worker::{FlushSignal, Worker, WorkerMsg};
