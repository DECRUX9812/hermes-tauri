//! Runs against a real Hermes, so it is ignored by default.
//!
//! HERMES_SIDECAR_REAL_CMD is a JSON argv array, for example
//! `["hermes","webapp","--isolated","--host","127.0.0.1","--port","0","--skip-build"]`.
//! HERMES_SIDECAR_REAL_ENV is an optional JSON object of extra environment variables.
//!
//! cargo test -p hermes-sidecar --test real_hermes -- --ignored

use std::time::Duration;

use hermes_sidecar::{connect, ShutdownReport, SidecarConfig};

#[test]
#[ignore = "needs a real Hermes: set HERMES_SIDECAR_REAL_CMD"]
fn spawns_and_stops_a_real_hermes() {
    let command: Vec<String> = serde_json::from_str(
        &std::env::var("HERMES_SIDECAR_REAL_CMD").expect("HERMES_SIDECAR_REAL_CMD is set"),
    )
    .expect("HERMES_SIDECAR_REAL_CMD is a JSON array of strings");
    let env: Vec<(String, String)> = match std::env::var("HERMES_SIDECAR_REAL_ENV") {
        Ok(json) => {
            let object: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(&json).expect("HERMES_SIDECAR_REAL_ENV is a JSON object");
            object
                .into_iter()
                .map(|(key, value)| {
                    let value = value.as_str().expect("env values are strings").to_owned();
                    (key, value)
                })
                .collect()
        }
        Err(_) => Vec::new(),
    };
    let cfg = SidecarConfig {
        command,
        env,
        ready_timeout: Duration::from_secs(60),
        ..SidecarConfig::default()
    };

    let backend =
        connect(&cfg, |phase| eprintln!("phase: {phase:?}")).expect("real Hermes becomes ready");
    assert!(backend.is_spawned());
    let report = backend.shutdown().expect("shutdown");
    assert!(
        matches!(report, ShutdownReport::Terminated | ShutdownReport::Killed),
        "{report:?}"
    );
}
