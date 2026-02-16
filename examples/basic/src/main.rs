/**
 * Minimal test harness for the Hawk Rust SDK.
 *
 * Replace the TOKEN constant with a real base64-encoded integration token
 * from your Hawk project settings, then run:
 *
 *   cargo run -p hawk_example
 *   cargo run -p hawk_example -- --panic        # test panic capture
 *   cargo run -p hawk_example -- --before-send  # test before_send filter
 */
use std::sync::Arc;

/// Paste your integration token here.
const TOKEN: &str = "PASTE_YOUR_TOKEN_HERE";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let test_panic = args.iter().any(|a| a == "--panic");
    let test_before_send = args.iter().any(|a| a == "--before-send");

    /*
     * Initialize the SDK.
     * If --before-send is passed, attach a callback that prefixes every title.
     */
    let _guard = if test_before_send {
        println!("[example] Initializing with before_send filter");
        hawk::init(hawk::Options {
            token: TOKEN.into(),
            before_send: Some(Arc::new(|mut event| {
                event.title = format!("[filtered] {}", event.title);
                println!("[before_send] Modified title → {}", event.title);
                Some(event) // None here would drop the event
            })),
            ..Default::default()
        })
    } else {
        hawk::init(TOKEN)
    };

    /*
     * Send a plain text message.
     */
    hawk::send("Hello from Hawk Rust SDK!");
    println!("[example] Sent a text message");

    /*
     * Capture a real error (file not found).
     */
    match std::fs::read_to_string("/nonexistent/path.txt") {
        Ok(_) => unreachable!(),
        Err(e) => {
            hawk::send(&e);
            println!("[example] Sent an io::Error: {e}");
        }
    }

    /*
     * Test panic capture if requested.
     * The panic hook (catch_panics = true by default) will intercept this
     * and send it to Hawk before the process aborts.
     */
    if test_panic {
        println!("[example] Triggering a panic...");
        panic!("Test panic from Hawk example");
    }

    println!("[example] Done. Events will be flushed when _guard drops.");
}
