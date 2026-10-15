use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use crate::error::SidecarError;
use crate::launch_link::{parse_launch_link, redact, LaunchLink};

const TAIL_LINES: usize = 20;

pub(crate) struct Output {
    log: Option<Mutex<File>>,
    tail: Mutex<VecDeque<String>>,
}

impl Output {
    pub(crate) fn open(log_path: Option<&Path>) -> Result<Self, SidecarError> {
        let log = match log_path {
            Some(path) => {
                let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|source| SidecarError::Log {
                        path: path.to_owned(),
                        source,
                    })?;
                Some(Mutex::new(file))
            }
            None => None,
        };
        Ok(Self {
            log,
            tail: Mutex::new(VecDeque::with_capacity(TAIL_LINES)),
        })
    }

    fn record(&self, line: &str) {
        let line = redact(line);
        if let Some(log) = &self.log {
            // A failed log write must not stop the child's output pipe from draining.
            let _ = writeln!(lock(log), "{line}");
        }
        let mut tail = lock(&self.tail);
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }

    pub(crate) fn tail(&self) -> Vec<String> {
        lock(&self.tail).iter().cloned().collect()
    }
}

/// Drains one child stream on a thread. The link is sent only after the line is recorded,
/// so a caller that has the link can rely on the log already containing it.
pub(crate) fn pump<R>(stream: R, output: Arc<Output>, links: Sender<LaunchLink>, eof: Sender<()>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&buf);
            let line = text.trim_end_matches(['\r', '\n']);
            output.record(line);
            if let Some(link) = parse_launch_link(line) {
                let _ = links.send(link);
            }
        }
        let _ = eof.send(());
    });
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
