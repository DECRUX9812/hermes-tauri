use hermes_sidecar::redact;

/// The custom scheme, distinct from the Electron shell's `hermes://`.
const PREFIX: &str = "hermes-tauri://";

/// The screen a deep link opens, as a hash fragment on the backend origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Home,
    Team,
    Kanban,
    Studio,
    Settings,
}

impl Route {
    /// The hash fragment the shell appends to the backend origin (`#/team`).
    pub fn hash(&self) -> &'static str {
        match self {
            Route::Home => "#/",
            Route::Team => "#/team",
            Route::Kanban => "#/kanban",
            Route::Studio => "#/studio",
            Route::Settings => "#/settings",
        }
    }

    fn from_path(path: &str) -> Option<Self> {
        match path {
            "/" => Some(Route::Home),
            "/team" => Some(Route::Team),
            "/kanban" => Some(Route::Kanban),
            "/studio" => Some(Route::Studio),
            "/settings" => Some(Route::Settings),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeepLinkError {
    #[error("Not a hermes-tauri:// deep link (starts with {0:?})")]
    WrongScheme(String),
    #[error("Unknown deep-link action {0:?} (expected \"open\")")]
    WrongAction(String),
    #[error("Deep link is missing the path query parameter")]
    MissingPath,
    #[error("Unknown deep-link route {0:?}")]
    UnknownRoute(String),
    #[error("Unexpected deep-link query parameter {0:?}")]
    ExtraQueryKey(String),
    #[error("Deep link must not carry a fragment")]
    Fragment,
    #[error("Deep-link path is not a safe route (traversal, backslash or control characters)")]
    UnsafePath,
    #[error("Deep-link path is not correctly percent-encoded")]
    BadEncoding,
}

/// Parses a deep link of exactly the form `hermes-tauri://open?path=<route>`.
///
/// The path is percent-decoded once and must then be one of the known routes,
/// so `%2e%2e`, double encoding, backslashes and control characters are all
/// rejected instead of ever reaching navigation.
pub fn parse(url: &str) -> Result<Route, DeepLinkError> {
    let rest = scheme_suffix(url)?;
    if rest.contains('#') {
        return Err(DeepLinkError::Fragment);
    }
    let (action, query) = match rest.split_once('?') {
        Some((action, query)) => (action, Some(query)),
        None => (rest, None),
    };
    if !action.eq_ignore_ascii_case("open") {
        return Err(DeepLinkError::WrongAction(action.to_owned()));
    }
    let query = query.ok_or(DeepLinkError::MissingPath)?;
    let raw_path = query_path(query)?;
    let decoded = percent_decode(raw_path)?;
    if decoded.is_empty() {
        return Err(DeepLinkError::MissingPath);
    }
    if decoded.contains('%') {
        // Something like %252e: decoding once was not enough.
        return Err(DeepLinkError::BadEncoding);
    }
    if decoded.contains("..") || decoded.contains('\\') || decoded.chars().any(char::is_control) {
        return Err(DeepLinkError::UnsafePath);
    }
    Route::from_path(&decoded).ok_or_else(|| DeepLinkError::UnknownRoute(redact(&decoded)))
}

/// The first argument that carries the deep-link scheme, as a second instance
/// receives it on Linux and Windows.
pub fn from_args(args: &[String]) -> Option<&str> {
    args.iter()
        .find(|arg| has_scheme_prefix(arg))
        .map(String::as_str)
}

fn has_scheme_prefix(text: &str) -> bool {
    let prefix = PREFIX.as_bytes();
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn scheme_suffix(url: &str) -> Result<&str, DeepLinkError> {
    if has_scheme_prefix(url) {
        // The prefix is ASCII, so this slice is on a char boundary.
        Ok(&url[PREFIX.len()..])
    } else {
        let head: String = url
            .chars()
            .take_while(|c| !matches!(c, ':' | '/' | '?' | '#'))
            .take(32)
            .collect();
        Err(DeepLinkError::WrongScheme(head))
    }
}

/// Reads exactly one `path` key out of the query string.
fn query_path(query: &str) -> Result<&str, DeepLinkError> {
    let mut path: Option<&str> = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| DeepLinkError::ExtraQueryKey(redact(pair)))?;
        if key != "path" {
            return Err(DeepLinkError::ExtraQueryKey(redact(key)));
        }
        if path.is_some() {
            return Err(DeepLinkError::ExtraQueryKey("path".to_owned()));
        }
        path = Some(value);
    }
    path.ok_or(DeepLinkError::MissingPath)
}

/// Strict single-pass percent-decoding: a stray `%` that is not a valid
/// escape is an error, not literal text.
fn percent_decode(text: &str) -> Result<String, DeepLinkError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = bytes
                .get(index + 1)
                .and_then(|b| hex_digit(*b))
                .ok_or(DeepLinkError::BadEncoding)?;
            let low = bytes
                .get(index + 2)
                .and_then(|b| hex_digit(*b))
                .ok_or(DeepLinkError::BadEncoding)?;
            out.push(high * 16 + low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|_| DeepLinkError::BadEncoding)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_documented_form() {
        assert_eq!(parse("hermes-tauri://open?path=%2F"), Ok(Route::Home));
        assert_eq!(parse("hermes-tauri://open?path=/"), Ok(Route::Home));
        assert_eq!(parse("hermes-tauri://open?path=%2Fteam"), Ok(Route::Team));
        assert_eq!(
            parse("hermes-tauri://open?path=%2Fkanban"),
            Ok(Route::Kanban)
        );
        assert_eq!(
            parse("hermes-tauri://open?path=%2Fstudio"),
            Ok(Route::Studio)
        );
        assert_eq!(
            parse("hermes-tauri://open?path=%2Fsettings"),
            Ok(Route::Settings)
        );
        // Schemes are case-insensitive; hosts are compared loosely too.
        assert_eq!(parse("HERMES-TAURI://open?path=%2Fteam"), Ok(Route::Team));
        assert_eq!(parse("hermes-tauri://Open?path=%2Fteam"), Ok(Route::Team));
    }

    #[test]
    fn hash_fragments_match_the_routes() {
        assert_eq!(Route::Home.hash(), "#/");
        assert_eq!(Route::Team.hash(), "#/team");
        assert_eq!(Route::Kanban.hash(), "#/kanban");
        assert_eq!(Route::Studio.hash(), "#/studio");
        assert_eq!(Route::Settings.hash(), "#/settings");
    }

    #[test]
    fn from_args_finds_the_first_deep_link() {
        let args = vec![
            "--some-flag".to_owned(),
            "HERMES-TAURI://open?path=%2Fteam".to_owned(),
            "hermes-tauri://open?path=%2Fstudio".to_owned(),
        ];
        assert_eq!(from_args(&args), Some("HERMES-TAURI://open?path=%2Fteam"));
        assert_eq!(from_args(&[]), None);
        assert_eq!(from_args(&["--url".to_owned()]), None);
    }
}
