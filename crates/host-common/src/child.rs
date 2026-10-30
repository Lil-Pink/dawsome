//! A scan child's platform half: started in a process tree of its own (a process group on
//! Linux, a job object on Windows), so that whatever it starts can be killed with it; told not
//! to leave crash reports behind; and its exit read as a crash or an exit code.

use std::io;
use std::process::{Child, Command, ExitStatus};

pub(crate) use platform::{failure, prepare_child, Tree};

impl Tree {
    /// Starts `command` in a tree of its own.
    pub(crate) fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        platform::spawn(command)
    }
}

#[cfg(unix)]
mod platform {
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    use super::{io, Child, Command, ExitStatus};

    /// The child's process group, whose id is the child's pid.
    pub(crate) struct Tree {
        group: libc::pid_t,
    }

    pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, Tree)> {
        let mut child = command.process_group(0).spawn()?;
        let Ok(group) = libc::pid_t::try_from(child.id()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("the child's pid is out of range"));
        };
        Ok((child, Tree { group }))
    }

    impl Tree {
        /// Kills every process in the group. Why its id names no other group once the child is
        /// reaped: the kernel keeps a group's id reserved while any member lives, and hands out
        /// a freed pid again only after going round all the others.
        pub(crate) fn kill(&self) {
            // SAFETY: kill(2) takes no pointer and only sends a signal. `group` is the child's
            // pid (`spawn`: `process_group(0)` makes it the group's id), so it is positive and
            // `-group` names that group alone, never ours (which `kill(0)` would).
            unsafe { libc::kill(-self.group, libc::SIGKILL) };
        }
    }

    /// Why an exited child failed, if it did.
    pub(crate) fn failure(status: ExitStatus) -> Option<String> {
        if let Some(signal) = status.signal() {
            return Some(format!("crashed: signal {signal}"));
        }
        match status.code() {
            Some(0) => None,
            Some(code) => Some(format!("exit code {code}")),
            None => Some("bad output".into()),
        }
    }

    /// Run in the child before it loads the plugin file. Why: a plugin that crashes here is
    /// expected; without this every crash would leave a core dump behind (or wake the system's
    /// crash collector) for a file that is simply recorded as failed. And the child dies with
    /// the scan that started it: in a process group of its own, a terminal's Ctrl-C no longer
    /// reaches it.
    pub(crate) fn prepare_child() {
        #[cfg(target_os = "linux")]
        {
            // SAFETY: prctl(PR_SET_DUMPABLE) takes plain integers, no pointer, and changes only
            // this process's flag.
            unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
            // SAFETY: as above, for PR_SET_PDEATHSIG.
            unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) };
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::os::windows::process::CommandExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::ErrorReporting::{WerSetFlags, WER_FAULT_REPORTING_NO_UI};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
    };

    use super::{io, Child, Command, ExitStatus};

    /// The job object the child runs in. Closing it (dropping this, or the scan's process
    /// ending) kills whatever is still in it.
    pub(crate) struct Tree {
        /// `None` when the child could not be put in a job, which can fail where the scan
        /// itself already runs in a job that allows no other: the child alone is then killed.
        job: Option<OwnedHandle>,
    }

    pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, Tree)> {
        // Started suspended, so it runs nothing before it is in the job and whatever it starts
        // is in the job too. No console window: the child prints to a pipe, never to a
        // terminal.
        let mut child = command
            .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
            .spawn()?;
        let job = killing_job().filter(|job| {
            // SAFETY: both handles are live: the job is ours and the child is not yet waited
            // for.
            unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) != 0 }
        });
        if let Err(error) = resume(child.id()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok((child, Tree { job }))
    }

    /// Lets process `pid`, started suspended, run: resumes its threads (a process started
    /// suspended has the one). Why found by a snapshot of the system's threads: the standard
    /// library closes the handle to the child's first thread as it starts the child.
    fn resume(pid: u32) -> io::Result<()> {
        // SAFETY: takes a snapshot of every thread in the system; no pointer is passed.
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a handle just created and owned by nobody else.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut entry = THREADENTRY32 {
            dwSize: size_of::<THREADENTRY32>() as u32,
            ..THREADENTRY32::default()
        };
        let mut resumed = false;
        // SAFETY: the snapshot is live and `entry` is the structure the call fills, its size
        // set.
        let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                // SAFETY: opens the thread by id, for suspending and resuming only.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: `thread` is a handle just opened and owned by nobody else.
                let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
                // SAFETY: the thread handle is live and has the access ResumeThread needs.
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                resumed = true;
            }
            // SAFETY: as for Thread32First.
            more = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
        }
        if resumed {
            Ok(())
        } else {
            Err(io::Error::other("the child's thread was not found"))
        }
    }

    /// A new job object that kills its processes once its last handle closes.
    fn killing_job() -> Option<OwnedHandle> {
        // SAFETY: a job with default security and no name.
        let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is a handle just created and owned by nobody else.
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is the structure this information class names, with its size.
        let limited = unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0
        };
        limited.then_some(job)
    }

    impl Tree {
        /// Kills every process in the job.
        pub(crate) fn kill(&self) {
            if let Some(job) = &self.job {
                // SAFETY: the job handle is live; terminating an empty job does nothing.
                unsafe { TerminateJobObject(job.as_raw_handle(), 1) };
            }
        }
    }

    /// Why an exited child failed, if it did. A process ended by an exception exits with its
    /// status code, an NTSTATUS whose top two bits are set: 0xC0000005 for an access violation,
    /// 0xC0000409 for an abort.
    pub(crate) fn failure(status: ExitStatus) -> Option<String> {
        match status.code() {
            Some(0) => None,
            Some(code) if (code as u32) >> 30 == 0b11 => {
                Some(format!("crashed: exception 0x{:08X}", code as u32))
            }
            Some(code) => Some(format!("exit code {code}")),
            None => Some("bad output".into()),
        }
    }

    /// Run in the child before it loads the plugin file. Why: a plugin that crashes here is
    /// expected, and without this every crash would wait on an error dialog nobody sees until
    /// the scan's timeout, for a file that is simply recorded as failed.
    pub(crate) fn prepare_child() {
        // SAFETY: takes flags, no pointer, and changes only this process's error mode.
        unsafe {
            SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX)
        };
        // SAFETY: as above, for this process's error reporting.
        unsafe { WerSetFlags(WER_FAULT_REPORTING_NO_UI) };
    }
}
