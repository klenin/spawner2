use command::Command;
use process::ProcessInfo;
use runner_impl::Message;
use std::sync::mpsc::Sender;

#[derive(Clone, Debug, PartialEq)]
pub enum TerminationReason {
    WallClockTimeLimitExceeded,
    IdleTimeLimitExceeded,
    UserTimeLimitExceeded,
    WriteLimitExceeded,
    MemoryLimitExceeded,
    ProcessLimitExceeded,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExitStatus {
    Crashed(u32, &'static str),
    Finished(u32),
    Terminated(TerminationReason),
}

#[derive(Clone, Debug)]
pub struct RunnerReport {
    pub command: Command,
    pub process_info: ProcessInfo,
    pub exit_status: ExitStatus,
}

#[derive(Clone)]
pub struct Runner {
    pub(crate) sender: Sender<Message>,
}

impl Runner {
    fn send(&self, msg: Message) {
        let _ = self.sender.send(msg);
    }

    pub fn terminate(&self) {
        self.send(Message::Terminate);
    }

    pub fn suspend(&self) {
        self.send(Message::Suspend);
    }

    pub fn resume(&self) {
        self.send(Message::Resume);
    }

    pub fn reset_timers(&self) {
        self.send(Message::ResetTimers);
    }
}
