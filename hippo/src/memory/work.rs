//! Workers' memories. A worker forks the main memory as a branch forks: at
//! its spawn it takes the view as it stands, with the nodes above it, and
//! goes on from there alone, never to resync. Its ids go on from the main
//! memory's, so a line from before the fork keeps its address and every
//! merge above the view is already built: the fork costs no model call. A
//! merge can reach across the fork, joining main's lines to the worker's.
//!
//! After that it is one more memory, with nothing of its own: under
//! `workers/<id>` in the store a log that follows `log.jsonl` in `agents/`
//! as code mode follows a harness's, the same tree and views, and the same
//! pump calling the same compactor, so its turns are summarized as they come
//! in. It stands apart from the main store because one tree covers one
//! timeline: in it a worker's steps would reach the main view.
//!
//! A worker can fork another worker's memory in the same way, one level
//! down: its head's view, which holds the head's fork of main, so its ids go
//! on from the head's. A line from before its fork is the head's, by the
//! same id, and one from before the head's fork is main's.
//!
//! A worker from before forks existed has no fork: its ids are its log's
//! entry numbers, and it starts from its own log. A memory opens at the
//! worker's first wake, so one that never wakes costs nothing, and an old
//! log is caught up from there. `log.jsonl` is only ever read.

use crate::code::source::agent_file;
use crate::memory::compact::Backend;
use crate::memory::compactor::{Pump, Spent, jobs, pause_file, pump};
use crate::memory::mask::mask;
use crate::memory::server::{Core, Shared, Stop, unbuilt_lines, view_lines};
use crate::memory::store::{Draft, Kind, Src, Store, parse_date};
use crate::memory::tree::{Node, Tree};
use crate::memory::view::View;
use chrono::Local;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

/// The workers' memories, in the store.
const WORKERS: &str = "workers";

/// In a worker's memory, the id it forked at.
const FORK: &str = "fork";

/// In a sub-worker's memory, the id of the worker whose memory it forked.
/// A worker without it forked the main memory.
const PARENT: &str = "parent";

/// In the main memory, the harness of a worker's message merged into it,
/// with the worker's id as its session: a message merged once is skipped by
/// the next merge. It sits in the main chat as the chat's own words do.
pub const MERGED: &str = "merged";

/// The most of a run of lines not summarized yet that a wake reads whole:
/// a log met for the first time is all such lines, and past this it reads
/// as `unbuilt_lines`, which a zoom or a search opens. It bounds the prompt
/// when no compactor can run. A log of 1 MiB is about 300k tokens.
const WHOLE_MAX: usize = 1 << 20;

/// How often a woken worker's log is read for new entries, as the main loop
/// reads transcripts.
const EVERY: Duration = Duration::from_secs(1);

#[derive(Deserialize)]
struct Entry {
    kind: String,
    text: String,
}

/// A log's kind; a kind this hippo doesn't know still takes its number.
fn kind_of(name: &str) -> Kind {
    match name {
        "user" => Kind::User,
        "talk" => Kind::Talk,
        "tool" => Kind::Tool,
        "echo" => Kind::Echo,
        "work" => Kind::Work,
        "from" => Kind::From,
        "to" => Kind::To,
        _ => Kind::Note,
    }
}

struct Worker {
    log: PathBuf,
    /// Bytes of the log in the store; None until counted, after a start or
    /// a failed write.
    at: Mutex<Option<u64>>,
    /// It has woken since this service started, so its memory is built.
    awake: AtomicBool,
    shared: Arc<Shared>,
}

impl Worker {
    /// Takes in what the log has gained. Entries are numbered as the log's
    /// readers number them, its unreadable lines skipped.
    fn follow(&self, c: &mut Core) -> Result<(), String> {
        let mut at = self.at.lock().unwrap();
        let (from, mut skip) = at.map_or((0, c.store.len() - c.store.base), |n| (n, 0));
        let raw = read_from(&self.log, from)?;
        // A line still being written stays for the next look.
        let end = raw.iter().rposition(|b| *b == b'\n').map_or(0, |k| k + 1);
        let mut drafts = Vec::new();
        for line in raw[..end].split(|b| *b == b'\n') {
            let Ok(e) = serde_json::from_slice::<Entry>(line) else {
                continue;
            };
            if skip > 0 {
                skip -= 1;
                continue;
            }
            drafts.push(Draft {
                display: None,
                kind: kind_of(&e.kind),
                chat: None,
                text: mask(&e.text),
                date: Local::now(),
                src: None,
            });
        }
        if drafts.is_empty() {
            *at = Some(from + end as u64);
            return Ok(());
        }
        let kept = c.store.append(drafts);
        *at = kept.is_ok().then_some(from + end as u64);
        kept?;
        c.sync();
        Ok(())
    }

    /// One pass of what the main loop does: follow, pump, save.
    fn tick(&self) {
        let mut guard = self.shared.core.lock().unwrap();
        let c = &mut *guard;
        let followed = self.follow(c);
        c.pump.wrote("store", &followed);
        pump(&self.shared, c);
        if c.stale {
            let saved = c.save_views();
            c.stale = saved.is_err();
            c.pump.wrote("view", &saved);
        }
    }
}

/// A worker's id, the folder its log is in.
fn id_of(log: &Path, name: &str) -> Result<String, String> {
    Ok(log
        .parent()
        .and_then(Path::file_name)
        .ok_or_else(|| format!("No worker {name}."))?
        .to_string_lossy()
        .into_owned())
}

fn read_from(path: &Path, from: u64) -> Result<Vec<u8>, String> {
    let shown = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(shown(e)),
    };
    f.seek(SeekFrom::Start(from)).map_err(shown)?;
    let mut raw = Vec::new();
    f.read_to_end(&mut raw).map_err(shown)?;
    Ok(raw)
}

/// Lines not summarized yet read as they were said, newest first in line
/// for `WHOLE_MAX`, so a wake never knows less than the log unless it must.
/// Only the host starts a line, as in Libri's own reading of a log.
fn whole(store: &Store, s: u64, n: u64) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut room = WHOLE_MAX;
    let mut i = s + n;
    while i > s {
        let line = format!(
            "{}+1|{}",
            i - 1,
            store.get(i - 1)?.render().replace('\n', "\n  ")
        );
        if line.len() > room {
            break;
        }
        room -= line.len();
        lines.push(line);
        i -= 1;
    }
    if i > s {
        lines.extend(unbuilt_lines(s, i - s).into_iter().rev());
    }
    lines.reverse();
    Ok(lines)
}

/// What a run reads of its own past.
fn text(c: &Core) -> Result<String, String> {
    let wrap = "<work>\n\n</work>".len() as u64;
    let lines = view_lines(c, c.budget.saturating_sub(wrap), |s, n| {
        whole(&c.store, s, n)
    })?;
    Ok(format!("<work>\n{}\n</work>", lines.join("\n")))
}

pub struct Workers {
    /// The main store.
    dir: PathBuf,
    budget: u64,
    backend: Option<Arc<dyn Backend>>,
    /// Model calls under way for all workers together, at most `JOBS`: each
    /// call is a process on the host, and several old logs caught up at once
    /// would otherwise run `JOBS` each. The main memory has its own `JOBS`,
    /// so a worker's backlog never holds up a turn.
    calls: Arc<AtomicUsize>,
    open: Mutex<HashMap<String, Arc<Worker>>>,
}

impl Workers {
    pub fn new(dir: &Path, budget: u64, backend: Option<Arc<dyn Backend>>) -> Arc<Self> {
        Arc::new(Self {
            dir: dir.to_owned(),
            budget,
            backend,
            calls: Arc::default(),
            open: Mutex::default(),
        })
    }

    /// Keeps the woken workers' memories moving, for good.
    pub fn run(self: Arc<Self>) {
        thread::spawn(move || {
            loop {
                let open: Vec<Arc<Worker>> = {
                    let open = self.open.lock().unwrap();
                    open.values()
                        .filter(|w| w.awake.load(Ordering::Relaxed))
                        .cloned()
                        .collect()
                };
                for w in open {
                    w.tick();
                }
                thread::sleep(EVERY);
            }
        });
    }

    /// Worker `name`'s memory, opened on first asking.
    fn open(&self, name: &str) -> Result<Arc<Worker>, String> {
        let log = agent_file(&self.dir.join("agents"), name)?
            .ok_or_else(|| format!("No worker {name}."))?;
        let id = id_of(&log, name)?;
        let mut open = self.open.lock().unwrap();
        if let Some(w) = open.get(&id) {
            return Ok(w.clone());
        }
        let dir = self.dir.join(WORKERS).join(&id);
        let base = fs::read_to_string(dir.join(FORK))
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0);
        // The main store's lock already keeps out a second service.
        let store = Store::open_from(&dir, false, base)?;
        let pump = Pump {
            spent: Spent::load(&dir)?,
            pool: Some(self.calls.clone()),
            ..Pump::default()
        };
        // An archived worker costs nothing more. Its words stay, and the
        // calls go on from where they stopped if it comes back.
        let folder = log.parent().unwrap_or(Path::new("")).to_owned();
        let stops = vec![Stop::File(pause_file(&self.dir)), Stop::Archived(folder)];
        let mut core = Core::open(store, None, self.budget, stops, pump)?;
        core.whose = Some(id.clone());
        let w = Arc::new(Worker {
            log,
            at: Mutex::new(None),
            awake: AtomicBool::new(false),
            shared: Arc::new(Shared {
                core: Mutex::new(core),
                cv: Condvar::new(),
                backend: self.backend.clone(),
                workers: None,
            }),
        });
        open.insert(id, w.clone());
        Ok(w)
    }

    /// Worker `name`, not yet at work, forks the main memory at the view
    /// `take` gives, with the nodes holding it. All of it lands at once or
    /// none does, and only once: a fork is never brought up to date. A wake
    /// waits for its writing, so a run never opens a memory a fork is still
    /// writing, and one that came first leaves the worker with no fork.
    pub fn fork(
        &self,
        name: &str,
        take: impl FnOnce() -> Result<(View, Vec<Node>), String>,
    ) -> Result<String, String> {
        self.fork_from(name, None, take)
    }

    /// Worker `name` forks the memory of worker `parent`, which must have
    /// forked main's itself or have none: the chain stops at two levels.
    pub fn fork_worker(&self, name: &str, parent: &str) -> Result<String, String> {
        let log = agent_file(&self.dir.join("agents"), parent)?
            .ok_or_else(|| format!("No worker {parent}."))?;
        let parent = id_of(&log, parent)?;
        if self.parent_of(&parent).is_some() {
            return Err(format!(
                "{parent} is a sub-worker itself; a fork goes two levels deep at most."
            ));
        }
        self.fork_from(name, Some(&parent), || {
            self.read(&parent, crate::memory::server::fork)
        })
    }

    /// The id of the worker whose memory worker `id` forked, if it forked a
    /// worker's rather than main's.
    fn parent_of(&self, id: &str) -> Option<String> {
        let t = fs::read_to_string(self.dir.join(WORKERS).join(id).join(PARENT)).ok()?;
        Some(t.trim().to_owned()).filter(|t| !t.is_empty())
    }

    /// The id worker `id`'s memory forked at, if it did.
    fn base_of(&self, id: &str) -> Option<u64> {
        fs::read_to_string(self.dir.join(WORKERS).join(id).join(FORK))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// The worker whose memory worker `name` forked, by id, if it forked a
    /// worker's: where its lines from before the fork are.
    pub fn parent(&self, name: &str) -> Result<Option<String>, String> {
        let log = agent_file(&self.dir.join("agents"), name)?
            .ok_or_else(|| format!("No worker {name}."))?;
        Ok(self.parent_of(&id_of(&log, name)?))
    }

    /// Whose memory holds line `id+n` of worker `name`'s: its own, its
    /// head's for a line from before its fork, main's (None) for one from
    /// before the first fork.
    pub fn holder(&self, name: &str, id: u64, n: u64) -> Result<Option<String>, String> {
        let mut at = name.to_owned();
        loop {
            if !self.read(&at, |c| Ok(id.saturating_add(n) <= c.store.base))? {
                return Ok(Some(at));
            }
            match self.parent(&at)? {
                Some(parent) => at = parent,
                None => return Ok(None),
            }
        }
    }

    fn fork_from(
        &self,
        name: &str,
        parent: Option<&str>,
        take: impl FnOnce() -> Result<(View, Vec<Node>), String>,
    ) -> Result<String, String> {
        let log = agent_file(&self.dir.join("agents"), name)?
            .ok_or_else(|| format!("No worker {name}."))?;
        let id = id_of(&log, name)?;
        // The memory forked first, unlocked: waiting on it with the workers'
        // lock held would stall every worker's memory. A view a moment old is
        // as good a fork.
        let (view, nodes) = take()?;
        let _open = self.open.lock().unwrap();
        let dir = self.dir.join(WORKERS).join(&id);
        if dir.exists() {
            return Err(format!("{name} has its memory already."));
        }
        let new = self.dir.join(WORKERS).join(format!(".{id}"));
        let shown = |e: std::io::Error| format!("{}: {e}", new.display());
        // What a crash left of this fork.
        let _ = fs::remove_dir_all(&new);
        fs::create_dir_all(&new).map_err(shown)?;
        let mut tree = Tree::open(&new)?;
        let count = nodes.len();
        for node in nodes {
            tree.save(node)?;
        }
        view.save(&new, "view.json")?;
        view.save(&new, "context.json")?;
        fs::write(new.join(FORK), view.t.to_string()).map_err(shown)?;
        if let Some(parent) = parent {
            fs::write(new.join(PARENT), parent).map_err(shown)?;
        }
        fs::rename(&new, &dir).map_err(shown)?;
        Ok(format!("{name} forked at {} with {count} nodes.", view.t))
    }

    /// A run of worker `name` is starting: what it reads, as `<work>`, and
    /// from here its memory is built as its log grows. What isn't summarized
    /// yet reads whole, so this never waits for the compactor as the main
    /// view does.
    pub fn wake(&self, name: &str) -> Result<String, String> {
        let w = self.open(name)?;
        w.awake.store(true, Ordering::Relaxed);
        w.tick();
        text(&w.shared.core.lock().unwrap())
    }

    /// What a run of worker `name` reads, headed by the line `fork <id>`, the
    /// id its memory forked the main one at, or `fork none`: the words a run
    /// is told of its memory follow what it holds, not what its spawn heard.
    /// A fork of a worker's memory reads `fork <id> <parent> <parent's id>`,
    /// the last `none` when the parent had no fork of main.
    pub fn answer(&self, name: &str) -> Result<String, String> {
        let text = self.wake(name)?;
        let base = self.read(name, |c| Ok(c.store.base))?;
        let shown = |base: Option<u64>| {
            base.filter(|b| *b > 0)
                .map_or("none".into(), |b| b.to_string())
        };
        let mut fork = shown(Some(base));
        if base > 0
            && let Some(parent) = self.parent(name)?
        {
            fork = format!("{fork} {parent} {}", shown(self.base_of(&parent)));
        }
        Ok(format!("fork {fork}\n{text}"))
    }

    /// Worker `name`'s own messages, those since its fork, as the main
    /// memory takes them in at a merge: each as it was, its kind, words and
    /// time, kept under its worker and id so that it lands once. What came
    /// before the fork the main memory has already, and so its brief and
    /// each run's report; a sub-worker's lines come in with its own merge.
    pub fn merge_drafts(&self, name: &str) -> Result<Vec<Draft>, String> {
        let log = agent_file(&self.dir.join("agents"), name)?
            .ok_or_else(|| format!("No worker {name}."))?;
        let id = id_of(&log, name)?;
        self.read(name, |c| {
            let msgs = (c.store.base..c.store.len())
                .map(|i| Ok((i, c.store.get(i)?)))
                .collect::<Result<Vec<_>, String>>()?;
            // Main has the brief, which it wrote, and each run's report, the
            // last thing the worker says before it is told something more,
            // so neither lands twice.
            // The note it was started with is its own.
            let brief = msgs.iter().position(|(_, m)| m.kind == Kind::User);
            let mut reported = vec![false; msgs.len()];
            let mut turn_open = true;
            for (n, (_, m)) in msgs.iter().enumerate().rev() {
                match m.kind {
                    Kind::User => turn_open = true,
                    Kind::Talk if turn_open => {
                        reported[n] = true;
                        turn_open = false;
                    }
                    _ => {}
                }
            }
            msgs.into_iter()
                .enumerate()
                .filter(|(n, _)| brief.is_none_or(|b| *n > b) && !reported[*n])
                .map(|(_, (i, m))| {
                    Ok(Draft {
                        display: m.display,
                        kind: m.kind,
                        chat: None,
                        date: parse_date(&m.date).unwrap_or_else(Local::now),
                        text: m.text,
                        src: Some(Src {
                            h: MERGED.into(),
                            s: id.clone(),
                            e: format!("{id}:{i}"),
                        }),
                    })
                })
                .collect()
        })
    }

    /// Reads worker `name`'s memory as it stands in the log now, at no cost
    /// in model calls.
    pub fn read<T>(
        &self,
        name: &str,
        read: impl FnOnce(&Core) -> Result<T, String>,
    ) -> Result<T, String> {
        let w = self.open(name)?;
        let mut c = w.shared.core.lock().unwrap();
        w.follow(&mut c)?;
        read(&c)
    }

    /// Closes every worker's memory opened so far, as `Shared::close`: once
    /// this returns no call of theirs writes to the store, so it can go.
    #[cfg(test)]
    pub fn close(&self) {
        let open: Vec<Arc<Worker>> = self.open.lock().unwrap().values().cloned().collect();
        for w in open {
            w.shared.close();
        }
    }

    /// The status line for the workers.
    pub fn status(&self) -> String {
        let open = self.open.lock().unwrap();
        let awake = open
            .values()
            .filter(|w| w.awake.load(Ordering::Relaxed))
            .count();
        let calls = self.calls.load(Ordering::Relaxed);
        format!(
            "workers: {awake} awake, {calls} of {} calls running",
            jobs()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::compact::{Chat, Fail};
    use std::fs;
    use std::sync::Condvar;

    /// A compactor that answers every call with a line of the right kind
    /// holding `tag`, and counts its calls.
    struct Stand(Arc<AtomicUsize>);

    impl Chat for Stand {
        fn say(&mut self, blocks: &[String]) -> Result<String, Fail> {
            self.0.fetch_add(1, Ordering::Relaxed);
            let input = blocks.last().map_or("", String::as_str);
            let kind = ["echo", "tool", "user", "talk", "from", "to"]
                .into_iter()
                .find(|k| input.contains(&format!("<input>\n{k}: ")))
                .unwrap_or("note");
            Ok(format!("{kind}: summary {}", "s".repeat(300)))
        }
        fn model(&self) -> String {
            "stand-in".to_owned()
        }
    }

    impl Backend for Stand {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Ok(Box::new(Stand(self.0.clone())))
        }
    }

    fn rig(name: &str, budget: u64) -> (PathBuf, Arc<Workers>, Arc<AtomicUsize>) {
        let dir = std::env::temp_dir().join(format!("hippo-work-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("agents/gil-1")).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let backend: Arc<dyn Backend> = Arc::new(Stand(calls.clone()));
        (
            dir.clone(),
            Workers::new(&dir, budget, Some(backend)),
            calls,
        )
    }

    fn log(dir: &Path, lines: &[(&str, &str)]) {
        use std::io::Write;
        let path = dir.join("agents/gil-1/log.jsonl");
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for (kind, text) in lines {
            let line = serde_json::json!({"kind": kind, "text": text});
            writeln!(f, "{line}").unwrap();
        }
    }

    /// Ends a test: its memories closed, its calls landed, then its folder
    /// goes. Removed first, a call still running wrote into it as it went.
    fn finish(w: &Workers, dir: PathBuf) {
        w.close();
        fs::remove_dir_all(dir).unwrap();
    }

    fn settle(w: &Workers, want: impl Fn(&Core) -> bool) {
        let worker = w.open("gil-1").unwrap();
        for _ in 0..500 {
            worker.tick();
            if want(&worker.shared.core.lock().unwrap()) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the memory never settled");
    }

    fn built(c: &Core) -> bool {
        c.view.t > 0 && c.view.unbuilt(&c.tree) == 0 && c.pump.running() == 0
    }

    #[test]
    fn a_wake_reads_new_entries_whole_then_their_summaries() {
        let dir = std::env::temp_dir().join(format!("hippo-work-whole-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("agents/gil-1")).unwrap();
        // The call is held, so the first wake reads before any summary: an
        // instant one can take the core first on a loaded machine.
        let gate = Gate::new();
        let w = Workers::new(&dir, 128_000, Some(Arc::new(gate.clone())));
        let long = "x".repeat(900);
        log(
            &dir,
            &[("note", "subagent"), ("user", "Do it."), ("echo", &long)],
        );
        let first = w.wake("gil-1").unwrap();
        // The short ones are their own lines, built by the wake's own pass.
        assert!(first.contains("0+1|note: subagent"), "{first}");
        assert!(first.contains("1+1|user: Do it."), "{first}");
        assert!(first.contains(&format!("2+1|echo: {long}")), "{first}");
        gate.open();
        settle(&w, built);
        // Only the long one cost a call: merging two short summaries costs
        // none.
        assert_eq!(gate.1.load(Ordering::Relaxed), 1);
        let second = w.wake("gil-1").unwrap();
        assert!(second.contains("2+1|echo: summary"), "{second}");
        assert!(!second.contains(&long));
        // The words stay reachable, by the same address as the log's.
        let open = w
            .read("gil-1", |c| {
                crate::memory::server::zoom(&c.store, &c.tree, 2, 1)
            })
            .unwrap();
        assert!(open.contains(&long));
        finish(&w, dir);
    }

    /// A compactor whose calls wait until the gate opens, and counts them.
    #[derive(Clone)]
    struct Gate(Arc<(Mutex<bool>, Condvar)>, Arc<AtomicUsize>);

    impl Gate {
        fn new() -> Gate {
            Gate(Arc::default(), Arc::default())
        }

        fn open(&self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }

    impl Chat for Gate {
        fn say(&mut self, _: &[String]) -> Result<String, Fail> {
            self.1.fetch_add(1, Ordering::Relaxed);
            let (open, cv) = &*self.0;
            let _open = cv.wait_while(open.lock().unwrap(), |o| !*o).unwrap();
            Ok("echo: summary".to_owned())
        }
        fn model(&self) -> String {
            "gate".to_owned()
        }
    }

    impl Backend for Gate {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Ok(Box::new(self.clone()))
        }
    }

    #[test]
    fn a_closed_memory_has_landed_every_call_and_starts_none() {
        let dir = std::env::temp_dir().join(format!("hippo-work-close-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("agents/gil-1")).unwrap();
        let gate = Gate::new();
        let w = Workers::new(&dir, 128_000, Some(Arc::new(gate.clone())));
        let long = "g".repeat(900);
        log(&dir, &[("echo", &long), ("echo", &long)]);
        w.wake("gil-1").unwrap();
        let busy = |w: &Workers| {
            w.open("gil-1")
                .unwrap()
                .shared
                .core
                .lock()
                .unwrap()
                .pump
                .running()
        };
        assert_eq!(busy(&w), 2, "both calls run, held at the gate");
        let (closed, heard) = std::sync::mpsc::channel();
        let closing = {
            let w = w.clone();
            thread::spawn(move || {
                w.close();
                closed.send(()).unwrap();
            })
        };
        // Held calls keep it open.
        assert!(heard.recv_timeout(Duration::from_millis(200)).is_err());
        gate.open();
        heard.recv().unwrap();
        closing.join().unwrap();
        // Both landed before it returned, and a closed memory starts nothing.
        let worker = w.open("gil-1").unwrap();
        let built = |i| worker.shared.core.lock().unwrap().tree.built(0, i);
        assert!(built(0) && built(1));
        log(&dir, &[("echo", &long)]);
        worker.tick();
        assert_eq!(busy(&w), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    /// A compactor whose every call panics.
    struct Panics;

    impl Chat for Panics {
        fn say(&mut self, _: &[String]) -> Result<String, Fail> {
            panic!("a summary call that panics");
        }
        fn model(&self) -> String {
            "panics".to_owned()
        }
    }

    impl Backend for Panics {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Ok(Box::new(Panics))
        }
    }

    #[test]
    fn a_call_that_panics_does_not_hold_the_memory_open() {
        let dir = std::env::temp_dir().join(format!("hippo-work-panics-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("agents/gil-1")).unwrap();
        let w = Workers::new(&dir, 128_000, Some(Arc::new(Panics)));
        log(&dir, &[("echo", &"p".repeat(900))]);
        w.wake("gil-1").unwrap();
        let (closed, heard) = std::sync::mpsc::channel();
        let closing = {
            let w = w.clone();
            thread::spawn(move || {
                w.close();
                closed.send(()).unwrap();
            })
        };
        // Bounded, so a hang fails the test rather than holding the suite.
        heard
            .recv_timeout(Duration::from_secs(10))
            .expect("a panicked call left its node busy");
        closing.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_log_past_the_budget_folds_as_the_main_view_does() {
        let (dir, w, _) = rig("fold", 8_000);
        let long = "y".repeat(900);
        let lines: Vec<(&str, &str)> = (0..64).map(|_| ("echo", long.as_str())).collect();
        log(&dir, &lines);
        w.wake("gil-1").unwrap();
        settle(&w, |c| {
            c.view.t == 64 && c.view.unbuilt(&c.tree) == 0 && c.view.size() <= 8_000
        });
        let seen = w.wake("gil-1").unwrap();
        assert!(seen.len() <= 8_000 + 32, "{}", seen.len());
        assert!(!seen.contains(&long));
        assert!(seen.lines().nth(1).unwrap().contains("|"));
        let found = w.read("gil-1", |c| search(&c.store, "yyyy")).unwrap();
        assert_eq!(found.lines().count(), 64);
        finish(&w, dir);
    }

    /// A main memory of `t` messages, summarized and merged by hand, with
    /// `lines` lines of view.
    fn main_memory(dir: &Path, t: u64, budget: u64) -> Core {
        let store = Store::open(&dir.join("main-store"), false).unwrap();
        let mut c = Core::for_test(
            store,
            Tree::open(&dir.join("main-store")).unwrap(),
            View::default(),
            budget,
        );
        let drafts = (0..t).map(|i| Draft {
            display: None,
            kind: Kind::User,
            chat: None,
            text: format!("main message {i}"),
            date: Local::now(),
            src: None,
        });
        c.store.append(drafts.collect()).unwrap();
        for l in 0..8u8 {
            for i in 0..(t >> l) {
                let text = format!("user: main line {l}/{i} {}", "m".repeat(200));
                let (l, i) = (l, i);
                c.tree
                    .save(Node {
                        l,
                        i,
                        size: text.len() as u64,
                        text,
                        model: String::new(),
                        prompt: String::new(),
                    })
                    .unwrap();
            }
        }
        c.sync();
        c
    }

    #[test]
    fn a_fork_keeps_main_addresses_and_folds_them_at_no_cost() {
        let (dir, w, calls) = rig("fork", 8_000);
        let main = main_memory(&dir, 100, 8_000);
        let (view, nodes) = crate::memory::server::fork(&main).unwrap();
        log(&dir, &[("note", "subagent"), ("user", "Go.")]);
        assert!(
            w.fork("gil-1", || Ok((view.clone(), nodes)))
                .unwrap()
                .contains("at 100")
        );
        let long = "e".repeat(900);
        let seen = w.wake("gil-1").unwrap();
        // Main's lines by main's addresses, then its own going on from them.
        let line = view.parts[0];
        let (s, n) = crate::memory::tree::addr(line.0, line.1);
        assert!(
            seen.contains(&format!("\n{s}+{n}|user: main line")),
            "{seen}"
        );
        assert!(seen.contains("100+1|note: subagent"), "{seen}");
        assert!(seen.contains("101+1|user: Go."), "{seen}");
        assert!(seen.len() <= 8_000);
        // Far past the budget, so the fold reaches back into main's lines:
        // those merges are paid only where they cross the fork.
        let lines: Vec<(&str, &str)> = (0..32).map(|_| ("echo", long.as_str())).collect();
        log(&dir, &lines);
        settle(&w, |c| {
            c.view.t == 134 && c.view.unbuilt(&c.tree) == 0 && c.view.size() <= 8_000
        });
        let own_nodes = |c: &Core| (100..134).filter(|&i| c.tree.built(0, i)).count();
        assert_eq!(w.read("gil-1", |c| Ok(own_nodes(c))).unwrap(), 34,);
        let before = |c: &Core, v: &View| {
            v.parts
                .iter()
                .filter(|p| crate::memory::tree::addr(p.0, p.1).0 < c.store.base)
                .count()
        };
        let folded = w.read("gil-1", |c| Ok(before(c, &c.view))).unwrap();
        let forked = w.read("gil-1", |c| Ok(before(c, &view))).unwrap();
        assert!(
            folded < forked,
            "{folded} of {forked}: main's lines never fold"
        );
        // Its 32 summaries, the merges over its 34 lines and one per level
        // across the fork: none for what was main's alone.
        assert!(calls.load(Ordering::Relaxed) <= 32 + 34 + 8, "{calls:?}");
        finish(&w, dir);
    }

    #[test]
    fn a_merge_takes_only_the_workers_own_lines() {
        let (dir, w, _) = rig("merge", 128_000);
        let mut main = main_memory(&dir, 16, 128_000);
        let (view, nodes) = crate::memory::server::fork(&main).unwrap();
        log(&dir, &[("note", "subagent"), ("user", "Go.")]);
        w.fork("gil-1", || Ok((view, nodes))).unwrap();
        // A tool call and its output come in whole, not its words alone.
        let output = format!("line one\n{}\nlast line", "x".repeat(3000));
        log(
            &dir,
            &[
                ("talk", "Looking."),
                ("tool", "Bash {\"command\":\"ls\"}"),
                ("echo", &output),
                ("talk", "Done."),
            ],
        );
        let drafts = w.merge_drafts("gil-1").unwrap();
        let said: Vec<String> = drafts
            .iter()
            .map(|d| crate::memory::store::render(d.kind, &d.text))
            .collect();
        // Its own lines as they were, from the fork at 16; main's 16 are
        // main's already, and so are the note and brief main started it
        // with and the report it gave, "Done.".
        let echoed = format!("echo: {output}");
        assert_eq!(
            said,
            ["talk: Looking.", "tool: Bash {\"command\":\"ls\"}", &echoed]
        );
        assert_eq!(drafts[0].src.as_ref().unwrap().key(), "merged:gil-1:18");
        let out = crate::memory::server::merge(&mut main.store, "Gil", None, drafts).unwrap();
        assert_eq!(out, "Merged 3 messages of Gil's.");
        // Zoomed, main gives the output back as the worker had it.
        assert_eq!(main.store.get(18).unwrap().render(), echoed);
        let again = w.merge_drafts("gil-1").unwrap();
        let out = crate::memory::server::merge(&mut main.store, "Gil", None, again).unwrap();
        assert_eq!(out, "Nothing of Gil's to merge.");
        // Told more, its next run comes in after, less that run's report.
        log(
            &dir,
            &[
                ("user", "And hidden ones?"),
                ("tool", "Bash {}"),
                ("echo", "1"),
                ("talk", "One."),
            ],
        );
        let more = w.merge_drafts("gil-1").unwrap();
        let out = crate::memory::server::merge(&mut main.store, "Gil", None, more).unwrap();
        assert_eq!(out, "Merged 3 messages of Gil's.");
        finish(&w, dir);
    }

    #[test]
    fn a_fork_is_taken_once_and_never_brought_up_to_date() {
        let (dir, w, _) = rig("once", 128_000);
        let main = main_memory(&dir, 16, 128_000);
        let (view, nodes) = crate::memory::server::fork(&main).unwrap();
        log(&dir, &[("note", "subagent"), ("user", "Go.")]);
        w.fork("gil-1", || Ok((view.clone(), nodes.clone())))
            .unwrap();
        assert!(w.fork("gil-1", || Ok((view.clone(), nodes))).is_err());
        assert!(!dir.join("workers/.gil-1").exists());
        finish(&w, dir);
    }

    #[test]
    fn a_slow_snapshot_holds_up_no_worker_and_a_wake_meanwhile_ends_the_fork() {
        let (dir, w, _) = rig("slow", 128_000);
        let main = main_memory(&dir, 16, 128_000);
        let (view, nodes) = crate::memory::server::fork(&main).unwrap();
        log(&dir, &[("note", "subagent"), ("user", "Go.")]);
        fs::create_dir_all(dir.join("agents/pia-1")).unwrap();
        fs::write(
            dir.join("agents/pia-1/log.jsonl"),
            "{\"kind\":\"note\",\"text\":\"subagent\"}\n",
        )
        .unwrap();
        let forking = {
            let w = w.clone();
            thread::spawn(move || {
                w.fork("gil-1", || {
                    // Main's memory busy, past the client's wait.
                    thread::sleep(Duration::from_millis(400));
                    Ok((view, nodes))
                })
            })
        };
        thread::sleep(Duration::from_millis(50));
        let began = std::time::Instant::now();
        assert!(w.answer("pia-1").unwrap().starts_with("fork none\n"));
        assert!(
            began.elapsed() < Duration::from_millis(200),
            "another worker waited"
        );
        // Its own run, started as its spawn gave up, comes first and keeps
        // no fork; the fork then lands nowhere, and the words stay true.
        let said = w.answer("gil-1").unwrap();
        assert!(
            said.starts_with("fork none\n<work>\n0+1|note: subagent"),
            "{said}"
        );
        assert!(forking.join().unwrap().is_err());
        assert!(w.answer("gil-1").unwrap().starts_with("fork none\n"));
        finish(&w, dir);
    }

    #[test]
    fn a_wake_before_a_fork_leaves_the_worker_without_one_and_says_so() {
        let (dir, w, _) = rig("first", 128_000);
        let main = main_memory(&dir, 16, 128_000);
        let (view, nodes) = crate::memory::server::fork(&main).unwrap();
        log(&dir, &[("note", "subagent"), ("user", "Go.")]);
        assert!(
            w.answer("gil-1")
                .unwrap()
                .starts_with("fork none\n<work>\n0+1|note: subagent")
        );
        assert!(w.fork("gil-1", || Ok((view, nodes))).is_err());
        assert!(w.answer("gil-1").unwrap().starts_with("fork none\n"));
        finish(&w, dir);
    }

    #[test]
    fn a_fork_comes_back_from_a_restart_and_goes_on() {
        let (dir, w, calls) = rig("back", 128_000);
        let main = main_memory(&dir, 16, 128_000);
        let (view, nodes) = crate::memory::server::fork(&main).unwrap();
        log(&dir, &[("note", "subagent"), ("user", "Go.")]);
        w.fork("gil-1", || Ok((view.clone(), nodes))).unwrap();
        w.wake("gil-1").unwrap();
        settle(&w, |c| c.view.t == 18);
        w.close();
        log(&dir, &[("talk", "Done.")]);
        let backend: Arc<dyn Backend> = Arc::new(Stand(calls));
        let w = Workers::new(&dir, 128_000, Some(backend));
        let seen = w.wake("gil-1").unwrap();
        assert!(seen.contains("18+1|talk: Done."), "{seen}");
        assert!(seen.contains("16+1|note: subagent"), "{seen}");
        assert_eq!(seen.matches("+1|note: subagent").count(), 1);
        finish(&w, dir);
    }

    fn log_to(dir: &Path, who: &str, lines: &[(&str, &str)]) {
        use std::io::Write;
        fs::create_dir_all(dir.join("agents").join(who)).unwrap();
        let path = dir.join("agents").join(who).join("log.jsonl");
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for (kind, text) in lines {
            let line = serde_json::json!({"kind": kind, "text": text});
            writeln!(f, "{line}").unwrap();
        }
    }

    fn settle_on(w: &Workers, who: &str, want: impl Fn(&Core) -> bool) {
        let worker = w.open(who).unwrap();
        for _ in 0..2000 {
            worker.tick();
            if want(&worker.shared.core.lock().unwrap()) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("{who}'s memory never settled");
    }

    #[test]
    fn a_sub_forks_its_heads_memory_and_reaches_main_through_it() {
        const BUDGET: u64 = 128_000;
        let (dir, w, _) = rig("chain", BUDGET);
        let main = main_memory(&dir, 400, BUDGET);
        let (view, nodes) = crate::memory::server::fork(&main).unwrap();
        let long = "h".repeat(2_000);
        let mut head: Vec<(&str, &str)> = vec![("note", "subagent"), ("user", "Head job.")];
        head.extend((0..120).map(|_| ("echo", long.as_str())));
        log_to(&dir, "ada-1", &head);
        w.fork("ada-1", || Ok((view, nodes))).unwrap();
        w.wake("ada-1").unwrap();
        let head_t = 400 + head.len() as u64;
        settle_on(&w, "ada-1", |c| c.view.t == head_t && built(c));
        // A sub forks the head, and the head no further.
        log_to(&dir, "gil-1", &[("note", "subagent"), ("user", "Sub job.")]);
        assert!(
            w.fork_worker("gil-1", "ada-1")
                .unwrap()
                .contains(&format!("at {head_t}"))
        );
        log_to(&dir, "pia-1", &[("note", "subagent")]);
        assert!(
            w.fork_worker("pia-1", "gil-1")
                .unwrap_err()
                .contains("two levels")
        );
        let said = w.answer("gil-1").unwrap();
        assert!(
            said.starts_with(&format!("fork {head_t} ada-1 400\n")),
            "{}",
            &said[..80]
        );
        assert!(w.answer("ada-1").unwrap().starts_with("fork 400\n"));
        // Main's lines, then the head's, then its own after the head's last.
        assert!(said.contains("|user: main line"), "{said}");
        assert!(
            said.contains(&format!("{head_t}+1|note: subagent")),
            "{said}"
        );
        assert!(
            said.contains(&format!("{}+1|user: Sub job.", head_t + 1)),
            "{said}"
        );
        // Each line opens where it was said.
        assert_eq!(w.holder("gil-1", 3, 1).unwrap(), None);
        assert_eq!(w.holder("gil-1", 401, 1).unwrap().as_deref(), Some("ada-1"));
        assert_eq!(
            w.holder("gil-1", head_t, 1).unwrap().as_deref(),
            Some("gil-1")
        );
        let opened = w
            .read("ada-1", |c| {
                crate::memory::server::zoom(&c.store, &c.tree, 401, 1)
            })
            .unwrap();
        assert!(opened.contains("Head job."), "{opened}");
        // The sub works past its budget and folds back into its head's and
        // main's lines, staying under it, as the head's does.
        let mut sub: Vec<(&str, &str)> = Vec::new();
        sub.extend((0..120).map(|_| ("echo", long.as_str())));
        log_to(&dir, "gil-1", &sub);
        let sub_t = head_t + 2 + 120;
        settle_on(&w, "gil-1", |c| {
            c.view.t == sub_t && built(c) && c.view.size() <= BUDGET
        });
        let head_seen = w.wake("ada-1").unwrap().len();
        let sub_seen = w.wake("gil-1").unwrap().len();
        eprintln!("wake bytes: head {head_seen}, sub {sub_seen}, budget {BUDGET}");
        assert!(sub_seen as u64 <= BUDGET + 32, "{sub_seen}");
        finish(&w, dir);
    }

    fn search(store: &Store, pattern: &str) -> Result<String, String> {
        crate::memory::server::search(store, &regex::Regex::new(pattern).unwrap())
    }

    #[test]
    fn a_memory_comes_back_from_a_restart_mid_fold_without_repeating_a_word() {
        let (dir, w, calls) = rig("restart", 128_000);
        let long = "z".repeat(900);
        log(
            &dir,
            &[("note", "subagent"), ("echo", &long), ("echo", &long)],
        );
        w.wake("gil-1").unwrap();
        settle(&w, built);
        let spent = calls.load(Ordering::Relaxed);
        log(&dir, &[("echo", &long), ("talk", "Done.")]);
        w.close();
        // A new service: nothing of the old one's but what is on disk.
        let backend: Arc<dyn Backend> = Arc::new(Stand(calls.clone()));
        let w = Workers::new(&dir, 128_000, Some(backend));
        w.wake("gil-1").unwrap();
        settle(&w, |c| built(c) && c.view.t == 5);
        let len = w.read("gil-1", |c| Ok(c.store.len())).unwrap();
        assert_eq!(len, 5);
        // The new echo and the merges over it; nothing built twice.
        assert_eq!(calls.load(Ordering::Relaxed), spent + 3);
        finish(&w, dir);
    }

    #[test]
    fn an_archived_worker_makes_no_calls_and_can_still_be_read() {
        let (dir, w, calls) = rig("archived", 128_000);
        let long = "a".repeat(900);
        log(&dir, &[("note", "subagent"), ("echo", &long)]);
        let record = dir.join("agents/gil-1/state.json");
        fs::write(&record, r#"{"phase":{"state":"archived","at":1}}"#).unwrap();
        let seen = w.wake("gil-1").unwrap();
        assert!(seen.contains(&long));
        thread::sleep(Duration::from_millis(50));
        w.open("gil-1").unwrap().tick();
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert!(search_of(&w, "aaaa").contains("echo"));
        fs::write(&record, r#"{"phase":{"state":"idle"}}"#).unwrap();
        settle(&w, built);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        finish(&w, dir);
    }

    fn search_of(w: &Workers, pattern: &str) -> String {
        w.read("gil-1", |c| search(&c.store, pattern)).unwrap()
    }

    #[test]
    fn a_read_alone_starts_no_calls() {
        let (dir, w, calls) = rig("read", 128_000);
        let long = "b".repeat(900);
        log(&dir, &[("echo", &long)]);
        assert!(search_of(&w, "bbbb").contains("echo"));
        thread::sleep(Duration::from_millis(50));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        finish(&w, dir);
    }

    #[test]
    fn a_worker_never_writes_to_its_log() {
        let (dir, w, _) = rig("untouched", 128_000);
        let long = "c".repeat(900);
        log(
            &dir,
            &[("user", "Go."), ("echo", &long), ("from", "Lark: fine")],
        );
        let path = dir.join("agents/gil-1/log.jsonl");
        let before = fs::read(&path).unwrap();
        w.wake("gil-1").unwrap();
        settle(&w, built);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(dir.join("workers/gil-1/main").is_dir());
        assert!(w.wake("gil-1").unwrap().contains("2+1|from: Lark: fine"));
        finish(&w, dir);
    }

    #[test]
    fn a_line_in_the_writing_waits_and_numbers_match_the_logs_readers() {
        let (dir, w, _) = rig("torn", 128_000);
        log(&dir, &[("note", "subagent")]);
        let path = dir.join("agents/gil-1/log.jsonl");
        let mut raw = fs::read(&path).unwrap();
        raw.extend_from_slice(b"not json\n{\"kind\":\"user\",\"text\":\"half");
        fs::write(&path, raw).unwrap();
        assert!(w.wake("gil-1").unwrap().contains("0+1|note: subagent"));
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "\"}}").unwrap();
        let seen = w.wake("gil-1").unwrap();
        // The bad line takes no number, as `task::log` reads it.
        assert!(seen.contains("1+1|user: half"), "{seen}");
        finish(&w, dir);
    }

    #[test]
    fn a_wake_with_no_compactor_stays_within_its_bound() {
        let dir = std::env::temp_dir().join(format!("hippo-work-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("agents/gil-1")).unwrap();
        let w = Workers::new(&dir, 128_000, None);
        let big = "d".repeat(4_000);
        let lines: Vec<(&str, &str)> = (0..600).map(|_| ("echo", big.as_str())).collect();
        log(&dir, &lines);
        let seen = w.wake("gil-1").unwrap();
        assert!(seen.len() <= WHOLE_MAX + 1024, "{}", seen.len());
        assert!(seen.contains("599+1|echo:"));
        assert!(seen.contains("0+"));
        assert!(seen.contains("not summarized yet"));
        finish(&w, dir);
    }
}
