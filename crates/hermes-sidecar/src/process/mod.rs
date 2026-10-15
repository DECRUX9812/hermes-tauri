#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::{adopt, configure, ProcessGroup};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{adopt, configure, ProcessGroup};
