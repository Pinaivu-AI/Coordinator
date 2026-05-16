//! Low-level system utilities for the enclave init path: mount, insmod,
//! dmesg, freopen, socket, entropy seeding.

use libc::{c_int, c_ulong, c_void};
use std::{ffi::CString, fmt, fs::File, mem::zeroed, os::unix::io::AsRawFd};

pub struct SystemError {
    pub message: String,
}

impl fmt::Display for SystemError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", boot_time(), self.message)
    }
}

pub fn boot_time() -> String {
    use libc::{clock_gettime, timespec, CLOCK_BOOTTIME};
    let mut t = timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { clock_gettime(CLOCK_BOOTTIME, &mut t as *mut timespec) };
    format!("[ {:>4}.{}]", t.tv_sec, t.tv_nsec / 1000)
}

pub fn dmesg(message: String) {
    println!("{} {}", boot_time(), message);
}

pub fn reboot() {
    use libc::{reboot, RB_AUTOBOOT};
    unsafe { reboot(RB_AUTOBOOT) };
}

pub fn mount(
    src: &str,
    target: &str,
    fstype: &str,
    flags: c_ulong,
    data: &str,
) -> Result<(), SystemError> {
    let src_cs = CString::new(src).unwrap();
    let target_cs = CString::new(target).unwrap();
    let fstype_cs = CString::new(fstype).unwrap();
    let data_cs = CString::new(data).unwrap();
    let ret = unsafe {
        libc::mount(
            src_cs.as_ptr(),
            target_cs.as_ptr(),
            fstype_cs.as_ptr(),
            flags,
            data_cs.as_ptr() as *const c_void,
        )
    };
    if ret != 0 {
        Err(SystemError { message: format!("mount {target} failed") })
    } else {
        Ok(())
    }
}

pub fn freopen(filename: &str, mode: &str, file: c_int) -> Result<(), SystemError> {
    let fn_cs = CString::new(filename).unwrap();
    let mode_cs = CString::new(mode).unwrap();
    let result = unsafe {
        libc::freopen(
            fn_cs.as_ptr(),
            mode_cs.as_ptr(),
            libc::fdopen(file, mode_cs.as_ptr()),
        )
    };
    if result.is_null() {
        Err(SystemError { message: format!("freopen {filename} failed") })
    } else {
        Ok(())
    }
}

pub fn insmod(path: &str, _params: &str) -> Result<(), SystemError> {
    use libc::{syscall, SYS_finit_module};
    let file = File::open(path).map_err(|e| SystemError {
        message: format!("open {path}: {e}"),
    })?;
    let fd = file.as_raw_fd();
    if unsafe { syscall(SYS_finit_module, fd, "\0".as_ptr(), 0) } < 0 {
        Err(SystemError { message: format!("insmod {path} failed") })
    } else {
        Ok(())
    }
}

pub fn socket_connect(family: c_int, port: u32, cid: u32) -> Result<c_int, SystemError> {
    use libc::{connect, sockaddr, sockaddr_vm, socket, SOCK_STREAM};
    let fd = unsafe { socket(family, SOCK_STREAM, 0) };
    let ret = unsafe {
        let mut sa: sockaddr_vm = zeroed();
        sa.svm_family = family as _;
        sa.svm_port = port;
        sa.svm_cid = cid;
        connect(fd, &sa as *const _ as *mut sockaddr, std::mem::size_of::<sockaddr_vm>() as _)
    };
    if ret < 0 {
        Err(SystemError { message: format!("connect port {port} cid {cid} failed") })
    } else {
        Ok(fd)
    }
}

pub fn seed_entropy(
    size: usize,
    source: fn(usize) -> Result<Vec<u8>, SystemError>,
) -> Result<usize, SystemError> {
    use std::io::Write;
    let entropy = source(size)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/urandom")
        .map_err(|e| SystemError { message: format!("open /dev/urandom: {e}") })?;
    f.write_all(&entropy)
        .map_err(|e| SystemError { message: format!("write entropy: {e}") })?;
    Ok(entropy.len())
}
