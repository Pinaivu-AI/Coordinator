//! Nitro Enclave init process — PID 1 inside the initramfs.
//!
//! Sequence:
//!   1. Mount pseudo-filesystems (proc, sys, dev, …)
//!   2. Reopen stdio on /dev/console
//!   3. Call aws::init_platform (initialises the NSM driver)
//!   4. Seed the kernel entropy pool via the NSM RNG
//!   5. Load nsm.ko
//!   6. Wait for the parent to push config over VSOCK:7000 (KEY=VALUE lines).
//!      Received values are set as env vars for the coordinator process.
//!   7. Spawn socat bridges so the coordinator can reach Postgres and Redis
//!      through the parent host's VSOCK-to-TCP forwarders, and so inbound
//!      HTTP + libp2p traffic reaches the coordinator from the parent.
//!   8. Exec the coordinator binary (replaces this process as PID 1).

mod config;

use std::{io::BufReader, os::unix::io::FromRawFd, process::Command};

use aws::{get_entropy, init_platform};
use system::{dmesg, freopen, mount, reboot, seed_entropy, vsock_accept};

/// CID of the parent partition. Fixed at 3 in the Nitro Enclave spec.
const PARENT_CID: u32 = 3;

fn init_rootfs() {
    use libc::{MS_NODEV, MS_NOEXEC, MS_NOSUID};
    let no_dse = MS_NODEV | MS_NOSUID | MS_NOEXEC;
    let no_se = MS_NOSUID | MS_NOEXEC;

    let mounts = [
        ("devtmpfs", "/dev", "devtmpfs", no_se, "mode=0755"),
        ("devpts", "/dev/pts", "devpts", no_se, ""),
        ("shm", "/dev/shm", "tmpfs", no_dse, "mode=0755"),
        ("proc", "/proc", "proc", no_dse, "hidepid=2"),
        ("tmpfs", "/run", "tmpfs", no_dse, "mode=0755"),
        ("tmpfs", "/tmp", "tmpfs", no_dse, ""),
        ("sysfs", "/sys", "sysfs", no_dse, ""),
    ];

    for (src, target, fstype, flags, data) in mounts {
        let _ = std::fs::create_dir_all(target);
        match mount(src, target, fstype, flags, data) {
            Ok(()) => dmesg(format!("mounted {target}")),
            Err(e) => eprintln!("mount {target}: {e}"),
        }
    }
}

fn init_console() {
    for (path, mode, fd) in [
        ("/dev/console", "r", 0),
        ("/dev/console", "w", 1),
        ("/dev/console", "w", 2),
    ] {
        if let Err(e) = freopen(path, mode, fd) {
            eprintln!("freopen {path}: {e}");
        }
    }
}

/// Spawn a socat bridge as a background process. Panics on exec failure.
fn bridge(left: &str, right: &str) {
    match Command::new("/socat").arg(left).arg(right).spawn() {
        Ok(_) => dmesg(format!("socat {left} {right}")),
        Err(e) => eprintln!("socat {left} {right}: {e}"),
    }
}

fn main() {
    init_rootfs();
    init_console();
    init_platform();

    match seed_entropy(4096, get_entropy) {
        Ok(n) => dmesg(format!("entropy seeded: {n} bytes")),
        Err(e) => eprintln!("entropy: {e}"),
    }

    dmesg("pinaivu coordinator enclave booted".into());

    // ── Config injection: parent pushes KEY=VALUE over VSOCK:7000 ────────────
    // The parent connects once, sends the env file, then closes the connection.
    // We apply the received vars before starting the coordinator so secrets
    // (DATABASE_URL, REDIS_URL, …) never appear in the enclave image.
    match vsock_accept(7000) {
        Ok(fd) => {
            let reader = BufReader::new(unsafe { std::fs::File::from_raw_fd(fd) });
            let pairs = config::read_config(reader);
            for (k, v) in &pairs {
                std::env::set_var(k, v);
            }
            dmesg(format!("config injected: {} vars", pairs.len()));
        }
        Err(e) => eprintln!("config vsock: {e}"),
    }

    // ── Outbound bridges: enclave TCP → parent VSOCK → external TCP ──────────
    // Coordinator reads Postgres via 127.0.0.1:5432; parent forwards
    // VSOCK:8101 → Postgres TCP.
    bridge(
        "TCP-LISTEN:5432,reuseaddr,fork",
        &format!("VSOCK-CONNECT:{PARENT_CID}:8101"),
    );
    // Coordinator reads Redis via 127.0.0.1:6379; parent forwards
    // VSOCK:8102 → Redis TCP.
    bridge(
        "TCP-LISTEN:6379,reuseaddr,fork",
        &format!("VSOCK-CONNECT:{PARENT_CID}:8102"),
    );

    // ── Inbound bridges: parent VSOCK → enclave TCP ───────────────────────────
    // HTTP clients reach the coordinator (127.0.0.1:4000) via VSOCK:4000.
    bridge(
        "VSOCK-LISTEN:4000,reuseaddr,fork",
        "TCP:127.0.0.1:4000",
    );
    // libp2p peers reach the swarm (127.0.0.1:4001) via VSOCK:4001.
    bridge(
        "VSOCK-LISTEN:4001,reuseaddr,fork",
        "TCP:127.0.0.1:4001",
    );

    // ── Log forwarder: coordinator stdout → parent VSOCK:5000 ─────────────────
    // Parent collects via: socat VSOCK-LISTEN:5000,reuseaddr,fork -
    // (not a socat bridge here; coordinator writes to stdout/stderr which
    //  the enclave console streams to the parent automatically in debug mode;
    //  production log collection wired in a later slice).

    // Fixed env vars that belong to the enclave image, not the config bundle.
    std::env::set_var("SSL_CERT_FILE", "/etc/ssl/certs/ca-certificates.crt");
    std::env::set_var("PATH", "/bin:/sbin:/usr/bin:/usr/sbin:/");
    // Defaults for vars not supplied by the parent's config push.
    // DATABASE_URL and REDIS_URL should come from VSOCK:7000; fall back to the
    // VSOCK-bridged local addresses so local dev runs still work without a config push.
    if std::env::var("PINAIVU_BIND").is_err() {
        std::env::set_var("PINAIVU_BIND", "127.0.0.1:4000");
    }
    if std::env::var("PINAIVU_LIBP2P_LISTEN").is_err() {
        std::env::set_var("PINAIVU_LIBP2P_LISTEN", "/ip4/0.0.0.0/tcp/4001");
    }
    if std::env::var("DATABASE_URL").is_err() {
        std::env::set_var("DATABASE_URL", "postgresql://coordinator@127.0.0.1:5432/coordinator");
    }
    if std::env::var("REDIS_URL").is_err() {
        std::env::set_var("REDIS_URL", "redis://127.0.0.1:6379");
    }

    dmesg("starting coordinator".into());
    let mut child = Command::new("/coordinator")
        .spawn()
        .expect("failed to exec coordinator");

    match child.wait() {
        Ok(s) => dmesg(format!("coordinator exited: {s}")),
        Err(e) => eprintln!("wait: {e}"),
    }

    reboot();
}
