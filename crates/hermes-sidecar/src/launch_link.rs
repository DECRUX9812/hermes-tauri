const SESSION_FRAGMENT: &str = "#hermes-session=";
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
    let (_, token) = candidate.split_once(SESSION_FRAGMENT)?;
    if token.len() != TOKEN_LEN || !token.bytes().all(is_token_byte) {
        return None;
    }
    Some(LaunchLink {
        url: candidate.to_owned(),
        origin: origin_of(candidate)?,
    })
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

/// Replaces the secret part of any `#hermes-session=` token with `<redacted>`.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find(SESSION_FRAGMENT) {
        let (head, tail) = rest.split_at(position + SESSION_FRAGMENT.len());
        out.push_str(head);
        out.push_str(REDACTED);
        let secret_len = tail.bytes().take_while(|b| is_token_byte(*b)).count();
        rest = &tail[secret_len..];
    }
    out.push_str(rest);
    out
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
}
