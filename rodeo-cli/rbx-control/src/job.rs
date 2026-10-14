//! Windows job object for serve-child teardown that still lets `--detach`
//! Studios escape.
//!
//! `process_wrap::tokio::JobObject` sets only `KILL_ON_JOB_CLOSE`, and Windows
//! job membership is inherited: a Studio spawned by a jobbed `__studio-backend`
//! is silently in the same job, so when the serve supervisor exits (closing the
//! job handle) the kernel kills the Studio — no userland detach logic ever
//! runs. This job adds `JOB_OBJECT_LIMIT_BREAKAWAY_OK`, so a child spawned with
//! `CREATE_BREAKAWAY_FROM_JOB` (every launch-control spawn: a detached Studio
//! outlives the serve, a non-detached one runs in a kill-on-close job of its
//! own instead) legitimately leaves the job while everything else keeps the
//! kill-on-close cascade.
//!
//! The child is assigned after a normal (non-suspended) spawn. The unguarded
//! window is a few microseconds against serve children that take seconds to
//! spawn their first grandchild, and losing the race only degrades to the
//! pre-job behavior (an orphan on hard-kill), so the suspended-start dance
//! process-wrap does is not worth its thread-resume hack here.

#![cfg(windows)]

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};

/// Owns one job handle. Dropping it (or the owning process exiting, however
/// abruptly) closes the handle, which kills every remaining member.
pub struct KillOnCloseJob {
    handle: HANDLE,
}

// HANDLE is a raw pointer; the job handle is only ever used for Assign/Close,
// both of which are thread-safe kernel calls.
unsafe impl Send for KillOnCloseJob {}
unsafe impl Sync for KillOnCloseJob {}

impl KillOnCloseJob {
    /// Create a job with `KILL_ON_JOB_CLOSE | BREAKAWAY_OK` limits.
    pub fn new() -> std::io::Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                CloseHandle(handle);
                return Err(err);
            }
            Ok(Self { handle })
        }
    }

    /// Assign a running process (by pid) to this job.
    pub fn assign_pid(&self, pid: u32) -> std::io::Result<()> {
        unsafe {
            // SET_QUOTA + TERMINATE is the documented access needed by
            // AssignProcessToJobObject.
            let proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if proc.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let ok = AssignProcessToJobObject(self.handle, proc);
            let err = std::io::Error::last_os_error();
            CloseHandle(proc);
            if ok == 0 {
                return Err(err);
            }
            Ok(())
        }
    }
}

impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}
