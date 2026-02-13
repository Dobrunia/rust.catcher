/**
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
 * # Usage
 *
 * ```ignore
 * use hawk_core as hawk;
 *
 * fn main() {
 *     let _guard = hawk::init("TOKEN", Default::default()).unwrap();
 *     hawk_panic::install();
 *
 *     // panics are now automatically captured and sent to Hawk
 *     panic!("something went wrong");
 * }
 * ```
 *
 * # Recursion safety
 *
 * The hook uses a `thread_local` boolean flag to prevent infinite recursion
 * if `hawk_core::capture_event` itself were to panic (it shouldn't, but
 * defensive programming is paramount in error-handling code).
 *
 * # Thread safety
 *
 * `std::panic::set_hook` is process-global. The hook closure is
 * `Send + Sync` because it only uses thread-local state and the
 * thread-safe `hawk_core` API.
 */

use std::cell::Cell;
use std::panic;
use std::panic::PanicHookInfo;

use hawk_core::{BacktraceFrame, EventData, CATCHER_VERSION};

// ---------------------------------------------------------------------------
// Thread-local recursion guard
// ---------------------------------------------------------------------------

thread_local! {
    /**
     * Per-thread flag that prevents re-entrancy into the panic hook.
     *
     * If `hawk_core::capture_event()` were to somehow panic (it wraps all
     * internals defensively, but just in case), the hook would be called
     * again on the same thread. This flag breaks the recursion.
     *
     * Using `Cell<bool>` (not `RefCell`) because we only need simple
     * get/set — no borrowing.
     */
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Installs the Hawk panic hook.
 *
 * This replaces the current panic hook with one that:
 * 1. Captures the panic as a Hawk event.
 * 2. Forwards to the *previous* hook (preserving default behaviour).
 *
 * Safe to call multiple times — each call chains on top of the previous
 * hook. However, calling it once after `hawk::init()` is the intended usage.
 *
 * # Important
 * This must be called AFTER `hawk_core::init()` — otherwise the captured
 * events have nowhere to go (they'll be silently dropped, which is fine
 * but pointless).
 */
pub fn install() {
    /*
     * Take the existing hook so we can call it after our processing.
     * `std::panic::take_hook()` returns the current hook and resets to default.
     */
    let previous_hook = panic::take_hook();

    panic::set_hook(Box::new(move |info| {
        /*
         * Recursion guard: if we're already inside the hook on this thread,
         * skip our processing and just forward to the previous hook.
         */
        let is_recursive = IN_HOOK.with(|flag| {
            if flag.get() {
                true
            } else {
                flag.set(true);
                false
            }
        });

        if !is_recursive {
            /*
             * Wrap the entire event-building logic in catch_unwind so that
             * our hook never causes a double-panic / abort.
             */
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handle_panic(info);
            }));

            /*
             * Reset the recursion flag so that if another panic happens
             * later on this thread, the hook will work again.
             */
            IN_HOOK.with(|flag| flag.set(false));
        }

        /*
         * Always forward to the previous hook — this preserves the default
         * stderr output ("thread 'main' panicked at ...") and any other
         * user-installed hooks.
         */
        previous_hook(info);
    }));
}

// ---------------------------------------------------------------------------
// Internal: build and send the panic event
// ---------------------------------------------------------------------------

/**
 * Extracts information from the `PanicInfo` and sends it as a Hawk event.
 *
 * This function is called inside `catch_unwind` so it's safe to use
 * operations that might fail — they won't crash the host application.
 *
 * # Arguments
 * * `info` — The `PanicInfo` provided by the panic runtime.
 */
fn handle_panic(info: &PanicHookInfo) {
    /*
     * Step 1: Extract the panic message.
     *
     * The panic payload can be:
     * - `&str` (from `panic!("message")`)
     * - `String` (from `panic!("formatted {}", value)`)
     * - Something else entirely (rare — custom panic payloads)
     *
     * We try to extract a meaningful string; fall back to "<unknown panic>".
     */
    let message = get_panic_message(info);

    /*
     * Step 2: Extract source location (file, line, column).
     *
     * `PanicInfo::location()` returns `Some(Location)` in most cases.
     * It can be `None` if the panic was invoked via `resume_unwind` or
     * in unusual no_std environments.
     */
    let (file, line, column) = match info.location() {
        Some(loc) => (
            Some(loc.file().to_string()),
            Some(loc.line()),
            Some(loc.column()),
        ),
        None => (None, None, None),
    };

    /*
     * Step 3: Get the panicking thread name.
     * Unnamed threads get a fallback of "<unnamed>".
     */
    let thread_name = std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_string();

    /*
     * Step 4: Capture the backtrace at the panic site.
     * We use the `backtrace` crate because `std::backtrace::Backtrace`
     * doesn't expose structured frame data in stable Rust.
     */
    let bt = backtrace::Backtrace::new();
    let frames = convert_panic_backtrace(&bt);

    /*
     * Step 5: Build the context object with panic-specific metadata.
     * This extra info helps with debugging in the Hawk UI.
     */
    let mut context_map = serde_json::Map::new();
    if let Some(ref f) = file {
        context_map.insert("file".into(), serde_json::Value::String(f.clone()));
    }
    if let Some(l) = line {
        context_map.insert("line".into(), serde_json::Value::Number(l.into()));
    }
    if let Some(c) = column {
        context_map.insert("column".into(), serde_json::Value::Number(c.into()));
    }
    context_map.insert(
        "thread".into(),
        serde_json::Value::String(thread_name),
    );

    /*
     * Step 6: Build the event title.
     * Format: "panic: <message>" — matches the SPEC convention.
     */
    let title = format!("panic: {message}");

    /*
     * Step 7: Assemble the EventData and send it via hawk_core.
     */
    let event = EventData {
        title,
        event_type: Some("fatal".to_string()),
        backtrace: if frames.is_empty() {
            None
        } else {
            Some(frames)
        },
        release: None,     /* filled in by Client::send_event from options */
        user: None,        /* filled in by Client::send_event from context */
        context: Some(serde_json::Value::Object(context_map)),
        catcher_version: CATCHER_VERSION.to_string(),
    };

    hawk_core::capture_event(event);
}

/**
 * Extracts a human-readable message from the panic payload.
 *
 * Tries (in order):
 * 1. Downcast to `&str`
 * 2. Downcast to `String`
 * 3. Fall back to `"<unknown panic>"`
 *
 * # Arguments
 * * `info` — The `PanicInfo` from the panic runtime.
 *
 * # Returns
 * A `String` containing the panic message.
 */
fn get_panic_message(info: &PanicHookInfo) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "<unknown panic>".to_string()
    }
}

/**
 * Converts a `backtrace::Backtrace` into a `Vec<BacktraceFrame>` for
 * the Hawk event payload.
 *
 * This is similar to `hawk_core::convert_backtrace()` but intentionally
 * duplicated here to avoid a public dependency on that internal helper.
 * The panic crate only depends on `hawk_core`'s public API.
 *
 * Filters out frames with no useful debugging information (no function
 * name AND no file path).
 *
 * # Arguments
 * * `bt` — A captured backtrace (already resolved).
 */
fn convert_panic_backtrace(bt: &backtrace::Backtrace) -> Vec<BacktraceFrame> {
    let mut frames = Vec::new();

    for frame in bt.frames() {
        for symbol in frame.symbols() {
            let function = symbol.name().map(|n| n.to_string());
            let file = symbol.filename().map(|p| p.display().to_string());
            let line = symbol.lineno();

            /*
             * Skip frames with no useful information — internal runtime
             * frames, linker trampolines, etc.
             */
            if function.is_none() && file.is_none() {
                continue;
            }

            frames.push(BacktraceFrame {
                file,
                line,
                column: symbol.colno(),
                function,
            });
        }
    }

    frames
}
