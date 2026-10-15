//! Rejection cases for deep-link parsing. Every case must come back with its
//! typed reason; none may ever produce a navigation.

use hermes_shell_config::{parse, DeepLinkError};

fn parse_err(url: &str) -> DeepLinkError {
    parse(url).expect_err("expected a rejection")
}

#[test]
fn rejects_wrong_schemes() {
    for url in [
        "https://open?path=%2Fteam",
        "http://open?path=%2Fteam",
        "hermes://open?path=%2Fteam",
        "open?path=%2Fteam",
        "",
        "hermes-tauri:open?path=%2Fteam",
    ] {
        assert!(
            matches!(parse_err(url), DeepLinkError::WrongScheme(_)),
            "expected WrongScheme for {url:?}"
        );
    }
}

#[test]
fn rejects_wrong_actions() {
    for url in [
        "hermes-tauri://start?path=%2Fteam",
        "hermes-tauri://close?path=%2Fteam",
        "hermes-tauri://open/team?path=%2Fteam",
        "hermes-tauri://open@evil?path=%2Fteam",
        "hermes-tauri://open:8080?path=%2Fteam",
    ] {
        assert!(
            matches!(parse_err(url), DeepLinkError::WrongAction(_)),
            "expected WrongAction for {url:?}"
        );
    }
}

#[test]
fn rejects_missing_paths() {
    for url in [
        "hermes-tauri://open",
        "hermes-tauri://open?path=",
        "hermes-tauri://open?",
    ] {
        assert!(
            matches!(parse_err(url), DeepLinkError::MissingPath),
            "expected MissingPath for {url:?}"
        );
    }
}

#[test]
fn rejects_unknown_routes() {
    for url in [
        "hermes-tauri://open?path=%2Fadmin",
        "hermes-tauri://open?path=%2Fteam%2F",
        "hermes-tauri://open?path=team",
        "hermes-tauri://open?path=%2Fsettings%2Fx",
    ] {
        assert!(
            matches!(parse_err(url), DeepLinkError::UnknownRoute(_)),
            "expected UnknownRoute for {url:?}"
        );
    }
}

#[test]
fn rejects_extra_or_repeated_query_keys() {
    assert_eq!(
        parse_err("hermes-tauri://open?path=%2Fteam&x=1"),
        DeepLinkError::ExtraQueryKey("x".to_owned())
    );
    assert_eq!(
        parse_err("hermes-tauri://open?path=%2Fteam&path=%2Fkanban"),
        DeepLinkError::ExtraQueryKey("path".to_owned())
    );
    assert!(matches!(
        parse_err("hermes-tauri://open?path&x=1"),
        DeepLinkError::ExtraQueryKey(_)
    ));
    // An extra key sitting where the path key should be.
    assert_eq!(
        parse_err("hermes-tauri://open?other=1"),
        DeepLinkError::ExtraQueryKey("other".to_owned())
    );
}

#[test]
fn rejects_fragments() {
    for url in [
        "hermes-tauri://open?path=%2Fteam#fragment",
        "hermes-tauri://open#team",
        "hermes-tauri://open?path=%2Fteam#",
    ] {
        assert_eq!(
            parse_err(url),
            DeepLinkError::Fragment,
            "expected Fragment for {url:?}"
        );
    }
}

#[test]
fn rejects_traversal_and_encoded_tricks() {
    for url in [
        // Raw ..
        "hermes-tauri://open?path=%2F..%2Fetc",
        // Encoded .. (%2e%2e)
        "hermes-tauri://open?path=%2e%2e",
        "hermes-tauri://open?path=%2F%2e%2e%2Fx",
        // Double encoding (%252e decodes once to %2e)
        "hermes-tauri://open?path=%252e%252e",
        "hermes-tauri://open?path=%252Fteam",
        // Backslashes, raw and encoded
        "hermes-tauri://open?path=%2F%5C%2Fteam",
        "hermes-tauri://open?path=%2F%00",
        "hermes-tauri://open?path=%2Fteam%0A",
        // Control characters
        "hermes-tauri://open?path=%09",
    ] {
        let error = parse_err(url);
        assert!(
            matches!(
                error,
                DeepLinkError::UnsafePath | DeepLinkError::BadEncoding
            ),
            "expected UnsafePath or BadEncoding for {url:?}, got {error:?}"
        );
    }
}

#[test]
fn rejects_broken_percent_encoding() {
    for url in [
        "hermes-tauri://open?path=%zz",
        "hermes-tauri://open?path=%2",
        "hermes-tauri://open?path=%",
        "hermes-tauri://open?path=100%",
    ] {
        assert_eq!(
            parse_err(url),
            DeepLinkError::BadEncoding,
            "expected BadEncoding for {url:?}"
        );
    }
}

#[test]
fn unknown_route_messages_hide_session_tokens() {
    // A crafted deep link must not leak a pasted launch-link token into the
    // error shown on the boot page.
    let token = "t".repeat(43);
    let url = format!("hermes-tauri://open?path=%2Fx%23hermes-session%3D{token}");
    let error = parse_err(&url);
    assert!(matches!(error, DeepLinkError::UnknownRoute(_)));
    assert!(!format!("{error}").contains(&token));
    assert!(!format!("{error:?}").contains(&token));
}
