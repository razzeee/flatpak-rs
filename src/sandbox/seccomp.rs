use std::io;

#[cfg(target_arch = "x86_64")]
pub(super) fn install(graphical: bool) -> io::Result<()> {
    fn statement(code: u16, k: u32) -> libc::sock_filter {
        libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }
    fn jump(k: u32, jt: u8, jf: u8) -> libc::sock_filter {
        libc::sock_filter {
            code: 0x15,
            jt,
            jf,
            k,
        }
    }
    let deny = 0x00050000 | libc::EPERM as u32;
    let allow = 0x7fff0000;
    // Only reviewed ordinary process/thread flags, not a blacklist of today's
    // namespace flags. Reject the entire high word, including future flags.
    let clone_flags = 0x7f
        | (libc::CLONE_VM
            | libc::CLONE_FS
            | libc::CLONE_FILES
            | libc::CLONE_SIGHAND
            | libc::CLONE_PIDFD
            | libc::CLONE_VFORK
            | libc::CLONE_PARENT
            | libc::CLONE_THREAD
            | libc::CLONE_SYSVSEM
            | libc::CLONE_SETTLS
            | libc::CLONE_PARENT_SETTID
            | libc::CLONE_CHILD_CLEARTID
            | libc::CLONE_CHILD_SETTID) as u32;
    let mut filter = vec![
        statement(0x20, 4),
        jump(0xc000003e, 1, 0),
        statement(0x06, 0x80000000),
        statement(0x20, 0),
        jump(libc::SYS_clone as u32, 0, 7),
        statement(0x20, 20),
        jump(0, 1, 0),
        statement(0x06, deny),
        statement(0x20, 16),
        libc::sock_filter {
            code: 0x45,
            jt: 0,
            jf: 1,
            k: !clone_flags,
        },
        statement(0x06, deny),
        statement(0x06, allow),
        jump(libc::SYS_clone3 as u32, 0, 1),
        statement(0x06, 0x00050000 | libc::ENOSYS as u32),
    ];
    if graphical {
        // socket() is restricted to AF_UNIX. Its pathname namespace is the
        // sandbox filesystem; abstract names are in the private network ns.
        filter.extend([
            jump(libc::SYS_socket as u32, 0, 4),
            statement(0x20, 16),
            jump(libc::AF_UNIX as u32, 0, 1),
            statement(0x06, allow),
            statement(0x06, deny),
        ]);
        for number in [
            libc::SYS_connect,
            libc::SYS_sendmsg,
            libc::SYS_recvmsg,
            libc::SYS_sendto,
            libc::SYS_recvfrom,
            libc::SYS_getsockopt,
            libc::SYS_setsockopt,
            libc::SYS_getsockname,
            libc::SYS_getpeername,
            libc::SYS_shutdown,
            libc::SYS_memfd_create,
        ] {
            filter.push(jump(number as u32, 0, 1));
            filter.push(statement(0x06, allow));
        }
    }
    // Native architecture only. Unlisted syscalls remain denied, including all
    // network, mount, namespace, kernel instrumentation and ptrace interfaces.
    let allowed = [
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_pread64,
        libc::SYS_pwrite64,
        libc::SYS_open,
        libc::SYS_openat,
        libc::SYS_close,
        libc::SYS_close_range,
        libc::SYS_fstat,
        libc::SYS_stat,
        libc::SYS_lstat,
        libc::SYS_newfstatat,
        libc::SYS_statx,
        libc::SYS_lseek,
        libc::SYS_mmap,
        libc::SYS_mprotect,
        libc::SYS_munmap,
        libc::SYS_brk,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_msync,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_rt_sigsuspend,
        libc::SYS_pause,
        libc::SYS_sigaltstack,
        libc::SYS_ioctl,
        libc::SYS_fcntl,
        libc::SYS_readlink,
        libc::SYS_readlinkat,
        libc::SYS_getcwd,
        libc::SYS_chdir,
        libc::SYS_fchdir,
        libc::SYS_access,
        libc::SYS_faccessat,
        libc::SYS_faccessat2,
        libc::SYS_getdents64,
        libc::SYS_getdents,
        libc::SYS_getuid,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getegid,
        libc::SYS_getresuid,
        libc::SYS_getresgid,
        libc::SYS_getgroups,
        libc::SYS_getpid,
        libc::SYS_getppid,
        libc::SYS_gettid,
        libc::SYS_uname,
        libc::SYS_clock_gettime,
        libc::SYS_clock_getres,
        libc::SYS_gettimeofday,
        libc::SYS_time,
        libc::SYS_nanosleep,
        libc::SYS_clock_nanosleep,
        libc::SYS_futex,
        libc::SYS_set_tid_address,
        libc::SYS_set_robust_list,
        libc::SYS_rseq,
        libc::SYS_arch_prctl,
        libc::SYS_prctl,
        libc::SYS_getrandom,
        libc::SYS_execve,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_wait4,
        libc::SYS_waitid,
        libc::SYS_kill,
        libc::SYS_tkill,
        libc::SYS_tgkill,
        libc::SYS_fork,
        libc::SYS_vfork,
        libc::SYS_pipe,
        libc::SYS_pipe2,
        libc::SYS_dup,
        libc::SYS_dup2,
        libc::SYS_dup3,
        libc::SYS_poll,
        libc::SYS_ppoll,
        libc::SYS_select,
        libc::SYS_pselect6,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_wait,
        libc::SYS_epoll_pwait,
        libc::SYS_eventfd2,
        libc::SYS_socketpair,
        libc::SYS_mkdir,
        libc::SYS_mkdirat,
        libc::SYS_rmdir,
        libc::SYS_unlink,
        libc::SYS_unlinkat,
        libc::SYS_rename,
        libc::SYS_renameat,
        libc::SYS_renameat2,
        libc::SYS_link,
        libc::SYS_linkat,
        libc::SYS_symlink,
        libc::SYS_symlinkat,
        libc::SYS_chmod,
        libc::SYS_fchmod,
        libc::SYS_fchmodat,
        libc::SYS_truncate,
        libc::SYS_ftruncate,
        libc::SYS_fsync,
        libc::SYS_fdatasync,
        libc::SYS_utimensat,
        libc::SYS_flock,
        libc::SYS_umask,
        libc::SYS_sched_yield,
        libc::SYS_sched_getaffinity,
        libc::SYS_getrlimit,
        libc::SYS_setrlimit,
        libc::SYS_prlimit64,
        libc::SYS_getrusage,
        libc::SYS_sysinfo,
        libc::SYS_sendfile,
        libc::SYS_splice,
        libc::SYS_tee,
    ];
    for number in allowed {
        filter.push(jump(number as u32, 0, 1));
        filter.push(statement(0x06, allow));
    }
    filter.push(statement(0x06, deny));
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };
    // SAFETY: the BPF program and backing instructions remain alive for this syscall.
    if unsafe { libc::syscall(libc::SYS_seccomp, 1u32, 0u32, &program) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub(super) fn install(_graphical: bool) -> io::Result<()> {
    Err(io::Error::other(
        "no reviewed syscall policy for this architecture",
    ))
}
