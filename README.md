# hawk.rust

Hawk error tracking SDK for Rust.

```
hawk.rust/
├── hawk_core/      # engine: transport, queue, worker
├── hawk_panic/     # addon: panic hook
├── hawk/           # facade: user-facing API
├── examples/basic/ # usage example
└── Cargo.toml      # workspace
```

## Feature checklist

Based on the [Hawk Catcher specification](https://docs.hawk.so).

| # | Feature | Status | Notes |
|---|---------|--------|-------|
| 1 | Bind global error handler | ✅ | `hawk_panic` — auto-installed via `catch_panics` option |
| 2 | Send errors using universal Event Format | ✅ | `HawkEvent { token, catcherType, payload }` |
| 3 | Collect and send code fragments for Stacktrace | ❌ | Rust binaries don't ship source; needs debug info / source map support |
| 4 | Allow to send events manually | ✅ | `hawk::send(msg)`, `hawk::capture_event(event)` |
| 5 | Free-format context object (global + per-event, merged) | ❌ | Planned for next iteration |
| 6 | User object (authenticated user / generated ID) | ❌ | Planned for next iteration |
| 7 | Language-specific addons field | ❌ | Planned |
| 8 | Extract runtime variable values from Stacktrace | ❌ | Limited in compiled languages without a debugger |
| 9 | Send own version with event | ✅ | `catcherVersion: "hawk-rust/0.1.0"` via `CARGO_PKG_VERSION` |
| 10 | Suspected Commits via git | ❌ | Planned |
| 11 | Error levels (Fatal, Warning, etc.) | ❌ | `type` field exists but levels not formalized yet |
| 12 | Integration with popular loggers | ❌ | `tracing` / `log` crate integration planned |
