use crate::command::{Command, EnvKind};
use crate::sys::windows::common::{cvt, to_utf16, Handle};
use crate::sys::windows::thread::ThreadIterator;
use crate::{Error, Result};

use winapi::shared::basetsd::{DWORD_PTR, SIZE_T};
use winapi::shared::minwindef::{DWORD, FALSE, TRUE, WORD};
use winapi::um::errhandlingapi::SetErrorMode;
use winapi::um::jobapi2::{
    AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject, TerminateJobObject,
};
use winapi::um::minwinbase::STILL_ACTIVE;
use winapi::um::processthreadsapi::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, OpenThread, ResumeThread, SuspendThread, TerminateProcess,
    UpdateProcThreadAttribute, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_LIST,
};
use winapi::um::userenv::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
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

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::collections::HashMap;
use std::fmt::{self, Write};
use std::mem;
use std::ptr;
use std::time::Instant;
use std::u32;

pub struct Process {
    handle: Handle,
    id: DWORD,
    job: Handle,
    creation_time: Instant,
}

pub enum Status {
    Running,
    Finished(u32),
    Crashed(u32, &'static str),
}

pub struct RawStdio {
    pub stdin: Handle,
    pub stdout: Handle,
    pub stderr: Handle,
}

impl Process {
    pub fn spawn(cmd: &Command, stdio: RawStdio) -> Result<Self> {
        let (handle, id) = create_suspended_process(cmd, stdio)?;
        let job = match assign_process_to_new_job(&handle) {
            Ok(x) => x,
            Err(e) => {
                unsafe {
                    TerminateProcess(handle.0, 0);
                }
                return Err(e);
            }
        };

        let creation_time = Instant::now();
        if !cmd.spawn_suspended {
            resume_process(id)?;
        }

        Ok(Process {
            handle: handle,
            id: id,
            job: job,
            creation_time: creation_time,
        })
    }

    pub fn status(&self) -> Result<Status> {
        let mut exit_code: DWORD = 0;
        unsafe {
            cvt(GetExitCodeProcess(self.handle.0, &mut exit_code))?;
        }
        Ok(match exit_code {
            STILL_ACTIVE => Status::Running,
            _ => {
                if let Some(cause) = crash_cause(exit_code) {
                    Status::Crashed(exit_code, cause)
                } else {
                    Status::Finished(exit_code)
                }
            }
        })
    }

    pub fn suspend(&self) -> Result<()> {
        for id in ThreadIterator::new(self.id) {
            unsafe {
                let handle = Handle(cvt(OpenThread(THREAD_SUSPEND_RESUME, FALSE, id))?);
                if SuspendThread(handle.0) == u32::MAX {
                    return Err(Error::last_os_error());
                }
            }
        }
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        resume_process(self.id)
    }

    pub fn basic_and_io_info(&self) -> Result<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION> {
        unsafe {
            let mut basic_and_io_info: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION =
                mem::zeroed();

            cvt(QueryInformationJobObject(
                /*hJob=*/ self.job.0,
                /*JobObjectInfoClass=*/ JobObjectBasicAndIoAccountingInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut basic_and_io_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&basic_and_io_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))
            .map(|_| basic_and_io_info)
        }
    }

    pub fn ext_limit_info(&self) -> Result<JOBOBJECT_EXTENDED_LIMIT_INFORMATION> {
        unsafe {
            let mut ext_info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
            cvt(QueryInformationJobObject(
                /*hJob=*/ self.job.0,
                /*JobObjectInfoClass=*/ JobObjectExtendedLimitInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut ext_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&ext_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))
            .map(|_| ext_info)
        }
    }

    pub fn creation_time(&self) -> Instant {
        self.creation_time
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            TerminateJobObject(self.job.0, 0);
        }
    }
}

fn create_suspended_process(cmd: &Command, stdio: RawStdio) -> Result<(Handle, DWORD)> {
    let mut argv =
        argv_to_cmd(std::iter::once(cmd.app.as_str()).chain(cmd.args.iter().map(|a| a.as_str())));
    let mut inherited_handles = [stdio.stdin.0, stdio.stdout.0, stdio.stderr.0];
    let (mut startup_info, att_list_size) =
        create_startup_info(cmd, &stdio, &mut inherited_handles)?;
    let current_dir = cmd
        .current_dir
        .as_ref()
        .map_or(ptr::null(), |dir| to_utf16(dir).as_mut_ptr());
    let creation_flags =
        CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;
    let mut env = create_env(cmd)?;
    let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };

    unsafe {
        SetErrorMode(SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS);
        let result = cvt(CreateProcessW(
            /*lpApplicationName=*/ ptr::null(),
            /*lpCommandLine=*/ argv.as_mut_ptr(),
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

        drop(Handle(process_info.hThread));
    }

    Ok((Handle(process_info.hProcess), process_info.dwProcessId))
}

fn create_startup_info(
    cmd: &Command,
    stdio: &RawStdio,
    inherited_handles: &mut [HANDLE],
) -> Result<(STARTUPINFOEXW, SIZE_T)> {
    let mut att_list_size: SIZE_T = 0;
    unsafe {
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut att_list_size);
    }

    let mut info: STARTUPINFOEXW = unsafe { mem::zeroed() };
    info.lpAttributeList = alloc_att_list(att_list_size)?;
    info.StartupInfo.cb = mem::size_of_val(&info) as DWORD;
    info.StartupInfo.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
    info.StartupInfo.lpDesktop = ptr::null_mut();
    info.StartupInfo.wShowWindow = if cmd.show_gui { SW_SHOW } else { SW_HIDE } as WORD;
    info.StartupInfo.hStdInput = stdio.stdin.0;
    info.StartupInfo.hStdOutput = stdio.stdout.0;
    info.StartupInfo.hStdError = stdio.stderr.0;

    // Unfortunately, winapi-rs does not define this.
    // Tested on windows 10 only.
    const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: DWORD_PTR = 131074;

    let result = unsafe {
        cvt(InitializeProcThreadAttributeList(
            /*lpAttributeList=*/ info.lpAttributeList,
            /*dwAttributeCount=*/ 1,
            /*dwFlags=*/ 0,
            /*lpSize=*/ &mut att_list_size,
        ))
        .and_then(|_| {
            cvt(UpdateProcThreadAttribute(
                /*lpAttributeList=*/ info.lpAttributeList,
                /*dwFlags=*/ 0,
                /*Attribute=*/ PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                /*lpValue=*/ mem::transmute(inherited_handles.as_mut_ptr()),
                /*cbSize=*/ inherited_handles.len() * mem::size_of::<HANDLE>(),
                /*lpPreviousValue=*/ ptr::null_mut(),
                /*lpReturnSize=*/ ptr::null_mut(),
            ))
        })
    };

    if let Err(e) = result {
        dealloc_att_list(info.lpAttributeList, att_list_size);
        return Err(e);
    }

    Ok((info, att_list_size))
}

fn create_env(cmd: &Command) -> Result<Vec<u16>> {
    // https://docs.microsoft.com/en-us/windows/desktop/api/processthreadsapi/nf-processthreadsapi-createprocessa
    //
    // An environment block consists of a null-terminated block of null-terminated strings.
    // Each string is in the following form:
    //     name=value\0
    //
    // A Unicode environment block is terminated by four zero bytes: two for the last string,
    // two more to terminate the block.
    let mut env = match cmd.env_kind {
        EnvKind::Clear => HashMap::new(),
        EnvKind::Inherit => std::env::vars().collect(),
        EnvKind::UserDefault => user_env()?,
    };
    for var in cmd.env_vars.iter() {
        env.insert(var.name.clone(), var.val.clone());
    }

    let mut result: Vec<u16> = env
        .iter()
        .map(|(name, val)| to_utf16(format!("{}={}", name, val)))
        .flatten()
        .chain(std::iter::once(0))
        .collect();
    if result.len() == 1 {
        result.push(0);
    }
    Ok(result)
}

fn resume_process(process_id: DWORD) -> Result<()> {
    for id in ThreadIterator::new(process_id) {
        unsafe {
            let handle = Handle(cvt(OpenThread(THREAD_SUSPEND_RESUME, FALSE, id))?);
            if ResumeThread(handle.0) == u32::MAX {
                return Err(Error::last_os_error());
            }
        }
    }
    Ok(())
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

fn assign_process_to_new_job(process: &Handle) -> Result<Handle> {
    unsafe {
        let job = Handle(cvt(CreateJobObjectW(ptr::null_mut(), ptr::null()))?);
        cvt(AssignProcessToJobObject(job.0, process.0)).map(|_| job)
    }
}

fn user_env() -> Result<HashMap<String, String>> {
    let env_block = create_env_block()?;
    let mut result: HashMap<String, String> = HashMap::new();
    for var in env_block.split(|c| *c == 0) {
        let nameval = String::from_utf16_lossy(var);
        if let Some(idx) = nameval.find('=') {
            result.insert(nameval[0..idx].to_string(), nameval[idx + 1..].to_string());
        }
    }
    destroy_env_block(env_block);
    Ok(result)
}

fn create_env_block<'a>() -> Result<&'a mut [u16]> {
    unsafe {
        let mut env_block: *mut u16 = ptr::null_mut();
        cvt(CreateEnvironmentBlock(
            mem::transmute(&mut env_block),
            ptr::null_mut(),
            FALSE,
        ))?;

        let mut i = 0;
        while *env_block.offset(i) != 0 && *env_block.offset(i + 1) != 0 {
            i += 1;
        }

        Ok(std::slice::from_raw_parts_mut(env_block, i as usize))
    }
}

fn destroy_env_block(block: &mut [u16]) {
    unsafe {
        DestroyEnvironmentBlock(mem::transmute(block.as_mut_ptr()));
    }
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

fn argv_to_cmd<T, U>(argv: T) -> Vec<u16>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    let mut cmd = String::new();
    for (idx, arg) in argv.into_iter().enumerate() {
        if idx != 0 {
            cmd.write_char(' ').unwrap();
        }
        write_quoted(&mut cmd, arg.as_ref());
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
