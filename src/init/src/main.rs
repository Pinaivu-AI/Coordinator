//! Nitro Enclave init process — PID 1 inside the initramfs.
//!
//! Sequence:
//!   1. Mount pseudo-filesystems (proc, sys, dev, …)
//!   2. Reopen stdio on /dev/console
//!   3. Call aws::init_platform (initialises the NSM driver)
//!   4. Seed the kernel entropy pool via the NSM RNG
//!   5. Load nsm.ko
//!   6. Spawn socat bridges so the coordinator can reach Postgres and Redis
//!      through the parent host's VSOCK-to-TCP forwarders, and so inbound
//!      HTTP + libp2p traffic reaches the coordinator from the parent.
//!   7. Exec the coordinator binary (replaces this process as PID 1).

use std::process::Command;

use aws::{get_entropy, init_platform};
use system::{dmesg, freopen, insmod, mount, reboot, seed_entropy};

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

    match insmod("/nsm.ko", "0") {
        Ok(()) => dmesg("nsm.ko loaded".into()),
        Err(e) => eprintln!("insmod nsm.ko: {e}"),
    }

    dmesg("pinaivu coordinator enclave booted".into());

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

    std::env::set_var("SSL_CERT_FILE", "/etc/ssl/certs/ca-certificates.crt");
    std::env::set_var("PATH", "/bin:/sbin:/usr/bin:/usr/sbin:/");
    std::env::set_var("PINAIVU_BIND", "127.0.0.1:4000");
    std::env::set_var("PINAIVU_LIBP2P_LISTEN", "/ip4/0.0.0.0/tcp/4001");
    // DATABASE_URL and REDIS_URL are injected from the config listener
    // (VSOCK:7000, slice 8) before this point; fall back to the VSOCK-bridged
    // local addresses if the env vars are not already set.
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
