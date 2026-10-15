use serde_json::to_string;

const ADAPTER: &str = include_str!("adapter.js");

pub fn script_for(backend_origin: Option<&str>) -> String {
    let origin = backend_origin
        .and_then(|origin| to_string(origin).ok())
        .unwrap_or_else(|| "null".to_owned());
    ADAPTER.replace("__HERMES_BACKEND_ORIGIN__", &origin)
}
