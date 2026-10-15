use std::ffi::c_void;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub(crate) fn configure(_command: &mut Command) {}

/// Assignment happens after spawn, so a grandchild started in that window escapes the job.
pub(crate) fn adopt(child: &mut Child) -> io::Result<ProcessGroup> {
    // SAFETY: null attributes and name create an unnamed job with default security.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let group = ProcessGroup { job };

    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `limits` is a live JOBOBJECT_EXTENDED_LIMIT_INFORMATION of the size passed.
    let set = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<c_void>(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set == 0 {
        let err = io::Error::last_os_error();
        let _ = child.kill();
        return Err(err);
    }

    // SAFETY: both handles are valid for the duration of the call.
    let assigned = unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) };
    if assigned == 0 {
        let err = io::Error::last_os_error();
        let _ = child.kill();
        return Err(err);
    }
    Ok(group)
}

pub(crate) struct ProcessGroup {
    job: HANDLE,
}

// SAFETY: a job handle is a kernel object reference, usable from any thread.
unsafe impl Send for ProcessGroup {}

impl ProcessGroup {
    /// Windows has no graceful group stop; callers fall through to `kill`.
    pub(crate) fn request_stop(&self) -> bool {
        false
    }

    pub(crate) fn kill(&self) {
        // SAFETY: the job handle is owned by this struct and still open.
        unsafe {
            TerminateJobObject(self.job, 1);
        }
    }

    pub(crate) fn has_members(&self) -> bool {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `info` matches the class's structure size; no return length is requested.
        let ok = unsafe {
            QueryInformationJobObject(
                self.job,
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast::<c_void>(),
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        ok == 0 || info.ActiveProcesses > 0
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // Closing the last handle to a KILL_ON_JOB_CLOSE job terminates every process in it.
        // SAFETY: the handle is owned by this struct and closed exactly once.
        unsafe {
            CloseHandle(self.job);
        }
    }
}
