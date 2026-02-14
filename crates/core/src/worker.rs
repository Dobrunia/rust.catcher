/**
 * Background worker thread that drains the event queue and sends events
 * to the Hawk collector.
 *
 * Architecture overview:
 *
 * ```text
 *  ┌─────────────┐     bounded channel     ┌────────────────┐
 *  │  User code   │ ───── WorkerMsg ──────► │  Worker thread  │
 *  │  (any thread)│                         │  (single)       │
 *  └─────────────┘                         └───────┬────────┘
 *                                                  │
 *                                           Transport::send()
 *                                                  │
 *                                           ┌──────▼──────┐
 *                                           │  Collector   │
 *                                           └─────────────┘
 * ```
 *
 * The channel carries `WorkerMsg` variants:
 * - `Event(HawkEvent)` — a serialized event ready to be POSTed.
 * - `Flush(Arc<FlushSignal>)` — a signal requesting the worker to notify
 *   the caller once all preceding events have been drained.
 *
 * The worker loop runs until the channel disconnects (i.e., all senders
 * are dropped), which happens when the `Client` is dropped.
 */
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use crossbeam_channel::Receiver;

use crate::transport::Transport;
use crate::types::HawkEvent;

// ---------------------------------------------------------------------------
// WorkerMsg — the messages sent through the bounded channel
// ---------------------------------------------------------------------------

/**
 * Messages that flow through the bounded channel from producer threads
 * to the single background worker.
 */
pub enum WorkerMsg {
    /**
     * A fully assembled `HawkEvent` envelope ready to be serialized and
     * POSTed to the collector.
     */
    Event(HawkEvent),

    /**
     * A flush request. The worker signals `FlushSignal` once all messages
     * that were in the channel *before* this `Flush` message have been
     * processed.
     */
    Flush(Arc<FlushSignal>),
}

// ---------------------------------------------------------------------------
// FlushSignal — condvar-based notification for flush completion
// ---------------------------------------------------------------------------

/**
 * Synchronization primitive used to block the caller of `flush()` until
 * the worker has drained all pending messages.
 *
 * Uses a `Mutex<bool>` + `Condvar` pair:
 * - The bool starts as `false` (not yet flushed).
 * - The worker sets it to `true` and notifies when it processes the
 *   `Flush` message.
 * - The caller waits on the condvar with a timeout.
 */
pub struct FlushSignal {
    /// Guard protecting the "done" flag.
    mutex: Mutex<bool>,

    /// Condition variable the caller waits on.
    condvar: Condvar,
}

impl FlushSignal {
    /**
     * Creates a new `FlushSignal` in the "not yet flushed" state.
     */
    pub fn new() -> Self {
        Self {
            mutex: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    /**
     * Called by the worker thread to indicate that the flush is complete.
     * Wakes up anyone waiting in `wait_timeout`.
     */
    pub fn notify(&self) {
        if let Ok(mut done) = self.mutex.lock() {
            *done = true;
            self.condvar.notify_all();
        }
    }

    /**
     * Blocks the calling thread until the worker signals completion,
     * or until `timeout` elapses — whichever comes first.
     *
     * # Arguments
     * * `timeout` — Maximum duration to wait.
     *
     * # Returns
     * `true` if the flush completed in time, `false` if the timeout expired.
     */
    pub fn wait_timeout(&self, timeout: std::time::Duration) -> bool {
        if let Ok(guard) = self.mutex.lock() {
            /*
             * Loop to handle spurious wakeups — standard condvar pattern.
             * The `wait_timeout` on the condvar returns a `(guard, WaitTimeoutResult)`.
             */
            let result = self
                .condvar
                .wait_timeout_while(guard, timeout, |done| !*done);

            match result {
                Ok((_, timeout_result)) => !timeout_result.timed_out(),
                Err(_) => false, /* poisoned mutex — treat as timeout */
            }
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Worker — the background thread
// ---------------------------------------------------------------------------

/**
 * Handle to the background worker thread.
 *
 * The worker is spawned during `Client::new()` and runs until the channel
 * disconnects (all senders dropped). It processes messages sequentially:
 * - `Event` → serialize + HTTP POST via `Transport`.
 * - `Flush` → signal the requester that all prior events are drained.
 */
pub struct Worker;

impl Worker {
    /**
     * Spawns the background worker thread.
     *
     * The thread runs until the channel disconnects (all senders dropped).
     * It is fire-and-forget — no join handle is stored because the
     * `Guard::drop()` → `flush()` path ensures all events are drained
     * before the process exits.
     *
     * # Arguments
     * * `receiver` — The receiving end of the bounded channel.
     * * `endpoint` — The collector URL to POST events to.
     * * `transport` — The HTTP transport used for sending.
     */
    pub fn spawn(receiver: Receiver<WorkerMsg>, endpoint: String, transport: Transport) {
        thread::Builder::new()
            .name("hawk-worker".into())
            .spawn(move || {
                Self::run_loop(&receiver, &endpoint, &transport);
            })
            .expect("[Hawk] Failed to spawn worker thread");
    }

    /**
     * The main event loop of the worker thread.
     *
     * Blocks on `receiver.recv()` waiting for the next message.
     * When the channel disconnects (all senders dropped), `recv()` returns
     * `Err(RecvError)` and the loop exits cleanly.
     *
     * # Arguments
     * * `receiver` — The bounded channel receiver.
     * * `endpoint` — The collector URL.
     * * `transport` — The HTTP transport.
     */
    fn run_loop(receiver: &Receiver<WorkerMsg>, endpoint: &str, transport: &Transport) {
        /*
         * Block on each incoming message. The loop exits when all senders
         * have been dropped and the channel is empty.
         */
        while let Ok(msg) = receiver.recv() {
            match msg {
                WorkerMsg::Event(event) => {
                    /*
                     * Send the event via HTTP. This is best-effort:
                     * Transport::send() logs errors internally and never panics.
                     */
                    transport.send(endpoint, &event);
                }
                WorkerMsg::Flush(signal) => {
                    /*
                     * All messages before this Flush have already been processed
                     * (channel is FIFO). Notify the waiter that flush is complete.
                     */
                    signal.notify();
                }
            }
        }

        /*
         * Channel disconnected — all senders dropped. Worker exits cleanly.
         * This is the normal shutdown path triggered by Guard::drop().
         */
    }

}
