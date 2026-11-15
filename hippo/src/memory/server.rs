//! The service: the only writer. In code mode its main loop follows the
//! transcripts; in chat mode the agent logs each message over the socket.
//! The CLI talks to it over `hippo.sock`, one JSON request and reply per
//! connection.

use crate::code::Follow;
use crate::code::live::Sources;
use crate::memory::compact::Backend;
use crate::memory::compactor::{Pump, Spent, pause_file, pump};
use crate::memory::store::{
    ChatRec, Draft, Kind, Src, Store, append_line, chat_key, day_name, fmt_date, parse_date,
};
use crate::memory::tree::{Node, Tree, addr, flat};
use crate::memory::view::{CONTEXT, PLACEHOLDER, VIEW, View, head, tiles};
use crate::memory::work::{MERGED, Workers};
use chrono::{DateTime, Local};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    pub out: String,
}

/// What stops a memory's compactor while it holds.
pub enum Stop {
    /// The file exists: `hippo pause`.
    File(PathBuf),
    /// The Libri worker in this folder is archived.
    Archived(PathBuf),
}

impl Stop {
    fn holds(&self) -> bool {
        match self {
            Stop::File(p) => p.exists(),
            Stop::Archived(d) => crate::code::source::archived(d),
        }
    }
}

pub struct Core {
    pub store: Store,
    pub tree: Tree,
    pub view: View,
    pub budget: u64,
    /// The compactions' own view, under `CONTEXT`, to the first message
    /// not built.
    pub context: View,
    pub pump: Pump,
    /// Code mode's transcript follower; chat mode has none.
    pub follow: Option<Follow>,
    pub started: DateTime<Local>,
    /// Rendered views being read page by page, by token.
    pages: Vec<(String, Vec<String>)>,
    /// The view changed since `view.md` was last written.
    pub stale: bool,
    /// While any of these holds the compactor makes no model calls.
    pub stops: Vec<Stop>,
    /// The worker this memory is, for the compactor's calls; None is main's.
    pub whose: Option<String>,
    /// The memory is closed for good: its compactor starts nothing more.
    pub closed: bool,
}

impl Core {
    #[cfg(test)]
    pub fn for_test(store: Store, tree: Tree, view: View, budget: u64) -> Core {
        Core {
            store,
            tree,
            view,
            budget,
            context: View::default(),
            pump: Pump::default(),
            follow: None,
            started: Local::now(),
            pages: Vec::new(),
            stale: true,
            stops: Vec::new(),
            whose: None,
            closed: false,
        }
    }

    /// The memory over `store`: its tree and both views, `budget` the
    /// view's size. The same for the main memory and every worker's.
    pub fn open(
        store: Store,
        follow: Option<Follow>,
        budget: u64,
        stops: Vec<Stop>,
        pump: Pump,
    ) -> Result<Core, String> {
        let mut store = store;
        let tree = Tree::open(&store.dir)?;
        store.reach(
            tree.nodes()
                .filter(|n| n.0 == 0)
                .map(|n| n.1 + 1)
                .max()
                .unwrap_or(0),
        );
        for at in store.torn.iter().chain(&tree.torn) {
            eprintln!("hippo: damage in the store: {at}");
        }
        let dir = store.dir.clone();
        let view = View::load(&dir, "view.json", &tree, store.len(), budget);
        let context = View::load(
            &dir,
            "context.json",
            &tree,
            store.len(),
            CONTEXT.min(budget),
        );
        let mut pump = pump;
        if store.base > 0 {
            pump.lo = view.floors(&tree, store.base);
        }
        let mut c = Core {
            store,
            tree,
            view,
            budget,
            context,
            pump,
            follow,
            started: Local::now(),
            pages: Vec::new(),
            stale: true,
            stops,
            whose: None,
            closed: false,
        };
        // A saved compactions' view may reach past what is built.
        c.fit(false);
        Ok(c)
    }

    /// Whether the compactor is stopped.
    pub fn paused(&self) -> bool {
        self.closed || self.stops.iter().any(Stop::holds)
    }

    pub fn context_budget(&self) -> u64 {
        CONTEXT.min(self.budget)
    }

    /// Both views changed only by appends and merges; the next start loads
    /// them instead of folding new ones, which would break the cache.
    pub fn save_views(&self) -> Result<(), String> {
        self.view.save(&self.store.dir, "view.json")?;
        self.context.save(&self.store.dir, "context.json")
    }

    /// Brings the view up to the log after messages were appended.
    pub fn sync(&mut self) {
        while self.view.t < self.store.len() {
            self.view.push();
            self.fit(true);
            self.stale = true;
        }
    }

    /// UniiChat's `Memory.fit`: the view to the log's end and the
    /// compactions' to the first message not built, each merged as `fit`
    /// allows; `next` at a new message.
    pub fn fit(&mut self, next: bool) {
        let first = self.first(0);
        self.context.reach(first);
        self.view.fit(&self.tree, self.budget, next);
        let budget = self.context_budget();
        self.context.fit(&self.tree, budget, next);
    }

    /// UniiChat's `Memory.first`: the message of the (k+1)th leaf not
    /// built, else the view's end.
    pub fn first(&mut self, k: usize) -> u64 {
        let lo = &mut self.pump.lo;
        if lo.is_empty() {
            lo.push(0);
        }
        while lo[0] < self.view.t && self.tree.built(0, lo[0]) {
            lo[0] += 1;
        }
        (lo[0]..self.view.t)
            .filter(|&i| !self.tree.built(0, i))
            .nth(k)
            .unwrap_or(self.view.t)
    }
}

pub struct Shared {
    pub core: Mutex<Core>,
    /// Signalled whenever the view may have changed.
    pub cv: Condvar,
    pub backend: Option<Arc<dyn Backend>>,
    /// The workers' memories, in the main memory.
    pub workers: Option<Arc<Workers>>,
}

impl Shared {
    /// Closes the memory to calls: its compactor starts none, and those
    /// under way land first, so no call of it writes once this returns.
    /// `tick` still reads the log and saves views, so a test ends its ticks
    /// before removing the folder. A call's thread is its own, so this waits
    /// for the store to say none is running, not for a time. A hold after a
    /// failure is the pump's, so it is not waited out.
    #[cfg(test)]
    pub fn close(&self) {
        let mut c = self.core.lock().unwrap();
        c.closed = true;
        while c.pump.running() > 0 {
            c = self.cv.wait(c).unwrap();
        }
    }
}

const SEARCH_MAX: usize = 100;
const TAIL_MAX: u64 = 500;
/// How long `hippo view` waits for the compactor before printing
/// placeholders.
#[cfg(not(test))]
const SETTLE: Duration = Duration::from_secs(30);
#[cfg(test)]
const SETTLE: Duration = Duration::from_millis(400);
/// How long a client may go quiet in the middle of its request or reply: each
/// connection holds a thread, and clients of this socket answer at once.
#[cfg(not(test))]
const READ_WAIT: Duration = Duration::from_secs(10);
#[cfg(test)]
const READ_WAIT: Duration = Duration::from_millis(100);
/// Bytes per page of `hippo view`, under the harnesses' output limits.
const PAGE: usize = 24_000;

/// Runs the service: in code mode with `sources`, in chat mode without.
/// Chat mode also serves and compacts a store nothing writes to (a replayed
/// scratch store, say).
pub fn serve(
    dir: &Path,
    sources: Option<Sources>,
    backend: Option<Box<dyn Backend>>,
) -> Result<(), String> {
    let store = Store::open(dir, true)?;
    let follow = match sources {
        Some(s) => Some(Follow::new(dir, &store, s)?),
        None => None,
    };
    let budget = match std::env::var("HIPPO_VIEW") {
        Ok(v) => v
            .parse()
            .map_err(|_| format!("HIPPO_VIEW is not a number: {v}"))?,
        Err(_) => VIEW,
    };
    let spent = Spent::load(dir)?;
    let compactor = Pump {
        spent,
        ..Pump::default()
    };
    let core = Core::open(
        store,
        follow,
        budget,
        vec![Stop::File(pause_file(dir))],
        compactor,
    )?;
    let backend: Option<Arc<dyn Backend>> = backend.map(Arc::from);
    let workers = Workers::new(dir, budget, backend.clone());
    workers.clone().run();
    let shared = Arc::new(Shared {
        core: Mutex::new(core),
        cv: Condvar::new(),
        backend,
        workers: Some(workers),
    });
    crate::memory::find::serve(dir);
    let sock = dir.join("hippo.sock");
    let _ = fs::remove_file(&sock);
    listen(&sock, &shared)?;
    eprintln!("hippo: serving {}", dir.display());
    loop {
        {
            let mut guard = shared.core.lock().unwrap();
            let c = &mut *guard;
            // A failed write is reported and tried again, not the end of the
            // service: a host's own hippo has no one to restart it.
            if let Some(f) = c.follow.as_mut() {
                let stepped = f.step(&mut c.store);
                c.pump.wrote("store", &stepped);
            }
            c.sync();
            pump(&shared, c);
            if c.stale {
                let published = publish(c);
                c.pump.wrote("view", &published);
            }
        }
        // A socket gone from the directory would leave this process serving
        // no one, with nothing to restart it: as Libri's compactor.sock.
        if !sock.exists() {
            match listen(&sock, &shared) {
                Ok(()) => eprintln!("hippo: {} was gone; bound again", sock.display()),
                Err(e) => eprintln!("hippo: {e}"),
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
}

/// Serves `sock` from a thread of its own, each connection on its own
/// thread. No thread to be had drops that connection, whose client sees it
/// closed, and leaves the socket serving: a panic here would leave the
/// process alive and refusing everyone.
fn listen(sock: &Path, shared: &Arc<Shared>) -> Result<(), String> {
    let listener = UnixListener::bind(sock).map_err(|e| format!("{}: {e}", sock.display()))?;
    let shared = shared.clone();
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let shared = shared.clone();
            let spawned = thread::Builder::new().spawn(move || {
                if let Err(e) = answer(conn, &shared) {
                    eprintln!("hippo: request failed: {e}");
                }
            });
            if let Err(e) = spawned {
                eprintln!("hippo: no thread for a connection: {e}");
            }
        }
    });
    Ok(())
}

fn answer(conn: UnixStream, shared: &Arc<Shared>) -> Result<(), String> {
    conn.set_read_timeout(Some(READ_WAIT))
        .and_then(|()| conn.set_write_timeout(Some(READ_WAIT)))
        .map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&conn)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let reply = match serde_json::from_str::<Request>(&line) {
        Ok(req) => match handle(&req, shared) {
            Ok(out) => Reply { ok: true, out },
            Err(out) => Reply { ok: false, out },
        },
        Err(e) => Reply {
            ok: false,
            out: format!("bad request: {e}"),
        },
    };
    let mut raw = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
    raw.push(b'\n');
    (&conn).write_all(&raw).map_err(|e| e.to_string())
}

fn num(args: &[String], k: usize, what: &str) -> Result<u64, String> {
    args.get(k)
        .ok_or_else(|| format!("Missing {what}."))?
        .parse()
        .map_err(|_| format!("{what} must be a number."))
}

/// The harness name of chats posted over the socket, not read from a
/// transcript.
const POSTED: &str = "yeoman";

/// A kind a client may tell the memory: what a harness's turn holds, its
/// calls and their results among it, as a transcript would.
fn told(kind: &str) -> Option<Kind> {
    Some(match kind {
        "user" => Kind::User,
        "talk" => Kind::Talk,
        "note" => Kind::Note,
        "work" => Kind::Work,
        "tool" => Kind::Tool,
        "echo" => Kind::Echo,
        _ => return None,
    })
}

/// The view as it stands, up to the first message not built, and the nodes
/// that hold it.
pub fn fork(c: &Core) -> Result<(View, Vec<Node>), String> {
    let mut view = c.view.clone();
    view.truncate(view.first(&c.tree));
    let mut nodes = Vec::new();
    for (l, i) in view.lineage() {
        nodes.extend(c.tree.get(l, i)?);
    }
    Ok((view, nodes))
}

/// A case-insensitive pattern, as every search takes it.
fn regex_of(pattern: Option<&String>) -> Result<regex::Regex, String> {
    RegexBuilder::new(pattern.ok_or("Missing regex.")?)
        .case_insensitive(true)
        .build()
        .map_err(|e| e.to_string())
}

fn workers(shared: &Shared) -> Result<&Arc<Workers>, String> {
    shared
        .workers
        .as_ref()
        .ok_or_else(|| "This memory keeps no workers.".to_owned())
}

fn handle(req: &Request, shared: &Arc<Shared>) -> Result<String, String> {
    let a = &req.args;
    let core = &shared.core;
    match req.cmd.as_str() {
        "view" => view(shared, a),
        "zoom" => {
            let (id, n) = (num(a, 0, "id")?, num(a, 1, "n")?);
            let c = core.lock().unwrap();
            zoom(&c.store, &c.tree, id, n)
        }
        "date" => {
            let id = num(a, 0, "id")?;
            let c = core.lock().unwrap();
            c.store
                .date(id)
                .map(|d| d.format("%Y-%m-%d %H:%M:%S %a").to_string())
                .ok_or_else(|| format!("No message {id}."))
        }
        // The last messages whole, for programs that show a conversation.
        "tail" => {
            let n = num(a, 0, "n")?.min(TAIL_MAX);
            tail(&core.lock().unwrap().store, n, a.get(1).map(String::as_str))
        }
        "chat" => {
            let [harness, session] = a.as_slice() else {
                return Err("Usage: hippo chat <harness> <session>".into());
            };
            this_chat(&core.lock().unwrap().store, harness, session)
        }
        // For people: words, ranked, main's and the workers' together, as
        // JSON. `find <words> [best|new] [limit]`.
        "find" => {
            use crate::memory::find::{Order, with};
            let words = a.first().ok_or("Missing words.")?;
            let order = match a.get(1).map(String::as_str) {
                None | Some("best") => Order::Best,
                Some("new") => Order::New,
                Some(o) => return Err(format!("No order {o}: best or new.")),
            };
            let limit = a.get(2).map_or(Ok(60), |_| num(a, 2, "limit"))? as usize;
            let (hits, loose) = with(|ix| ix.find(words, order, limit))?;
            Ok(serde_json::json!({ "hits": hits, "loose": loose }).to_string())
        }
        // `around <source> <at> <before> <after>`: a hit in its place, as JSON.
        "around" => {
            let src = a.first().ok_or("Missing source.")?;
            let (at, before, after) = (num(a, 1, "at")?, num(a, 2, "before")?, num(a, 3, "after")?);
            let lines = crate::memory::find::with(|ix| {
                ix.around(src, at, before.min(200), after.min(200))
            })?;
            serde_json::to_string(&lines).map_err(|e| e.to_string())
        }
        // The workers search reaches, named, as JSON.
        "sources" => {
            let all = crate::memory::find::with(|ix| ix.sources())?;
            serde_json::to_string(&all).map_err(|e| e.to_string())
        }
        "search" => {
            let re = regex_of(a.first())?;
            let c = core.lock().unwrap();
            search(&c.store, &re)
        }
        // A worker's own memory, built as this one is: `work` at each wake
        // gives what the run reads.
        "work" => {
            let [name] = a.as_slice() else {
                return Err("Name the worker.".into());
            };
            workers(shared)?.answer(name)
        }
        // A line from before the fork is the memory forked's, by the same
        // id: the head's for a sub-worker, then main's.
        "work-zoom" => {
            let name = a.first().ok_or("Name the worker.")?;
            let (id, n) = (num(a, 1, "id")?, num(a, 2, "n")?);
            let w = workers(shared)?;
            match w.holder(name, id, n)? {
                Some(at) => w.read(&at, |c| zoom(&c.store, &c.tree, id, n)),
                None => {
                    let c = core.lock().unwrap();
                    zoom(&c.store, &c.tree, id, n)
                }
            }
        }
        // `fork <worker>` forks main's memory; `fork <worker> <parent>` the
        // parent worker's.
        "fork" => match a.as_slice() {
            [name] => workers(shared)?.fork(name, || fork(&core.lock().unwrap())),
            [name, parent] => workers(shared)?.fork_worker(name, parent),
            _ => Err("Name the worker.".into()),
        },
        // `merge <worker> <name> [title]`: the worker's messages since its
        // fork, or since its last merge, join this memory at its end as if
        // said in the chat so titled.
        "merge" => {
            let (name, label, title) = match a.as_slice() {
                [n, l] => (n, l, None),
                [n, l, t] => (n, l, Some(t.as_str())),
                _ => return Err("Usage: merge <worker> <name> [title]".into()),
            };
            let drafts = workers(shared)?.merge_drafts(name)?;
            let mut c = core.lock().unwrap();
            let kept = merge(&mut c.store, label, title, drafts);
            c.pump.wrote("store", &kept);
            let out = kept?;
            c.sync();
            pump(shared, &mut c);
            Ok(out)
        }
        "work-search" => {
            let name = a.first().ok_or("Name the worker.")?;
            let re = regex_of(a.get(1))?;
            workers(shared)?.read(name, |c| search(&c.store, &re))
        }
        // Only a store that follows nothing takes messages over the
        // socket: one that follows transcripts learns everything there, and
        // a logged `user` line would put words in the user's mouth.
        "log" => {
            if core.lock().unwrap().follow.is_some() {
                return Err("This memory follows transcripts; it takes no logged messages.".into());
            }
            let (kind, text) = a.split_first().ok_or("Missing kind.")?;
            let kind = told(kind).ok_or_else(|| format!("Cannot log a {kind} message."))?;
            let text = text.join(" ");
            if text.trim().is_empty() {
                return Err("Nothing to log.".into());
            }
            let mut c = core.lock().unwrap();
            let kept = c.store.append(vec![Draft {
                display: None,
                kind,
                chat: None,
                text: crate::memory::mask::mask(text.trim()),
                date: Local::now(),
                src: None,
            }]);
            c.pump.wrote("store", &kept);
            let ids = kept?;
            c.sync();
            pump(shared, &mut c);
            Ok(format!("Logged as message {}.", ids[0]))
        }
        // A client that shares a store with followed transcripts keeps its
        // own chat here, under its title: its turns are in no transcript.
        "post" => {
            let [title, kind, text @ ..] = a.as_slice() else {
                return Err("Usage: post <title> <kind> <text>".into());
            };
            let kind = told(kind).ok_or_else(|| format!("Cannot post a {kind} message."))?;
            let text = text.join(" ");
            if title.trim().is_empty() || text.trim().is_empty() {
                return Err("Nothing to post.".into());
            }
            let mut c = core.lock().unwrap();
            let kept = post(&mut c.store, title, kind, &text);
            c.pump.wrote("store", &kept);
            let i = kept?;
            c.sync();
            pump(shared, &mut c);
            Ok(format!("Posted as message {i}."))
        }
        // Words a client kept aside while this service was away, or may have
        // reached it unanswered: dated when said, and once under their key.
        "keep" | "keep_display" => {
            let [key, date, title, kind, text @ ..] = a.as_slice() else {
                return Err("Usage: keep <key> <date> <title> <kind> <text>".into());
            };
            let kind = told(kind).ok_or_else(|| format!("Cannot keep a {kind} message."))?;
            let (display, text) = if req.cmd == "keep_display" {
                let (record, words) = text.split_first().ok_or("Missing display record")?;
                (
                    Some(
                        serde_json::from_str::<serde_json::Value>(record)
                            .map_err(|e| e.to_string())?,
                    ),
                    words.join(" "),
                )
            } else {
                (None, text.join(" "))
            };
            if text.trim().is_empty() {
                return Err("Nothing to keep.".into());
            }
            let mut c = core.lock().unwrap();
            if title.is_empty() && c.follow.is_some() {
                return Err("This memory follows transcripts; it takes no logged messages.".into());
            }
            let kept = keep_display(&mut c.store, key, date, title, kind, &text, display);
            c.pump.wrote("store", &kept);
            let i = kept?;
            c.sync();
            pump(shared, &mut c);
            Ok(match i {
                Some(i) => format!("Kept as message {i}."),
                None => "Already kept.".to_owned(),
            })
        }
        "status" => {
            let mut out = status(&core.lock().unwrap(), shared.backend.is_some());
            if let Some(w) = &shared.workers {
                out.push('\n');
                out.push_str(&w.status());
            }
            Ok(out)
        }
        "health" => Ok(health(&mut core.lock().unwrap(), shared.backend.is_some()).to_string()),
        "pause" => {
            let c = core.lock().unwrap();
            fs::write(crate::memory::compactor::pause_file(&c.store.dir), "")
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "Paused: no new model calls; {} running finish. Logging goes on.",
                c.pump.running()
            ))
        }
        "resume" => {
            let mut c = core.lock().unwrap();
            match fs::remove_file(crate::memory::compactor::pause_file(&c.store.dir)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
            pump(shared, &mut c);
            Ok("Resumed.".into())
        }
        // A memory that forgets (amnesia's): what is older than the minutes
        // given goes, oldest first.
        "forget" => {
            let minutes = num(a, 0, "minutes")?;
            let mut guard = core.lock().unwrap();
            let out = forget(&mut guard, minutes);
            pump(shared, &mut guard);
            out
        }
        other => Err(format!("Unknown command {other}.")),
    }
}

/// Drops the messages sent more than `minutes` ago and every summary that
/// read them, and numbers the rest from 0 again: the log, the tree and both
/// views are rewritten. A level-0 summary moves with its message; a merge
/// moves only while it still covers a whole aligned span, the rest are
/// built again as the views need them.
fn forget(c: &mut Core, minutes: u64) -> Result<String, String> {
    if c.follow.is_some() {
        return Err("This memory follows transcripts; it forgets nothing.".into());
    }
    let cutoff = Local::now().timestamp() - (minutes * 60) as i64;
    let k = c.store.older(cutoff);
    if k == 0 {
        return Ok("Forgot 0 messages.".into());
    }
    // A summary under way would land under its old number.
    if c.pump.running() > 0 {
        return Ok("Forgot 0 messages: the compactor is busy, try again.".into());
    }
    let dir = c.store.dir.clone();
    let (main, tree) = (dir.join("main"), dir.join("tree"));
    let (main_new, tree_new) = (dir.join("main.new"), dir.join("tree.new"));
    for d in [&main_new, &tree_new] {
        let _ = fs::remove_dir_all(d);
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut out: Option<(chrono::NaiveDate, fs::File)> = None;
    for i in k..c.store.len() {
        let mut m = c.store.get(i)?;
        m.i -= k;
        let day = parse_date(&m.date).map_or(Local::now().date_naive(), |d| d.date_naive());
        // Ids stay in file order: a day never goes back.
        let day = out.as_ref().map_or(day, |(d, _)| day.max(*d));
        if out.as_ref().is_none_or(|(d, _)| *d != day) {
            let path = main_new.join(day_name(day));
            let f = fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out = Some((day, f));
        }
        let line = serde_json::to_string(&m).map_err(|e| e.to_string())?;
        append_line(&mut out.as_mut().unwrap().1, &line)?;
    }
    let path = tree_new.join(day_name(Local::now().date_naive()));
    let mut nodes = fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let kept: Vec<(u8, u64)> = c.tree.nodes().collect();
    for (l, i) in kept {
        let (s, n) = addr(l, i);
        if s < k || !k.is_multiple_of(n) {
            continue;
        }
        if let Some(mut node) = c.tree.get(l, i)? {
            node.i = (s - k) / n;
            let line = serde_json::to_string(&node).map_err(|e| e.to_string())?;
            append_line(&mut nodes, &line)?;
        }
    }
    // The old tree goes first: cut short anywhere after, the tree is
    // missing or new and the log old or new, and a missing tree is only
    // built again.
    let gone = dir.join("tree.gone");
    let _ = fs::remove_dir_all(&gone);
    fs::rename(&tree, &gone).map_err(|e| e.to_string())?;
    let main_gone = dir.join("main.gone");
    let _ = fs::remove_dir_all(&main_gone);
    fs::rename(&main, &main_gone).map_err(|e| e.to_string())?;
    fs::rename(&main_new, &main).map_err(|e| e.to_string())?;
    fs::rename(&tree_new, &tree).map_err(|e| e.to_string())?;
    fs::remove_dir_all(&gone).map_err(|e| e.to_string())?;
    fs::remove_dir_all(&main_gone).map_err(|e| e.to_string())?;
    c.store.reload()?;
    c.tree = Tree::open(&dir)?;
    let len = c.store.len();
    c.view = View::fold(&c.tree, len, c.budget);
    c.context = View::fold(&c.tree, len, c.context_budget());
    c.pump.lo.clear();
    // Nodes were numbered by the messages forgotten.
    c.pump.nodes.clear();
    c.pages.clear();
    c.fit(false);
    publish(c)?;
    Ok(format!("Forgot {k} messages; {len} left."))
}

/// How many of the captain's messages the chat's own block quotes.
const OWN_USER: usize = 4;
/// A quoted message is cut past this many characters; `hippo zoom` has it.
const OWN_CUT: usize = 800;

/// `hippo chat <harness> <session>`: one chat's latest words, verbatim,
/// for the hook that reopens it after a compaction or a resume. The view
/// holds every chat in one timeline, so without this a compacted chat can
/// take up another's work. The captain's last few messages and the chat's
/// last reply, oldest first; nothing for a chat with no messages yet.
/// The last `n` messages whole, one JSON object a line, oldest first, for
/// programs that show a conversation (libri's web chat loads its history
/// so). With a title, only those of the chats that go by it: a Libri seat
/// runs each turn as a session of its own, all under one name.
/// Appends one message to the chat titled `title`, registering it first.
fn post(store: &mut Store, title: &str, kind: Kind, text: &str) -> Result<u64, String> {
    let now = Local::now();
    let label = title.to_lowercase().replace(char::is_whitespace, "-");
    store.register(ChatRec {
        chat: label.clone(),
        harness: POSTED.into(),
        session: title.into(),
        title: title.into(),
        cwd: String::new(),
        first: fmt_date(&now),
    })?;
    let e = format!("post-{}", store.len());
    let ids = store.append(vec![Draft {
        display: None,
        kind,
        chat: Some(label),
        text: crate::memory::mask::mask(text.trim()),
        date: now,
        src: Some(Src {
            h: POSTED.into(),
            s: title.into(),
            e,
        }),
    }])?;
    Ok(ids[0])
}

/// Takes in a worker's messages at a merge, skipping those an earlier merge
/// took, as a pull request's commits join a branch: after what the memory
/// has, each as it was, and in the chat titled `title` as its own words are,
/// so the compactor, a search and a zoom treat them as the chat's own.
pub(crate) fn merge(
    store: &mut Store,
    label: &str,
    title: Option<&str>,
    drafts: Vec<Draft>,
) -> Result<String, String> {
    let mut new: Vec<Draft> = drafts
        .into_iter()
        .filter(|d| d.src.as_ref().is_some_and(|s| !store.seen(&s.key())))
        .collect();
    let Some(first) = new.first() else {
        return Ok(format!("Nothing of {label}'s to merge."));
    };
    if let (Some(title), Some(src)) = (title.filter(|t| !t.is_empty()), &first.src) {
        let chat = title.to_lowercase().replace(char::is_whitespace, "-");
        store.register(ChatRec {
            chat: chat.clone(),
            harness: MERGED.into(),
            session: src.s.clone(),
            title: title.into(),
            cwd: String::new(),
            first: fmt_date(&first.date),
        })?;
        for d in &mut new {
            d.chat = Some(chat.clone());
        }
    }
    let ids = store.append(new)?;
    let n = ids.len();
    let s = if n == 1 { "" } else { "s" };
    Ok(format!("Merged {n} message{s} of {label}'s."))
}

/// Keeps a client's words under its own key, with no chat when it has no
/// title; the id they took, or none when that key was kept before.
#[cfg(test)]
fn keep(
    store: &mut Store,
    key: &str,
    date: &str,
    title: &str,
    kind: Kind,
    text: &str,
) -> Result<Option<u64>, String> {
    keep_display(store, key, date, title, kind, text, None)
}

#[allow(clippy::too_many_arguments)]
fn keep_display(
    store: &mut Store,
    key: &str,
    date: &str,
    title: &str,
    kind: Kind,
    text: &str,
    display: Option<serde_json::Value>,
) -> Result<Option<u64>, String> {
    let date = parse_date(date).ok_or_else(|| format!("Bad date {date}."))?;
    let label = (!title.is_empty()).then(|| title.to_lowercase().replace(char::is_whitespace, "-"));
    if let Some(label) = &label {
        store.register(ChatRec {
            chat: label.clone(),
            harness: POSTED.into(),
            session: title.into(),
            title: title.into(),
            cwd: String::new(),
            first: fmt_date(&date),
        })?;
    }
    let ids = store.append(vec![Draft {
        display,
        kind,
        chat: label,
        text: crate::memory::mask::mask(text.trim()),
        date,
        src: Some(Src {
            h: POSTED.into(),
            s: title.into(),
            e: key.to_owned(),
        }),
    }])?;
    Ok(ids.first().copied())
}

fn tail(store: &Store, n: u64, title: Option<&str>) -> Result<String, String> {
    let ids: Vec<u64> = match title {
        Some(title) => {
            let keys: HashSet<&str> = store
                .chats
                .iter()
                .filter(|(_, r)| r.title == title)
                .map(|(k, _)| k.as_str())
                .collect();
            let mut ids: Vec<u64> = (0..store.len())
                .rev()
                .filter(|&i| store.chat(i).is_some_and(|k| keys.contains(k)))
                .take(n as usize)
                .collect();
            ids.reverse();
            ids
        }
        None => (store.len().saturating_sub(n)..store.len()).collect(),
    };
    let mut msgs = ids
        .into_iter()
        .map(|i| store.get(i))
        .collect::<Result<Vec<_>, _>>()?;
    // A transcript is read when it next changes, so a turn can be stored
    // after a later one (a reminder's, say); a chat reads in time order.
    msgs.sort_by_key(|m| chrono::DateTime::parse_from_rfc3339(&m.date).ok());
    let mut out = Vec::new();
    for m in msgs {
        out.push(serde_json::to_string(&m).map_err(|e| e.to_string())?);
    }
    Ok(out.join("\n"))
}

fn this_chat(store: &Store, harness: &str, session: &str) -> Result<String, String> {
    let key = chat_key(harness, session);
    let (mut users, mut talk) = (0, false);
    let mut own = Vec::new();
    for i in store.of_chat(&key) {
        if users == OWN_USER {
            break;
        }
        let m = store.get(i)?;
        match m.kind {
            Kind::User => users += 1,
            Kind::Talk if !talk => talk = true,
            _ => continue,
        }
        own.push((i, m));
    }
    if own.is_empty() {
        return Ok(String::new());
    }
    let mut out = String::from("## This chat\n\nYou are ");
    match store.chats.get(&key) {
        Some(rec) if !rec.cwd.is_empty() => out.push_str(&format!(
            "the {harness} chat \"{}\", in {}",
            rec.chat, rec.cwd
        )),
        Some(rec) => out.push_str(&format!("the {harness} chat \"{}\"", rec.chat)),
        None => out.push_str(&key),
    }
    out.push_str(
        ". The view holds every chat; these are this chat's own latest \
         messages, word for word (`hippo zoom <id> 1` opens one whole). \
         Carry on with this chat's work.\n\n",
    );
    for (i, m) in own.iter().rev() {
        let mut text: String = m.text.chars().take(OWN_CUT).collect();
        if text.len() < m.text.len() {
            text.push('…');
        }
        out.push_str(&format!("{i} {}: {text}\n", m.kind.name()));
    }
    Ok(out.trim_end().to_owned())
}

/// `hippo view [page token]`: the whole view, paged. The first call waits
/// for the compactor (up to `SETTLE`, not while it is limited, paused or
/// stuck on a failure),
/// renders once and keeps the pages,
/// so later pages come from the same render. `view whole`, for programs
/// that put the view into a prompt, answers in one piece.
fn view(shared: &Arc<Shared>, a: &[String]) -> Result<String, String> {
    let mut c = shared.core.lock().unwrap();
    let whole = a == ["whole"];
    if !a.is_empty() && !whole {
        let [k, token] = a else {
            return Err("Name the page and its view: hippo view <page> <token>.".into());
        };
        let k: usize = k.parse().map_err(|_| "page must be a number.")?;
        let pages = &c
            .pages
            .iter()
            .find(|(t, _)| t == token)
            .ok_or("That view expired; run hippo view again.")?
            .1;
        let page = pages
            .get(k.wrapping_sub(1))
            .ok_or_else(|| format!("No page {k}."))?;
        return Ok(paged(page, k, pages.len(), token));
    }
    let deadline = std::time::Instant::now() + SETTLE;
    let stalled = |c: &Core| {
        shared.backend.is_none()
            || c.pump.held_until().is_some()
            || c.paused()
            || (c.pump.running() == 0 && c.pump.any_parked())
            || (c.pump.running() > 0 && c.pump.nothing_to_wait_on())
    };
    while c.view.unbuilt(&c.tree) > 0 && !stalled(&c) {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            // The calls that outlasted this wait won't be waited on again,
            // or a hung compactor would cost every turn `SETTLE`.
            c.pump.give_up_on_running();
            break;
        }
        c = shared.cv.wait_timeout(c, left).unwrap().0;
    }
    if whole {
        return text(&c);
    }
    let lines = render(&c)?;
    let mut pages = vec!["<chat>".to_owned()];
    for line in lines {
        let page = pages.last_mut().unwrap();
        if page.len() + line.len() + 1 > PAGE {
            pages.push(line);
        } else {
            page.push('\n');
            page.push_str(&line);
        }
    }
    let last = pages.last_mut().unwrap();
    last.push_str("\n</chat>");
    let token = format!("{}", c.view.t);
    let n = pages.len();
    let first = paged(&pages[0], 1, n, &token);
    c.pages.retain(|(t, _)| *t != token);
    c.pages.push((token, pages));
    if c.pages.len() > 8 {
        c.pages.remove(0);
    }
    Ok(first)
}

/// The view as text: one line per part, oldest first, each a tree node:
/// UniiChat's view, with no note after it. A part not summarized yet shows
/// as `PLACEHOLDER`, as UniiChat's zoom shows one; its turns never see one,
/// since a turn waits for every line before it (`settle`), while hippo's
/// wait is bounded (`SETTLE`) so a stalled compactor never stops a chat.
/// With `<chat>` around it, the whole stays within the budget.
fn render(c: &Core) -> Result<Vec<String>, String> {
    // A session loads the view once, so unlike the compactions' view it
    // need not keep its head between renders.
    let wrap = "<chat>\n\n</chat>\n".len() as u64;
    view_lines(c, c.budget.saturating_sub(wrap), |s, n| {
        Ok(unbuilt_lines(s, n))
    })
}

/// The view's lines under `budget`, oldest first, what `unbuilt` gives
/// standing for each run of parts not built.
pub fn view_lines(
    c: &Core,
    budget: u64,
    unbuilt: impl Fn(u64, u64) -> Result<Vec<String>, String>,
) -> Result<Vec<String>, String> {
    let cut = c.view.cut(&c.tree, budget, c.view.t);
    let mut lines = head(&c.tree, cut)?;
    let mut run: Option<(u64, u64)> = None;
    for &(l, i) in &c.view.parts {
        let (s, n) = addr(l, i);
        if s < cut {
            continue;
        }
        match c.tree.text(l, i)? {
            Some(text) => {
                if let Some((s, n)) = run.take() {
                    lines.extend(unbuilt(s, n)?);
                }
                lines.push(format!("{s}+{n}|{}", flat(&text)));
            }
            None => {
                run = Some(match run {
                    Some((rs, rn)) => (rs, rn + n),
                    None => (s, n),
                });
            }
        }
    }
    if let Some((s, n)) = run {
        lines.extend(unbuilt(s, n)?);
    }
    Ok(lines)
}

/// The lines for `n` unbuilt messages from `s`: the fewest nodes tiling
/// them, each `PLACEHOLDER`, so a zoom opens any of them. One line per
/// message once grew the view past what it was meant to hold whenever the
/// compactor stalled.
pub fn unbuilt_lines(s: u64, n: u64) -> Vec<String> {
    tiles(s, n)
        .into_iter()
        .map(|(l, i)| {
            let (s, n) = addr(l, i);
            format!("{s}+{n}|{PLACEHOLDER}")
        })
        .collect()
}

/// The whole view as one text, as `view.md` holds it.
fn text(c: &Core) -> Result<String, String> {
    let lines = render(c)?;
    Ok(format!("<chat>\n{}\n</chat>\n", lines.join("\n")))
}

/// While this file exists, `view.md` holds its text instead of the view, so
/// a repair can keep the history out of every new context; like a pause,
/// it outlives restarts. It takes effect at the next publish: at start, or
/// when the next message arrives.
pub fn override_file(store: &Path) -> PathBuf {
    store.join("state/view-override.md")
}

/// Writes the whole view to `view.md` in the store, which the seat's Claude
/// Code loads as a rules file at session start and after each compaction,
/// and both views' parts, which the next start loads.
pub fn publish(c: &mut Core) -> Result<(), String> {
    let over = override_file(&c.store.dir);
    let text = match fs::read_to_string(&over) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => text(c)?,
        Err(e) => return Err(format!("{}: {e}", over.display())),
    };
    let path = c.store.dir.join("view.md");
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    c.save_views()?;
    c.stale = false;
    Ok(())
}

fn paged(page: &str, k: usize, n: usize, token: &str) -> String {
    if n == 1 {
        return page.to_owned();
    }
    let mut out = format!("Your memory, part {k} of {n}.\n{page}");
    if k < n {
        out.push_str(&format!(
            "\nNot all read yet. Run: hippo view {} {token}",
            k + 1
        ));
    }
    out
}

/// Line `id+n` opened into its two halves; `n = 1` gives the message whole.
pub fn zoom(store: &Store, tree: &Tree, id: u64, n: u64) -> Result<String, String> {
    let t = store.len();
    if n == 0 || !n.is_power_of_two() || !id.is_multiple_of(n) || id + n > t {
        return Err(format!("No line {id}+{n}."));
    }
    if n == 1 {
        let m = store.get(id)?;
        return Ok(format!("{id}+1|{}", m.labelled()));
    }
    let l = n.trailing_zeros() as u8 - 1;
    let half = n / 2;
    let mut out = Vec::new();
    for (k, i) in [(id, id / half), (id + half, id / half + 1)] {
        let text = tree.text(l, i)?.unwrap_or_else(|| PLACEHOLDER.to_owned());
        out.push(format!("{k}+{half}|{}", flat(&text)));
    }
    Ok(out.join("\n"))
}

pub fn search(store: &Store, re: &regex::Regex) -> Result<String, String> {
    let mut hits = Vec::new();
    let mut total = 0usize;
    store.scan(|m| {
        if let Some(at) = re.find(&m.text) {
            total += 1;
            let label = m
                .chat
                .as_deref()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            let date = m.date.get(..16).unwrap_or(&m.date).replace('T', " ");
            let excerpt = excerpt(&m.text, at.start(), at.end());
            let line = format!("{} {}{label} {date}: {excerpt}", m.i, m.kind.name());
            hits.push(line);
            if hits.len() > SEARCH_MAX {
                hits.remove(0);
            }
        }
        true
    })?;
    if hits.is_empty() {
        return Ok("No match.".into());
    }
    let mut out = String::new();
    if total > hits.len() {
        out.push_str(&format!(
            "{total} matches; the newest {} follow. Narrow the regex for older ones.\n",
            hits.len()
        ));
    }
    out.push_str(&hits.join("\n"));
    Ok(out)
}

/// About 200 bytes around a match, on one line.
fn excerpt(text: &str, start: usize, end: usize) -> String {
    let mut lo = start.saturating_sub(80);
    while !text.is_char_boundary(lo) {
        lo -= 1;
    }
    let mut hi = (end + 120).min(text.len());
    while !text.is_char_boundary(hi) {
        hi += 1;
    }
    let flat = text[lo..hi]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{}{flat}{}",
        if lo > 0 { "…" } else { "" },
        if hi < text.len() { "…" } else { "" }
    )
}

/// The memory's state for a program to show, from what truly happened: see
/// `Pump::failing`. Failure times live in memory and restart with hippo.
fn health(c: &mut Core, compacting: bool) -> serde_json::Value {
    let time = |t: DateTime<Local>| t.to_rfc3339();
    let lo = c.first(0);
    let waiting = (lo..c.view.t).filter(|&i| !c.tree.built(0, i)).count();
    let failing = c.pump.failing();
    let idle = !compacting || c.paused();
    let state = match &failing {
        // No compactor: the log is all it keeps, as Libri runs it.
        _ if !compacting => "recording",
        _ if idle => "paused",
        Some(_) => "failing",
        None if waiting > crate::memory::compactor::jobs() => "catching_up",
        None => "working",
    };
    let (nodes, calls) = c.pump.spent.last_hour();
    let newest = c.pump.spent.newest.as_ref();
    serde_json::json!({
        "state": state,
        "reason": failing.as_ref().filter(|_| !idle).map(|f| f.0.clone()),
        "since": failing.as_ref().filter(|_| !idle).map(|f| time(f.1)),
        "waiting": waiting,
        "newest_summary": newest.map(|n| time(n.0)),
        "latest_message": c.store.len().checked_sub(1).and_then(|i| c.store.date(i)).map(time),
        "model": newest.map(|n| n.1.clone()),
        "lines": c.store.lines(),
        "bytes": c.store.bytes(),
        "last_error": c.pump.last_error().map(|t| serde_json::json!({
            "kind": t.kind,
            "error": t.error,
            "at": time(t.at),
            "since": time(t.since),
            "cleared": t.cleared.map(time),
        })),
        "retries_last_hour": {"nodes": nodes, "calls": calls, "wasted": calls - nodes},
    })
}

fn status(c: &Core, compacting: bool) -> String {
    let s = &c.store;
    let following = match &c.follow {
        Some(f) => format!(
            "code mode, following transcripts since {}",
            f.live.since().map(|d| fmt_date(&d)).unwrap_or_default()
        ),
        None => "chat mode, taking messages over the socket".to_owned(),
    };
    let mut out = format!(
        "{} messages in {} chats, at {}; {following}.\n",
        s.len(),
        s.chats.len(),
        s.dir.display(),
    );
    let first = c.view.first(&c.tree);
    let built: usize = c.tree.counts().iter().sum();
    out.push_str(&format!(
        "tree: {built} nodes; messages summarized in order up to {first} of {}; view {} lines, {} bytes, {} unsummarized\n",
        s.len(),
        c.view.parts.len(),
        c.view.size(),
        c.view.unbuilt(&c.tree)
    ));
    if !s.torn.is_empty() {
        out.push_str(&format!(
            "store: {} damaged places at the last load; the log names them\n",
            s.torn.len()
        ));
    }
    if override_file(&s.dir).exists() {
        out.push_str(&format!(
            "view.md: overridden by {}\n",
            override_file(&s.dir).display()
        ));
    }
    if !compacting {
        out.push_str("compactor: off\n");
    } else if c.paused() {
        out.push_str(&format!(
            "compactor: paused (hippo resume); {} running\n",
            c.pump.running()
        ));
    } else {
        out.push_str(&format!("compactor: {} running\n", c.pump.running()));
    }
    let today = Local::now().date_naive();
    let tokens = |u: &crate::memory::compact::Usage| {
        format!(
            "{} input, {} cache read, {} cache write, {} output",
            u.input, u.cache_read, u.cache_write, u.output
        )
    };
    let (calls, used) = c.pump.spent.days.get(&today).copied().unwrap_or_default();
    out.push_str(&format!(
        "compactor today: {calls} calls; tokens {}\n",
        tokens(&used)
    ));
    let (mut week_calls, mut week) = (0, crate::memory::compact::Usage::default());
    for (_, (n, u)) in c
        .pump
        .spent
        .days
        .range(today - chrono::Duration::days(6)..=today)
    {
        week_calls += n;
        week.add(*u);
    }
    out.push_str(&format!(
        "compactor last 7 days: {week_calls} calls; tokens {}\n",
        tokens(&week)
    ));
    if let Some(held) = c.pump.held() {
        out.push_str(&format!("{held}\n"));
    }
    let failing: Vec<_> = c.pump.failing_nodes().collect();
    if !failing.is_empty() {
        out.push_str(&format!("failing nodes: {}\n", failing.len()));
        for &((l, i), f) in failing.iter().take(5) {
            let e = &f.error;
            let (s, n) = addr(l, i);
            let e: String = e.chars().take(200).collect();
            out.push_str(&format!("  {s}+{n}: {e}\n"));
        }
    }
    if let Some(f) = &c.follow {
        out.push_str(&f.status(s, &c.started));
    }
    let torn: Vec<&String> = s.torn.iter().chain(&c.tree.torn).collect();
    if !torn.is_empty() {
        let torn: Vec<&str> = torn.iter().map(|t| t.as_str()).collect();
        out.push_str(&format!(
            "torn lines skipped at load: {}\n",
            torn.join(", ")
        ));
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::tree::Node;

    /// A store of `n` messages, each its own built line, and no merges:
    /// the state of a paused or backlogged compactor.
    fn stalled(name: &str, n: u64, budget: u64) -> Core {
        let dir = std::env::temp_dir().join(format!("hippo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir, true).unwrap();
        let drafts = (0..n)
            .map(|k| Draft {
                display: None,
                kind: Kind::Talk,
                chat: Some("c".into()),
                text: format!("message {k} {}", "x".repeat(200)),
                date: Local::now(),
                src: None,
            })
            .collect();
        store.append(drafts).unwrap();
        let mut tree = Tree::open(&dir).unwrap();
        for i in 0..n {
            let text = format!("talk: message {i} {}", "x".repeat(200));
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
        let mut c = Core::for_test(store, tree, View::default(), budget);
        c.sync();
        c
    }

    #[test]
    fn health_counts_the_log_and_the_files() {
        let mut c = stalled("health-size", 3, 50_000);
        let log = fs::metadata(
            c.store
                .dir
                .join("main")
                .join(day_name(Local::now().date_naive())),
        )
        .unwrap()
        .len();
        let outside = c.store.dir.with_extension("outside");
        fs::write(&outside, "x".repeat(1 << 20)).unwrap();
        std::os::unix::fs::symlink(&outside, c.store.dir.join("agents")).unwrap();
        let health = health(&mut c, true);
        assert_eq!(health["lines"], 3);
        let bytes = health["bytes"].as_u64().unwrap();
        assert!(
            bytes > log && bytes < 1 << 20,
            "{bytes} from a log of {log}"
        );
        let _ = fs::remove_file(&outside);
    }

    struct Absent;
    impl crate::memory::compact::Backend for Absent {
        fn start(
            &self,
        ) -> Result<Box<dyn crate::memory::compact::Chat>, crate::memory::compact::Fail> {
            Err(crate::memory::compact::Fail::Other("absent".into()))
        }
    }

    #[test]
    fn display_survives_disk_without_changing_the_model_row() {
        let mut c = stalled("display-record", 1, 50_000);
        let date = "2026-10-30T12:00:00-04:00";
        let display = serde_json::json!({"version":1,"result":{"details":{"diff":"- off\n+ on"}}});
        keep_display(
            &mut c.store,
            "edit.result",
            date,
            "Iris",
            Kind::Echo,
            "Updated file",
            Some(display.clone()),
        )
        .unwrap();
        c.store.reload().unwrap();
        let row = c.store.get(1).unwrap();
        assert_eq!(row.display, Some(display));
        assert_eq!(row.render(), "echo: Updated file");
        assert_eq!(row.size, row.render().len() as u64);
    }

    #[test]
    fn words_kept_twice_under_one_key_are_one_message() {
        let mut c = stalled("keep", 1, 50_000);
        let date = "2026-10-30T12:00:00-04:00";
        let first = keep(&mut c.store, "k1", date, "Iris", Kind::User, "words").unwrap();
        let again = keep(&mut c.store, "k1", date, "Iris", Kind::User, "words").unwrap();
        assert_eq!((first, again), (Some(1), None));
        assert_eq!(c.store.len(), 2);
        // The same moment in this machine's zone.
        assert_eq!(parse_date(&c.store.get(1).unwrap().date), parse_date(date));
        keep(&mut c.store, "k2", date, "", Kind::Talk, "off a seat").unwrap();
        assert_eq!(c.store.get(2).unwrap().chat, None);
    }

    #[test]
    fn a_stalled_client_is_dropped() {
        let shared = Arc::new(Shared {
            core: Mutex::new(stalled("stalled-client", 1, 50_000)),
            cv: Condvar::new(),
            backend: None,
            workers: None,
        });
        let (ours, _held) = UnixStream::pair().unwrap();
        let (done, went) = std::sync::mpsc::channel();
        thread::spawn(move || done.send(answer(ours, &shared)));
        assert!(went.recv_timeout(READ_WAIT * 4).is_ok());
    }

    #[test]
    fn a_deleted_socket_comes_back() {
        let dir = std::env::temp_dir().join(format!("hippo-sock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let serving = dir.clone();
        thread::spawn(move || serve(&serving, None, None));
        let sock = dir.join("hippo.sock");
        let until = |what: &dyn Fn() -> bool| {
            (0..80).any(|_| {
                thread::sleep(Duration::from_millis(100));
                what()
            })
        };
        assert!(until(&|| sock.exists()));
        fs::remove_file(&sock).unwrap();
        assert!(until(&|| sock.exists()));
        let mut conn = UnixStream::connect(&sock).unwrap();
        conn.write_all(b"{\"cmd\":\"health\",\"args\":[]}\n")
            .unwrap();
        let mut line = String::new();
        BufReader::new(&conn).read_line(&mut line).unwrap();
        assert!(line.contains("\"ok\":true"), "{line}");
    }

    #[test]
    fn a_hung_compactor_delays_one_view_not_every_turn() {
        let mut c = stalled("hung", 6, 50_000);
        c.store
            .append(vec![Draft {
                display: None,
                kind: Kind::Talk,
                chat: Some("c".into()),
                text: "unbuilt".into(),
                date: Local::now(),
                src: None,
            }])
            .unwrap();
        c.sync();
        c.pump.nodes.insert(
            (1, 0),
            crate::memory::compactor::NodeState::Running {
                overdue: false,
                failing: None,
            },
        );
        let shared = Arc::new(Shared {
            core: Mutex::new(c),
            cv: Condvar::new(),
            backend: Some(Arc::new(Absent)),
            workers: None,
        });
        let args = ["whole".to_owned()];
        let timed = || {
            let at = std::time::Instant::now();
            view(&shared, &args).unwrap();
            at.elapsed()
        };
        assert!(timed() >= SETTLE);
        assert!(timed() < SETTLE / 2);
    }

    #[test]
    fn forgetting_drops_the_oldest_and_renumbers_the_rest() {
        let mut c = stalled("forget", 6, 50_000);
        let old = Local::now() - chrono::Duration::hours(2);
        // Messages 0-5 now; make 0-3 two hours old in the log itself.
        let dir = c.store.dir.clone();
        let day = fs::read_dir(dir.join("main"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text: Vec<String> = fs::read_to_string(&day)
            .unwrap()
            .lines()
            .map(|l| {
                let mut m: crate::memory::store::Msg = serde_json::from_str(l).unwrap();
                if m.i < 4 {
                    m.date = fmt_date(&old);
                }
                serde_json::to_string(&m).unwrap()
            })
            .collect();
        fs::write(&day, text.join("\n") + "\n").unwrap();
        c.store.reload().unwrap();
        c.tree
            .save(Node {
                l: 1,
                i: 2,
                size: 3,
                text: "4-5".into(),
                model: String::new(),
                prompt: String::new(),
            })
            .unwrap();
        c.tree
            .save(Node {
                l: 1,
                i: 1,
                size: 3,
                text: "2-3".into(),
                model: String::new(),
                prompt: String::new(),
            })
            .unwrap();
        assert_eq!(forget(&mut c, 60).unwrap(), "Forgot 4 messages; 2 left.");
        assert_eq!(c.store.len(), 2);
        assert!(c.store.get(0).unwrap().text.starts_with("message 4 "));
        assert!(c.tree.text(0, 1).unwrap().unwrap().contains("message 5 "));
        // 4-5 still covers a whole span, now 0-1; 2-3 read what went.
        assert_eq!(c.tree.text(1, 0).unwrap().as_deref(), Some("4-5"));
        assert!(!c.tree.built(1, 1) && !c.tree.built(0, 2));
        assert_eq!(c.view.t, 2);
        let reopened = Store::open(&dir, false).unwrap();
        assert_eq!(reopened.len(), 2);
        assert_eq!(forget(&mut c, 60).unwrap(), "Forgot 0 messages.");
    }

    #[test]
    fn a_backlog_reads_as_catching_up() {
        let mut c = stalled("backlog", 12, 50_000);
        assert_eq!(health(&mut c, true)["state"], "working");
        let more = (0..=crate::memory::compactor::jobs())
            .map(|k| Draft {
                display: None,
                kind: Kind::Talk,
                chat: Some("c".into()),
                text: format!("unsummarized {k}"),
                date: Local::now(),
                src: None,
            })
            .collect();
        c.store.append(more).unwrap();
        c.sync();
        let health = health(&mut c, true);
        assert_eq!(health["state"], "catching_up", "{health}");
        assert_eq!(health["waiting"], crate::memory::compactor::jobs() + 1);
        assert_eq!(health["reason"], serde_json::Value::Null);
    }

    #[test]
    fn a_stalled_view_publishes_within_its_budget() {
        let mut c = stalled("stalled", 2_000, 50_000);
        assert!(
            c.view.size() > 4 * c.budget,
            "the view did not outgrow its budget"
        );
        publish(&mut c).unwrap();
        let file = fs::read_to_string(c.store.dir.join("view.md")).unwrap();
        assert!(
            file.len() as u64 <= c.budget,
            "view.md {} bytes",
            file.len()
        );
        assert!(file.starts_with("<chat>\n0+1024|") && file.ends_with("</chat>\n"));
        // The newest message keeps its line, and what was cut still opens.
        assert!(file.contains("\n1999+1|talk: message 1999 "));
        assert!(
            zoom(&c.store, &c.tree, 0, 1024)
                .unwrap()
                .starts_with("0+512|")
        );
        assert!(
            zoom(&c.store, &c.tree, 0, 1)
                .unwrap()
                .contains("message 0 ")
        );
    }

    #[test]
    fn a_posted_message_joins_its_seats_chat() {
        let dir = std::env::temp_dir().join(format!("hippo-post-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir, true).unwrap();
        post(&mut store, "Iris", Kind::Talk, "Reminder: stretch").unwrap();
        post(&mut store, "Other", Kind::Talk, "elsewhere").unwrap();
        post(&mut store, "Iris", Kind::User, "second").unwrap();
        let out = tail(&store, 10, Some("Iris")).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("Reminder: stretch") && lines[1].contains("second"));
        // Both land after a reload, under one registered chat.
        let reopened = Store::open(&dir, false).unwrap();
        assert_eq!(
            tail(&reopened, 10, Some("Iris")).unwrap().lines().count(),
            2
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chat_read_late_still_shows_in_time_order() {
        let dir = std::env::temp_dir().join(format!("hippo-late-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir, true).unwrap();
        post(&mut store, "Iris", Kind::Talk, "the reminder, later").unwrap();
        // The turn before it, stored only now as its transcript is read.
        store
            .append(vec![Draft {
                display: None,
                kind: Kind::User,
                chat: Some("iris".into()),
                text: "the turn, earlier".into(),
                date: Local::now() - chrono::Duration::minutes(5),
                src: Some(crate::memory::store::Src {
                    h: POSTED.into(),
                    s: "Iris".into(),
                    e: "late".into(),
                }),
            }])
            .unwrap();
        let out = tail(&store, 10, Some("Iris")).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].contains("the turn, earlier") && lines[1].contains("the reminder, later"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chat_quotes_only_its_own_latest_words() {
        let dir = std::env::temp_dir().join(format!("hippo-own-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir, true).unwrap();
        let msg = |k: usize, kind, s: &str, text: String| Draft {
            display: None,
            kind,
            chat: Some(s.into()),
            text,
            date: Local::now(),
            src: Some(crate::memory::store::Src {
                h: "claude".into(),
                s: s.into(),
                e: k.to_string(),
            }),
        };
        let mut drafts = Vec::new();
        for k in 0..6 {
            drafts.push(msg(4 * k, Kind::User, "a", format!("a asks {k}")));
            drafts.push(msg(4 * k + 1, Kind::Tool, "a", format!("a runs {k}")));
            drafts.push(msg(4 * k + 2, Kind::Talk, "a", format!("a says {k}")));
            drafts.push(msg(4 * k + 3, Kind::User, "b", format!("b asks {k}")));
        }
        drafts.push(msg(99, Kind::User, "a", "y".repeat(OWN_CUT + 5)));
        store.append(drafts).unwrap();

        let out = this_chat(&store, "claude", "a").unwrap();
        let lines: Vec<&str> = out.lines().filter(|l| l.contains(": ")).collect();
        assert!(out.starts_with("## This chat"));
        assert!(!out.contains("b asks") && !out.contains("a runs"));
        // The last four of the captain's, the last reply only, oldest first.
        assert_eq!(lines[0], "12 user: a asks 3");
        assert_eq!(lines[1], "16 user: a asks 4");
        assert_eq!(lines[2], "20 user: a asks 5");
        assert_eq!(lines[3], "22 talk: a says 5");
        assert!(lines[4].starts_with("24 user: yyy") && lines[4].ends_with("y…"));
        assert_eq!(lines.len(), 5);
        assert_eq!(this_chat(&store, "claude", "fresh").unwrap(), "");
    }

    #[test]
    fn a_merge_lands_once_as_the_chats_own_messages() {
        let dir = std::env::temp_dir().join(format!("hippo-merge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir, true).unwrap();
        keep(
            &mut store,
            "k0",
            "2026-11-06T08:00:00+00:00",
            "Main",
            Kind::User,
            "main 0",
        )
        .unwrap();
        keep(
            &mut store,
            "k1",
            "2026-11-06T08:01:00+00:00",
            "Main",
            Kind::Talk,
            "main 1",
        )
        .unwrap();
        // As `Workers::merge_drafts` gives a worker's messages from 40, its
        // fork: each its own kind and time.
        let lines = |to: u64| -> Vec<Draft> {
            (40..to)
                .map(|i| Draft {
                    display: None,
                    kind: if i % 2 == 0 { Kind::Tool } else { Kind::Echo },
                    chat: None,
                    text: format!("line {i}"),
                    date: parse_date(&format!("2026-11-06T08:{:02}:00+00:00", i - 38)).unwrap(),
                    src: Some(Src {
                        h: MERGED.into(),
                        s: "gil-1".into(),
                        e: format!("gil-1:{i}"),
                    }),
                })
                .collect()
        };
        let out = merge(&mut store, "Gil", Some("Main"), lines(43)).unwrap();
        assert_eq!(out, "Merged 3 messages of Gil's.");
        // After main's own, as they were: no heading, no tag, their times.
        let m = store.get(2).unwrap();
        assert_eq!(m.render(), "tool: line 40");
        assert_eq!(parse_date(&m.date), parse_date("2026-11-06T08:02:00+00:00"));
        assert_eq!(store.get(3).unwrap().render(), "echo: line 41");
        assert_eq!(store.get(0).unwrap().render(), "user: main 0");
        // Again with nothing new: nothing lands. With more: only the rest.
        assert_eq!(
            merge(&mut store, "Gil", Some("Main"), lines(43)).unwrap(),
            "Nothing of Gil's to merge."
        );
        keep(
            &mut store,
            "k5",
            "2026-11-06T08:10:00+00:00",
            "Main",
            Kind::Talk,
            "main 5",
        )
        .unwrap();
        let out = merge(&mut store, "Gil", Some("Main"), lines(44)).unwrap();
        assert_eq!(out, "Merged 1 message of Gil's.");
        assert_eq!(store.len(), 7);
        // The chat's history shows them among its own words, by time as it
        // reads every chat; the memory keeps them after what it had.
        let texts: Vec<String> = tail(&store, 10, Some("Main"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["text"].to_string())
            .collect();
        assert_eq!(texts.len(), 7, "{texts:?}");
        assert!(
            texts[2].contains("line 40") && texts[5].contains("line 43"),
            "{texts:?}"
        );
        assert!(texts[6].contains("main 5"), "{texts:?}");
        assert_eq!(store.get(6).unwrap().render(), "echo: line 43");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_titled_tail_keeps_to_the_chats_so_named() {
        let dir = std::env::temp_dir().join(format!("hippo-tail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir, true).unwrap();
        let msg = |k: usize, s: &str| Draft {
            display: None,
            kind: Kind::Talk,
            chat: Some(s.into()),
            text: format!("{s} {k}"),
            date: Local::now(),
            src: Some(crate::memory::store::Src {
                h: "claude".into(),
                s: s.into(),
                e: k.to_string(),
            }),
        };
        let chat = |s: &str, title: &str| crate::memory::store::ChatRec {
            chat: s.into(),
            harness: "claude".into(),
            session: s.into(),
            title: title.into(),
            cwd: String::new(),
            first: String::new(),
        };
        // Two turns of the seat, each its own session, around another chat.
        store.register(chat("t1", "Iris")).unwrap();
        store.register(chat("other", "Fix hippo")).unwrap();
        store.register(chat("t2", "Iris")).unwrap();
        let order = ["t1", "other", "t1", "other", "t2", "other", "t2"];
        store
            .append(order.iter().enumerate().map(|(k, s)| msg(k, s)).collect())
            .unwrap();
        let texts = |out: String| -> Vec<String> {
            out.lines()
                .map(|l| {
                    serde_json::from_str::<serde_json::Value>(l).unwrap()["text"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect()
        };
        assert_eq!(
            texts(tail(&store, 3, Some("Iris")).unwrap()),
            ["t1 2", "t2 4", "t2 6"]
        );
        assert_eq!(texts(tail(&store, 2, None).unwrap()), ["other 5", "t2 6"]);
        assert_eq!(tail(&store, 5, Some("Nobody")).unwrap(), "");
    }

    #[test]
    fn an_override_replaces_the_published_view() {
        let mut c = stalled("override", 20, VIEW);
        let over = override_file(&c.store.dir);
        fs::create_dir_all(over.parent().unwrap()).unwrap();
        fs::write(&over, "Repair: history is offline.\n").unwrap();
        publish(&mut c).unwrap();
        let path = c.store.dir.join("view.md");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "Repair: history is offline.\n"
        );
        assert!(status(&c, false).contains("view.md: overridden"));
        fs::remove_file(&over).unwrap();
        publish(&mut c).unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("19+1|talk: message 19")
        );
    }
}
