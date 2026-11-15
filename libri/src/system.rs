//! The host's own state, read from /proc without a model: CPU, memory and its
//! pressure, swap, disks, network, Libri's cgroups, each worker's share and
//! the services the house leans on. The daemon samples it every few seconds,
//! keeps a bounded history for the System tile, and raises an alert note in
//! the main chat when a limit is crossed, once until it clears. It never
//! stops or kills anything.
//!
//! Workers are told apart by `LIBRI_WORKER` in a process's environment: Libri
//! sets it on every harness it starts for a worker, and everything the
//! harness starts inherits it (Paseo's Machine Status does the same with
//! `PASEO_AGENT_ID`).

use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How often the host is sampled.
const EVERY: Duration = Duration::from_secs(3);
/// How often the process table is read: dearer than the rest.
const PROCS_EVERY: Duration = Duration::from_secs(10);
/// How often services are asked after.
const SERVICES_EVERY: Duration = Duration::from_secs(30);
/// An hour of samples at full grain.
const FINE: usize = 1200;
/// Three days at one a minute.
const COARSE: usize = 3 * 24 * 60;
/// The services the house leans on, by unit name.
const SERVICES: [&str; 6] = [
    "frigate",
    "hob",
    "immich-server",
    "nix-daemon",
    "libri",
    "hippo2",
];
/// The mounts watched for space.
const MOUNTS: [&str; 3] = ["/", "/nix", "/srv/storage"];
/// Where the raised alerts are kept for the `system` tool.
const ALERTS: &str = "system-alerts.json";
/// The locks Libri's checks take.
const LOCKS: [&str; 2] = ["/tmp/libri-check.lock", "/tmp/libri-build.lock"];

/// One moment of the host.
#[derive(Clone, Copy, Default, Serialize)]
pub struct Sample {
    /// Unix seconds.
    pub t: i64,
    /// CPU in use, 0-100, over the last interval.
    pub cpu: f32,
    pub load1: f32,
    pub mem_total: u64,
    pub mem_available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    /// Share of the last 10 s some tasks stalled on memory, and all of them.
    pub mem_some: f32,
    pub mem_full: f32,
    pub cpu_some: f32,
    pub io_full: f32,
    /// Bytes a second.
    pub net_rx: u64,
    pub net_tx: u64,
    pub disk_read: u64,
    pub disk_write: u64,
}

#[derive(Clone, Serialize)]
pub struct Disk {
    pub mount: String,
    pub total: u64,
    pub available: u64,
}

#[derive(Clone, Serialize)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    /// CPU over the last process read, in cores (1.0 is one core busy).
    pub cpu: f32,
    pub rss: u64,
    pub worker: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct WorkerUse {
    pub worker: String,
    pub cpu: f32,
    pub rss: u64,
    pub procs: u32,
}

#[derive(Clone, Serialize, serde::Deserialize)]
pub struct Alert {
    pub key: String,
    pub text: String,
    pub since: i64,
}

/// What the tile and the tool show.
#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub now: Sample,
    pub cores: usize,
    pub disks: Vec<Disk>,
    /// Memory of Libri's own cgroups: the daemon, its harnesses, its browser.
    pub libri: HashMap<String, u64>,
    pub workers: Vec<WorkerUse>,
    pub top: Vec<Proc>,
    pub services: Vec<(String, String)>,
    pub alerts: Vec<Alert>,
    /// The daemon's longest stall since the last sample, in ms.
    pub stall_ms: u64,
}

#[derive(Serialize)]
pub struct Report<'a> {
    #[serde(flatten)]
    pub snapshot: &'a Snapshot,
    pub fine: Vec<Sample>,
    pub coarse: Vec<Sample>,
}

/// Counters whose differences make rates.
#[derive(Default, Clone, Copy)]
struct Counters {
    at: Option<Instant>,
    cpu_busy: u64,
    cpu_all: u64,
    rx: u64,
    tx: u64,
    read: u64,
    written: u64,
}

/// A process's CPU ticks when last read, and who it belongs to.
struct Seen {
    ticks: u64,
    start: u64,
    ppid: u32,
    name: String,
    worker: Option<String>,
}

#[derive(Default)]
struct Rules {
    /// When each condition began, while it holds.
    began: HashMap<String, Instant>,
    /// The alerts raised and not yet cleared.
    raised: HashMap<String, Alert>,
}

pub struct Monitor {
    snapshot: Snapshot,
    fine: VecDeque<Sample>,
    coarse: VecDeque<Sample>,
    counters: Counters,
    procs: HashMap<u32, Seen>,
    procs_at: Option<Instant>,
    services_at: Option<Instant>,
    /// Lock holders: pid with when first seen holding and its CPU ticks then.
    holders: HashMap<u32, (Instant, u64)>,
    rules: Rules,
    last_coarse: i64,
    /// Conditions found on the slower process read, kept until the next.
    slow: Vec<Condition>,
}

/// The daemon's longest event-loop stall since the sampler last read it, ms.
pub static STALL_MS: AtomicU64 = AtomicU64::new(0);

pub type Shared = Arc<Mutex<Monitor>>;

impl Monitor {
    pub fn new() -> Shared {
        Arc::new(Mutex::new(Monitor {
            snapshot: Snapshot::default(),
            fine: VecDeque::with_capacity(FINE),
            coarse: VecDeque::with_capacity(COARSE),
            counters: Counters::default(),
            procs: HashMap::new(),
            procs_at: None,
            services_at: None,
            holders: HashMap::new(),
            rules: Rules::default(),
            last_coarse: 0,
            slow: Vec::new(),
        }))
    }

    pub fn report(&self) -> String {
        serde_json::to_string(&Report {
            snapshot: &self.snapshot,
            fine: self.fine.iter().copied().collect(),
            coarse: self.coarse.iter().copied().collect(),
        })
        .unwrap_or_default()
    }
}

/// Starts the sampler and the stall watch. Alerts go to the main chat's
/// inbox, as a note from the system monitor.
pub fn start(monitor: Shared, state: PathBuf) {
    // A task that should wake every half second, and how late it wakes, is
    // how long the runtime was kept from it (T3 Code's event-loop monitor).
    tokio::spawn(async {
        let mut at = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let late = at.elapsed().saturating_sub(Duration::from_millis(500));
            STALL_MS.fetch_max(late.as_millis() as u64, Ordering::Relaxed);
            at = Instant::now();
        }
    });
    // A check's throwaway Libri shares the host it runs on: the host's
    // pressure is not its news, and a note in its main chat starts a turn
    // its counts don't expect. LIBRI_SYSTEM_ALERTS=0 keeps the alerts on
    // the tile and out of the chat.
    let tell = std::env::var("LIBRI_SYSTEM_ALERTS").map_or(true, |v| v != "0");
    std::thread::Builder::new()
        .name("system".into())
        .spawn(move || {
            let mut shown = String::new();
            loop {
                let (notes, alerts) = {
                    let mut m = monitor.lock().unwrap_or_else(|p| p.into_inner());
                    let notes = m.sample();
                    (
                        notes,
                        serde_json::to_string(&m.snapshot.alerts).unwrap_or_default(),
                    )
                };
                // The tool runs in another process: it reads the alerts here.
                if alerts != shown {
                    let _ = fs::write(state.join(ALERTS), &alerts);
                    shown = alerts;
                }
                for text in notes.into_iter().filter(|_| tell) {
                    let _ = crate::a2a::leave(&crate::a2a::inbox(&state), "System monitor", &text);
                }
                std::thread::sleep(EVERY);
            }
        })
        .ok();
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Monitor {
    /// One sample; returns the alert notes newly raised.
    fn sample(&mut self) -> Vec<String> {
        let at = Instant::now();
        let mut s = Sample {
            t: now(),
            ..Sample::default()
        };
        let mem = read_kv("/proc/meminfo");
        s.mem_total = mem.get("MemTotal").copied().unwrap_or(0) * 1024;
        s.mem_available = mem.get("MemAvailable").copied().unwrap_or(0) * 1024;
        s.swap_total = mem.get("SwapTotal").copied().unwrap_or(0) * 1024;
        s.swap_used = s
            .swap_total
            .saturating_sub(mem.get("SwapFree").copied().unwrap_or(0) * 1024);
        s.load1 = fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|l| l.split_whitespace().next()?.parse().ok())
            .unwrap_or(0.0);
        (s.mem_some, s.mem_full) = pressure("memory");
        (s.cpu_some, _) = pressure("cpu");
        (_, s.io_full) = pressure("io");

        let (busy, all) = cpu_ticks();
        let (rx, tx) = net_bytes();
        let (read, written) = disk_bytes();
        let c = self.counters;
        if let Some(prev) = c.at {
            let secs = at.duration_since(prev).as_secs_f64().max(0.001);
            let rate = |a: u64, b: u64| (a.saturating_sub(b) as f64 / secs) as u64;
            let d_all = all.saturating_sub(c.cpu_all);
            if d_all > 0 {
                s.cpu = (busy.saturating_sub(c.cpu_busy) as f32 / d_all as f32 * 100.0).min(100.0);
            }
            s.net_rx = rate(rx, c.rx);
            s.net_tx = rate(tx, c.tx);
            s.disk_read = rate(read, c.read);
            s.disk_write = rate(written, c.written);
        }
        self.counters = Counters {
            at: Some(at),
            cpu_busy: busy,
            cpu_all: all,
            rx,
            tx,
            read,
            written,
        };

        let snap = &mut self.snapshot;
        snap.now = s;
        snap.cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        snap.disks = MOUNTS.iter().filter_map(|m| disk(m)).collect();
        snap.libri = libri_cgroups();
        snap.stall_ms = STALL_MS.swap(0, Ordering::Relaxed);

        if self.procs_at.is_none_or(|p| p.elapsed() >= PROCS_EVERY) {
            self.read_procs();
            self.procs_at = Some(at);
            let mut slow = self.lock_conditions();
            slow.extend(self.long_runs());
            self.slow = slow;
        }
        if self
            .services_at
            .is_none_or(|p| p.elapsed() >= SERVICES_EVERY)
        {
            self.snapshot.services = SERVICES
                .iter()
                .map(|u| ((*u).to_string(), unit_state(u)))
                .collect();
            self.services_at = Some(at);
        }

        if self.fine.len() == FINE {
            self.fine.pop_front();
        }
        self.fine.push_back(s);
        if s.t - self.last_coarse >= 60 {
            if self.coarse.len() == COARSE {
                self.coarse.pop_front();
            }
            self.coarse.push_back(s);
            self.last_coarse = s.t;
        }
        let conditions = self.conditions();
        self.judge(conditions)
    }

    /// Reads the process table: each process's CPU since the last read, its
    /// memory, and its worker.
    fn read_procs(&mut self) {
        let secs = self
            .procs_at
            .map_or(PROCS_EVERY.as_secs_f32(), |p| p.elapsed().as_secs_f32())
            .max(0.001);
        let hz = clock_ticks();
        let page = page_size();
        let mut alive = HashSet::new();
        let mut all = Vec::new();
        let Ok(dir) = fs::read_dir("/proc") else {
            return;
        };
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(line) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let Some(st) = parse_stat(&line) else {
                continue;
            };
            alive.insert(pid);
            let seen = self.procs.get(&pid).filter(|s| s.start == st.start);
            let cpu = seen.map_or(0.0, |s| st.ticks.saturating_sub(s.ticks) as f32 / hz / secs);
            let worker = match seen {
                Some(s) => s.worker.clone(),
                None => worker_of(pid),
            };
            self.procs.insert(
                pid,
                Seen {
                    ticks: st.ticks,
                    start: st.start,
                    ppid: ppid(&line).unwrap_or(0),
                    name: st.name.clone(),
                    worker: worker.clone(),
                },
            );
            all.push(Proc {
                pid,
                name: st.name,
                cpu,
                rss: st.rss_pages * page,
                worker,
            });
        }
        self.procs.retain(|pid, _| alive.contains(pid));
        let mut workers: HashMap<String, WorkerUse> = HashMap::new();
        for p in &all {
            if let Some(w) = &p.worker {
                let u = workers.entry(w.clone()).or_insert_with(|| WorkerUse {
                    worker: w.clone(),
                    cpu: 0.0,
                    rss: 0,
                    procs: 0,
                });
                u.cpu += p.cpu;
                u.rss += p.rss;
                u.procs += 1;
            }
        }
        let mut workers: Vec<_> = workers.into_values().collect();
        workers.sort_by_key(|a| std::cmp::Reverse(a.rss));
        self.snapshot.workers = workers;
        let mut by_cpu = all.clone();
        by_cpu.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        let mut by_mem = all;
        by_mem.sort_by_key(|a| std::cmp::Reverse(a.rss));
        // The busiest by CPU and the largest by memory; an idle process
        // ranks only by its memory, so the first read (no CPU yet) lists the
        // largest rather than the lowest process ids.
        let mut top: Vec<Proc> = by_cpu
            .into_iter()
            .take_while(|p| p.cpu >= 0.05)
            .take(8)
            .collect();
        for p in by_mem.into_iter().take(8) {
            if !top.iter().any(|t| t.pid == p.pid) {
                top.push(p);
            }
        }
        self.snapshot.top = top;
    }

    /// The conditions that hold now, by key, with their text.
    fn conditions(&mut self) -> Vec<Condition> {
        let s = self.snapshot.now;
        let mut out = Vec::new();
        if s.mem_full >= 10.0 {
            out.push(Condition::sustained(
                "memory",
                Duration::from_secs(120),
                format!(
                    "{} is short of memory: all tasks stalled on memory {:.0}% of the last 10 s, {} available of {}.",
                    host(),
                    s.mem_full,
                    gib(s.mem_available),
                    gib(s.mem_total)
                ),
            ));
        }
        if s.swap_total > 0 && s.swap_used * 4 > s.swap_total {
            out.push(Condition::sustained(
                "swap",
                Duration::from_secs(120),
                format!(
                    "{} is swapping: {} of {} swap in use.",
                    host(),
                    gib(s.swap_used),
                    gib(s.swap_total)
                ),
            ));
        }
        for d in &self.snapshot.disks {
            if nearly_full(d) {
                out.push(Condition::now(
                    &format!("disk {}", d.mount),
                    format!(
                        "{} is nearly full: {} free of {}.",
                        d.mount,
                        gib(d.available),
                        gib(d.total)
                    ),
                ));
            }
        }
        if self.snapshot.stall_ms >= 2000 {
            out.push(Condition::now(
                "stall",
                format!(
                    "Libri itself stalled for {:.1} s.",
                    self.snapshot.stall_ms as f64 / 1000.0
                ),
            ));
        }
        out.extend(self.slow.iter().map(|c| Condition {
            key: c.key.clone(),
            text: c.text.clone(),
            after: c.after,
        }));
        out
    }

    /// A lock held over 30 minutes by a process tree that used almost no CPU
    /// in that time.
    fn lock_conditions(&mut self) -> Vec<Condition> {
        let holders = lock_holders();
        self.holders
            .retain(|pid, _| holders.iter().any(|(_, p)| p == pid));
        let mut out = Vec::new();
        for (lock, pid) in holders {
            let ticks = self.tree_ticks(pid);
            let (since, start_ticks) = *self.holders.entry(pid).or_insert((Instant::now(), ticks));
            let held = since.elapsed();
            if held >= Duration::from_secs(30 * 60) {
                let used = ticks.saturating_sub(start_ticks) as f32 / clock_ticks();
                if used < 2.0 {
                    out.push(Condition::now(
                        &format!("lock {lock} {pid}"),
                        format!(
                            "{lock} has been held for {} min by pid {pid} ({}), whose processes used {used:.1} s of CPU in that time: it looks stuck.",
                            held.as_secs() / 60,
                            self.procs.get(&pid).map(|s| s.name.clone()).unwrap_or_default()
                        ),
                    ));
                }
                // Restart the window so "stuck" means the last 30 minutes.
                if held >= Duration::from_secs(60 * 60) {
                    self.holders
                        .insert(pid, (Instant::now() - Duration::from_secs(30 * 60), ticks));
                }
            }
        }
        out
    }

    /// Raises newly held conditions and clears gone ones.
    fn judge(&mut self, held: Vec<Condition>) -> Vec<String> {
        let mut notes = Vec::new();
        let keys: HashSet<String> = held.iter().map(|c| c.key.clone()).collect();
        self.rules.began.retain(|k, _| keys.contains(k));
        self.rules.raised.retain(|k, _| keys.contains(k));
        for c in held {
            let began = *self
                .rules
                .began
                .entry(c.key.clone())
                .or_insert_with(Instant::now);
            if began.elapsed() < c.after || self.rules.raised.contains_key(&c.key) {
                continue;
            }
            notes.push(format!(
                "{} (System monitor; it won't repeat until this clears.)",
                c.text
            ));
            self.rules.raised.insert(
                c.key.clone(),
                Alert {
                    key: c.key,
                    text: c.text,
                    since: now(),
                },
            );
        }
        self.snapshot.alerts = self.rules.raised.values().cloned().collect();
        notes
    }
}

impl Monitor {
    /// CPU ticks of a process and everything below it, from the last read.
    fn tree_ticks(&self, root: u32) -> u64 {
        self.procs
            .iter()
            .filter(|(pid, _)| {
                let mut p = **pid;
                for _ in 0..64 {
                    if p == root {
                        return true;
                    }
                    match self.procs.get(&p) {
                        Some(s) if s.ppid != 0 => p = s.ppid,
                        _ => return false,
                    }
                }
                false
            })
            .map(|(_, s)| s.ticks)
            .sum()
    }

    /// A release or a 400 ms gate running over 90 minutes; only shells and
    /// Pythons have their command lines read.
    fn long_runs(&self) -> Vec<Condition> {
        let boot = boot_time();
        let hz = clock_ticks() as u64;
        let mut out = Vec::new();
        for (pid, seen) in &self.procs {
            if !(seen.name.starts_with("bash") || seen.name.starts_with("python")) {
                continue;
            }
            let Ok(cmd) = fs::read(format!("/proc/{pid}/cmdline")) else {
                continue;
            };
            let cmd = String::from_utf8_lossy(&cmd).replace('\0', " ");
            let what = if cmd.contains("release_libri.sh") {
                "A Libri release"
            } else if cmd.contains("check_ui_latency.py") && !cmd.contains("--part") {
                "The 400 ms gate"
            } else {
                continue;
            };
            let started = boot + seen.start / hz;
            let mins = (now() as u64).saturating_sub(started) / 60;
            if mins >= 90 {
                out.push(Condition::now(
                    &format!("run {pid}"),
                    format!("{what} (pid {pid}) has been running for {mins} min."),
                ));
            }
        }
        out
    }
}

struct Condition {
    key: String,
    text: String,
    after: Duration,
}

impl Condition {
    fn now(key: &str, text: String) -> Self {
        Self::sustained(key, Duration::ZERO, text)
    }
    fn sustained(key: &str, after: Duration, text: String) -> Self {
        Condition {
            key: key.into(),
            text,
            after,
        }
    }
}

struct Stat {
    name: String,
    ticks: u64,
    start: u64,
    rss_pages: u64,
}

/// /proc/<pid>/stat: the name sits in parentheses and may hold spaces.
fn parse_stat(line: &str) -> Option<Stat> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    let name = line[open + 1..close].to_string();
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    // Fields from 3 (state) on: utime is 14, stime 15, starttime 22, rss 24.
    let f = |n: usize| rest.get(n - 3)?.parse::<u64>().ok();
    Some(Stat {
        name,
        ticks: f(14)? + f(15)?,
        start: f(22)?,
        rss_pages: f(24)?,
    })
}

/// The worker a process belongs to, from its environment: only readable for
/// the daemon's own user's processes, which the workers' are.
fn worker_of(pid: u32) -> Option<String> {
    let env = fs::read(format!("/proc/{pid}/environ")).ok()?;
    env.split(|b| *b == 0)
        .find_map(|kv| kv.strip_prefix(b"LIBRI_WORKER="))
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .filter(|v| !v.is_empty())
}

fn ppid(line: &str) -> Option<u32> {
    let close = line.rfind(')')?;
    line[close + 1..].split_whitespace().nth(1)?.parse().ok()
}

/// Who holds each of Libri's check locks, from /proc/locks by inode.
fn lock_holders() -> Vec<(String, u32)> {
    use std::os::unix::fs::MetadataExt;
    let inodes: Vec<(String, u64)> = LOCKS
        .iter()
        .filter_map(|p| Some(((*p).to_string(), fs::metadata(p).ok()?.ino())))
        .collect();
    if inodes.is_empty() {
        return Vec::new();
    }
    let locks = fs::read_to_string("/proc/locks").unwrap_or_default();
    parse_locks(&locks, &inodes)
}

/// Lines like `1: FLOCK  ADVISORY  WRITE 4242 00:2a:1234 0 EOF`; waiters
/// (`->`) are left out.
fn parse_locks(locks: &str, inodes: &[(String, u64)]) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for line in locks.lines() {
        if line.contains("->") {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        let (Some(pid), Some(dev)) = (f.get(4), f.get(5)) else {
            continue;
        };
        let Some(ino) = dev.rsplit(':').next().and_then(|i| i.parse::<u64>().ok()) else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        for (path, i) in inodes {
            if *i == ino && !out.iter().any(|(p, q)| p == path && *q == pid) {
                out.push((path.clone(), pid));
            }
        }
    }
    out
}

fn read_kv(path: &str) -> HashMap<String, u64> {
    parse_kv(&fs::read_to_string(path).unwrap_or_default())
}

fn parse_kv(text: &str) -> HashMap<String, u64> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((
                k.trim().to_string(),
                v.split_whitespace().next()?.parse().ok()?,
            ))
        })
        .collect()
}

/// Under 5% free, or under 10 GiB free and under 20% free: the 10 GiB floor
/// catches a large disk filling fast, and the 20% bound keeps it from calling
/// a small disk full (a 1 GB partition with 0.9 GB free is not).
fn nearly_full(d: &Disk) -> bool {
    d.total > 0
        && (d.available * 20 < d.total || (d.available < 10 << 30 && d.available * 5 < d.total))
}

/// /proc/pressure/<what>: `some avg10` and `full avg10`, percent.
fn pressure(what: &str) -> (f32, f32) {
    parse_pressure(&fs::read_to_string(format!("/proc/pressure/{what}")).unwrap_or_default())
}

fn parse_pressure(text: &str) -> (f32, f32) {
    let avg10 = |kind: &str| {
        text.lines()
            .find(|l| l.starts_with(kind))
            .and_then(|l| l.split_whitespace().find_map(|w| w.strip_prefix("avg10=")))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0)
    };
    (avg10("some"), avg10("full"))
}

fn cpu_ticks() -> (u64, u64) {
    let text = fs::read_to_string("/proc/stat").unwrap_or_default();
    parse_cpu(&text)
}

/// The first `cpu` line: busy is everything but idle and iowait.
fn parse_cpu(text: &str) -> (u64, u64) {
    let Some(line) = text.lines().find(|l| l.starts_with("cpu ")) else {
        return (0, 0);
    };
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    // user nice system idle iowait irq softirq steal (guests are inside user).
    let all: u64 = v.iter().take(8).sum();
    let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
    (all.saturating_sub(idle), all)
}

fn net_bytes() -> (u64, u64) {
    parse_net(&fs::read_to_string("/proc/net/dev").unwrap_or_default())
}

/// Every interface but loopback.
fn parse_net(text: &str) -> (u64, u64) {
    let mut rx = 0;
    let mut tx = 0;
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        if name.trim() == "lo" {
            continue;
        }
        let v: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        rx += v.first().copied().unwrap_or(0);
        tx += v.get(8).copied().unwrap_or(0);
    }
    (rx, tx)
}

fn disk_bytes() -> (u64, u64) {
    parse_disks(&fs::read_to_string("/proc/diskstats").unwrap_or_default())
}

/// Whole disks only (nvme0n1, sda, vda), not partitions or zram, in 512-byte
/// sectors.
fn parse_disks(text: &str) -> (u64, u64) {
    let mut read = 0;
    let mut written = 0;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some(name) = f.get(2) else { continue };
        let whole = (name.starts_with("nvme") && !name.contains('p'))
            || ((name.starts_with("sd") || name.starts_with("vd"))
                && !name.chars().last().is_some_and(|c| c.is_ascii_digit()));
        if !whole {
            continue;
        }
        let n = |i: usize| f.get(i).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
        read += n(5) * 512;
        written += n(9) * 512;
    }
    (read, written)
}

fn disk(mount: &str) -> Option<Disk> {
    let path = std::ffi::CString::new(mount).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid C string and a zeroed statvfs for the call to fill.
    if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let block = st.f_frsize as u64;
    Some(Disk {
        mount: mount.into(),
        total: st.f_blocks as u64 * block,
        available: st.f_bavail as u64 * block,
    })
}

/// Memory of Libri's cgroups (cgroup.rs makes them beside the daemon's).
fn libri_cgroups() -> HashMap<String, u64> {
    let own = fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    let Some(rel) = own.lines().find_map(|l| l.strip_prefix("0::")) else {
        return HashMap::new();
    };
    let leaf = Path::new("/sys/fs/cgroup").join(rel.trim_start_matches('/'));
    let Some(unit) = leaf.parent() else {
        return HashMap::new();
    };
    ["server", "harness", "browser"]
        .iter()
        .filter_map(|n| {
            let v = fs::read_to_string(unit.join(n).join("memory.current")).ok()?;
            Some(((*n).to_string(), v.trim().parse().ok()?))
        })
        .collect()
}

fn unit_state(unit: &str) -> String {
    std::process::Command::new("systemctl")
        .args(["is-active", unit])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn boot_time() -> u64 {
    fs::read_to_string("/proc/stat")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())
        .unwrap_or(0)
}

fn clock_ticks() -> f32 {
    // SAFETY: sysconf reads a constant.
    (unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).max(1) as f32
}

fn page_size() -> u64 {
    // SAFETY: sysconf reads a constant.
    (unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).max(1) as u64
}

/// The machine's name, as the alerts call it.
fn host() -> String {
    let name = fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    let name = name.trim();
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => "The host".into(),
    }
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

/// A snapshot taken on the spot, for the `system` tool in its own process:
/// two samples a second apart, with the daemon's raised alerts.
pub fn now_report(state: &Path) -> String {
    let m = Monitor::new();
    let mut m = m.lock().unwrap_or_else(|p| p.into_inner());
    m.sample();
    std::thread::sleep(Duration::from_secs(1));
    m.procs_at = None;
    m.sample();
    m.snapshot.alerts = fs::read_to_string(state.join(ALERTS))
        .ok()
        .and_then(|a| serde_json::from_str(&a).ok())
        .unwrap_or_default();
    let r: serde_json::Value = serde_json::from_str(&m.report()).unwrap_or_default();
    summary(&r)
}

/// The snapshot in a few lines, for the `system` tool.
pub fn summary(report: &serde_json::Value) -> String {
    let n = &report["now"];
    let g = |k: &str| n[k].as_u64().unwrap_or(0);
    let f = |k: &str| n[k].as_f64().unwrap_or(0.0);
    let mut out = vec![format!(
        "CPU {:.0}% of {} cores, load {:.1}; memory {} available of {} (stalled {:.0}% some, {:.0}% full over 10 s); swap {} of {}.",
        f("cpu"),
        report["cores"].as_u64().unwrap_or(0),
        f("load1"),
        gib(g("mem_available")),
        gib(g("mem_total")),
        f("mem_some"),
        f("mem_full"),
        gib(g("swap_used")),
        gib(g("swap_total")),
    )];
    if let Some(d) = report["disks"].as_array() {
        out.push(
            d.iter()
                .map(|d| {
                    format!(
                        "{} {} free",
                        d["mount"].as_str().unwrap_or(""),
                        gib(d["available"].as_u64().unwrap_or(0))
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if let Some(l) = report["libri"].as_object() {
        out.push(format!(
            "Libri: daemon {}, harnesses {}, browser {}.",
            gib(l.get("server").and_then(|v| v.as_u64()).unwrap_or(0)),
            gib(l.get("harness").and_then(|v| v.as_u64()).unwrap_or(0)),
            gib(l.get("browser").and_then(|v| v.as_u64()).unwrap_or(0)),
        ));
    }
    if let Some(w) = report["workers"].as_array().filter(|w| !w.is_empty()) {
        out.push(format!(
            "Workers: {}.",
            w.iter()
                .map(|w| format!(
                    "{} {:.1} cores {}",
                    w["worker"].as_str().unwrap_or(""),
                    w["cpu"].as_f64().unwrap_or(0.0),
                    gib(w["rss"].as_u64().unwrap_or(0))
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    if let Some(a) = report["alerts"].as_array().filter(|a| !a.is_empty()) {
        out.push(format!(
            "Alerts: {}",
            a.iter()
                .filter_map(|a| a["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_stat_line_whose_name_has_spaces() {
        let line = "4242 (Web Content) S 1 4242 4242 0 -1 4194560 100 0 0 0 150 50 0 0 20 0 30 0 123456 1000000 2560 18446744073709551615";
        let s = parse_stat(line).unwrap();
        assert_eq!(s.name, "Web Content");
        assert_eq!(s.ticks, 200);
        assert_eq!(s.start, 123456);
        assert_eq!(s.rss_pages, 2560);
        assert_eq!(ppid(line), Some(1));
    }

    #[test]
    fn reads_pressure_and_cpu_and_memory() {
        let p = "some avg10=12.50 avg60=3.00 avg300=1.00 total=1\nfull avg10=4.25 avg60=1.00 avg300=0.10 total=1\n";
        assert_eq!(parse_pressure(p), (12.5, 4.25));
        let (busy, all) = parse_cpu("cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3\n");
        assert_eq!((busy, all), (150, 1000));
        let kv = parse_kv("MemTotal:       97000000 kB\nMemAvailable:   65000000 kB\n");
        assert_eq!(kv["MemAvailable"], 65000000);
    }

    #[test]
    fn counts_whole_disks_and_real_interfaces() {
        let d = " 259 0 nvme0n1 10 0 200 0 5 0 100 0\n 259 1 nvme0n1p1 9 0 199 0 5 0 100 0\n 252 0 zram0 1 0 1000 0 1 0 1000 0\n";
        assert_eq!(parse_disks(d), (200 * 512, 100 * 512));
        let n = "h\nh\n    lo: 999 0 0 0 0 0 0 0 999\n  eth0: 100 0 0 0 0 0 0 0 40 0\n";
        assert_eq!(parse_net(n), (100, 40));
    }

    #[test]
    fn finds_lock_holders_not_waiters() {
        let locks = "1: FLOCK  ADVISORY  WRITE 4242 00:2a:777 0 EOF\n1: -> FLOCK  ADVISORY  WRITE 5151 00:2a:777 0 EOF\n2: FLOCK  ADVISORY  READ 6161 00:2a:888 0 EOF\n";
        let got = parse_locks(locks, &[("check".into(), 777), ("build".into(), 888)]);
        assert_eq!(got, vec![("check".into(), 4242), ("build".into(), 6161)]);
    }

    #[test]
    fn calls_a_disk_nearly_full_by_its_share_free() {
        let disk = |total: u64, available: u64| Disk {
            mount: "/".into(),
            total,
            available,
        };
        const GIB: u64 = 1 << 30;
        assert!(
            !nearly_full(&disk(GIB, 9 * GIB / 10)),
            "a small disk mostly free"
        );
        assert!(
            nearly_full(&disk(GIB, GIB / 30)),
            "a small disk under 5% free"
        );
        assert!(
            nearly_full(&disk(2000 * GIB, 9 * GIB)),
            "a large disk under 10 GiB free"
        );
        assert!(
            !nearly_full(&disk(40 * GIB, 9 * GIB)),
            "under 10 GiB but over 20% free"
        );
        assert!(!nearly_full(&disk(0, 0)));
    }

    #[test]
    fn raises_an_alert_once_and_again_only_after_it_clears() {
        let m = Monitor::new();
        let mut m = m.lock().unwrap();
        let held = || vec![Condition::now("disk /", "full".into())];
        assert_eq!(m.judge(held()).len(), 1);
        assert!(m.judge(held()).is_empty(), "no repeat while it holds");
        assert!(m.judge(Vec::new()).is_empty());
        assert_eq!(m.judge(held()).len(), 1, "raised again after clearing");
        let slow = || {
            vec![Condition::sustained(
                "swap",
                Duration::from_secs(120),
                "swap".into(),
            )]
        };
        assert!(m.judge(slow()).is_empty(), "a sustained condition waits");
    }

    #[test]
    fn samples_this_host() {
        let m = Monitor::new();
        let mut m = m.lock().unwrap();
        m.sample();
        m.sample();
        let s = m.snapshot.now;
        assert!(s.mem_total > 0 && s.mem_available <= s.mem_total);
        assert!(!m.snapshot.top.is_empty());
        let r: serde_json::Value = serde_json::from_str(&m.report()).unwrap();
        assert!(summary(&r).starts_with("CPU "));
    }
}
