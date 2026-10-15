mod common;

use common::*;
use hermes_sidecar::connect;

#[test]
fn log_keeps_the_launch_link_with_its_token_redacted() {
    let log = log_path("redaction");
    let cfg = base_config(fixture_command(&["--token", TOKEN]), &log);
    let backend = connect(&cfg, |_| {}).expect("fixture becomes ready");

    let text = std::fs::read_to_string(&log).expect("log exists");
    assert!(
        text.contains("#hermes-session=<redacted>"),
        "log was: {text}"
    );
    assert!(!text.contains(TOKEN));
    assert!(backend
        .launch_url()
        .ends_with(&format!("#hermes-session={TOKEN}")));
    assert!(!format!("{backend:?}").contains(TOKEN));

    backend.shutdown().expect("shutdown");
    let _ = std::fs::remove_file(log);
}
