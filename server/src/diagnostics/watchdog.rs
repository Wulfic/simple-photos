//! N2 watchdog: a dedicated OS thread that records what the server looks like
//! when it stops answering HTTP while the process stays alive.
//!
//! It is deliberately NOT a tokio task — if the runtime's workers or its
//! blocking pool are the thing that wedged, a task would wedge with them and
//! the death would go unrecorded. Every [`INTERVAL`] it measures:
//!
//! - **async ping** — time for the runtime to run a spawned task (S2: workers
//!   blocked in sync I/O);
//! - **blocking ping** — time for `spawn_blocking` to run (S1: the blocking
//!   pool is exhausted, e.g. by stalled decrypt streams);
//! - **self-probe** — a plain `GET /health` on the server's own port (accept
//!   loop + full HTTP path; green while browsers time out ⇒ S4, outside the
//!   process);
//! - `/proc/self` threads, RSS and FD count; tokio task / queue counts; open
//!   requests by phase (S3: a handler wedged awaiting headers).
//!
//! An INFO heartbeat goes out every [`HEARTBEAT_EVERY`] ticks. Any anomaly is
//! a WARN at once, with a thread table (name, state, wchan) at most every
//! [`DUMP_EVERY`] while it lasts, and an INFO line when it clears.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::runtime::Handle;

use super::inflight::{self, InFlight, Phase, Registry};

pub const INTERVAL: Duration = Duration::from_secs(30);
/// 20 × 30 s = one heartbeat every 10 minutes.
pub const HEARTBEAT_EVERY: u64 = 20;
pub const DUMP_EVERY: Duration = Duration::from_secs(300);

const PING_TIMEOUT: Duration = Duration::from_secs(5);
const PING_SLOW: Duration = Duration::from_secs(1);
const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_SLOW: Duration = Duration::from_secs(5);
/// A handler still without headers after this long counts as wedged.
const STUCK_HANDLER: Duration = Duration::from_secs(30);
const OLDEST_SHOWN: usize = 5;

// ── Measurements ─────────────────────────────────────────────────────────

/// Time for the runtime to run a freshly spawned task; `None` = not within
/// `timeout`. A timed-out ping task stays queued and runs (harmlessly) once
/// the runtime recovers.
pub fn ping_async(handle: &Handle, timeout: Duration) -> Option<Duration> {
    let (tx, rx) = mpsc::sync_channel::<()>(1);
    let sent = Instant::now();
    handle.spawn(async move {
        // Err = the watchdog already gave up on this ping and reported it.
        let _ = tx.send(());
    });
    rx.recv_timeout(timeout).ok().map(|()| sent.elapsed())
}

/// Same as [`ping_async`] but through the blocking pool.
pub fn ping_blocking(handle: &Handle, timeout: Duration) -> Option<Duration> {
    let (tx, rx) = mpsc::sync_channel::<()>(1);
    let sent = Instant::now();
    handle.spawn_blocking(move || {
        // Err = the watchdog already gave up on this ping and reported it.
        let _ = tx.send(());
    });
    rx.recv_timeout(timeout).ok().map(|()| sent.elapsed())
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProbeOutcome {
    Ok {
        status: u16,
        connect: Duration,
        total: Duration,
    },
    ConnectFailed(String),
    NoResponse {
        connect: Duration,
        error: String,
    },
    Malformed {
        connect: Duration,
        first_line: String,
    },
}

/// Plain-HTTP `GET /health` against the server's own listener.
pub fn probe_health(
    addr: SocketAddr,
    connect_timeout: Duration,
    response_timeout: Duration,
) -> ProbeOutcome {
    let start = Instant::now();
    let mut stream = match TcpStream::connect_timeout(&addr, connect_timeout) {
        Ok(s) => s,
        Err(e) => return ProbeOutcome::ConnectFailed(e.to_string()),
    };
    let connect = start.elapsed();
    let no_response = |error: String| ProbeOutcome::NoResponse { connect, error };

    if let Err(e) = stream
        .set_read_timeout(Some(response_timeout))
        .and_then(|()| stream.set_write_timeout(Some(response_timeout)))
    {
        return no_response(format!("set timeout: {e}"));
    }
    let request = "GET /health HTTP/1.1\r\nHost: localhost\r\nUser-Agent: sp-watchdog\r\n\
                   Connection: close\r\n\r\n";
    if let Err(e) = stream.write_all(request.as_bytes()) {
        return no_response(format!("write: {e}"));
    }

    // Read until the status line is complete, the peer closes, or the overall
    // deadline passes (a per-read timeout alone would let a trickle run on).
    let deadline = start + connect_timeout + response_timeout;
    let mut buf = Vec::with_capacity(128);
    let mut chunk = [0u8; 128];
    while !buf.contains(&b'\n') && buf.len() < 1024 {
        if Instant::now() >= deadline {
            return no_response("deadline passed before the status line".into());
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => return no_response(format!("read: {e}")),
        }
    }
    let total = start.elapsed();
    let first_line = String::from_utf8_lossy(&buf)
        .lines()
        .next()
        .unwrap_or("")
        .to_owned();
    match parse_status_line(&first_line) {
        Some(status) => ProbeOutcome::Ok {
            status,
            connect,
            total,
        },
        None if buf.is_empty() => no_response("connection closed with no response".into()),
        None => ProbeOutcome::Malformed {
            connect,
            first_line: first_line.chars().take(80).collect(),
        },
    }
}

fn parse_status_line(line: &str) -> Option<u16> {
    let mut parts = line.split_whitespace();
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

/// The address to self-probe for a listener bound to `host:port`, or `None`
/// when no probe makes sense (TLS: the probe speaks plain HTTP).
pub fn probe_addr(host: &str, port: u16, tls: bool) -> Option<SocketAddr> {
    if tls {
        return None;
    }
    let ip: IpAddr = host.parse().ok()?;
    let ip = match ip {
        IpAddr::V4(v4) if v4.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        other => other,
    };
    Some(SocketAddr::new(ip, port))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcStats {
    pub threads: Option<u64>,
    pub rss_kib: Option<u64>,
    pub fds: Option<usize>,
}

/// `Threads:` and `VmRSS:` (kB) from a `/proc/<pid>/status` text.
pub fn parse_proc_status(text: &str) -> (Option<u64>, Option<u64>) {
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|n| n.parse().ok())
    };
    (field("Threads:"), field("VmRSS:"))
}

/// Linux only; every field is `None` elsewhere or when unreadable.
fn read_proc_stats() -> ProcStats {
    let (threads, rss_kib) = std::fs::read_to_string("/proc/self/status")
        .map(|t| parse_proc_status(&t))
        .unwrap_or((None, None));
    let fds = std::fs::read_dir("/proc/self/fd").ok().map(|d| d.count());
    ProcStats {
        threads,
        rss_kib,
        fds,
    }
}

/// `(comm, state)` from a `/proc/<pid>/task/<tid>/stat` line. `comm` is the
/// text between the first `(` and the LAST `)` — it may contain both.
pub fn parse_task_stat(text: &str) -> Option<(String, char)> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close <= open {
        return None;
    }
    let comm = text[open + 1..close].to_owned();
    let state = text[close + 1..].trim_start().chars().next()?;
    Some((comm, state))
}

/// Group threads by (name, state, wchan) — "500 × tokio-runtime-w S futex_wait"
/// reads at a glance where a per-thread list would not. Most frequent first.
pub fn group_threads(threads: &[(String, char, String)]) -> Vec<((String, char, String), usize)> {
    let mut groups: BTreeMap<(String, char, String), usize> = BTreeMap::new();
    for t in threads {
        *groups.entry(t.clone()).or_default() += 1;
    }
    let mut out: Vec<_> = groups.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// The thread table for an anomaly WARN; a one-line note when /proc is absent.
fn thread_table() -> String {
    let dir = match std::fs::read_dir("/proc/self/task") {
        Ok(d) => d,
        Err(e) => return format!("(no /proc/self/task: {e})"),
    };
    let mut threads = Vec::new();
    for entry in dir.flatten() {
        let p = entry.path();
        let Some((comm, state)) = std::fs::read_to_string(p.join("stat"))
            .ok()
            .and_then(|s| parse_task_stat(&s))
        else {
            continue; // the thread exited between read_dir and the read
        };
        let wchan = std::fs::read_to_string(p.join("wchan"))
            .map(|w| w.trim().to_owned())
            .ok()
            .filter(|w| !w.is_empty() && w != "0")
            .unwrap_or_else(|| "-".into());
        threads.push((comm, state, wchan));
    }
    let mut out = String::new();
    for ((comm, state, wchan), n) in group_threads(&threads) {
        let _ = writeln!(out, "  {n:>4} × {comm} {state} {wchan}");
    }
    out
}

// ── One sample + its verdict ─────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Sample {
    pub async_ping: Option<Duration>,
    pub blocking_ping: Option<Duration>,
    /// `None` = self-probe disabled (TLS, unparsable host).
    pub probe: Option<ProbeOutcome>,
    pub proc: ProcStats,
    pub alive_tasks: usize,
    pub global_queue: usize,
    pub in_flight: Vec<InFlight>,
    pub decrypt_streams: usize,
}

impl Sample {
    fn count(&self, phase: Phase) -> usize {
        self.in_flight.iter().filter(|r| r.phase == phase).count()
    }

    fn stuck_handlers(&self) -> usize {
        self.in_flight
            .iter()
            .filter(|r| r.phase == Phase::AwaitingHeaders && r.age >= STUCK_HANDLER)
            .count()
    }

    /// One line, every field, for the heartbeat and the WARN.
    pub fn summary(&self) -> String {
        let ms = |d: Option<Duration>| match d {
            Some(d) => format!("{:.1}ms", d.as_secs_f64() * 1000.0),
            None => "TIMEOUT".into(),
        };
        let opt = |v: Option<u64>| v.map_or("?".into(), |v| v.to_string());
        let probe = match &self.probe {
            None => "off".to_owned(),
            Some(ProbeOutcome::Ok { status, total, .. }) => {
                format!("{status} in {:.1}ms", total.as_secs_f64() * 1000.0)
            }
            Some(ProbeOutcome::ConnectFailed(e)) => format!("CONNECT FAILED ({e})"),
            Some(ProbeOutcome::NoResponse { connect, error }) => format!(
                "NO RESPONSE after connect in {:.1}ms ({error})",
                connect.as_secs_f64() * 1000.0
            ),
            Some(ProbeOutcome::Malformed { first_line, .. }) => {
                format!("MALFORMED {first_line:?}")
            }
        };
        let mut s = format!(
            "async_ping={} blocking_ping={} probe={} threads={} rss_mib={} fds={} \
             tasks={} global_queue={} in_flight={} (awaiting={} streaming={}) decrypt_streams={}",
            ms(self.async_ping),
            ms(self.blocking_ping),
            probe,
            opt(self.proc.threads),
            self.proc
                .rss_kib
                .map_or("?".into(), |k| (k / 1024).to_string()),
            self.proc.fds.map_or("?".into(), |n| n.to_string()),
            self.alive_tasks,
            self.global_queue,
            self.in_flight.len(),
            self.count(Phase::AwaitingHeaders),
            self.count(Phase::StreamingBody),
            self.decrypt_streams,
        );
        if !self.in_flight.is_empty() {
            s.push_str("; oldest:");
            for r in self.in_flight.iter().take(OLDEST_SHOWN) {
                let _ = write!(
                    s,
                    " [{} {} {}s {}]",
                    r.method,
                    r.path,
                    r.age.as_secs(),
                    r.phase.as_str()
                );
            }
        }
        s
    }
}

/// What is wrong with a sample; empty = healthy. Pure, so the thresholds are
/// tested without a wedged runtime.
pub fn anomalies(s: &Sample) -> Vec<String> {
    let mut out = Vec::new();
    let ping = |name: &str, d: Option<Duration>, out: &mut Vec<String>| match d {
        None => out.push(format!(
            "{name} did not run a task within {}s",
            PING_TIMEOUT.as_secs()
        )),
        Some(d) if d >= PING_SLOW => {
            out.push(format!("{name} took {:.1}s to run a task", d.as_secs_f64()))
        }
        Some(_) => {}
    };
    ping("async runtime", s.async_ping, &mut out);
    ping("blocking pool", s.blocking_ping, &mut out);

    match &s.probe {
        None => {}
        Some(ProbeOutcome::Ok { status, total, .. }) => {
            if *status != 200 {
                out.push(format!("self-probe /health answered {status}"));
            } else if *total >= PROBE_SLOW {
                out.push(format!(
                    "self-probe /health took {:.1}s",
                    total.as_secs_f64()
                ));
            }
        }
        Some(ProbeOutcome::ConnectFailed(e)) => out.push(format!("self-probe connect failed: {e}")),
        Some(ProbeOutcome::NoResponse { error, .. }) => {
            out.push(format!("self-probe got no response: {error}"))
        }
        Some(ProbeOutcome::Malformed { first_line, .. }) => {
            out.push(format!("self-probe malformed response: {first_line:?}"))
        }
    }

    let stuck = s.stuck_handlers();
    if stuck > 0 {
        out.push(format!(
            "{stuck} request(s) awaiting headers for over {}s",
            STUCK_HANDLER.as_secs()
        ));
    }
    out
}

// ── The thread ───────────────────────────────────────────────────────────

fn collect(handle: &Handle, probe: Option<SocketAddr>, registry: &Registry) -> Sample {
    let async_ping = ping_async(handle, PING_TIMEOUT);
    let blocking_ping = ping_blocking(handle, PING_TIMEOUT);
    let probe = probe.map(|addr| probe_health(addr, PROBE_CONNECT_TIMEOUT, PROBE_RESPONSE_TIMEOUT));
    let metrics = handle.metrics();
    Sample {
        async_ping,
        blocking_ping,
        probe,
        proc: read_proc_stats(),
        alive_tasks: metrics.num_alive_tasks(),
        global_queue: metrics.global_queue_depth(),
        in_flight: registry.snapshot(Instant::now()),
        decrypt_streams: inflight::decrypt_streams(),
    }
}

/// Start the watchdog thread. Failing to start is logged by the caller and
/// never stops the server.
pub fn spawn(
    handle: Handle,
    probe: Option<SocketAddr>,
    registry: Arc<Registry>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("sp-watchdog".into())
        .spawn(move || run(handle, probe, registry))
}

fn run(handle: Handle, probe: Option<SocketAddr>, registry: Arc<Registry>) {
    tracing::info!(
        probe = ?probe,
        interval_secs = INTERVAL.as_secs(),
        "N2 watchdog started"
    );
    let mut tick: u64 = 0;
    let mut anomalous_since: Option<Instant> = None;
    let mut last_dump: Option<Instant> = None;
    loop {
        let sample = collect(&handle, probe, &registry);
        let problems = anomalies(&sample);
        if problems.is_empty() {
            if let Some(since) = anomalous_since.take() {
                tracing::info!(
                    lasted_secs = since.elapsed().as_secs(),
                    "N2 watchdog: recovered; {}",
                    sample.summary()
                );
            } else if tick.is_multiple_of(HEARTBEAT_EVERY) {
                tracing::info!("N2 watchdog heartbeat: {}", sample.summary());
            }
        } else {
            let since = *anomalous_since.get_or_insert_with(Instant::now);
            tracing::warn!(
                anomalous_for_secs = since.elapsed().as_secs(),
                "N2 watchdog ANOMALY: {}; {}",
                problems.join("; "),
                sample.summary()
            );
            if last_dump.is_none_or(|t| t.elapsed() >= DUMP_EVERY) {
                tracing::warn!("N2 watchdog thread table:\n{}", thread_table());
                last_dump = Some(Instant::now());
            }
        }
        tick = tick.wrapping_add(1);
        std::thread::sleep(INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn healthy() -> Sample {
        Sample {
            async_ping: Some(Duration::from_micros(100)),
            blocking_ping: Some(Duration::from_micros(200)),
            probe: Some(ProbeOutcome::Ok {
                status: 200,
                connect: Duration::from_micros(50),
                total: Duration::from_millis(2),
            }),
            proc: ProcStats::default(),
            alive_tasks: 10,
            global_queue: 0,
            in_flight: Vec::new(),
            decrypt_streams: 0,
        }
    }

    fn req(phase: Phase, age_secs: u64) -> InFlight {
        InFlight {
            method: "GET".into(),
            path: "/api/x".into(),
            age: Duration::from_secs(age_secs),
            phase,
        }
    }

    #[test]
    fn healthy_sample_has_no_anomalies() {
        assert!(anomalies(&healthy()).is_empty());
    }

    #[test]
    fn each_failure_is_reported() {
        let mut s = healthy();
        s.async_ping = None;
        s.blocking_ping = Some(Duration::from_secs(2));
        s.probe = Some(ProbeOutcome::NoResponse {
            connect: Duration::from_millis(1),
            error: "read: timed out".into(),
        });
        s.in_flight = vec![
            req(Phase::AwaitingHeaders, 31),
            req(Phase::AwaitingHeaders, 5),
            req(Phase::StreamingBody, 3600),
        ];
        let a = anomalies(&s);
        assert_eq!(a.len(), 4, "{a:?}");
        assert!(a[0].contains("async runtime did not run"));
        assert!(a[1].contains("blocking pool took 2.0s"));
        assert!(a[2].contains("no response"));
        assert!(a[3].starts_with("1 request(s) awaiting headers"));
    }

    #[test]
    fn long_streams_are_not_anomalies_but_bad_probes_are() {
        let mut s = healthy();
        s.in_flight = vec![req(Phase::StreamingBody, 86_400)];
        assert!(anomalies(&s).is_empty(), "SSE / video streams are normal");

        s.probe = Some(ProbeOutcome::Ok {
            status: 503,
            connect: Duration::ZERO,
            total: Duration::ZERO,
        });
        assert_eq!(anomalies(&s), vec!["self-probe /health answered 503"]);

        s.probe = Some(ProbeOutcome::ConnectFailed("refused".into()));
        assert_eq!(anomalies(&s), vec!["self-probe connect failed: refused"]);

        s.probe = None;
        assert!(anomalies(&s).is_empty(), "probe off is not an anomaly");
    }

    #[test]
    fn summary_names_every_field_and_the_oldest_requests() {
        let mut s = healthy();
        s.async_ping = None;
        s.in_flight = vec![req(Phase::StreamingBody, 42)];
        let line = s.summary();
        for needle in [
            "async_ping=TIMEOUT",
            "blocking_ping=0.2ms",
            "probe=200 in 2.0ms",
            "threads=?",
            "in_flight=1 (awaiting=0 streaming=1)",
            "decrypt_streams=0",
            "[GET /api/x 42s streaming_body]",
        ] {
            assert!(line.contains(needle), "{needle:?} missing from {line}");
        }
    }

    #[test]
    fn proc_status_parsing() {
        let text = "Name:\tsimple-photos\nVmRSS:\t 1363148 kB\nThreads:\t24\n";
        assert_eq!(parse_proc_status(text), (Some(24), Some(1_363_148)));
        assert_eq!(parse_proc_status("Name:\tx\n"), (None, None));
    }

    #[test]
    fn task_stat_parsing_handles_parens_and_spaces_in_comm() {
        assert_eq!(
            parse_task_stat("1234 (tokio-runtime-w) S 1 1 1 0"),
            Some(("tokio-runtime-w".into(), 'S'))
        );
        assert_eq!(
            parse_task_stat("99 (a) b (c)) D 1 2"),
            Some(("a) b (c)".into(), 'D'))
        );
        assert_eq!(parse_task_stat("garbage"), None);
    }

    #[test]
    fn threads_are_grouped_most_frequent_first() {
        let t = |c: &str, s: char, w: &str| (c.to_owned(), s, w.to_owned());
        let groups = group_threads(&[
            t("main", 'S', "ep_poll"),
            t("tokio-runtime-w", 'S', "futex_wait"),
            t("tokio-runtime-w", 'S', "futex_wait"),
            t("tokio-runtime-w", 'D', "cifs_wait"),
        ]);
        assert_eq!(groups[0], (t("tokio-runtime-w", 'S', "futex_wait"), 2));
        assert_eq!(groups.len(), 3);
    }

    #[test]
    fn probe_addr_maps_wildcards_to_loopback_and_skips_tls() {
        assert_eq!(
            probe_addr("0.0.0.0", 3000, false),
            Some("127.0.0.1:3000".parse().unwrap())
        );
        assert_eq!(
            probe_addr("::", 3000, false),
            Some("[::1]:3000".parse().unwrap())
        );
        assert_eq!(
            probe_addr("192.168.1.5", 80, false),
            Some("192.168.1.5:80".parse().unwrap())
        );
        assert_eq!(probe_addr("0.0.0.0", 3000, true), None);
        assert_eq!(probe_addr("not-an-ip", 3000, false), None);
    }

    #[test]
    fn probe_reads_the_status_of_a_live_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf).unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        let out = probe_health(addr, Duration::from_secs(2), Duration::from_secs(2));
        server.join().unwrap();
        assert!(
            matches!(out, ProbeOutcome::Ok { status: 200, .. }),
            "{out:?}"
        );
    }

    #[test]
    fn probe_times_out_on_a_listener_that_accepts_but_never_answers() {
        // The N2 browser symptom: connection accepted, then nothing.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (hold_tx, hold_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (_s, _) = listener.accept().unwrap();
            let _ = hold_rx.recv(); // keep the socket open, say nothing
        });
        let started = Instant::now();
        let out = probe_health(addr, Duration::from_secs(2), Duration::from_millis(300));
        hold_tx.send(()).unwrap();
        server.join().unwrap();
        assert!(matches!(out, ProbeOutcome::NoResponse { .. }), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn probe_reports_a_refused_connection() {
        let addr = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        }; // listener dropped ⇒ port closed
        let out = probe_health(addr, Duration::from_secs(2), Duration::from_secs(1));
        assert!(matches!(out, ProbeOutcome::ConnectFailed(_)), "{out:?}");
    }

    #[test]
    fn pings_detect_a_runtime_whose_only_worker_is_blocked() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(1)
            .build()
            .unwrap();
        let handle = rt.handle().clone();
        assert!(ping_async(&handle, Duration::from_secs(2)).is_some());
        assert!(ping_blocking(&handle, Duration::from_secs(2)).is_some());

        // Wedge the only worker with sync blocking (S2) and the only blocking
        // thread with a parked send (S1).
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
        handle.spawn(async move {
            entered_tx.send(()).unwrap();
            let _ = release_rx.recv();
        });
        entered_rx.recv().unwrap();
        let (release_b_tx, release_b_rx) = std::sync::mpsc::channel::<()>();
        let (entered_b_tx, entered_b_rx) = std::sync::mpsc::channel::<()>();
        handle.spawn_blocking(move || {
            entered_b_tx.send(()).unwrap();
            let _ = release_b_rx.recv();
        });
        entered_b_rx.recv().unwrap();

        assert_eq!(ping_async(&handle, Duration::from_millis(300)), None);
        assert_eq!(ping_blocking(&handle, Duration::from_millis(300)), None);

        release_tx.send(()).unwrap();
        release_b_tx.send(()).unwrap();
        assert!(ping_async(&handle, Duration::from_secs(2)).is_some());
        assert!(ping_blocking(&handle, Duration::from_secs(2)).is_some());
        rt.shutdown_timeout(Duration::from_secs(1));
    }
}
