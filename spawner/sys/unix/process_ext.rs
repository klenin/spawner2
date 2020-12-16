use crate::process::ProcessInfo;
use crate::sys::unix::missing_decls::{
    self, sock_filter, BPF_ABS, BPF_JEQ, BPF_JMP, BPF_K, BPF_LD, BPF_RET, BPF_W, SECCOMP_RET_ALLOW,
    SECCOMP_RET_KILL,
};
use crate::sys::AsInnerMut;

use nix::libc::{__u16, __u32, __u8};
pub use nix::sched::CpuSet;

// https://outflux.net/teach-seccomp
pub struct SyscallFilter(Vec<sock_filter>);

pub struct SyscallFilterBuilder(Vec<sock_filter>);

pub trait ProcessInfoExt {
    fn syscall_filter(&mut self, filter: SyscallFilter) -> &mut Self;
    fn cpuset(&mut self, cpuset: CpuSet) -> &mut Self;
}

#[cfg(target_arch = "x86")]
const ARCH_NR: __u32 = missing_decls::AUDIT_ARCH_I386;

#[cfg(target_arch = "x86_64")]
const ARCH_NR: __u32 = missing_decls::AUDIT_ARCH_X86_64;

impl SyscallFilterBuilder {
    pub fn block_all() -> Self {
        let arch_offset = 4; // offsetof(struct seccomp_data, arch)
        let nr_offset = 0; // offsetof(struct seccomp_data, nr)
        Self(vec![
            // Validate architecture.
            bpf_stmt(BPF_LD + BPF_W + BPF_ABS, arch_offset),
            bpf_jump(BPF_JMP + BPF_JEQ + BPF_K, ARCH_NR, 1, 0),
            bpf_stmt(BPF_RET + BPF_K, SECCOMP_RET_KILL),
            // Examine syscall.
            bpf_stmt(BPF_LD + BPF_W + BPF_ABS, nr_offset),
        ])
    }

    pub fn allow(&mut self, syscall: __u32) -> &mut Self {
        self.0
            .push(bpf_jump(BPF_JMP + BPF_JEQ + BPF_K, syscall, 0, 1));
        self.0.push(bpf_stmt(BPF_RET + BPF_K, SECCOMP_RET_ALLOW));
        self
    }

    pub fn build(mut self) -> SyscallFilter {
        // Kill process.
        self.0.push(bpf_stmt(BPF_RET + BPF_K, SECCOMP_RET_KILL));
        SyscallFilter(self.0)
    }
}

impl AsInnerMut<Vec<sock_filter>> for SyscallFilter {
    fn as_inner_mut(&mut self) -> &mut Vec<sock_filter> {
        &mut self.0
    }
}

impl ProcessInfoExt for ProcessInfo {
    fn syscall_filter(&mut self, filter: SyscallFilter) -> &mut Self {
        self.as_inner_mut().syscall_filter(filter);
        self
    }

    fn cpuset(&mut self, cpuset: CpuSet) -> &mut Self {
        self.as_inner_mut().cpuset(cpuset);
        self
    }
}

fn bpf_stmt(code: __u16, k: __u32) -> sock_filter {
    sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

fn bpf_jump(code: __u16, k: __u32, jt: __u8, jf: __u8) -> sock_filter {
    sock_filter { code, jt, jf, k }
}
