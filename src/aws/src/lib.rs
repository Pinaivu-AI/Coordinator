//! AWS Nitro platform support: entropy from NSM and heartbeat.
//!
//! With the `nsm` feature: uses the real Nitro Security Module device.
//! Without `nsm`: falls back to reading /dev/urandom (dev and test use).

use system::SystemError;

#[cfg(feature = "nsm")]
pub fn get_entropy(size: usize) -> Result<Vec<u8>, SystemError> {
    use nsm_lib::{nsm_get_random, nsm_lib_init};
    use aws_nitro_enclaves_nsm_api::api::ErrorCode;

    let nsm_fd = nsm_lib_init();
    if nsm_fd < 0 {
        return Err(SystemError { message: "NSM init failed".into() });
    }
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let mut buf = [0u8; 256];
        let mut len = buf.len();
        match unsafe { nsm_get_random(nsm_fd, buf.as_mut_ptr(), &mut len) } {
            ErrorCode::Success => out.extend_from_slice(&buf[..len]),
            _ => return Err(SystemError { message: "NSM get_random failed".into() }),
        }
    }
    Ok(out)
}

#[cfg(not(feature = "nsm"))]
pub fn get_entropy(size: usize) -> Result<Vec<u8>, SystemError> {
    use std::io::Read;
    let mut buf = vec![0u8; size];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| SystemError { message: format!("read /dev/urandom: {e}") })?;
    Ok(buf)
}

/// Called from init before spawning the main process. Sends an NSM
/// heartbeat (in nsm builds) and loads the kernel module.
pub fn init_platform() {
    #[cfg(feature = "nsm")]
    {
        use libc::{close, read, write, AF_VSOCK};
        use system::socket_connect;
        let mut buf = [0xB7u8; 1];
        if let Ok(fd) = socket_connect(AF_VSOCK, 9000, 3) {
            unsafe {
                write(fd, buf.as_ptr() as _, 1);
                read(fd, buf.as_ptr() as _, 1);
                close(fd);
            }
            system::dmesg("NSM heartbeat sent".into());
        }
    }
    // nsm.ko is loaded directly in init/main.rs after init_platform returns.
}
