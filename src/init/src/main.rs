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

/// Tail a file and stream every new byte to a VSOCK peer.
///
/// Earlier versions wrapped the VSOCK fd in std::fs::File and used
/// write_all/flush. That hides socket-level errors (EPIPE on peer
/// close, SIGPIPE on signal) — sends after the host's socat child
/// exited just silently dropped. This version uses raw libc::send
/// with MSG_NOSIGNAL so we get -1 + errno back and can reconnect.
fn log_forwarder(path: &str, cid: u32, port: u32) {
    use std::io::{Read, Seek, SeekFrom};
    use std::time::Duration;

    let mut pos: u64 = 0;
    let mut buf = [0u8; 4096];

    'outer: loop {
        let sock_fd = match vsock_connect(cid, port) {
            Ok(fd) => fd,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        loop {
            let mut file = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
            };
            let size = file.metadata().map(|m| m.len()).unwrap_or(0);
            if size < pos {
                pos = 0;
            }
            if size <= pos {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            if file.seek(SeekFrom::Start(pos)).is_err() {
                pos = 0;
                continue;
            }
            let n = match file.read(&mut buf) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Ok(n) => n,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };

            // Send with MSG_NOSIGNAL — partial sends loop, peer close
            // gives -1/EPIPE which kicks us to reconnect.
            let mut sent = 0usize;
            while sent < n {
                let rc = unsafe {
                    libc::send(
                        sock_fd,
                        buf[sent..n].as_ptr() as *const libc::c_void,
                        n - sent,
                        libc::MSG_NOSIGNAL,
                    )
                };
                if rc < 0 {
                    unsafe { libc::close(sock_fd) };
                    std::thread::sleep(Duration::from_millis(200));
                    continue 'outer;
                }
                sent += rc as usize;
            }
            pos += n as u64;
        }
    }
}

/// Open a VSOCK SOCK_STREAM connection to (cid, port) and return the fd.
fn vsock_connect(cid: u32, port: u32) -> Result<i32, String> {
    use libc::{connect, sockaddr, sockaddr_vm, socket, AF_VSOCK, SOCK_STREAM};
    let fd = unsafe { socket(AF_VSOCK, SOCK_STREAM, 0) };
    if fd < 0 {
        return Err("vsock socket() failed".into());
    }
    let ret = unsafe {
        let mut sa: sockaddr_vm = std::mem::zeroed();
        sa.svm_family = AF_VSOCK as _;
        sa.svm_port = port;
        sa.svm_cid = cid;
        connect(
            fd,
            &sa as *const _ as *const sockaddr,
            std::mem::size_of::<sockaddr_vm>() as _,
        )
    };
    if ret < 0 {
        unsafe { libc::close(fd) };
        Err(format!("vsock connect({cid}, {port}) failed"))
    } else {
        Ok(fd)
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
    // Sidecar + coordinator share /tmp/coordinator.log so the existing
    // VSOCK:5000 relay surfaces both. Both processes MUST open the file
    // in O_APPEND mode — otherwise the second writer (without O_APPEND)
    // races against the first and silently overwrites its bytes, which
    // is why earlier logs ended abruptly at random offsets.
    //
    // Truncate once here so each enclave boot starts with a clean log.
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("/tmp/coordinator.log");

    let sidecar_log = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/coordinator.log")
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
    // Same log file as the sidecar; open with O_APPEND so atomic writes
    // from both processes interleave cleanly instead of racing.
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/coordinator.log")
        .expect("open coordinator log");
    let log2 = log.try_clone().expect("clone log fd");

    dmesg("starting coordinator".into());
    let mut child = Command::new("/coordinator")
        .stdout(log)
        .stderr(log2)
        .spawn()
        .expect("failed to spawn coordinator");

    // Forward enclave logs to the parent host via a native Rust thread
    // instead of `socat EXEC:tail`. Previous shell-stack approaches all
    // hit a buffering wall after the first chunk: tail's libc stdio went
    // block-buffered against socat's pipe and never flushed line-sized
    // writes. Reading the file directly and pushing to the VSOCK socket
    // ourselves means there's no userland buffer between writers and host.
    std::thread::spawn(|| log_forwarder("/tmp/coordinator.log", PARENT_CID, 5000));

    match child.wait() {
        Ok(s) => dmesg(format!("coordinator exited: {s}")),
        Err(e) => eprintln!("wait: {e}"),
    }

    // Allow the log-relay bridge to drain before the enclave shuts down.
    std::thread::sleep(std::time::Duration::from_secs(5));
    reboot();
}
