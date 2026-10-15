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
}
