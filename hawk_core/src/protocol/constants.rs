/**
 * SDK-wide constants.
 *
 * These values are baked into every event envelope and identify
 * the catcher type and version to the Hawk backend.
 */

/// The catcher type identifier sent in every `HawkEvent` envelope.
/// Tells the backend which SDK family produced this event.
pub const CATCHER_TYPE: &str = "errors/rust";

/// SDK version string included in every event payload.
/// Derived at compile time from the `hawk_core` package version in `Cargo.toml`.
pub const CATCHER_VERSION: &str = concat!("hawk-rust/", env!("CARGO_PKG_VERSION"));
