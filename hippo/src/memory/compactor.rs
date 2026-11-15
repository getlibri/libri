//! The pump: builds tree nodes as UniiChat's `Memory.starts` does. A node
//! starts when it is not built or running and its sources exist, up to the
//! JOBS-th message not built, in the order the messages came; each call's
//! context is the compactions' view up to the node and the first message
//! not built. Departure: JOBS is 8, not Unii's 64, since each call here is
//! a local `claude` process rather than a request.

use crate::memory::compact::{self, Fail, PROMPT_VERSION, Step, Usage};
use crate::memory::server::{Core, Shared};
use crate::memory::store::Msg;
use crate::memory::tree::{NODE, Node, addr, flat};
use chrono::{DateTime, Local};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// Compactor calls running at once: 8, or `HIPPO_JOBS` for a scratch store
/// built in bulk (the memory test), where waiting on 8 would take a day.
pub fn jobs() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("HIPPO_JOBS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(8)
    })
}
/// Wait on a limit that gives no reset time.
const LIMIT_WAIT: i64 = 300;
/// Seconds the queue waits on a model that failed: no reset is coming, so
/// what brings it back is a try.
const MODEL_WAIT: i64 = 60;
/// Seconds it waits on Libri, restarting or down. A try costs nothing, and
/// a restart is over in seconds.
const UNREACHABLE_WAIT: i64 = 5;
/// Seconds a failure must stand, with nothing succeeding, before it counts
/// as failing. A restart or one bad call is over well within this.
pub const FAILING_AFTER: i64 = 180;

/// Compactor calls and tokens per local day, kept in `state/usage.jsonl`.
#[derive(Default)]
pub struct Spent {
    pub days: std::collections::BTreeMap<chrono::NaiveDate, (u64, Usage)>,
    /// The newest summary: when, and by which model.
    pub newest: Option<(DateTime<Local>, String)>,
    /// Calls each summary of the last hour took.
    calls: std::collections::VecDeque<(DateTime<Local>, u64)>,
    path: Option<std::path::PathBuf>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Spend {
    date: String,
    l: u8,
    i: u64,
    model: String,
    #[serde(flatten)]
    usage: Usage,
}

impl Spent {
    pub fn load(store: &std::path::Path) -> Result<Spent, String> {
        let path = store.join("state/usage.jsonl");
        let mut spent = Spent {
            newest: None,
            calls: Default::default(),
            days: Default::default(),
            path: Some(path.clone()),
        };
        let mut torn = Vec::new();
        crate::memory::store::read_jsonl::<Spend>(&path, &mut torn, |s, _, _| {
            if let Some(d) = crate::memory::store::parse_date(&s.date) {
                let day = spent.days.entry(d.date_naive()).or_default();
                day.0 += 1;
                day.1.add(s.usage);
                spent.note(d, &s.model, s.usage.calls);
            }
        })?;
        Ok(spent)
    }

    fn note(&mut self, at: DateTime<Local>, model: &str, calls: u64) {
        self.newest = Some((at, model.to_owned()));
        // Lines from before `calls` was kept say 0, and say nothing of it.
        if calls > 0 {
            self.calls.push_back((at, calls));
            self.prune();
        }
    }

    fn prune(&mut self) {
        let cutoff = Local::now() - chrono::Duration::hours(1);
        while self.calls.front().is_some_and(|(at, _)| *at < cutoff) {
            self.calls.pop_front();
        }
    }

    /// Nodes built and calls they took in the last hour.
    pub fn last_hour(&mut self) -> (u64, u64) {
        self.prune();
        (
            self.calls.len() as u64,
            self.calls.iter().map(|(_, n)| n).sum(),
        )
    }

    fn record(&mut self, l: u8, i: u64, model: &str, usage: Usage) {
        // Also one journal line per call, where `usage` counts each
        // assistant's share of the subscriptions.
        eprintln!(
            "hippo: tokens model={model} input={} cache_read={} cache_write={} output={}",
            usage.input, usage.cache_read, usage.cache_write, usage.output
        );
        let now = Local::now();
        let day = self.days.entry(now.date_naive()).or_default();
        day.0 += 1;
        day.1.add(usage);
        self.note(now, model, usage.calls);
        let Some(path) = &self.path else { return };
        let line = Spend {
            date: crate::memory::store::fmt_date(&now),
            l,
            i,
            model: model.to_owned(),
            usage,
        };
        let written = serde_json::to_string(&line)
            .map_err(|e| e.to_string())
            .and_then(|line| {
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|e| e.to_string())?;
                crate::memory::store::append_line(&mut f, &line)
            });
        if let Err(e) = written {
            eprintln!("hippo: recording usage: {e}");
        }
    }
}

/// Signals a condvar when dropped: a call's end, however it ends.
struct Ended<'a> {
    shared: &'a Shared,
    node: (u8, u64),
}

impl Drop for Ended<'_> {
    fn drop(&mut self) {
        // A call that panicked never reached the arm that frees its node, so
        // it is parked here, as any failure of its own is: else `close`
        // waits for it forever, and the next pump would panic it again. The
        // lock may be poisoned by that same panic; the map is still whole.
        if thread::panicking() {
            let mut c = self.shared.core.lock().unwrap_or_else(|e| e.into_inner());
            c.pump.failed(self.node, "the call panicked".into());
        }
        self.shared.cv.notify_all();
    }
}

/// A place among the calls a pool allows, given back when dropped. The main
/// memory has no pool.
pub struct Slot(Option<Arc<AtomicUsize>>);

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(pool) = &self.0 {
            pool.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// A node's first failure with no success since.
#[derive(Clone, Debug, PartialEq)]
pub struct Failing {
    pub error: String,
    pub since: DateTime<Local>,
}

/// Where a node not yet built stands; one with neither a call nor a failure
/// has no entry.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeState {
    /// A call is at work on it. `overdue` once a view gave up waiting for
    /// it, so no later view waits on that call again.
    Running {
        overdue: bool,
        failing: Option<Failing>,
    },
    /// Its last call failed for a reason of its own. Parked until the next
    /// message, then tried again.
    Failed { failing: Failing, parked: bool },
}

/// Why the queue is held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cause {
    Limit,
    Unreachable,
    Model,
    /// A built node could not be saved.
    Write,
}

/// No model call starts before `until`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hold {
    pub until: DateTime<Local>,
    pub cause: Cause,
}

#[derive(Default)]
pub struct Pump {
    pub spent: Spent,
    /// Each node with a call at work or a failure standing.
    pub nodes: BTreeMap<(u8, u64), NodeState>,
    /// Messages when parked nodes were last let go.
    pub t: u64,
    pub hold: Option<Hold>,
    /// The latest failure, kept once it clears.
    pub trouble: Option<Trouble>,
    /// The latest failure to write the store.
    pub writes: Option<Trouble>,
    /// Lowest index per level that may still be unbuilt.
    pub lo: Vec<u64>,
    /// The calls this pump shares with others, up to `JOBS` in all.
    pub pool: Option<Arc<AtomicUsize>>,
}

/// A failure and how long it has stood.
#[derive(Clone, Debug)]
pub struct Trouble {
    pub kind: &'static str,
    pub error: String,
    /// The latest time it failed so.
    pub at: DateTime<Local>,
    /// The first failure with no success since.
    pub since: DateTime<Local>,
    /// When it stopped; a refusal is over as soon as its fallback is saved.
    pub cleared: Option<DateTime<Local>>,
}

impl Pump {
    /// A place for one more call, or None while the pool is full.
    fn slot(&self) -> Option<Slot> {
        let Some(pool) = &self.pool else {
            return Some(Slot(None));
        };
        pool.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            (n < jobs()).then_some(n + 1)
        })
        .ok()?;
        Some(Slot(Some(pool.clone())))
    }

    /// Holds the queue until `until` at least: calls in flight fail at once
    /// for different waits, and a short one (unreachable, seconds) must not
    /// cut a rate limit's (hours) and send calls at a limited account. The
    /// cause is the longest hold's.
    fn hold_until(&mut self, until: DateTime<Local>, cause: Cause) {
        if self.hold.is_none_or(|h| h.until < until) {
            self.hold = Some(Hold { until, cause });
        }
    }

    /// The hold still standing.
    pub fn held_until(&self) -> Option<Hold> {
        self.hold.filter(|h| Local::now() < h.until)
    }

    /// How many calls are at work.
    pub fn running(&self) -> usize {
        self.nodes
            .values()
            .filter(|n| matches!(n, NodeState::Running { .. }))
            .count()
    }

    /// Whether node `n` can't start now: a call is on it, or it is parked.
    fn blocked(&self, n: (u8, u64)) -> bool {
        matches!(
            self.nodes.get(&n),
            Some(NodeState::Running { .. } | NodeState::Failed { parked: true, .. })
        )
    }

    /// A call starts on node `n`.
    fn start(&mut self, n: (u8, u64)) {
        let failing = match self.nodes.remove(&n) {
            Some(NodeState::Failed { failing, .. }) => Some(failing),
            _ => None,
        };
        self.nodes.insert(
            n,
            NodeState::Running {
                overdue: false,
                failing,
            },
        );
    }

    /// Node `n` is built, by its call or by hand.
    fn built(&mut self, n: (u8, u64)) {
        self.nodes.remove(&n);
    }

    /// Node `n`'s call failed for a reason of its own: parked until the next
    /// message, its first failure kept.
    pub fn failed(&mut self, n: (u8, u64), error: String) {
        let failing = match self.nodes.remove(&n) {
            Some(NodeState::Running {
                failing: Some(f), ..
            })
            | Some(NodeState::Failed { failing: f, .. }) => f,
            _ => {
                let (l, i) = n;
                let (s, k) = addr(l, i);
                eprintln!("hippo: node {s}+{k} failed: {error}");
                Failing {
                    error,
                    since: Local::now(),
                }
            }
        };
        self.nodes.insert(
            n,
            NodeState::Failed {
                failing,
                parked: true,
            },
        );
    }

    /// Node `n`'s call ended on a failure of the model or the account,
    /// which the hold waits out: the node is free for the next call.
    fn released(&mut self, n: (u8, u64)) {
        if let Some(NodeState::Running {
            failing: Some(failing),
            ..
        }) = self.nodes.remove(&n)
        {
            self.nodes.insert(
                n,
                NodeState::Failed {
                    failing,
                    parked: false,
                },
            );
        }
    }

    /// A message came: parked nodes are tried again.
    fn unpark(&mut self) {
        for n in self.nodes.values_mut() {
            if let NodeState::Failed { parked, .. } = n {
                *parked = false;
            }
        }
    }

    /// A view gives up waiting on the calls at work now.
    pub fn give_up_on_running(&mut self) {
        for n in self.nodes.values_mut() {
            if let NodeState::Running { overdue, .. } = n {
                *overdue = true;
            }
        }
    }

    /// Whether nothing a view could wait on is under way: no call runs, or
    /// every one that does was given up on.
    pub fn nothing_to_wait_on(&self) -> bool {
        self.nodes.values().all(|n| match n {
            NodeState::Running { overdue, .. } => *overdue,
            NodeState::Failed { .. } => true,
        })
    }

    /// Whether a node waits for the next message.
    pub fn any_parked(&self) -> bool {
        self.nodes
            .values()
            .any(|n| matches!(n, NodeState::Failed { parked: true, .. }))
    }

    /// The failures standing, oldest node first.
    pub fn failing_nodes(&self) -> impl Iterator<Item = ((u8, u64), &Failing)> {
        self.nodes.iter().filter_map(|(&k, n)| match n {
            NodeState::Failed { failing, .. }
            | NodeState::Running {
                failing: Some(failing),
                ..
            } => Some((k, failing)),
            NodeState::Running { failing: None, .. } => None,
        })
    }

    fn failed_call(&mut self, f: &Fail) {
        let now = Local::now();
        let since = match &self.trouble {
            Some(t) if t.cleared.is_none() => t.since,
            _ => now,
        };
        self.trouble = Some(Trouble {
            kind: f.kind(),
            error: f.text(),
            at: now,
            since,
            cleared: matches!(f, Fail::Refused(_)).then_some(now),
        });
    }

    fn call_worked(&mut self) {
        if let Some(t) = self.trouble.as_mut().filter(|t| t.cleared.is_none()) {
            t.cleared = Some(Local::now());
        }
    }

    /// Notes how a write went: `kind` "store" for messages and nodes, "view"
    /// for the published view. A small view can fit on a disk too full for
    /// a message, so its success says nothing of a failing store.
    pub fn wrote<T>(&mut self, kind: &'static str, result: &Result<T, String>) {
        match result {
            Ok(_) => {
                if let Some(t) = self
                    .writes
                    .as_mut()
                    .filter(|t| t.cleared.is_none() && (kind == "store" || t.kind == kind))
                {
                    t.cleared = Some(Local::now());
                }
            }
            Err(e) => {
                let now = Local::now();
                let since = match &self.writes {
                    Some(t) if t.cleared.is_none() => t.since,
                    _ => {
                        eprintln!("hippo: writing the store: {e}");
                        now
                    }
                };
                self.writes = Some(Trouble {
                    kind,
                    error: e.clone(),
                    at: now,
                    since,
                    cleared: None,
                });
            }
        }
    }

    /// The latest failure, one that still stands before one that is over.
    pub fn last_error(&self) -> Option<&Trouble> {
        let standing = |t: &&Trouble| t.cleared.is_none();
        let writes = self.writes.as_ref();
        writes.filter(standing).or(self.trouble.as_ref()).or(writes)
    }

    /// The failure that makes the memory failing now: its reason in a
    /// person's words and its start. One that has stood under
    /// `FAILING_AFTER` is a blip.
    pub fn failing(&self) -> Option<(String, DateTime<Local>)> {
        let cutoff = Local::now() - chrono::Duration::seconds(FAILING_AFTER);
        if let Some(t) = &self.writes
            && t.cleared.is_none()
            && t.since <= cutoff
        {
            return Some(("The memory can't write to its store.".to_owned(), t.since));
        }
        if let Some(t) = &self.trouble
            && t.cleared.is_none()
            && t.since <= cutoff
        {
            let reason = match t.kind {
                "unreachable" => "The summarizer can't be reached.".to_owned(),
                "rate_limit" => match self.held_until().filter(|h| h.cause == Cause::Limit) {
                    Some(h) => format!(
                        "The summarizer is rate limited until {}.",
                        h.until.format("%H:%M")
                    ),
                    None => "The summarizer is rate limited.".to_owned(),
                },
                _ => "The summarizer's model is failing.".to_owned(),
            };
            return Some((reason, t.since));
        }
        let oldest = self.failing_nodes().map(|(_, f)| f.since).min()?;
        (oldest <= cutoff).then(|| ("A summary keeps failing.".to_owned(), oldest))
    }

    /// What holds the queue, in a person's words, while it does.
    pub fn held(&self) -> Option<String> {
        let Hold { until, cause } = self.held_until()?;
        let at = until.format("%H:%M");
        Some(match cause {
            Cause::Limit => format!("compactor rate-limited until {at}"),
            Cause::Write => format!("compactor can't save to its store, next try at {at}"),
            Cause::Unreachable | Cause::Model => {
                format!("compactor failing, next try at {at}")
            }
        })
    }
}

/// `hippo pause` leaves this file; while it exists no model call starts.
/// Logging goes on, and the pause outlives restarts.
pub fn pause_file(store: &std::path::Path) -> std::path::PathBuf {
    store.join("state/paused")
}

/// What a level-0 node is built from.
fn message(c: &Core, i: u64) -> Result<Msg, String> {
    c.store.get(i)
}

/// UniiChat's compactions' view of node `(l, i)`: its built lines before
/// `ctx`, the node's place, and before the first message not built, so
/// calls share the lines no node still to come changes.
fn context(c: &mut Core, ctx: u64) -> Result<Vec<String>, String> {
    let upto = ctx.min(c.first(0));
    let mut out = Vec::new();
    for &(l, i) in &c.context.parts {
        let (s, n) = addr(l, i);
        if s + n > upto {
            break;
        }
        if let Some(t) = c.tree.text(l, i)? {
            out.push(format!("{s}+{n}|{}", flat(&t)));
        }
    }
    Ok(out)
}

/// UniiChat's `Memory.ctx`: where node `(l, i)` stands in the chat, the
/// message its context ends before: a leaf's own, a merge's end.
fn ctx(l: u8, i: u64) -> u64 {
    if l == 0 { i } else { (i + 1) << l }
}

enum Job {
    Free(String),
    Call {
        context: Vec<String>,
        a: String,
        /// The second line of a merge.
        b: Option<String>,
    },
}

/// Prepares node `(l, i)`: free if its source fits in `NODE`.
fn prepare(c: &mut Core, l: u8, i: u64) -> Result<Option<Job>, String> {
    if l == 0 {
        let src = message(c, i)?.render();
        if src.len() <= NODE {
            return Ok(Some(Job::Free(src)));
        }
        return Ok(Some(Job::Call {
            context: context(c, ctx(l, i))?,
            a: src,
            b: None,
        }));
    }
    let (Some(a), Some(b)) = (c.tree.text(l - 1, 2 * i)?, c.tree.text(l - 1, 2 * i + 1)?) else {
        return Ok(None);
    };
    let joined = format!("{a}\n{b}");
    if joined.len() <= NODE {
        return Ok(Some(Job::Free(joined)));
    }
    Ok(Some(Job::Call {
        context: context(c, ctx(l, i))?,
        a,
        b: Some(b),
    }))
}

/// Saves a finished node. One that can't be written is built again, so the
/// queue waits rather than pay for the same call in a loop.
fn keep(c: &mut Core, l: u8, i: u64, line: String, model: &str) {
    let saved = save(c, l, i, line, model, PROMPT_VERSION);
    c.pump.wrote("store", &saved);
    if let Err(e) = saved {
        eprintln!("hippo: saving node {l}/{i}: {e}");
        let until = Local::now() + chrono::Duration::seconds(MODEL_WAIT);
        c.pump.hold_until(until, Cause::Write);
    }
}

fn save(
    c: &mut Core,
    l: u8,
    i: u64,
    text: String,
    model: &str,
    prompt: &str,
) -> Result<(), String> {
    c.tree.save(Node {
        l,
        i,
        size: text.len() as u64,
        text,
        model: model.to_owned(),
        prompt: prompt.to_owned(),
    })?;
    c.fit(false);
    c.stale = true;
    Ok(())
}

/// Starts every node that may start. Call with the core locked, after any
/// change: a new message, a node built, a wait over.
pub fn pump(shared: &Arc<Shared>, c: &mut Core) {
    if shared.backend.is_none() || c.paused() {
        return;
    }
    if c.pump.t != c.view.t {
        c.pump.t = c.view.t;
        c.pump.unpark();
    }
    if c.pump.hold.is_some() {
        if c.pump.held_until().is_some() {
            return;
        }
        c.pump.hold = None;
    }
    // UniiChat's `Memory.starts`: the nodes ready up to the JOBS-th leaf
    // not built, in the order the messages came (at a tie, a merge first),
    // while JOBS run.
    loop {
        let mut progressed = false;
        let (t, end) = (c.view.t, c.first(jobs() - 1));
        let mut next = Vec::new();
        let mut l = 0u8;
        while (1u64 << l) <= t {
            if c.pump.lo.len() <= l as usize {
                c.pump.lo.push(0);
            }
            while c.tree.built(l, c.pump.lo[l as usize]) {
                c.pump.lo[l as usize] += 1;
            }
            let mut i = c.pump.lo[l as usize];
            while (i + 1) << l <= t && ctx(l, i) <= end {
                let ready =
                    l == 0 || (c.tree.built(l - 1, 2 * i) && c.tree.built(l - 1, 2 * i + 1));
                if ready && !c.tree.built(l, i) {
                    next.push((ctx(l, i), std::cmp::Reverse(l), i));
                }
                i += 1;
            }
            l += 1;
        }
        next.sort_unstable();
        for (_, std::cmp::Reverse(l), i) in next {
            if c.pump.running() >= jobs() {
                break;
            }
            if c.pump.blocked((l, i)) {
                continue;
            }
            match prepare(c, l, i) {
                Ok(Some(Job::Free(text))) => {
                    if let Err(e) = save(c, l, i, text, "", "") {
                        eprintln!("hippo: saving node {l}/{i}: {e}");
                        return;
                    }
                    progressed = true;
                }
                Ok(Some(Job::Call { context, a, b })) => {
                    let Some(slot) = c.pump.slot() else { break };
                    c.pump.start((l, i));
                    if let Err(e) = spawn(shared.clone(), l, i, context, a, b, slot) {
                        eprintln!("hippo: no thread for node {l}/{i}: {e}");
                        c.pump.released((l, i));
                        return;
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("hippo: preparing node {l}/{i}: {e}"),
            }
        }
        shared.cv.notify_all();
        if !progressed {
            return;
        }
    }
}

fn spawn(
    shared: Arc<Shared>,
    l: u8,
    i: u64,
    context: Vec<String>,
    a: String,
    b: Option<String>,
    slot: Slot,
) -> std::io::Result<()> {
    // `thread::spawn` would panic with the core locked when threads run out,
    // and the poisoned lock would then end the process: the next pump tries
    // again.
    thread::Builder::new().spawn(move || {
        let _slot = slot;
        // Whichever way the call ends, those waiting on it hear, once the
        // core is unlocked: a closed memory's pump returns before it
        // signals.
        let _ended = Ended {
            shared: &shared,
            node: (l, i),
        };
        let step = match &b {
            Some(b) => Step::Merge {
                l,
                i,
                a: &a,
                b: b.as_str(),
            },
            None => Step::Compress { id: i, msg: &a },
        };
        let backend = shared
            .backend
            .as_deref()
            .expect("pump runs only with a backend");
        let backend = backend.for_level(l).unwrap_or(backend);
        // Set when the memory opened, never changed: the lock is the pump's
        // for a moment.
        let whose = shared.core.lock().unwrap().whose.clone();
        let result = compact::run(backend, whose.as_deref(), &context, &step);
        let mut c = shared.core.lock().unwrap();
        match result {
            Ok((line, model, usage)) => {
                c.pump.spent.record(l, i, &model, usage);
                c.pump.call_worked();
                keep(&mut c, l, i, line, &model);
                c.pump.built((l, i));
            }
            // The model or the account failed, not this node: the queue is
            // held, and the hold is waited out by the pump, not by this
            // call, which gives back its node and its place in the pool.
            Err(f @ (Fail::Limit(_) | Fail::Unreachable(_) | Fail::Model(_))) => {
                let wait = |s| Local::now() + chrono::Duration::seconds(s);
                let (until, cause) = match f {
                    Fail::Limit(Some(until)) => (until, Cause::Limit),
                    Fail::Limit(None) => (wait(LIMIT_WAIT), Cause::Limit),
                    Fail::Unreachable(_) => (wait(UNREACHABLE_WAIT), Cause::Unreachable),
                    _ => (wait(MODEL_WAIT), Cause::Model),
                };
                if c.pump.hold.is_none() {
                    match &f {
                        Fail::Limit(_) => {
                            eprintln!("hippo: compactor limited until {}", until.format("%H:%M"))
                        }
                        _ => eprintln!(
                            "hippo: compactor held until {}: {}: {}",
                            until.format("%H:%M:%S"),
                            f.kind(),
                            f.text()
                        ),
                    }
                }
                c.pump.failed_call(&f);
                c.pump.hold_until(until, cause);
                c.pump.released((l, i));
            }
            Err(Fail::Refused(e)) => {
                // UniiChat's: no line is its input's doing, retrying
                // fails the same way, and every later node waits on this
                // one, so its node is its source clipped.
                let (s, n) = addr(l, i);
                eprintln!("hippo: node {s}+{n}: {e}; clipped");
                c.pump.failed_call(&Fail::Refused(e));
                keep(&mut c, l, i, compact::clipped(&step), "clipped");
                c.pump.built((l, i));
            }
            Err(Fail::Other(e)) => {
                c.pump.failed_call(&Fail::Other(e.clone()));
                c.pump.failed((l, i), e);
            }
        }
        pump(&shared, &mut c);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::compact::{Backend, Chat};
    use crate::memory::server::Core;
    use crate::memory::tree::Tree;
    use crate::memory::view::{PLACEHOLDER, View};
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    /// Answers every step with a short line and records what it was shown.
    struct Fake(Arc<Mutex<Vec<String>>>);
    struct FakeChat(Arc<Mutex<Vec<String>>>);
    impl Chat for FakeChat {
        fn say(&mut self, blocks: &[String]) -> Result<String, Fail> {
            self.0.lock().unwrap().push(blocks.join("\n"));
            let step = blocks.last().unwrap();
            let body = step.lines().rev().nth(1).unwrap_or("");
            Ok(format!("talk: {}", &body[..body.len().min(60)]))
        }
        fn model(&self) -> String {
            "fake".into()
        }
    }
    impl Backend for Fake {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Ok(Box::new(FakeChat(self.0.clone())))
        }
    }

    #[test]
    fn a_context_ends_at_the_first_message_not_built() {
        let dir = std::env::temp_dir().join(format!("hippo-ctx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = crate::memory::store::Store::open(&dir, true).unwrap();
        let n = 1_000u64;
        let drafts = (0..n)
            .map(|k| crate::memory::store::Draft {
                display: None,
                kind: crate::memory::store::Kind::Talk,
                chat: Some("c".into()),
                text: format!("message {k} ") + &"x".repeat(200),
                date: chrono::Local::now(),
                src: None,
            })
            .collect();
        store.append(drafts).unwrap();
        let mut tree = Tree::open(&dir).unwrap();
        for i in 0..n {
            let text = format!("talk: message {i} ") + &"x".repeat(200);
            tree.save(Node {
                l: 0,
                i,
                size: text.len() as u64,
                text,
                model: String::new(),
                prompt: String::new(),
            })
            .unwrap();
        }
        // A backlog behind them: logged, not summarized.
        let backlog = (0..500)
            .map(|k| crate::memory::store::Draft {
                display: None,
                kind: crate::memory::store::Kind::Talk,
                chat: Some("c".into()),
                text: format!("later {k} ") + &"x".repeat(200),
                date: chrono::Local::now(),
                src: None,
            })
            .collect();
        store.append(backlog).unwrap();
        let mut c = Core::for_test(store, tree, View::default(), crate::memory::view::VIEW);
        c.sync();
        // The compactions' view stops at the first message not built, so
        // every call in the backlog shares it whole.
        assert_eq!(c.context.t, n);
        assert_eq!(c.first(0), n);
        assert_eq!(c.first(7), n + 7);
        let last = format!("{}+1|", n - 1);
        let a = context(&mut c, n).unwrap();
        assert!(a.last().unwrap().starts_with(&last));
        assert_eq!(context(&mut c, n + 300).unwrap(), a);
        // A node further back sees the history before it only.
        let earlier = context(&mut c, n - 20).unwrap();
        assert!(earlier.len() < a.len() && a.starts_with(&earlier));
    }

    #[test]
    fn builds_in_order_and_folds_the_view() {
        let dir = std::env::temp_dir().join(format!("hippo-pump-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = crate::memory::store::Store::open(&dir, true).unwrap();
        // A store of long messages, so most nodes need a call.
        let drafts = (0..40)
            .map(|k| crate::memory::store::Draft {
                display: None,
                kind: crate::memory::store::Kind::Talk,
                chat: Some("c".into()),
                text: format!("message {k} ") + &"x".repeat(if k % 3 == 0 { 10 } else { 600 }),
                date: chrono::Local::now(),
                src: None,
            })
            .collect();
        store.append(drafts).unwrap();
        let tree = Tree::open(&dir).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let budget = 2_000;
        let shared = Arc::new(Shared {
            core: Mutex::new(Core::for_test(store, tree, View::default(), budget)),
            cv: Condvar::new(),
            backend: Some(Arc::new(Fake(seen.clone()))),
            workers: None,
        });
        {
            let mut c = shared.core.lock().unwrap();
            c.sync();
            pump(&shared, &mut c);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut c = shared.core.lock().unwrap();
        while (c.view.unbuilt(&c.tree) > 0 || c.pump.running() > 0)
            && std::time::Instant::now() < deadline
        {
            c = shared
                .cv
                .wait_timeout(c, Duration::from_millis(200))
                .unwrap()
                .0;
        }
        assert_eq!(c.view.unbuilt(&c.tree), 0);
        assert!(c.view.size() <= budget, "view {} bytes", c.view.size());
        assert!(c.view.parts.iter().any(|&(l, _)| l > 0), "nothing merged");
        // Short messages are their own line, word for word, without their chat.
        assert_eq!(
            c.tree.text(0, 0).unwrap().unwrap(),
            "talk: message 0 xxxxxxxxxx"
        );
        let ids = regex::Regex::new(r"(?m)^\d+\+\d+\|").unwrap();
        for call in seen.lock().unwrap().iter() {
            assert!(
                !call.contains(PLACEHOLDER),
                "a call saw an unsummarized line"
            );
            assert!(call.starts_with("<chat>\n"));
            assert!(
                call.contains("</chat>\nCompaction: "),
                "the task names its message or lines"
            );
            if let Some(ctx) = call
                .split("</chat>")
                .next()
                .unwrap()
                .strip_prefix("<chat>\n")
            {
                for line in ctx.lines() {
                    assert!(ids.is_match(line), "a bare context line: {line}");
                }
            }
        }
        // The published view is the whole view in one block, unpaged.
        assert!(c.stale);
        crate::memory::server::publish(&mut c).unwrap();
        assert!(!c.stale);
        let file = std::fs::read_to_string(dir.join("view.md")).unwrap();
        assert!(file.starts_with("<chat>\n") && file.ends_with("\n</chat>\n"));
        assert!(file.lines().nth(1).unwrap().starts_with("0+"));
        assert_eq!(file.lines().count(), c.view.parts.len() + 2);
    }

    /// Fails its first call, then answers.
    struct Flaky(Arc<Mutex<u32>>);
    struct FlakyChat(Arc<Mutex<u32>>);
    impl Chat for FlakyChat {
        fn say(&mut self, _: &[String]) -> Result<String, Fail> {
            let mut n = self.0.lock().unwrap();
            *n += 1;
            if *n == 1 {
                return Err(Fail::Other("boom".into()));
            }
            Ok("sum".into())
        }
        fn model(&self) -> String {
            "fake".into()
        }
    }
    impl Backend for Flaky {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Ok(Box::new(FlakyChat(self.0.clone())))
        }
    }

    /// Fails every call the way it is told.
    struct Failing(fn() -> Fail);
    impl Backend for Failing {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Err((self.0)())
        }
    }

    fn failed_once(name: &str, fail: fn() -> Fail) -> (Arc<Shared>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("hippo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = crate::memory::store::Store::open(&dir, true).unwrap();
        store
            .append(vec![crate::memory::store::Draft {
                display: None,
                kind: crate::memory::store::Kind::Talk,
                chat: Some("c".into()),
                text: "x".repeat(600),
                date: chrono::Local::now(),
                src: None,
            }])
            .unwrap();
        let tree = Tree::open(&dir).unwrap();
        let shared = Arc::new(Shared {
            core: Mutex::new(Core::for_test(store, tree, View::default(), 100_000)),
            cv: Condvar::new(),
            backend: Some(Arc::new(Failing(fail))),
            workers: None,
        });
        {
            let mut c = shared.core.lock().unwrap();
            c.sync();
            pump(&shared, &mut c);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let c = shared.core.lock().unwrap();
            if c.pump.trouble.is_some() {
                break;
            }
            drop(c);
            assert!(std::time::Instant::now() < deadline, "no failure noted");
            thread::sleep(Duration::from_millis(10));
        }
        (shared, dir)
    }

    #[test]
    fn a_dead_libri_holds_the_queue_for_seconds_and_is_no_limit() {
        let (shared, _dir) = failed_once("dead", || {
            Fail::Unreachable("Libri can't be reached: Connection refused (os error 111)".into())
        });
        let c = shared.core.lock().unwrap();
        let t = c.pump.trouble.as_ref().unwrap();
        assert_eq!(t.kind, "unreachable");
        assert!(t.cleared.is_none());
        let wait = c.pump.hold.unwrap().until - Local::now();
        assert!(wait.num_seconds() <= UNREACHABLE_WAIT, "{wait}");
        let held = c.pump.held().unwrap();
        assert!(!held.contains("limit"), "{held}");
    }

    #[test]
    fn a_blip_is_no_failure_and_a_standing_outage_is() {
        let (shared, _dir) = failed_once("outage", || {
            Fail::Unreachable("Libri can't be reached: Connection refused (os error 111)".into())
        });
        let mut c = shared.core.lock().unwrap();
        assert!(c.pump.failing().is_none(), "a new failure is a blip");
        let began = Local::now() - chrono::Duration::seconds(FAILING_AFTER + 1);
        c.pump.trouble.as_mut().unwrap().since = began;
        let (reason, since) = c.pump.failing().unwrap();
        assert_eq!(reason, "The summarizer can't be reached.");
        assert_eq!(since, began);
        c.pump.failed_call(&Fail::Unreachable("again".into()));
        assert_eq!(c.pump.trouble.as_ref().unwrap().since, began);
        c.pump.call_worked();
        assert!(c.pump.failing().is_none(), "a call that worked ends it");
    }

    #[test]
    fn a_refusal_is_over_at_once_and_a_node_that_keeps_failing_is_not() {
        let (shared, _dir) = failed_once("refusal", || Fail::Refused("flagged".into()));
        let mut c = shared.core.lock().unwrap();
        c.pump.trouble.as_mut().unwrap().since = Local::now() - chrono::Duration::hours(1);
        assert!(c.pump.failing().is_none());
        let old = Local::now() - chrono::Duration::seconds(FAILING_AFTER + 1);
        c.pump.nodes.insert(
            (0, 0),
            NodeState::Failed {
                failing: super::Failing {
                    error: "bad".into(),
                    since: old,
                },
                parked: true,
            },
        );
        let (reason, since) = c.pump.failing().unwrap();
        assert_eq!(reason, "A summary keeps failing.");
        assert_eq!(since, old);
    }

    #[test]
    fn a_short_wait_never_cuts_a_longer_hold() {
        let mut p = Pump::default();
        let limit = Local::now() + chrono::Duration::hours(2);
        p.hold_until(limit, Cause::Limit);
        let soon = Local::now() + chrono::Duration::seconds(UNREACHABLE_WAIT);
        p.hold_until(soon, Cause::Unreachable);
        assert_eq!(
            p.hold.map(|h| (h.until, h.cause)),
            Some((limit, Cause::Limit))
        );
        let later = limit + chrono::Duration::minutes(1);
        p.hold_until(later, Cause::Model);
        assert_eq!(p.hold.map(|h| h.until), Some(later));
    }

    #[test]
    fn a_limited_call_gives_back_its_node_and_slot_while_the_hold_runs() {
        let (shared, _dir) = failed_once("limited", || {
            Fail::Limit(Some(Local::now() + chrono::Duration::hours(1)))
        });
        let c = shared.core.lock().unwrap();
        assert_eq!(c.pump.running(), 0, "the wait was slept in a call");
        assert!(c.pump.nodes.is_empty(), "a limit is no failure of the node");
        assert_eq!(c.pump.held_until().map(|h| h.cause), Some(Cause::Limit));
        drop(c);
        // Nothing to wait for: the hold is the pump's, not a call's.
        shared.close();
    }

    #[test]
    fn a_store_that_cant_save_is_no_rate_limit() {
        let mut p = Pump::default();
        p.hold_until(Local::now() + chrono::Duration::seconds(60), Cause::Write);
        let held = p.held().unwrap();
        assert!(!held.contains("rate-limited"), "{held}");
        assert!(held.contains("save"), "{held}");
    }

    #[test]
    fn a_call_given_up_on_doesnt_mark_the_next_call_on_its_node() {
        let mut p = Pump::default();
        p.start((0, 0));
        p.give_up_on_running();
        assert!(p.nothing_to_wait_on());
        p.built((0, 0));
        p.start((0, 0));
        assert!(!p.nothing_to_wait_on(), "a new call read as overdue");
        // A failure keeps its first time across retries.
        p.failed((0, 0), "bad".into());
        let since = p.failing_nodes().next().unwrap().1.since;
        p.unpark();
        p.start((0, 0));
        p.failed((0, 0), "worse".into());
        let (_, f) = p.failing_nodes().next().unwrap();
        assert_eq!((f.error.as_str(), f.since), ("bad", since));
    }

    #[test]
    fn the_last_hour_counts_calls_per_summary() {
        let mut spent = Spent::default();
        let now = Local::now();
        spent.note(now - chrono::Duration::hours(2), "m", 9);
        spent.note(now, "m", 1);
        spent.note(now, "m", 3);
        spent.note(now, "n", 0);
        assert_eq!(spent.last_hour(), (2, 4));
        assert_eq!(spent.newest.as_ref().unwrap().1, "n");
    }

    #[test]
    fn a_failed_model_is_no_limit_and_a_true_limit_is() {
        let (shared, _dir) = failed_once("model", || Fail::Model("claude exited 1".into()));
        let c = shared.core.lock().unwrap();
        assert_eq!(c.pump.trouble.as_ref().unwrap().kind, "model");
        assert!(!c.pump.held().unwrap().contains("limit"));
        drop(c);
        let (shared, _dir) = failed_once("limit", || Fail::Limit(None));
        let c = shared.core.lock().unwrap();
        assert_eq!(c.pump.trouble.as_ref().unwrap().kind, "rate_limit");
        assert!(c.pump.held().unwrap().contains("rate-limited"));
    }

    #[test]
    fn a_refused_node_is_its_source_clipped_and_the_queue_runs_on() {
        let (shared, _dir) = failed_once("refused", || Fail::Refused("no".into()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut c = shared.core.lock().unwrap();
        while !c.tree.built(0, 0) && std::time::Instant::now() < deadline {
            c = shared
                .cv
                .wait_timeout(c, Duration::from_millis(50))
                .unwrap()
                .0;
        }
        let line = c.tree.text(0, 0).unwrap().unwrap();
        let msg = message(&c, 0).unwrap().render();
        assert_eq!(line, compact::cut(&crate::memory::tree::flat(&msg), NODE));
        assert!(c.pump.hold.is_none());
        let t = c.pump.trouble.as_ref().unwrap();
        assert_eq!(t.kind, "refused");
        assert!(t.cleared.is_some(), "a refusal with its line is over");
    }

    #[test]
    fn a_failed_call_waits_for_the_next_message() {
        let dir = std::env::temp_dir().join(format!("hippo-park-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = crate::memory::store::Store::open(&dir, true).unwrap();
        let draft = |k: u32| crate::memory::store::Draft {
            display: None,
            kind: crate::memory::store::Kind::Talk,
            chat: Some("c".into()),
            text: format!("message {k} ") + &"x".repeat(600),
            date: chrono::Local::now(),
            src: None,
        };
        store.append(vec![draft(0)]).unwrap();
        let tree = Tree::open(&dir).unwrap();
        let calls = Arc::new(Mutex::new(0));
        let shared = Arc::new(Shared {
            core: Mutex::new(Core::for_test(store, tree, View::default(), 100_000)),
            cv: Condvar::new(),
            backend: Some(Arc::new(Flaky(calls.clone()))),
            workers: None,
        });
        let settle = |shared: &Arc<Shared>| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut c = shared.core.lock().unwrap();
            while c.pump.running() > 0 && std::time::Instant::now() < deadline {
                c = shared
                    .cv
                    .wait_timeout(c, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
        };
        {
            let mut c = shared.core.lock().unwrap();
            c.sync();
            pump(&shared, &mut c);
        }
        settle(&shared);
        {
            let mut c = shared.core.lock().unwrap();
            assert!(c.pump.blocked((0, 0)) && c.pump.running() == 0);
            pump(&shared, &mut c);
            assert_eq!(c.pump.running(), 0, "retried before a message");
            c.store.append(vec![draft(1)]).unwrap();
            c.sync();
            pump(&shared, &mut c);
        }
        settle(&shared);
        let c = shared.core.lock().unwrap();
        assert!(c.tree.built(0, 0));
        assert!(c.pump.nodes.is_empty());
    }

    /// Anthropic's prompt cache, faked by its rules as UniiChat's
    /// cache.test.ts fakes it: an entry is written only at a mark, keyed by
    /// the prefix up to it, found walking back at most `LOOK` blocks from a
    /// mark, served once its writer's answer has begun, and alive `TTL` past
    /// its last use. Claude Code marks the system prompt and the request's
    /// last block; `marked` marks the context. Time runs a thousand times
    /// fast, the cache's lifetime with it.
    struct Anthropic {
        clock: Arc<Clock>,
        entries: Mutex<std::collections::HashMap<u64, (Instant, Instant)>>,
        /// Bytes of the calls' input read from the cache, written to it and
        /// sent uncached. Claude Code's mark on the last block writes each
        /// call's own task (or retry note) too, as the live log counts it.
        used: Mutex<(u64, u64, u64)>,
        /// Calls that read none of the context, only the system prompt.
        cold: Mutex<u64>,
        flights: Option<Arc<Flights>>,
        seed: Mutex<u64>,
    }
    use crate::memory::compact::marked;
    use crate::memory::flight::{self, Flights, LOOK, TTL};
    /// The cache's five minutes, at the replay's speed.
    const SIM_TTL: Duration = Duration::from_millis(TTL.as_millis() as u64 / 1000);
    use std::collections::BTreeSet;
    use std::time::Instant;

    /// The replay's time. It moves only once every call in flight sleeps,
    /// and wakes one sleeper at a time, so the machine's speed cannot let a
    /// cache entry lapse: a replay on a machine under load is the same chat.
    struct Clock {
        start: Instant,
        time: Mutex<Time>,
        woken: Condvar,
    }

    #[derive(Default)]
    struct Time {
        now: Duration,
        /// By when each wakes, then a count that parts two waking together.
        asleep: BTreeSet<(Duration, u64)>,
        sleeps: u64,
    }

    impl Clock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                start: Instant::now(),
                time: Default::default(),
                woken: Condvar::new(),
            })
        }

        /// A call's sleep: `pass` wakes it.
        fn sleep(&self, d: Duration) {
            let mut time = self.time.lock().unwrap();
            time.sleeps += 1;
            let me = (time.now + d, time.sleeps);
            time.asleep.insert(me);
            while time.asleep.contains(&me) {
                time = self.woken.wait(time).unwrap();
            }
        }

        /// Runs the calls in flight through `d` of the replay. A call is
        /// busy from the moment `pump` starts it, so one still starting
        /// holds the clock too.
        fn pass(&self, d: Duration, shared: &Shared) {
            let end = self.time.lock().unwrap().now + d;
            let hung = Instant::now() + Duration::from_secs(120);
            loop {
                let busy = shared.core.lock().unwrap().pump.running();
                let mut time = self.time.lock().unwrap();
                if time.asleep.len() < busy {
                    drop(time);
                    assert!(Instant::now() < hung, "a call never slept or ended");
                    thread::sleep(Duration::from_micros(50));
                    continue;
                }
                match time.asleep.first().copied() {
                    Some(next) if next.0 <= end => {
                        time.now = next.0;
                        time.asleep.remove(&next);
                        self.woken.notify_all();
                    }
                    _ => {
                        time.now = end;
                        return;
                    }
                }
            }
        }
    }

    impl flight::Clock for Clock {
        fn now(&self) -> Instant {
            self.start + self.time.lock().unwrap().now
        }
        fn nap(&self) {
            self.sleep(Duration::from_micros(250));
        }
    }

    /// When an answer begins, after its request.
    const FIRST: Duration = Duration::from_millis(3);

    impl Anthropic {
        fn rand(&self, lo: u64, hi: u64) -> u64 {
            let mut s = self.seed.lock().unwrap();
            *s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            lo + (*s >> 33) % (hi - lo)
        }

        fn request(&self, blocks: &[(String, bool)]) {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let now = flight::Clock::now(&*self.clock);
            let last = blocks.len() - 1;
            let (mut keys, mut ends, mut key, mut end) = (vec![], vec![], 0u64, 0u64);
            for (text, _) in blocks {
                let mut h = DefaultHasher::new();
                (key, text).hash(&mut h);
                key = h.finish();
                end += text.len() as u64;
                keys.push(key);
                ends.push(end);
            }
            let marks: Vec<usize> = (0..=last).filter(|&k| blocks[k].1 || k == last).collect();
            let mut entries = self.entries.lock().unwrap();
            let live = |k: usize| {
                entries
                    .get(&keys[k])
                    .is_some_and(|&(from, until)| from <= now && now < until)
            };
            let mut hit: Option<usize> = None;
            for &m in &marks {
                if let Some(k) = (m.saturating_sub(LOOK - 1)..=m).rev().find(|&k| live(k)) {
                    hit = hit.max(Some(k));
                }
            }
            if let Some(h) = hit {
                entries.get_mut(&keys[h]).unwrap().1 = now + SIM_TTL;
            }
            for &m in marks.iter().filter(|&&m| hit.is_none_or(|h| m > h)) {
                if !entries.get(&keys[m]).is_some_and(|&(_, until)| now < until) {
                    entries.insert(keys[m], (now + FIRST, now + SIM_TTL));
                }
            }
            if hit.is_none_or(|h| h == 0) {
                *self.cold.lock().unwrap() += 1;
            }
            let counted = ends[last];
            let read = hit.map_or(0, |h| ends[h]).min(counted);
            let written = ends[*marks.iter().max().unwrap()]
                .min(counted)
                .saturating_sub(read);
            let mut used = self.used.lock().unwrap();
            used.0 += read;
            used.1 += written;
            used.2 += counted - read - written;
        }
    }

    struct Sim(Arc<Anthropic>);
    struct SimChat {
        api: Arc<Anthropic>,
        blocks: Vec<(String, bool)>,
        begun: Option<Box<dyn FnOnce() + Send>>,
    }
    impl Backend for Sim {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Ok(Box::new(SimChat {
                api: self.0.clone(),
                blocks: vec![(crate::memory::compact::COMPACT.to_owned(), true)],
                begun: None,
            }))
        }
        fn flights(&self) -> Option<&Arc<Flights>> {
            self.0.flights.as_ref()
        }
    }
    impl Chat for SimChat {
        fn say(&mut self, blocks: &[String]) -> Result<String, Fail> {
            let first = self.blocks.len() == 1;
            for b in marked(blocks) {
                let text = b["text"].as_str().unwrap().to_owned();
                self.blocks.push((text, b.get("cache_control").is_some()));
            }
            self.api.request(&self.blocks);
            self.api.clock.sleep(FIRST);
            if let Some(f) = self.begun.take() {
                f();
            }
            let answer = Duration::from_millis(self.api.rand(4, 12));
            self.api.clock.sleep(answer);
            // A compression answers in its message's kind; a fourth of the
            // first answers run too long, as real ones do.
            let step = blocks.last().unwrap();
            let kind = step
                .split("<input>\n")
                .nth(1)
                .and_then(|r| r.split(':').next())
                .unwrap_or("talk");
            let long = if first && self.api.rand(0, 4) == 0 {
                400
            } else {
                0
            };
            let line = format!(
                "{kind}: {}",
                "x".repeat(self.api.rand(120, 450) as usize + long)
            );
            self.blocks.push((line.clone(), false));
            Ok(line)
        }
        fn model(&self) -> String {
            "sim".into()
        }
        fn on_begin(&mut self, f: Box<dyn FnOnce() + Send>) {
            self.begun = Some(f);
        }
    }

    /// Replays a synthetic chat of `n` messages, turns of tool steps, a
    /// thousand times fast on `Clock`; the share of the compactions' input
    /// read from the cache.
    fn replay(n: usize, single_flight: bool) -> f64 {
        let dir = std::env::temp_dir().join(format!(
            "hippo-cache-{}-{single_flight}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = crate::memory::store::Store::open(&dir, true).unwrap();
        let tree = Tree::open(&dir).unwrap();
        let clock = Clock::new();
        let api = Arc::new(Anthropic {
            clock: clock.clone(),
            entries: Default::default(),
            used: Default::default(),
            cold: Default::default(),
            flights: single_flight.then(|| Arc::new(Flights::with_clock(SIM_TTL, clock.clone()))),
            seed: Mutex::new(1),
        });
        let shared = Arc::new(Shared {
            core: Mutex::new(Core::for_test(
                store,
                tree,
                View::default(),
                crate::memory::view::VIEW,
            )),
            cv: Condvar::new(),
            backend: Some(Arc::new(Sim(api.clone()))),
            workers: None,
        });
        use crate::memory::store::Kind;
        let sent = std::cell::Cell::new(0);
        let say = |kind: Kind, lo: u64, hi: u64, gap: (u64, u64)| {
            clock.pass(
                Duration::from_micros(api.rand(gap.0 * 1000, gap.1 * 1000)),
                &shared,
            );
            let text = format!("{} ", sent.get()) + &"y".repeat(api.rand(lo, hi) as usize);
            let mut c = shared.core.lock().unwrap();
            c.store
                .append(vec![crate::memory::store::Draft {
                    display: None,
                    kind,
                    chat: Some("c".into()),
                    text,
                    date: chrono::Local::now(),
                    src: None,
                }])
                .unwrap();
            c.sync();
            pump(&shared, &mut c);
            sent.set(sent.get() + 1);
        };
        while sent.get() < n {
            let steps = api.rand(0, 7);
            say(Kind::User, 40, 1500, (20, 240));
            for _ in 0..steps {
                say(Kind::Tool, 80, 600, (2, 12));
                say(Kind::Echo, 40, 8000, (1, 6));
            }
            say(Kind::Talk, 100, 2500, (2, 9));
        }
        let mut c = shared.core.lock().unwrap();
        while c.pump.running() > 0 {
            drop(c);
            clock.pass(Duration::from_millis(1), &shared);
            c = shared.core.lock().unwrap();
        }
        assert_eq!(c.view.unbuilt(&c.tree), 0, "the replay never caught up");
        let (read, written, input) = *api.used.lock().unwrap();
        eprintln!(
            "single flight {single_flight}: {} cold calls",
            api.cold.lock().unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
        read as f64 / (read + written + input) as f64
    }

    /// Each call writes its own task, so about 91% is the most it reads, as
    /// live calls between rewrites do.
    #[test]
    fn compactions_read_most_of_their_input_from_the_cache() {
        let read = replay(1_200, true);
        assert!(read >= 0.89, "read {:.1}% from the cache", 100.0 * read);
    }

    /// Single flight against none, by hand: `cargo test -- --ignored
    /// --nocapture single_flight`. On 2026-10-19 it was 97.1% against
    /// 96.5%; with UniiChat's view rules, 97.1% against 97.2%, 38 calls
    /// rereading the whole context each. Those runs left out each call's
    /// task and never let an entry lapse; counting both, 91.0% against
    /// 91.3%.
    #[test]
    #[ignore]
    fn single_flight_against_none() {
        let (with, without) = (replay(1_200, true), replay(1_200, false));
        eprintln!(
            "{:.1}% single flight, {:.1}% without",
            100.0 * with,
            100.0 * without
        );
    }
}
