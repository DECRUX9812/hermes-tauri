use std::time::Duration;

/// Delay before the first update check after startup.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
/// Interval between update checks after the first one.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdaterSettings {
    /// The updater stays inactive; the reason is shown on the settings page.
    Disabled { reason: String },
    Enabled {
        pubkey: String,
        endpoints: Vec<String>,
    },
}

/// Builds the updater settings from the signing key and feed endpoint the
/// build or user config provides. Either part missing, blank, or a feed that
/// is not `https://` disables the updater with a reason; this never panics.
pub fn settings(pubkey: Option<&str>, endpoint: Option<&str>) -> UpdaterSettings {
    let Some(pubkey) = present(pubkey) else {
        return UpdaterSettings::Disabled {
            reason: "no updater signing public key is configured".to_owned(),
        };
    };
    let Some(endpoint) = present(endpoint) else {
        return UpdaterSettings::Disabled {
            reason: "no update feed endpoint is configured".to_owned(),
        };
    };
    let endpoints: Vec<String> = endpoint
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect();
    if endpoints.is_empty() {
        return UpdaterSettings::Disabled {
            reason: "no update feed endpoint is configured".to_owned(),
        };
    }
    if let Some(not_https) = endpoints
        .iter()
        .find(|entry| !entry.starts_with("https://"))
    {
        return UpdaterSettings::Disabled {
            reason: format!("update feeds must use https://, but {not_https} does not"),
        };
    }
    UpdaterSettings::Enabled {
        pubkey: pubkey.to_owned(),
        endpoints,
    }
}

fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_when_either_part_is_missing_or_blank() {
        assert_eq!(
            settings(None, Some("https://example.com/feed.json")),
            UpdaterSettings::Disabled {
                reason: "no updater signing public key is configured".to_owned()
            }
        );
        assert_eq!(
            settings(Some("minisign-key"), None),
            UpdaterSettings::Disabled {
                reason: "no update feed endpoint is configured".to_owned()
            }
        );
        assert_eq!(
            settings(Some("  "), Some("https://example.com/feed.json")),
            settings(None, Some("https://example.com/feed.json"))
        );
    }

    #[test]
    fn enabled_with_trimmed_https_endpoints() {
        assert_eq!(
            settings(Some("key"), Some("https://example.com/feed.json")),
            UpdaterSettings::Enabled {
                pubkey: "key".to_owned(),
                endpoints: vec!["https://example.com/feed.json".to_owned()],
            }
        );
        assert_eq!(
            settings(
                Some("key"),
                Some(" https://a.example.com/feed.json , https://b.example.com/feed.json ")
            ),
            UpdaterSettings::Enabled {
                pubkey: "key".to_owned(),
                endpoints: vec![
                    "https://a.example.com/feed.json".to_owned(),
                    "https://b.example.com/feed.json".to_owned(),
                ],
            }
        );
    }

    #[test]
    fn non_https_feed_disables_the_updater() {
        assert_eq!(
            settings(Some("key"), Some("http://example.com/feed.json")),
            UpdaterSettings::Disabled {
                reason: "update feeds must use https://, but http://example.com/feed.json does not"
                    .to_owned()
            }
        );
        assert_eq!(
            settings(
                Some("key"),
                Some("https://a.example.com/f.json,http://b.example.com/f.json")
            ),
            UpdaterSettings::Disabled {
                reason: "update feeds must use https://, but http://b.example.com/f.json does not"
                    .to_owned()
            }
        );
        assert_eq!(
            settings(Some("key"), Some("  ,  ")),
            UpdaterSettings::Disabled {
                reason: "no update feed endpoint is configured".to_owned()
            }
        );
    }

    #[test]
    fn schedule_constants() {
        assert_eq!(FIRST_CHECK_DELAY, Duration::from_secs(30));
        assert_eq!(CHECK_INTERVAL, Duration::from_secs(6 * 60 * 60));
    }
}
