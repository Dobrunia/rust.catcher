/**
 * Minimal test harness for the Hawk Rust SDK.
 *
 * Replace the TOKEN constant with a real base64-encoded integration token
 * from your Hawk project settings, then run:
 *
 *   cargo run -p hawk_example
 */

/// Paste your integration token here.
const TOKEN: &str = "PASTE_YOUR_TOKEN_HERE";

fn main() {
    /*
     * Initialize the SDK — just the token, everything else is defaults.
     * Panic hook is installed automatically (catch_panics = true).
     * Returns a Guard that flushes pending events on drop.
     */
    let _guard = hawk::init(TOKEN);

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
     * Uncomment to test panic capture:
     */
    // panic!("Test panic from Hawk example");

    println!("[example] Done. Events will be flushed when _guard drops.");
}
