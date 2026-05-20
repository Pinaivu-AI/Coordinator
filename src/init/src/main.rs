//! Nitro Enclave init process — PID 1 inside the initramfs.
//!
//! Sequence:
//!   1. Mount pseudo-filesystems (proc, sys, dev, …)
//!   2. Reopen stdio on /dev/console
//!   3. Call aws::init_platform (NSM heartbeat → nitro-cli ready signal, insmod nsm.ko)
//!   4. Seed the kernel entropy pool
//!   5. Bring up loopback (127.0.0.1) — required for all TCP binds inside the enclave
//!   6. Wait for the parent to push config over VSOCK:7000 (KEY=VALUE lines)
//!   7. Start socat VSOCK↔TCP bridges
//!   8. Spawn the coordinator binary; tail its log file to parent VSOCK:5000

mod config;

use std::{
    fs::{File, OpenOptions},
    io::BufReader,
    os::unix::io::FromRawFd,
    process::Command,
};

use aws::{get_entropy, init_platform};
use system::{dmesg, freopen, mount, reboot, seed_entropy, vsock_accept};

/// CID of the parent partition — always 3 in the Nitro Enclave spec.
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
        ("cgroup_root", "/sys/fs/cgroup", "tmpfs", no_dse, "mode=0755"),
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

/// Bring up the loopback interface.
///
/// The kernel creates the `lo` device but leaves it down. Without it, any
/// TCP bind to 127.0.0.1 (coordinator, socat bridges) silently fails.
fn setup_loopback() {
    let _ = Command::new("/bin/busybox")
        .args(["ip", "addr", "add", "127.0.0.1/8", "dev", "lo"])
        .status();
    let _ = Command::new("/bin/busybox")
        .args(["ip", "link", "set", "dev", "lo", "up"])
        .status();
    let _ = std::fs::write("/etc/hosts", "127.0.0.1 localhost\n");
    dmesg("loopback up".into());
}

/// Spawn a socat bridge as a detached background process.
fn bridge(left: &str, right: &str) {
    match Command::new("/socat").arg(left).arg(right).spawn() {
        Ok(_) => dmesg(format!("bridge {left} ↔ {right}")),
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

    setup_loopback();

    dmesg("enclave booted".into());

    // ── Config injection via VSOCK:7000 ──────────────────────────────────────
    // The parent connects once after launching the enclave, sends KEY=VALUE
    // lines, and closes the connection. Secrets never appear in the EIF image.
    match vsock_accept(7000) {
        Ok(fd) => {
            let reader = BufReader::new(unsafe { File::from_raw_fd(fd) });
            let pairs = config::read_config(reader);
            for (k, v) in &pairs {
                std::env::set_var(k, v);
            }
            dmesg(format!("config injected: {} vars", pairs.len()));
        }
        Err(e) => eprintln!("config vsock: {e}"),
    }

    // ── Apply env var defaults ────────────────────────────────────────────────
    std::env::set_var("SSL_CERT_FILE", "/etc/ssl/certs/ca-certificates.crt");
    std::env::set_var("PATH", "/bin:/sbin:/usr/bin:/usr/sbin:/");
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

    // ── /etc/hosts overrides for TLS-SNI-sensitive upstreams ─────────────────
    // Postgres + Redis live behind a loopback socat bridge that forwards
    // VSOCK to the real host. If we connect by IP, the client sends SNI=IP
    // and TLS-terminating proxies (Supabase, Upstash) silently drop the
    // connection. By mapping the real hostnames to 127.0.0.1, the URL
    // hostname can stay correct while traffic still goes through the bridge.
    {
        let mut hosts = String::from("127.0.0.1 localhost\n");
        for var in ["POSTGRES_BRIDGE_HOST", "REDIS_BRIDGE_HOST"] {
            if let Ok(h) = std::env::var(var) {
                let h = h.trim();
                if !h.is_empty() {
                    hosts.push_str(&format!("127.0.0.1 {h}\n"));
                    dmesg(format!("/etc/hosts: 127.0.0.1 -> {h}"));
                }
            }
        }
        // Sidecar reaches Sui RPC by hostname — pull it out of SUI_RPC_URL
        // so we don't need a second env var.
        if let Ok(url) = std::env::var("SUI_RPC_URL") {
            if let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) {
                let host = rest.split('/').next().unwrap_or("").split(':').next().unwrap_or("");
                if !host.is_empty() {
                    hosts.push_str(&format!("127.0.0.1 {host}\n"));
                    dmesg(format!("/etc/hosts: 127.0.0.1 -> {host}"));
                }
            }
        }
        let _ = std::fs::write("/etc/hosts", hosts);
    }

    // ── Sidecar coordination ──────────────────────────────────────────────────
    // Sidecar runs on loopback inside the enclave and signs Sui PTBs
    // on the coordinator's behalf. Both processes share SIDECAR_SECRET
    // for the authenticated HTTP hop; generate one if config didn't push
    // a fixed value (rotates per enclave boot in that case).
    if std::env::var("SIDECAR_URL").is_err() {
        std::env::set_var("SIDECAR_URL", "http://127.0.0.1:8200");
    }
    if std::env::var("SIDECAR_SECRET").is_err() {
        let bytes = get_entropy(32).unwrap_or_default();
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in &bytes {
            hex.push_str(&format!("{:02x}", b));
        }
        std::env::set_var("SIDECAR_SECRET", hex);
    }

    // ── VSOCK↔TCP bridges ────────────────────────────────────────────────────
    // Outbound: coordinator reaches Postgres/Redis via local TCP;
    // parent forwards the VSOCK side to the real external hosts.
    bridge(
        "TCP-LISTEN:5432,reuseaddr,fork",
        &format!("VSOCK-CONNECT:{PARENT_CID}:8101"),
    );
    bridge(
        "TCP-LISTEN:6379,reuseaddr,fork",
        &format!("VSOCK-CONNECT:{PARENT_CID}:8102"),
    );
    // Sui RPC: sidecar dials https://<SUI_RPC_URL host>:443 → /etc/hosts maps
    // that to 127.0.0.1 → this bridge → VSOCK:8103 → parent → real Sui.
    bridge(
        "TCP-LISTEN:443,reuseaddr,fork",
        &format!("VSOCK-CONNECT:{PARENT_CID}:8103"),
    );
    // Inbound: parent delivers HTTP and libp2p traffic via VSOCK.
    bridge("VSOCK-LISTEN:4000,reuseaddr,fork", "TCP:127.0.0.1:4000");
    bridge("VSOCK-LISTEN:4001,reuseaddr,fork", "TCP:127.0.0.1:4001");

    // ── Spawn TS sidecar (Sui PTB signer) ─────────────────────────────────────
    // The node binary + npm-installed scripts live at /usr/local/bin
    // and /scripts respectively (see Containerfile). Sidecar inherits
    // SIDECAR_SECRET, OPERATOR_PRIVATE_KEY, and Pinaivu contract IDs
    // from the process env (populated by the VSOCK:7000 config push).
    let sidecar_log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("/tmp/sidecar.log")
        .expect("open sidecar log");
    let sidecar_log2 = sidecar_log.try_clone().expect("clone sidecar log fd");

    dmesg("starting sidecar".into());
    let _sidecar = Command::new("/usr/local/bin/node")
        .args(["/scripts/node_modules/tsx/dist/cli.mjs", "/scripts/sidecar-server.ts"])
        .env("LD_LIBRARY_PATH", "/usr/lib:/usr/local/lib")
        .env("NODE_PATH", "/scripts/node_modules")
        .stdout(sidecar_log)
        .stderr(sidecar_log2)
        .spawn()
        .expect("failed to spawn sidecar");

    // ── Spawn coordinator ─────────────────────────────────────────────────────
    // Redirect both streams to a temp file so we can tail it to the parent.
    let log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("/tmp/coordinator.log")
        .expect("open coordinator log");
    let log2 = log.try_clone().expect("clone log fd");

    dmesg("starting coordinator".into());
    let mut child = Command::new("/coordinator")
        .stdout(log)
        .stderr(log2)
        .spawn()
        .expect("failed to spawn coordinator");

    // Forward coordinator logs to the parent host (collects into /tmp/coordinator.log there).
    // Uses tail -f so the socat process stays alive as long as the coordinator writes.
    bridge(
        "EXEC:'/bin/busybox tail -f /tmp/coordinator.log'",
        &format!("VSOCK-CONNECT:{PARENT_CID}:5000"),
    );

    match child.wait() {
        Ok(s) => dmesg(format!("coordinator exited: {s}")),
        Err(e) => eprintln!("wait: {e}"),
    }

    // Allow the log-relay bridge to drain before the enclave shuts down.
    std::thread::sleep(std::time::Duration::from_secs(5));
    reboot();
}
