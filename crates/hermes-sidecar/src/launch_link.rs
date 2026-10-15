const SESSION_FRAGMENT: &str = "#hermes-session=";
const FRONT_DOOR_QUERY: &str = "?token=";
const TOKEN_LEN: usize = 43;
const REDACTED: &str = "<redacted>";

#[derive(Debug, Clone)]
pub(crate) struct LaunchLink {
    pub url: String,
    pub origin: String,
}

pub(crate) fn parse_launch_link(text: &str) -> Option<LaunchLink> {
    let start = ["http://", "https://"]
        .iter()
        .filter_map(|scheme| text.find(scheme))
        .min()?;
    let candidate = text[start..].split_whitespace().next()?;
    launch_token(candidate)?;
    Some(LaunchLink {
        url: candidate.to_owned(),
        origin: origin_of(candidate)?,
    })
}

/// The session token of a candidate launch URL, in either shape a backend
/// prints: the webapp's `#hermes-session=<token>` fragment, or the bundled
/// `hermes_plus` front door's `?token=<token>` query (docs/python-sidecar.md §1).
fn launch_token(candidate: &str) -> Option<&str> {
    let token = match candidate.split_once(SESSION_FRAGMENT) {
        Some((_, token)) => token,
        None => {
            let (_, rest) = candidate.split_once(FRONT_DOOR_QUERY)?;
            rest.split(['&', '#']).next()?
        }
    };
    (token.len() == TOKEN_LEN && token.bytes().all(is_token_byte)).then_some(token)
}

pub(crate) fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let location = rest.split(['?', '#']).next()?.trim_end_matches('/');
    let authority = location.split('/').next()?;
    if authority.is_empty() {
        return None;
    }
    Some(format!("{}://{location}", scheme.to_ascii_lowercase()))
}

/// True when `text` is a usable attach URL: a full launch link carrying a
/// session token, or a bare http/https origin.
///
/// Text that looks like a launch link but carries a malformed token is
/// rejected rather than silently demoted to an origin (which would lose the
/// session token).
pub fn is_attach_url(text: &str) -> bool {
    if text.contains(SESSION_FRAGMENT) {
        parse_launch_link(text).is_some()
    } else {
        origin_of(text).is_some()
    }
}

/// Replaces the secret part of any `#hermes-session=` token or `?token=`
/// front-door token with `<redacted>`.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((at, marker)) = earliest_marker(rest) {
        let secret_start = at + marker.len();
        let secret_len = rest[secret_start..]
            .bytes()
            .take_while(|b| is_token_byte(*b))
            .count();
        out.push_str(&rest[..secret_start]);
        out.push_str(REDACTED);
        rest = &rest[secret_start + secret_len..];
    }
    out.push_str(rest);
    out
}

/// The first secret marker in `text`: both markers can appear in one line.
fn earliest_marker(text: &str) -> Option<(usize, &'static str)> {
    [SESSION_FRAGMENT, FRONT_DOOR_QUERY]
        .into_iter()
        .filter_map(|marker| text.find(marker).map(|at| (at, marker)))
        .min_by_key(|(at, _)| *at)
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_launch_link_and_bare_origin() {
        let link = format!("http://127.0.0.1:18888/#hermes-session={}", "a".repeat(43));
        assert!(is_attach_url(&link));
        assert!(is_attach_url("http://127.0.0.1:9119"));
        assert!(is_attach_url("https://hermes.example.com"));
    }

    #[test]
    fn rejects_malformed_links_and_non_urls() {
        // A malformed token must not fall back to the origin.
        let bad_token = "http://127.0.0.1:18888/#hermes-session=short";
        assert!(!is_attach_url(bad_token));
        assert!(!is_attach_url("ftp://127.0.0.1:9119"));
        assert!(!is_attach_url("127.0.0.1:9119"));
        assert!(!is_attach_url(""));
    }

    /// The bundled payload's front door prints this line on stdout
    /// (`hermes_plus/server/runtime.py`, `_log_login_url`).
    #[test]
    fn accepts_the_front_door_login_line() {
        let token = "b".repeat(TOKEN_LEN);
        let line = format!("Hermes Plus login: http://127.0.0.1:18801/?token={token}");
        let link = parse_launch_link(&line).expect("front-door login line is a launch link");
        assert_eq!(link.url, format!("http://127.0.0.1:18801/?token={token}"));
        assert_eq!(link.origin, "http://127.0.0.1:18801");

        // The token length is checked here too, so a truncated line is ignored.
        let short = "Hermes Plus login: http://127.0.0.1:18801/?token=short";
        assert!(parse_launch_link(short).is_none());
        // The front-door URL is still usable as an attach URL: `connect` keeps
        // the text it is given, so the token survives.
        assert!(is_attach_url(&format!(
            "http://127.0.0.1:18801/?token={token}"
        )));
    }

    #[test]
    fn redact_masks_both_token_shapes() {
        let webapp = format!(
            "ready at http://127.0.0.1:18888/#hermes-session={}",
            "a".repeat(TOKEN_LEN)
        );
        assert_eq!(
            redact(&webapp),
            "ready at http://127.0.0.1:18888/#hermes-session=<redacted>"
        );
        let front_door = format!(
            "Hermes Plus login: http://127.0.0.1:18801/?token={}",
            "b".repeat(TOKEN_LEN)
        );
        assert_eq!(
            redact(&front_door),
            "Hermes Plus login: http://127.0.0.1:18801/?token=<redacted>"
        );
        // A line carrying both is fully masked.
        let both = format!("{webapp} | {front_door}");
        assert!(!redact(&both).contains("aaaa"));
        assert!(!redact(&both).contains("bbbb"));
        // Nothing else is touched.
        assert_eq!(redact("plain text, no token"), "plain text, no token");
    }
}
