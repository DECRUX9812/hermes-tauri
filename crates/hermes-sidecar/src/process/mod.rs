#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::{adopt, command, configure, ProcessGroup};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{adopt, command, configure, ProcessGroup};
