use command::CommandInner;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::time::Duration;
pub use sys::process_common::{ProcessTreeStatus, SummaryInfo};
use sys::windows::common::{ok_neq_minus_one, ok_nonzero};
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
use winapi::um::winbase::{CREATE_SUSPENDED, STARTF_USESHOWWINDOW};
use winapi::um::winnt::{
    JobObjectBasicAndIoAccountingInformation, JobObjectExtendedLimitInformation, HANDLE,
    JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    THREAD_SUSPEND_RESUME,
};
use winapi::um::winuser::{SW_HIDE, SW_SHOW};

pub struct ProcessTree {
    root: PROCESS_INFORMATION,
    job: HANDLE,
}

unsafe impl Send for ProcessTree {}

impl ProcessTree {
    pub(crate) fn spawn(cmd: &CommandInner) -> io::Result<Self> {
        let info = create_suspended_process(cmd)?;
        let job = assign_process_to_new_job(info.hProcess).map_err(|e| {
            match unsafe { ok_nonzero(TerminateProcess(info.hProcess, 0)) } {
                Ok(_) => e,
                Err(te) => te,
            }
        })?;
        match resume_process(info.dwProcessId) {
            Ok(_) => Ok(Self {
                root: info,
                job: job,
            }),
            Err(e) => {
                unsafe {
                    ok_nonzero(TerminateJobObject(job, 0))?;
                }
                Err(e)
            }
        }
    }

    pub fn status(&self) -> io::Result<ProcessTreeStatus> {
        let mut exit_code: DWORD = 0;
        unsafe {
            ok_nonzero(GetExitCodeProcess(self.root.hProcess, &mut exit_code))?;
        }
        if exit_code == STILL_ACTIVE {
            Ok(ProcessTreeStatus::Alive(self.summary_info()?))
        } else {
            Ok(ProcessTreeStatus::Finished(exit_code as i32))
        }
    }

    pub fn kill(self) -> io::Result<()> {
        unsafe {
            ok_nonzero(TerminateJobObject(self.job, 0))?;
        }
        Ok(())
    }

    fn summary_info(&self) -> io::Result<SummaryInfo> {
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

            Ok(SummaryInfo {
                total_user_time: Duration::from_nanos(user_time * 100),
                total_kernel_time: Duration::from_nanos(kernel_time * 100),
                peak_memory_used: ext_info.PeakJobMemoryUsed as u64,
                total_processes: basic_and_io_info.BasicInfo.TotalProcesses as u64,
                total_bytes_written: basic_and_io_info.IoInfo.WriteTransferCount,
            })
        }
    }
}

impl fmt::Debug for ProcessTree {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "ProcessTree {{ root: {{ dwProcessId: {}, dwThreadId: {} }} }}",
            self.root.dwProcessId, self.root.dwThreadId,
        )
    }
}

fn create_suspended_process(cmd: &CommandInner) -> io::Result<PROCESS_INFORMATION> {
    let mut cmdline = argv_to_cmd(&cmd.app, &cmd.args)?;
    let creation_flags = CREATE_SUSPENDED;
    let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let mut startup_info: STARTUPINFOW = unsafe { mem::zeroed() };
    startup_info.cb = mem::size_of_val(&startup_info) as DWORD;
    startup_info.dwFlags = STARTF_USESHOWWINDOW;
    startup_info.lpDesktop = ptr::null_mut();
    startup_info.wShowWindow = if cmd.display_gui { SW_SHOW } else { SW_HIDE } as WORD;

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
        ))?
    };
    Ok(process_info)
}

fn assign_process_to_new_job(process: HANDLE) -> io::Result<HANDLE> {
    unsafe {
        let job = ok_nonzero(CreateJobObjectW(ptr::null_mut(), ptr::null()))?;
        ok_nonzero(AssignProcessToJobObject(job, process))?;
        Ok(job)
    }
}

fn resume_process(process_id: DWORD) -> io::Result<()> {
    for id in ThreadIterator::new(process_id) {
        unsafe {
            let handle = ok_nonzero(OpenThread(THREAD_SUSPEND_RESUME, FALSE, id))?;
            ok_neq_minus_one(ResumeThread(handle))?;
            ok_nonzero(CloseHandle(handle))?;
        }
    }
    Ok(())
}

fn argv_to_cmd(app: &OsString, args: &Vec<OsString>) -> io::Result<Vec<u16>> {
    let mut result = match Path::new(app).canonicalize() {
        Ok(buf) => quote(&buf.into_os_string()),
        Err(_) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("cannot find {}", app.to_string_lossy()),
            ));
        }
    };
    for arg in args {
        result.push(" ");
        result.push(quote(arg));
    }
    Ok(result.encode_wide().collect())
}

fn quote(s: &OsString) -> OsString {
    if s.to_string_lossy().find(' ').is_some() {
        let mut quoted = OsString::new();
        quoted.push("\"");
        quoted.push(s);
        quoted.push("\"");
        quoted
    } else {
        s.clone()
    }
}
