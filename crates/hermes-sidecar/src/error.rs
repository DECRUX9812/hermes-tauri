use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error("No Hermes URL is configured and starting Hermes is disabled")]
    NothingToConnect,
    #[error("The attach URL must be an http or https link or origin")]
    InvalidAttachUrl,
    #[error("Hermes at {origin} is not responding")]
    Unreachable { origin: String },
    #[error("Starting Hermes needs a command, but none is configured")]
    NoCommand,
    #[error("Could not start `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("Could not open the sidecar log at {}: {source}", .path.display())]
    Log {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Could not check on the Hermes process: {source}")]
    Wait {
        #[source]
        source: io::Error,
    },
    #[error("Hermes exited before it became ready ({}){}", exit_text(.code), last_output(.tail))]
    ChildExited {
        code: Option<i32>,
        tail: Vec<String>,
    },
    #[error("Timed out after {after:?} waiting for {waiting_for}{}", last_output(.tail))]
    Timeout {
        waiting_for: String,
        after: Duration,
        tail: Vec<String>,
    },
    #[error("Hermes (pid {pid}) is still running after SIGKILL")]
    StillRunning { pid: u32 },
    #[error("The connection attempt was cancelled")]
    Cancelled,
}

fn exit_text(code: &Option<i32>) -> String {
    match code {
        Some(code) => format!("exit code {code}"),
        None => "terminated by a signal".to_owned(),
    }
}

fn last_output(tail: &[String]) -> String {
    if tail.is_empty() {
        String::new()
    } else {
        format!("\nLast output:\n{}", tail.join("\n"))
    }
}
