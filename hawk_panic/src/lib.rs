/*!
 * Hawk Panic Hook — automatic panic capture for the Hawk SDK.
 *
 * This crate provides a single function `install()` that registers a
 * custom `std::panic::set_hook` handler. When a panic occurs, it:
 *
 * 1. Extracts the panic message, source location, and thread name.
 * 2. Captures a backtrace at the panic site.
 * 3. Builds an `EventData` with `type = "fatal"` and sends it via
 *    `hawk_core::capture_event()`.
 * 4. Calls the previous panic hook (so the default stderr output is preserved).
 *
 * # Recursion safety
 *
 * The hook uses a `thread_local` boolean flag to prevent infinite recursion
 * if `hawk_core::capture_event` itself were to panic.
 */

use std::cell::Cell;
use std::panic;
use std::panic::PanicHookInfo;
use std::sync::atomic::{AtomicBool, Ordering};

use hawk_core::{EventData, CATCHER_VERSION};

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

/// Ensures `install()` is idempotent — calling it multiple times
/// won't stack hooks and produce duplicate events per panic.
static INSTALLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /**
     * Per-thread flag that prevents re-entrancy into the panic hook.
     * Breaks recursion if `hawk_core::capture_event` itself panics.
     */
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Installs the Hawk panic hook.
 *
 * Replaces the current panic hook with one that:
 * 1. Captures the panic as a Hawk event.
 * 2. Forwards to the *previous* hook (preserving default behaviour).
 *
 * Idempotent — subsequent calls are silent no-ops.
 *
 * Must be called AFTER `hawk_core::init()` — otherwise captured events
 * have nowhere to go.
 */
pub fn install() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let previous_hook = panic::take_hook();

    panic::set_hook(Box::new(move |info| {
        let is_recursive = IN_HOOK.with(|flag| {
            if flag.get() {
                true
            } else {
                flag.set(true);
                false
            }
        });

        if !is_recursive {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handle_panic(info);
            }));

            IN_HOOK.with(|flag| flag.set(false));
        }

        previous_hook(info);
    }));
}

// ---------------------------------------------------------------------------
// Internal: build and send the panic event
// ---------------------------------------------------------------------------

fn handle_panic(info: &PanicHookInfo) {
    let message = match info.payload().downcast_ref::<&str>() {
        Some(s) => (*s).to_string(),
        None => match info.payload().downcast_ref::<String>() {
            Some(s) => s.clone(),
            None => "<unknown panic>".to_string(),
        },
    };

    let (file, line) = match info.location() {
        Some(loc) => (Some(loc.file().to_string()), Some(loc.line())),
        None => (None, None),
    };

    let thread_name = std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_string();

    let bt = backtrace::Backtrace::new();
    let frames = hawk_core::convert_backtrace(&bt);

    let location_str = match (&file, line) {
        (Some(f), Some(l)) => format!(" at {f}:{l}"),
        _ => String::new(),
    };
    let title = format!("panic: {message}{location_str} [thread: {thread_name}]");

    let event = EventData {
        title,
        event_type: Some("fatal".to_string()),
        backtrace: if frames.is_empty() { None } else { Some(frames) },
        catcher_version: CATCHER_VERSION.to_string(),
    };

    hawk_core::capture_event(event);
}
