/**
 * Thread-safe context manager for the Hawk SDK.
 *
 * The `ContextManager` accumulates ambient state that gets attached to every
 * outgoing event:
 *
 * - **Tags** — string key-value pairs for indexing / filtering in the UI.
 * - **Extras** — arbitrary string key-value pairs for additional debugging info.
 * - **User** — the currently authenticated user.
 * - **Breadcrumbs** — a ring buffer of recent actions / log entries (max 50).
 *
 * All public methods acquire a `RwLock` and are safe to call from any thread.
 * The manager is wrapped in `Arc` so it can be shared between the public API
 * layer and the worker thread that serializes events.
 *
 * Context merging follows the Node.js catcher convention:
 * `Object.assign({}, globalContext, eventContext)` — a shallow merge where
 * per-event fields override global ones.
 */
use std::collections::{HashMap, VecDeque};
use std::sync::RwLock;

use crate::types::{Breadcrumb, User, MAX_BREADCRUMBS};

// ---------------------------------------------------------------------------
// ContextManager
// ---------------------------------------------------------------------------

/**
 * Holds all mutable ambient state that gets attached to every event.
 *
 * Internally protected by a `RwLock` so that reads (building context for an
 * event) and writes (user calling `set_tag`, `add_breadcrumb`, etc.) can
 * coexist with minimal contention.
 */
pub struct ContextManager {
    /// Internal mutable state guarded by a readers-writer lock.
    inner: RwLock<Inner>,
}

/**
 * The actual mutable state behind the `RwLock`.
 */
struct Inner {
    /// Indexed key-value tags (e.g. `"region" => "eu"`).
    tags: HashMap<String, String>,

    /// Free-form extra data (e.g. `"user_id" => "123"`).
    extras: HashMap<String, String>,

    /// The currently authenticated user, if any.
    user: Option<User>,

    /// Fixed-capacity ring buffer of breadcrumbs.
    /// When full, the oldest entry is evicted to make room.
    breadcrumbs: VecDeque<Breadcrumb>,

    /// Whether breadcrumb collection is enabled.
    /// When `false`, `add_breadcrumb` is a no-op.
    breadcrumbs_enabled: bool,
}

impl ContextManager {
    /**
     * Creates a new, empty `ContextManager`.
     *
     * # Arguments
     * * `breadcrumbs_enabled` — If `false`, calls to `add_breadcrumb` are
     *   silently ignored. This mirrors the Node.js `breadcrumbs: false` option.
     */
    pub fn new(breadcrumbs_enabled: bool) -> Self {
        Self {
            inner: RwLock::new(Inner {
                tags: HashMap::new(),
                extras: HashMap::new(),
                user: None,
                breadcrumbs: VecDeque::with_capacity(MAX_BREADCRUMBS),
                breadcrumbs_enabled,
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Tags
    // -----------------------------------------------------------------------

    /**
     * Sets a single tag. Overwrites any existing tag with the same key.
     *
     * Tags are string key-value pairs useful for filtering events in the
     * Hawk dashboard (e.g. `"region"`, `"deployment"`).
     *
     * # Arguments
     * * `key` — The tag name.
     * * `value` — The tag value.
     */
    pub fn set_tag(&self, key: impl Into<String>, value: impl Into<String>) {
        if let Ok(mut inner) = self.inner.write() {
            inner.tags.insert(key.into(), value.into());
        }
    }

    // -----------------------------------------------------------------------
    // Extras
    // -----------------------------------------------------------------------

    /**
     * Sets a single extra. Overwrites any existing extra with the same key.
     *
     * Extras are free-form string key-value pairs that provide additional
     * context for debugging (e.g. `"user_id"`, `"request_path"`).
     *
     * # Arguments
     * * `key` — The extra key.
     * * `value` — The extra value.
     */
    pub fn set_extra(&self, key: impl Into<String>, value: impl Into<String>) {
        if let Ok(mut inner) = self.inner.write() {
            inner.extras.insert(key.into(), value.into());
        }
    }

    // -----------------------------------------------------------------------
    // User
    // -----------------------------------------------------------------------

    /**
     * Sets the current user. Replaces any previously set user.
     *
     * The user is attached to every subsequent event until changed or cleared.
     *
     * # Arguments
     * * `user` — The affected user to attach to events.
     */
    pub fn set_user(&self, user: User) {
        if let Ok(mut inner) = self.inner.write() {
            inner.user = Some(user);
        }
    }

    /**
     * Returns a clone of the currently set user, if any.
     */
    pub fn get_user(&self) -> Option<User> {
        self.inner.read().ok().and_then(|inner| inner.user.clone())
    }

    // -----------------------------------------------------------------------
    // Breadcrumbs
    // -----------------------------------------------------------------------

    /**
     * Appends a breadcrumb to the ring buffer.
     *
     * If breadcrumbs are disabled (via `Options.disable_breadcrumbs`), this
     * is a no-op.
     *
     * If the buffer is full (50 entries), the oldest breadcrumb is evicted
     * before the new one is appended.
     *
     * # Arguments
     * * `breadcrumb` — The breadcrumb entry to record.
     */
    pub fn add_breadcrumb(&self, breadcrumb: Breadcrumb) {
        if let Ok(mut inner) = self.inner.write() {
            if !inner.breadcrumbs_enabled {
                return;
            }
            if inner.breadcrumbs.len() >= MAX_BREADCRUMBS {
                inner.breadcrumbs.pop_front();
            }
            inner.breadcrumbs.push_back(breadcrumb);
        }
    }

    /**
     * Takes all current breadcrumbs, leaving the buffer empty.
     *
     * Returns `None` if the buffer is empty or breadcrumbs are disabled,
     * matching the Node.js convention of sending `null` instead of `[]`.
     */
    pub fn take_breadcrumbs(&self) -> Option<Vec<Breadcrumb>> {
        if let Ok(mut inner) = self.inner.write() {
            if inner.breadcrumbs.is_empty() {
                return None;
            }
            let crumbs: Vec<Breadcrumb> = inner.breadcrumbs.drain(..).collect();
            Some(crumbs)
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // Context building
    // -----------------------------------------------------------------------

    /**
     * Builds the merged context JSON value.
     *
     * The context is a shallow merge following the same semantics as the
     * Node.js catcher (`Object.assign({}, globalContext, eventContext)`):
     *
     * 1. Start with global tags and extras.
     * 2. If `event_context` is provided, its top-level keys override.
     *
     * The resulting JSON object has the shape:
     * ```json
     * {
     *   "tags": { "region": "eu" },
     *   "extras": { "user_id": "123" },
     *   ...event_context_keys
     * }
     * ```
     *
     * Returns `None` if there is absolutely no context data to send.
     *
     * # Arguments
     * * `event_context` — Optional per-event context that overrides globals.
     */
    pub fn build_context(
        &self,
        event_context: Option<&serde_json::Value>,
    ) -> Option<serde_json::Value> {
        let inner = match self.inner.read() {
            Ok(guard) => guard,
            Err(_) => return event_context.cloned(),
        };

        let mut ctx = serde_json::Map::new();

        /*
         * Add global tags if any exist.
         */
        if !inner.tags.is_empty() {
            let tags_val = serde_json::to_value(&inner.tags).unwrap_or_default();
            ctx.insert("tags".into(), tags_val);
        }

        /*
         * Add global extras if any exist.
         */
        if !inner.extras.is_empty() {
            let extras_val = serde_json::to_value(&inner.extras).unwrap_or_default();
            ctx.insert("extras".into(), extras_val);
        }

        /*
         * Merge per-event context on top (shallow — top-level keys override).
         * This mirrors `Object.assign(contextMerged, context)` in Node.js.
         */
        if let Some(serde_json::Value::Object(event_map)) = event_context {
            for (k, v) in event_map {
                ctx.insert(k.clone(), v.clone());
            }
        }

        if ctx.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(ctx))
        }
    }
}
