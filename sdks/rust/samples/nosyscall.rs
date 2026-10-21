//! Call sites make no system call and no allocation (Linux only, x86-64 and aarch64).
//!
//! A fresh thread installs a seccomp-BPF filter that allows only `exit`, `exit_group` and `rt_sigreturn`, arms an
//! allocation counter, then runs a million evaluations of every kind (true and false alternating, the slot claim of
//! the thread's first call included), `work` and `thread`, and ends with a raw `exit`. Any other system call kills
//! the process, so the parent sees a signal instead of a clean exit; an allocation (which a thread's own arena can
//! serve without a system call, so the filter alone would miss it) is counted and fails the check.
//!
//! Usage: `hs_sample_nosyscall` runs the check in a child process and exits 0 on success; the child mode is private.

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod check {
    use hairspring_sdk::{
        always, always_or_unreachable, reachable, sometimes, unreachable_point, work,
    };
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::os::raw::{c_int, c_long, c_ulong};
    use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};

    const EVALUATIONS: u32 = 1_000_000;
    const CHILD_FLAG: &str = "--child";
    const FILTER_FAILED: i32 = 90;
    const SPAWN_FAILED: i32 = 91;
    const ALLOCATED: i32 = 92;
    const POLL: std::time::Duration = std::time::Duration::from_millis(5);

    /// Forwards to the system allocator and counts calls while armed.
    struct CountingAlloc;
    static ARMED: AtomicBool = AtomicBool::new(false);
    static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    // SAFETY: every call is forwarded unchanged to the system allocator.
    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if ARMED.load(Relaxed) {
                ALLOCATIONS.fetch_add(1, Relaxed);
            }
            System.alloc(layout)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout);
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            if ARMED.load(Relaxed) {
                ALLOCATIONS.fetch_add(1, Relaxed);
            }
            System.realloc(ptr, layout, new_size)
        }
    }

    #[global_allocator]
    static ALLOC: CountingAlloc = CountingAlloc;

    /// Set by the checked thread just before its raw exit: 1 = clean, 2 = it allocated.
    static OUTCOME: AtomicU32 = AtomicU32::new(0);

    #[cfg(target_arch = "x86_64")]
    mod nr {
        pub const EXIT: u32 = 60;
        pub const EXIT_GROUP: u32 = 231;
        pub const RT_SIGRETURN: u32 = 15;
    }
    #[cfg(target_arch = "aarch64")]
    mod nr {
        pub const EXIT: u32 = 93;
        pub const EXIT_GROUP: u32 = 94;
        pub const RT_SIGRETURN: u32 = 139;
    }

    const BPF_LD_W_ABS: u16 = 0x20;
    const BPF_JEQ_K: u16 = 0x15;
    const BPF_RET_K: u16 = 0x06;
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const PR_SET_NO_NEW_PRIVS: c_int = 38;
    const PR_SET_SECCOMP: c_int = 22;
    const SECCOMP_MODE_FILTER: c_ulong = 2;

    #[repr(C)]
    struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    #[repr(C)]
    struct SockFprog {
        len: u16,
        filter: *const SockFilter,
    }

    extern "C" {
        fn prctl(option: c_int, ...) -> c_int;
        fn syscall(number: c_long, ...) -> c_long;
    }

    const fn stmt(code: u16, k: u32) -> SockFilter {
        SockFilter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }

    const fn jump(code: u16, k: u32, jt: u8) -> SockFilter {
        SockFilter { code, jt, jf: 0, k }
    }

    fn allow_only_exit() {
        // The syscall number is the first word of `seccomp_data`.
        let program = [
            stmt(BPF_LD_W_ABS, 0),
            jump(BPF_JEQ_K, nr::EXIT, 3),
            jump(BPF_JEQ_K, nr::EXIT_GROUP, 2),
            jump(BPF_JEQ_K, nr::RT_SIGRETURN, 1),
            stmt(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
            stmt(BPF_RET_K, SECCOMP_RET_ALLOW),
        ];
        let fprog = SockFprog {
            len: program.len() as u16,
            filter: program.as_ptr(),
        };
        // SAFETY: prctl is called with the integer and pointer arguments the options document; `program` and `fprog`
        // outlive the call (the kernel copies the filter).
        let failed = unsafe {
            prctl(
                PR_SET_NO_NEW_PRIVS,
                1 as c_ulong,
                0 as c_ulong,
                0 as c_ulong,
                0 as c_ulong,
            ) != 0
                || prctl(
                    PR_SET_SECCOMP,
                    SECCOMP_MODE_FILTER,
                    std::ptr::addr_of!(fprog),
                ) != 0
        };
        if failed {
            // The filter could not be installed: do not claim a pass.
            // SAFETY: a raw exit of the whole process.
            unsafe { syscall(c_long::from(nr::EXIT_GROUP), c_long::from(FILTER_FAILED)) };
        }
    }

    fn asserting_thread() {
        allow_only_exit();
        ARMED.store(true, Relaxed);
        let mut flip = 0u32;
        for i in 0..EVALUATIONS {
            flip = std::hint::black_box(flip + 1);
            let ok = flip & 1 == 1;
            always!(ok, "nosys.always", "alternates", i);
            always_or_unreachable!(ok, "nosys.aou", "alternates", i);
            sometimes!(ok, "nosys.sometimes", "alternates", i);
            reachable!("nosys.reachable", "hit", i);
            unreachable_point!("nosys.unreachable", "hit", i);
            work(1);
        }
        // Naming the thread is not a call site, but it must not make a system call or allocate either.
        let _ = hairspring_sdk::thread("nosys", 1);
        ARMED.store(false, Relaxed);
        let clean = ALLOCATIONS.load(Relaxed) == 0;
        OUTCOME.store(if clean { 1 } else { 2 }, Release);
        // SAFETY: a raw exit of this thread only; no destructor or runtime code runs after the filter.
        unsafe { syscall(c_long::from(nr::EXIT), 0 as c_long) };
    }

    /// The child: run the checked thread and wait for its verdict. The thread ends without returning a result, so it
    /// is not joined (the standard library would panic); the process exits once the outcome is set.
    fn child() -> i32 {
        if std::thread::Builder::new().spawn(asserting_thread).is_err() {
            return SPAWN_FAILED;
        }
        loop {
            match OUTCOME.load(Acquire) {
                0 => std::thread::sleep(POLL),
                1 => return 0,
                _ => return ALLOCATED,
            }
        }
    }

    pub fn main() -> i32 {
        if std::env::args().nth(1).as_deref() == Some(CHILD_FLAG) {
            return child();
        }
        let exe = std::env::current_exe().expect("own path");
        let status = std::process::Command::new(exe).arg(CHILD_FLAG).status();
        match status {
            Ok(s) if s.success() => {
                println!("ok   no system call and no allocation during {EVALUATIONS} evaluations of every kind");
                0
            }
            other => {
                println!("FAIL call sites made a system call or allocated, or the filter could not be installed ({other:?})");
                1
            }
        }
    }
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn main() {
    std::process::exit(check::main());
}

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
fn main() {
    println!("skip: the seccomp check needs Linux on x86-64 or aarch64");
}
