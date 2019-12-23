use winapi::shared::basetsd::DWORD_PTR;
use winapi::shared::minwindef::{BOOL, DWORD, PDWORD, UCHAR, ULONG};
use winapi::um::winnt::PVOID;
use winapi::{ENUM, STRUCT};

pub const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: DWORD_PTR = 131_074;

ENUM! {
    enum TCP_TABLE_CLASS {
        TCP_TABLE_BASIC_LISTENER = 0,
        TCP_TABLE_BASIC_CONNECTIONS = 1,
        TCP_TABLE_BASIC_ALL = 2,
        TCP_TABLE_OWNER_PID_LISTENER = 3,
        TCP_TABLE_OWNER_PID_CONNECTIONS = 4,
        TCP_TABLE_OWNER_PID_ALL = 5,
        TCP_TABLE_OWNER_MODULE_LISTENER = 6,
        TCP_TABLE_OWNER_MODULE_CONNECTIONS = 7,
        TCP_TABLE_OWNER_MODULE_ALL = 8,
    }
}

ENUM! {
    enum UDP_TABLE_CLASS {
        UDP_TABLE_BASIC = 0,
        UDP_TABLE_OWNER_PID = 1,
        UDP_TABLE_OWNER_MODULE = 2,
    }
}

STRUCT! {
    struct MIB_TCPROW_OWNER_PID {
        dwState: DWORD,
        dwLocalAddr: DWORD,
        dwLocalPort: DWORD,
        dwRemoteAddr: DWORD,
        dwRemotePort: DWORD,
        dwOwningPid: DWORD,
    }
}

STRUCT! {
    struct MIB_TCPTABLE_OWNER_PID {
        dwNumEntries: DWORD,
        table: [MIB_TCPROW_OWNER_PID; 1],
    }
}

STRUCT! {
    struct MIB_TCP6ROW_OWNER_PID {
        ucLocalAddr: [UCHAR; 16],
        dwLocalScopeId: DWORD,
        dwLocalPort: DWORD,
        ucRemoteAddr: [UCHAR; 16],
        dwRemoteScopeId: DWORD,
        dwRemotePort: DWORD,
        dwState: DWORD,
        dwOwningPid: DWORD,
    }
}

STRUCT! {
    struct MIB_TCP6TABLE_OWNER_PID {
        dwNumEntries: DWORD,
        table: [MIB_TCP6ROW_OWNER_PID; 1],
    }
}

STRUCT! {
    struct MIB_UDPROW_OWNER_PID {
        dwLocalAddr: DWORD,
        dwLocalPort: DWORD,
        dwOwningPid: DWORD,
    }
}

STRUCT! {
    struct MIB_UDPTABLE_OWNER_PID {
        dwNumEntries: DWORD,
        table: [MIB_UDPROW_OWNER_PID; 1],
    }
}

STRUCT! {
    struct MIB_UDP6ROW_OWNER_PID {
        ucLocalAddr: [UCHAR; 16],
        dwLocalScopeId: DWORD,
        dwLocalPort: DWORD,
        dwOwningPid: DWORD,
    }
}

STRUCT! {
    struct MIB_UDP6TABLE_OWNER_PID {
        dwNumEntries: DWORD,
        table: [MIB_UDP6ROW_OWNER_PID; 1],
    }
}

#[link(name = "iphlpapi")]
extern "system" {
    pub fn GetExtendedTcpTable(
        pTcpTable: PVOID,
        pdwSize: PDWORD,
        bOrder: BOOL,
        ulAf: ULONG,
        TableClass: TCP_TABLE_CLASS,
        Reserved: ULONG,
    ) -> DWORD;

    pub fn GetExtendedUdpTable(
        pUdpTable: PVOID,
        pdwSize: PDWORD,
        bOrder: BOOL,
        ulAf: ULONG,
        TableClass: UDP_TABLE_CLASS,
        Reserved: ULONG,
    ) -> DWORD;
}
