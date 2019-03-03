use crate::{Error, Result};
use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt::{self, Write};
use std::mem;
use std::ptr;
use std::time::{Duration, Instant};
use std::u32;
use sys::windows::common::{cvt, to_utf16, Handle};
use sys::windows::pipe::{ReadPipe, WritePipe};
use sys::windows::thread::ThreadIterator;
use sys::IntoInner;
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

pub struct Process {
    handle: Handle,
    id: DWORD,
    job: Handle,
    creation_time: Instant,
}

unsafe impl Send for Process {}

pub struct ExitStatus(u32);

#[derive(Clone)]
pub struct ProcessInfo {
    wall_clock_time: Duration,
    basic_and_io_info: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION,
    ext_info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
}

pub struct ProcessStdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

pub struct ProcessBuilder {
    argv: Vec<u16>,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
    current_dir: Option<Vec<u16>>,
    env: HashMap<String, String>,
    spawn_suspended: bool,
    show_window: bool,
}

impl Process {
    pub fn exit_status(&self) -> Result<Option<ExitStatus>> {
        let mut exit_code: DWORD = 0;
        unsafe {
            cvt(GetExitCodeProcess(self.handle.0, &mut exit_code))?;
        }
        Ok(match exit_code {
            STILL_ACTIVE => None,
            _ => Some(ExitStatus(exit_code)),
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

    pub fn info(&self) -> Result<ProcessInfo> {
        unsafe {
            let mut basic_and_io_info: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION =
                mem::zeroed();
            let mut ext_info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();

            cvt(QueryInformationJobObject(
                /*hJob=*/ self.job.0,
                /*JobObjectInfoClass=*/ JobObjectBasicAndIoAccountingInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut basic_and_io_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&basic_and_io_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))?;
            cvt(QueryInformationJobObject(
                /*hJob=*/ self.job.0,
                /*JobObjectInfoClass=*/ JobObjectExtendedLimitInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut ext_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&ext_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))?;

            Ok(ProcessInfo {
                wall_clock_time: self.creation_time.elapsed(),
                basic_and_io_info: basic_and_io_info,
                ext_info: ext_info,
            })
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            TerminateJobObject(self.job.0, 0);
        }
    }
}

impl ExitStatus {
    pub fn code(&self) -> u32 {
        self.0
    }

    pub fn crash_cause(&self) -> Option<&'static str> {
        match self.0 {
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
}

impl ProcessInfo {
    pub fn wall_clock_time(&self) -> Duration {
        self.wall_clock_time
    }

    pub fn total_user_time(&self) -> Duration {
        // Total user time in 100-nanosecond ticks.
        let user_time =
            unsafe { *self.basic_and_io_info.BasicInfo.TotalUserTime.QuadPart() } as u64;
        Duration::from_nanos(user_time * 100)
    }

    pub fn total_kernel_time(&self) -> Duration {
        // Total kernel time in 100-nanosecond ticks.
        let kernel_time =
            unsafe { *self.basic_and_io_info.BasicInfo.TotalKernelTime.QuadPart() } as u64;
        Duration::from_nanos(kernel_time * 100)
    }

    pub fn peak_memory_used(&self) -> u64 {
        self.ext_info.PeakJobMemoryUsed as u64
    }

    pub fn total_processes_created(&self) -> usize {
        self.basic_and_io_info.BasicInfo.TotalProcesses as usize
    }

    pub fn total_bytes_written(&self) -> u64 {
        self.basic_and_io_info.IoInfo.WriteTransferCount
    }
}

impl ProcessBuilder {
    pub fn new<T, U>(argv: T, stdio: ProcessStdio) -> Self
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        Self {
            argv: to_utf16(argv_to_cmd(argv)),
            stdin: stdio.stdin.into_inner(),
            stdout: stdio.stdout.into_inner(),
            stderr: stdio.stderr.into_inner(),
            current_dir: None,
            env: HashMap::new(),
            show_window: false,
            spawn_suspended: false,
        }
    }

    pub fn spawn_suspended(&mut self, v: bool) -> &mut Self {
        self.spawn_suspended = v;
        self
    }

    pub fn show_window(&mut self, v: bool) -> &mut Self {
        self.show_window = v;
        self
    }

    pub fn current_dir<S: AsRef<OsStr>>(&mut self, dir: Option<S>) -> &mut Self {
        self.current_dir = dir.map(|d| to_utf16(d.as_ref()));
        self
    }

    pub fn clear_env(&mut self) -> &mut Self {
        self.env = HashMap::new();
        self
    }

    pub fn inherit_env(&mut self) -> &mut Self {
        self.env = std::env::vars().collect();
        self
    }

    pub fn user_env(&mut self) -> Result<&mut Self> {
        self.env = user_env()?;
        Ok(self)
    }

    pub fn env_var<S: AsRef<str>>(&mut self, key: S, val: S) -> &mut Self {
        self.env
            .insert(key.as_ref().to_string(), val.as_ref().to_string());
        self
    }

    pub fn spawn(self) -> Result<Process> {
        let spawn_suspended = self.spawn_suspended;
        let (handle, id) = self.create_suspended_process()?;
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
        if !spawn_suspended {
            resume_process(id)?;
        }

        Ok(Process {
            handle: handle,
            id: id,
            job: job,
            creation_time: creation_time,
        })
    }

    fn create_suspended_process(mut self) -> Result<(Handle, DWORD)> {
        let mut inherited_handles = [self.stdin.0, self.stdout.0, self.stderr.0];
        let (mut startup_info, att_list_size) = self.create_startup_info(&mut inherited_handles)?;
        let current_dir = self
            .current_dir
            .as_mut()
            .map_or(ptr::null(), |dir| dir.as_mut_ptr());
        let creation_flags =
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;
        let mut env = self.create_env();
        let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };

        unsafe {
            SetErrorMode(SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS);
            let result = cvt(CreateProcessW(
                /*lpApplicationName=*/ ptr::null(),
                /*lpCommandLine=*/ self.argv.as_mut_ptr(),
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
        &self,
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
        info.StartupInfo.wShowWindow = if self.show_window { SW_SHOW } else { SW_HIDE } as WORD;
        info.StartupInfo.hStdInput = self.stdin.0;
        info.StartupInfo.hStdOutput = self.stdout.0;
        info.StartupInfo.hStdError = self.stderr.0;

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

    fn create_env(&self) -> Vec<u16> {
        // https://docs.microsoft.com/en-us/windows/desktop/api/processthreadsapi/nf-processthreadsapi-createprocessa
        //
        // An environment block consists of a null-terminated block of null-terminated strings.
        // Each string is in the following form:
        //     name=value\0
        //
        // A Unicode environment block is terminated by four zero bytes: two for the last string,
        // two more to terminate the block.
        let mut result: Vec<u16> = self
            .env
            .iter()
            .map(|(name, val)| to_utf16(format!("{}={}", name, val)))
            .flatten()
            .chain(std::iter::once(0))
            .collect();
        if result.len() == 1 {
            result.push(0);
        }
        result
    }
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

fn argv_to_cmd<T, U>(argv: T) -> String
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
    cmd
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
