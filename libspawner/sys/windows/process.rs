use crate::{Error, Result};
use command::Command;
use std::fmt;
use std::mem;
use std::path::Path;
use std::ptr;
use std::time::Duration;
pub use sys::process_common::{Statistics, Status, Stdio};
use sys::windows::common::{ok_neq_minus_one, ok_nonzero, to_utf16};
use sys::windows::thread::ThreadIterator;
use winapi::shared::minwindef::{DWORD, FALSE, TRUE, WORD};
use winapi::um::handleapi::CloseHandle;
use winapi::um::jobapi2::{
    AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject, TerminateJobObject,
};
use winapi::um::minwinbase::STILL_ACTIVE;
use winapi::um::processthreadsapi::{
    CreateProcessW, GetExitCodeProcess, OpenThread, ResumeThread, TerminateProcess,
    PROCESS_INFORMATION, STARTUPINFOW,
};
use winapi::um::winbase::{CREATE_SUSPENDED, STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES};
use winapi::um::winnt::{
    JobObjectBasicAndIoAccountingInformation, JobObjectExtendedLimitInformation, HANDLE,
    JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    THREAD_SUSPEND_RESUME,
};
use winapi::um::winuser::{SW_HIDE, SW_SHOW};

/// This structure is used to represent and manage root process and all its descendants.
/// The `status` method will query information about parent process and all
/// its childred. Note that `Status::Finished` does not guarantee that all child processes
/// are finished.
pub struct Process {
    handle: HANDLE,
    id: DWORD,
    job: HANDLE,
    _stdio: Stdio,
}

unsafe impl Send for Process {}

impl Process {
    /// Spawns process from given command and stdio streams
    pub fn spawn(cmd: &Command, stdio: Stdio) -> Result<Self> {
        let (handle, id) = create_suspended_process(cmd, &stdio)?;
        let job = match assign_process_to_new_job(handle) {
            Ok(x) => x,
            Err(e) => {
                unsafe {
                    TerminateProcess(handle, 0);
                    CloseHandle(handle);
                }
                return Err(e);
            }
        };

        match resume_process(id) {
            Ok(_) => Ok(Self {
                handle: handle,
                id: id,
                job: job,
                _stdio: stdio,
            }),
            Err(e) => {
                unsafe {
                    TerminateJobObject(job, 0);
                    CloseHandle(handle);
                    CloseHandle(job);
                }
                Err(e)
            }
        }
    }

    pub fn status(&self) -> Result<Status> {
        let mut exit_code: DWORD = 0;
        unsafe {
            ok_nonzero(GetExitCodeProcess(self.handle, &mut exit_code))?;
        }
        if exit_code == STILL_ACTIVE {
            Ok(Status::Alive(self.statistics()?))
        } else {
            Ok(Status::Finished(exit_code as i32))
        }
    }

    fn statistics(&self) -> Result<Statistics> {
        unsafe {
            let mut basic_and_io_info: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION =
                mem::zeroed();
            let mut ext_info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();

            ok_nonzero(QueryInformationJobObject(
                /*hJob=*/ self.job,
                /*JobObjectInfoClass=*/ JobObjectBasicAndIoAccountingInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut basic_and_io_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&basic_and_io_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))?;
            ok_nonzero(QueryInformationJobObject(
                /*hJob=*/ self.job,
                /*JobObjectInfoClass=*/ JobObjectExtendedLimitInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut ext_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&ext_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))?;

            // total user time in in 100-nanosecond ticks
            let user_time = *basic_and_io_info.BasicInfo.TotalUserTime.QuadPart() as u64;
            // total kernel time in in 100-nanosecond ticks
            let kernel_time = *basic_and_io_info.BasicInfo.TotalKernelTime.QuadPart() as u64;

            Ok(Statistics {
                total_user_time: Duration::from_nanos(user_time * 100),
                total_kernel_time: Duration::from_nanos(kernel_time * 100),
                peak_memory_used: ext_info.PeakJobMemoryUsed as u64,
                total_processes: basic_and_io_info.BasicInfo.TotalProcesses as u64,
                total_bytes_written: basic_and_io_info.IoInfo.WriteTransferCount,
            })
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            TerminateJobObject(self.job, 0);
            CloseHandle(self.job);
        }
    }
}

impl fmt::Debug for Process {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Process {{ id: {} }}", self.id,)
    }
}

fn create_suspended_process(cmd: &Command, stdio: &Stdio) -> Result<(HANDLE, DWORD)> {
    let mut cmdline = argv_to_cmd(&cmd.app, &cmd.args)?;
    let creation_flags = CREATE_SUSPENDED;
    let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let mut startup_info: STARTUPINFOW = unsafe { mem::zeroed() };
    startup_info.cb = mem::size_of_val(&startup_info) as DWORD;
    startup_info.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
    startup_info.lpDesktop = ptr::null_mut();
    startup_info.wShowWindow = if cmd.show_gui { SW_SHOW } else { SW_HIDE } as WORD;
    startup_info.hStdInput = stdio.stdin.handle;
    startup_info.hStdOutput = stdio.stdout.handle;
    startup_info.hStdError = stdio.stderr.handle;

    unsafe {
        ok_nonzero(CreateProcessW(
            /*lpApplicationName=*/ ptr::null(),
            /*lpCommandLine=*/ cmdline.as_mut_ptr(),
            /*lpProcessAttributes=*/ ptr::null_mut(),
            /*lpThreadAttributes=*/ ptr::null_mut(),
            /*bInheritHandles=*/ TRUE,
            /*dwCreationFlags=*/ creation_flags,
            /*lpEnvironment=*/ ptr::null_mut(),
            /*lpCurrentDirectory=*/ ptr::null(),
            /*lpStartupInfo=*/ &mut startup_info,
            /*lpProcessInformation=*/ &mut process_info,
        ))?;
        CloseHandle(process_info.hThread);
    }

    Ok((process_info.hProcess, process_info.dwProcessId))
}

fn assign_process_to_new_job(process: HANDLE) -> Result<HANDLE> {
    unsafe {
        let job = ok_nonzero(CreateJobObjectW(ptr::null_mut(), ptr::null()))?;
        match ok_nonzero(AssignProcessToJobObject(job, process)) {
            Ok(_) => Ok(job),
            Err(e) => {
                CloseHandle(job);
                Err(e)
            }
        }
    }
}

fn resume_process(process_id: DWORD) -> Result<()> {
    for id in ThreadIterator::new(process_id) {
        unsafe {
            let handle = ok_nonzero(OpenThread(THREAD_SUSPEND_RESUME, FALSE, id))?;
            let result = ok_neq_minus_one(ResumeThread(handle));
            CloseHandle(handle);
            if let Err(e) = result {
                return Err(e);
            }
        }
    }
    Ok(())
}

fn argv_to_cmd(app: &String, args: &Vec<String>) -> Result<Vec<u16>> {
    let mut result = match Path::new(app).canonicalize() {
        Ok(buf) => quote(&buf.to_str().unwrap()),
        Err(e) => return Err(Error::from(e)),
    };
    for arg in args {
        result.push(' ');
        result.push_str(quote(arg).as_str());
    }
    Ok(to_utf16(result))
}

fn quote<S: AsRef<str>>(s: S) -> String {
    let escaped = s.as_ref().replace("\"", "\\\"");
    if escaped.find(' ').is_some() {
        format!("\"{}\"", escaped)
    } else {
        escaped
    }
}
