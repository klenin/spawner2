use nix::libc::{__u16, __u32, __u64, __u8, c_int, c_ushort};

#[repr(C)]
pub struct sock_filter {
    pub code: __u16,
    pub jt: __u8,
    pub jf: __u8,
    pub k: __u32,
}

#[repr(C)]
pub struct sock_fprog {
    pub len: c_ushort,
    pub filter: *mut sock_filter,
}

#[repr(C)]
pub struct seccomp_data {
    pub nr: c_int,
    pub arch: __u32,
    pub instruction_pointer: __u64,
    pub args: [__u64; 6],
}

pub const AUDIT_ARCH_I386: __u32 = 0x4000_0003;
pub const AUDIT_ARCH_X86_64: __u32 = 0xC000_003E;

pub const SECCOMP_RET_KILL: __u32 = 0x0000_0000;
pub const SECCOMP_RET_ALLOW: __u32 = 0x7fff_0000;

pub const SECCOMP_MODE_FILTER: c_int = 2;

pub const BPF_LD: __u16 = 0x00;
pub const BPF_JMP: __u16 = 0x05;
pub const BPF_RET: __u16 = 0x06;

// ld/ldx fields.
pub const BPF_W: __u16 = 0x00;
pub const BPF_ABS: __u16 = 0x20;

// alu/jmp fields.
pub const BPF_JEQ: __u16 = 0x10;
pub const BPF_K: __u16 = 0x00;
