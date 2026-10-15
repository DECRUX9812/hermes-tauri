const ADAPTER: &str = include_str!("adapter.js");

/// The adapter is origin-independent: in SPAWN mode the backend origin is
/// unknown until connect resolves, so it runs on every http(s) page and the
/// runtime capability (scoped to the exact origin after connect) is the IPC
/// boundary. See adapter.js and docs/webkit-compat.md for the reasoning.
pub fn script() -> &'static str {
    ADAPTER
}
