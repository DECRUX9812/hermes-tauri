use std::fs;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

pub(crate) fn configure(command: &mut Command) {
    command.process_group(0);
}

pub(crate) fn adopt(child: &mut Child) -> io::Result<ProcessGroup> {
    Ok(ProcessGroup {
        pgid: child.id() as libc::pid_t,
    })
}

pub(crate) struct ProcessGroup {
    pgid: libc::pid_t,
}

impl ProcessGroup {
    /// Sends SIGTERM to the whole group. Returns false when the platform has no graceful stop.
    pub(crate) fn request_stop(&self) -> bool {
        // SAFETY: kill(2) with a negative pid only signals that process group.
        unsafe {
            libc::kill(-self.pgid, libc::SIGTERM);
        }
        true
    }

    pub(crate) fn kill(&self) {
        // SAFETY: as in request_stop.
        unsafe {
            libc::kill(-self.pgid, libc::SIGKILL);
        }
    }

    /// Zombie members count as present, which keeps the pgid from being reused while we still need it.
    pub(crate) fn has_members(&self) -> bool {
        // SAFETY: signal 0 only probes for existence.
        let probe = unsafe { libc::kill(-self.pgid, 0) };
        probe == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    /// True when a live (not dead, not zombie) process is in the group.
    ///
    /// A zombie member has exited and holds no resources; it only waits for
    /// whoever reparented to reap it, which in a container without an init
    /// may never happen. Waiting on zombies would report `StillRunning`
    /// forever, so shutdown considers them gone.
    pub(crate) fn has_live_members(&self) -> bool {
        let entries = match fs::read_dir("/proc") {
            Ok(entries) => entries,
            // Without /proc there is nothing better than the signal probe.
            Err(_) => return self.has_members(),
        };
        for entry in entries.flatten() {
            let pid: u32 = match entry.file_name().to_string_lossy().parse() {
                Ok(pid) => pid,
                Err(_) => continue,
            };
            let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
                Ok(stat) => stat,
                Err(_) => continue,
            };
            // After the final ')' the fields are: state pgrp session …
            let mut fields = match stat.rsplit(')').next() {
                Some(fields) => fields.split_whitespace(),
                None => continue,
            };
            let (state, pgrp) = match (fields.next(), fields.next()) {
                (Some(state), Some(pgrp)) => (state, pgrp),
                _ => continue,
            };
            if pgrp == self.pgid.to_string() && !state.starts_with(['Z', 'X']) {
                return true;
            }
        }
        false
    }
}
