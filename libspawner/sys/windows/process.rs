use crate::{Error, Result};
use command::Command;
use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::fmt;
use std::fmt::Write;
use std::mem;
use std::ptr;
use std::time::Duration;
use std::time::Instant;
pub use sys::process_common::*;
use sys::windows::common::{ok_neq_minus_one, ok_nonzero, to_utf16};
use sys::windows::env;
use sys::windows::thread::ThreadIterator;
use winapi::shared::basetsd::{DWORD_PTR, SIZE_T};
use winapi::shared::minwindef::{DWORD, FALSE, TRUE, WORD};
use winapi::um::errhandlingapi::SetErrorMode;
use winapi::um::handleapi::CloseHandle;
use winapi::um::jobapi2::{
    AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject, TerminateJobObject,
};
use winapi::um::minwinbase::STILL_ACTIVE;
use winapi::um::processthreadsapi::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, OpenThread, ResumeThread, SuspendThread, TerminateProcess,
    UpdateProcThreadAttribute, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_LIST,
};
use winapi::um::winbase::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
    SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES,
    STARTUPINFOEXW,
};
use winapi::um::winnt::{
    JobObjectBasicAndIoAccountingInformation, JobObjectExtendedLimitInformation, HANDLE,
    JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    STATUS_ACCESS_VIOLATION, STATUS_ARRAY_BOUNDS_EXCEEDED, STATUS_BREAKPOINT,
    STATUS_CONTROL_C_EXIT, STATUS_DATATYPE_MISALIGNMENT, STATUS_FLOAT_DENORMAL_OPERAND,
    STATUS_FLOAT_INEXACT_RESULT, STATUS_FLOAT_INVALID_OPERATION, STATUS_FLOAT_MULTIPLE_FAULTS,
    STATUS_FLOAT_MULTIPLE_TRAPS, STATUS_FLOAT_OVERFLOW, STATUS_FLOAT_STACK_CHECK,
    STATUS_FLOAT_UNDERFLOW, STATUS_GUARD_PAGE_VIOLATION, STATUS_ILLEGAL_INSTRUCTION,
    STATUS_INTEGER_DIVIDE_BY_ZERO, STATUS_INTEGER_OVERFLOW, STATUS_INVALID_DISPOSITION,
    STATUS_IN_PAGE_ERROR, STATUS_NONCONTINUABLE_EXCEPTION, STATUS_PRIVILEGED_INSTRUCTION,
    STATUS_REG_NAT_CONSUMPTION, STATUS_SINGLE_STEP, STATUS_STACK_OVERFLOW, THREAD_SUSPEND_RESUME,
};
use winapi::um::winuser::{SW_HIDE, SW_SHOW};

/// This structure is used to represent and manage root process and all its descendants.
pub struct Process {
    handle: HANDLE,
    id: DWORD,
    job: HANDLE,
    creation_time: Instant,
}

unsafe impl Send for Process {}

impl Process {
    /// Spawns process from given command and stdio streams.
    pub fn spawn(cmd: &Command, stdio: ProcessStdio) -> Result<Self> {
        let (handle, id) = create_suspended_process(cmd, stdio)?;
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
        let creation_time = Instant::now();
        match resume_process(id) {
            Ok(_) => Ok(Self {
                handle: handle,
                id: id,
                job: job,
                creation_time: creation_time,
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

    /// Returns status of the root process. Note that `Status::Finished` does not guarantee
    /// that all child processes are finished.
    pub fn status(&self) -> Result<ProcessStatus> {
        let mut exit_code: DWORD = 0;
        unsafe {
            ok_nonzero(GetExitCodeProcess(self.handle, &mut exit_code))?;
        }
        Ok(match exit_code {
            STILL_ACTIVE => ProcessStatus::Running,
            _ => {
                if let Some(cause) = crash_cause(exit_code) {
                    ProcessStatus::Crashed(ProcessStatusCrashed {
                        exit_code: exit_code,
                        cause: cause,
                    })
                } else {
                    ProcessStatus::Finished(exit_code)
                }
            }
        })
    }

    /// Suspends the root process.
    pub fn suspend(&self) -> Result<()> {
        for id in ThreadIterator::new(self.id) {
            unsafe {
                let handle = ok_nonzero(OpenThread(THREAD_SUSPEND_RESUME, FALSE, id))?;
                let result = ok_neq_minus_one(SuspendThread(handle));
                CloseHandle(handle);
                if let Err(e) = result {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Resumes the root process.
    pub fn resume(&self) -> Result<()> {
        resume_process(self.id)
    }

    pub fn info(&self) -> Result<ProcessInfo> {
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

            Ok(ProcessInfo {
                wall_clock_time: self.creation_time.elapsed(),
                total_user_time: Duration::from_nanos(user_time * 100),
                total_kernel_time: Duration::from_nanos(kernel_time * 100),
                peak_memory_used: ext_info.PeakJobMemoryUsed as u64,
                total_processes: basic_and_io_info.BasicInfo.TotalProcesses as usize,
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

fn create_suspended_process(cmd: &Command, stdio: ProcessStdio) -> Result<(HANDLE, DWORD)> {
    let mut cmdline = argv_to_cmd(&cmd.app, &cmd.args);
    let current_dir = cmd
        .current_dir
        .as_ref()
        .map_or(ptr::null(), |dir| to_utf16(dir).as_mut_ptr());

    let creation_flags =
        CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;
    let mut env = env::create(cmd.env_kind, &cmd.env_vars)?;
    let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let mut inherited_handles = vec![stdio.stdin.handle, stdio.stdout.handle, stdio.stderr.handle];
    let (mut startup_info, att_list_size) =
        create_startup_info(cmd, &stdio, &mut inherited_handles)?;

    unsafe {
        SetErrorMode(SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS);
        let result = ok_nonzero(CreateProcessW(
            /*lpApplicationName=*/ ptr::null(),
            /*lpCommandLine=*/ cmdline.as_mut_ptr(),
            /*lpProcessAttributes=*/ ptr::null_mut(),
            /*lpThreadAttributes=*/ ptr::null_mut(),
            /*bInheritHandles=*/ TRUE,
            /*dwCreationFlags=*/ creation_flags,
            /*lpEnvironment=*/ mem::transmute(env.as_mut_ptr()),
            /*lpCurrentDirectory=*/ current_dir,
            /*lpStartupInfo=*/ mem::transmute(&mut startup_info),
            /*lpProcessInformation=*/ &mut process_info,
        ));

        // Restore default error mode.
        SetErrorMode(0);
        DeleteProcThreadAttributeList(startup_info.lpAttributeList);
        dealloc_att_list(startup_info.lpAttributeList, att_list_size);
        result?;

        CloseHandle(process_info.hThread);
    }

    drop(stdio);
    Ok((process_info.hProcess, process_info.dwProcessId))
}

fn create_startup_info(
    cmd: &Command,
    stdio: &ProcessStdio,
    inherited_handles: &mut Vec<HANDLE>,
) -> Result<(STARTUPINFOEXW, SIZE_T)> {
    let mut info: STARTUPINFOEXW = unsafe { mem::zeroed() };
    info.StartupInfo.cb = mem::size_of_val(&info) as DWORD;
    info.StartupInfo.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
    info.StartupInfo.lpDesktop = ptr::null_mut();
    info.StartupInfo.wShowWindow = if cmd.show_gui { SW_SHOW } else { SW_HIDE } as WORD;
    info.StartupInfo.hStdInput = stdio.stdin.handle;
    info.StartupInfo.hStdOutput = stdio.stdout.handle;
    info.StartupInfo.hStdError = stdio.stderr.handle;

    let mut size: SIZE_T = 0;
    unsafe {
        // Unfortunately, winapi-rs does not define this.
        // Tested on windows 10 only.
        const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: DWORD_PTR = 131074;

        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
        info.lpAttributeList = alloc_att_list(size)?;
        let result = ok_nonzero(InitializeProcThreadAttributeList(
            /*lpAttributeList=*/ info.lpAttributeList,
            /*dwAttributeCount=*/ 1,
            /*dwFlags=*/ 0,
            /*lpSize=*/ &mut size,
        ))
        .and_then(|_| {
            ok_nonzero(UpdateProcThreadAttribute(
                /*lpAttributeList=*/ info.lpAttributeList,
                /*dwFlags=*/ 0,
                /*Attribute=*/ PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                /*lpValue=*/ mem::transmute(inherited_handles.as_mut_ptr()),
                /*cbSize=*/ inherited_handles.len() * mem::size_of::<HANDLE>(),
                /*lpPreviousValue=*/ ptr::null_mut(),
                /*lpReturnSize=*/ ptr::null_mut(),
            ))
        });
        if let Err(e) = result {
            dealloc_att_list(info.lpAttributeList, size);
            return Err(e);
        }
    }

    Ok((info, size))
}

fn alloc_att_list(size: SIZE_T) -> Result<*mut PROC_THREAD_ATTRIBUTE_LIST> {
    unsafe {
        let list = alloc_zeroed(Layout::from_size_align_unchecked(size, 4));
        if list == ptr::null_mut() {
            Err(Error::from(
                "cannot allocate memory for PROC_THREAD_ATTRIBUTE_LIST",
            ))
        } else {
            Ok(mem::transmute(list))
        }
    }
}

fn dealloc_att_list(list: *mut PROC_THREAD_ATTRIBUTE_LIST, size: SIZE_T) {
    unsafe {
        dealloc(
            mem::transmute(list),
            Layout::from_size_align_unchecked(size, 4),
        );
    }
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

fn argv_to_cmd(app: &String, args: &Vec<String>) -> Vec<u16> {
    let mut cmd = String::new();
    write_quoted(&mut cmd, app);
    for arg in args {
        cmd.write_char(' ').unwrap();
        write_quoted(&mut cmd, arg);
    }
    to_utf16(cmd)
}

fn write_quoted<W, S>(w: &mut W, s: S)
where
    W: fmt::Write,
    S: AsRef<str>,
{
    let escaped = s.as_ref().replace("\"", "\\\"");
    if escaped.find(' ').is_some() {
        write!(w, "\"{}\"", escaped)
    } else {
        w.write_str(escaped.as_str())
    }
    .unwrap();
}

fn crash_cause(exit_code: DWORD) -> Option<&'static str> {
    match exit_code {
        STATUS_ACCESS_VIOLATION => Some("AccessViolation"),
        STATUS_ARRAY_BOUNDS_EXCEEDED => Some("ArrayBoundsExceeded"),
        STATUS_BREAKPOINT => Some("Breakpoint"),
        STATUS_CONTROL_C_EXIT => Some("Control_C_Exit"),
        STATUS_DATATYPE_MISALIGNMENT => Some("DatatypeMisalignment"),
        STATUS_FLOAT_DENORMAL_OPERAND => Some("FloatDenormalOperand"),
        STATUS_FLOAT_INEXACT_RESULT => Some("FloatInexactResult"),
        STATUS_FLOAT_INVALID_OPERATION => Some("FloatInvalidOperation"),
        STATUS_FLOAT_MULTIPLE_FAULTS => Some("FloatMultipleFaults"),
        STATUS_FLOAT_MULTIPLE_TRAPS => Some("FloatMultipleTraps"),
        STATUS_FLOAT_OVERFLOW => Some("FloatOverflow"),
        STATUS_FLOAT_STACK_CHECK => Some("FloatStackCheck"),
        STATUS_FLOAT_UNDERFLOW => Some("FloatUnderflow"),
        STATUS_GUARD_PAGE_VIOLATION => Some("GuardPageViolation"),
        STATUS_ILLEGAL_INSTRUCTION => Some("IllegalInstruction"),
        STATUS_IN_PAGE_ERROR => Some("InPageError"),
        STATUS_INVALID_DISPOSITION => Some("InvalidDisposition"),
        STATUS_INTEGER_DIVIDE_BY_ZERO => Some("IntegerDivideByZero"),
        STATUS_INTEGER_OVERFLOW => Some("IntegerOverflow"),
        STATUS_NONCONTINUABLE_EXCEPTION => Some("NoncontinuableException"),
        STATUS_PRIVILEGED_INSTRUCTION => Some("PrivilegedInstruction"),
        STATUS_REG_NAT_CONSUMPTION => Some("RegNatConsumption"),
        STATUS_SINGLE_STEP => Some("SingleStep"),
        STATUS_STACK_OVERFLOW => Some("StackOverflow"),
        _ => None,
    }
}
