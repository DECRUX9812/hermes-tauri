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

pub(crate) fn redact(text: &str) -> String {
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
