//! The log: every message, word for word, append-only, one line per message
//! in `main/YYYY-MM-DD.jsonl`, written with one `write` and an fsync. Only
//! an index stays in memory; texts are read back from disk when asked for.

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    User,
    Talk,
    Tool,
    Echo,
    /// A subagent's report: `[Name]`, its words, and where its whole chat is.
    Work,
    Note,
    /// A worker's log only: another worker's message to it, and its own.
    From,
    To,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Talk => "talk",
            Kind::Tool => "tool",
            Kind::Echo => "echo",
            Kind::Work => "work",
            Kind::Note => "note",
            Kind::From => "from",
            Kind::To => "to",
        }
    }
}

/// Where a message came from: harness, session and entry, so it can be
/// traced to the raw transcript. `h + e` is unique: resumed and forked
/// sessions repeat entries under the same entry id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Src {
    pub h: String,
    pub s: String,
    pub e: String,
}

impl Src {
    pub fn key(&self) -> String {
        format!("{}:{}", self.h, self.e)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Msg {
    /// Versioned presentation data. Text alone defines rendering, size and summaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<serde_json::Value>,
    pub i: u64,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
    pub text: String,
    pub size: u64,
    pub date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<Src>,
}

impl Msg {
    /// The message as level 0 of the tree sees it: `kind: text`. Chats share
    /// one timeline; a line names its subject in words, never its chat.
    pub fn render(&self) -> String {
        render(self.kind, &self.text)
    }

    /// The message with its chat, for reading it whole: `kind [chat]: text`.
    pub fn labelled(&self) -> String {
        match &self.chat {
            Some(chat) => format!("{} [{chat}]: {}", self.kind.name(), self.text),
            None => self.render(),
        }
    }
}

pub fn render(kind: Kind, text: &str) -> String {
    format!("{}: {text}", kind.name())
}

/// A message ready to be logged; the store gives it its id and size.
#[derive(Clone, Debug)]
pub struct Draft {
    pub display: Option<serde_json::Value>,
    pub kind: Kind,
    pub chat: Option<String>,
    pub text: String,
    pub date: DateTime<Local>,
    pub src: Option<Src>,
}

/// One registered chat, written to `chats.jsonl` the first time a message
/// of it is logged; a later line for the same chat replaces it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatRec {
    pub chat: String,
    pub harness: String,
    pub session: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    pub first: String,
}

/// The most messages one gap can account for, a few months of chat: a larger
/// leap in the ids is a damaged line, and filling it would take the memory.
const MOST_LOST: u64 = 1 << 22;

struct Meta {
    file: u32,
    off: u64,
    len: u32,
    date: i64,
    /// The chat's `harness:session` key; labels repeat across chats.
    chat: Option<String>,
}

pub struct Store {
    pub dir: PathBuf,
    /// The first id of this log. A fork goes on from where the memory it
    /// forked from stood, so an id means the same on both up to here.
    pub base: u64,
    days: Vec<NaiveDate>,
    index: Vec<Meta>,
    seen: HashSet<String>,
    out: Option<(NaiveDate, File, u64)>,
    pub chats: HashMap<String, ChatRec>,
    /// What could not be read at load: lines that were not valid JSON, as
    /// `file:line`, and messages lost or set aside.
    pub torn: Vec<String>,
    /// Last message per harness: (id, date).
    pub last: HashMap<String, (u64, i64)>,
    _lock: Option<UnixListener>,
}

/// What stands in for a message whose line was damaged or whose file is gone.
fn lost(i: u64, date: i64) -> Msg {
    let date = DateTime::from_timestamp(date, 0).map(|d| d.with_timezone(&Local));
    let text = "(this message was lost to damage in the store)".to_owned();
    Msg {
        display: None,
        i,
        kind: Kind::Note,
        chat: None,
        size: render(Kind::Note, &text).len() as u64,
        text,
        date: date.map(|d| fmt_date(&d)).unwrap_or_default(),
        src: None,
    }
}

pub fn chat_key(harness: &str, session: &str) -> String {
    format!("{harness}:{session}")
}

/// Appends one line with a single write, then fsyncs.
/// Characters in a page of a message: UniiChat's `CAP` (Clip.ts), the most
/// text a model gets in one piece.
pub const PAGE: usize = 30_000;

/// A text in pages of at most `PAGE` characters, each ending after its
/// last newline if it has one: UniiChat's `clip_pages`.
pub fn pages(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        let Some((cap, _)) = rest.char_indices().nth(PAGE) else {
            out.push(rest);
            return out;
        };
        let end = rest[..cap].rfind('\n').map_or(cap, |nl| nl + 1);
        out.push(&rest[..end]);
        rest = &rest[end..];
    }
}

pub fn append_line(file: &mut File, line: &str) -> Result<(), String> {
    let mut buf = Vec::with_capacity(line.len() + 1);
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    let start = file.metadata().map_err(|e| e.to_string())?.len();
    file.write_all(&buf)
        .and_then(|()| file.sync_data())
        .map_err(|e| {
            // A full disk cuts a write short. The next line would land on
            // the piece, and the store then won't open for the gap in its ids.
            let _ = file.set_len(start);
            e.to_string()
        })
}

fn open_append(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// A file not ending in a newline (a crash mid-write) gets one, so the next
/// line starts on its own.
fn heal(path: &Path) -> Result<(), String> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::End(-1)).map_err(|e| e.to_string())?;
    let mut last = [0u8];
    file.read_exact(&mut last).map_err(|e| e.to_string())?;
    if last[0] != b'\n' {
        let mut file = open_append(path)?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.sync_data().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Reads a JSONL file line by line with byte offsets, healing it first.
/// Lines that are not valid JSON land in `torn`.
pub fn read_jsonl<T: for<'de> Deserialize<'de>>(
    path: &Path,
    torn: &mut Vec<String>,
    mut each: impl FnMut(T, u64, u32),
) -> Result<(), String> {
    heal(path)?;
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut reader = BufReader::new(file);
    let (mut off, mut n, mut line) = (0u64, 0usize, Vec::new());
    loop {
        line.clear();
        let len = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if len == 0 {
            break;
        }
        n += 1;
        match serde_json::from_slice::<T>(&line) {
            Ok(v) => each(v, off, len as u32),
            Err(_) if line.iter().all(u8::is_ascii_whitespace) => {}
            Err(e) => {
                let at = format!("{}:{n}", path.display());
                eprintln!("hippo: torn line skipped at {at}: {e}");
                torn.push(at);
            }
        }
        off += len as u64;
    }
    Ok(())
}

pub fn day_name(day: NaiveDate) -> String {
    day.format("%Y-%m-%d.jsonl").to_string()
}

pub fn parse_date(s: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Local))
}

pub fn fmt_date(d: &DateTime<Local>) -> String {
    d.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// Ends a `forget` stopped between its renames, whose new directories are
/// complete by then: before the log moved it goes back, after it goes forward.
fn settle_forget(dir: &Path) -> Result<(), String> {
    let path = |name: &str| dir.join(name);
    let moved = |from: &str, to: &str| {
        eprintln!("hippo: finishing a forget that stopped: {from} to {to}");
        fs::rename(path(from), path(to)).map_err(|e| format!("{from}: {e}"))
    };
    if !path("main").exists() && path("main.new").exists() {
        moved("main.new", "main")?;
    }
    if !path("tree").exists() {
        if path("main.new").exists() && path("tree.gone").exists() {
            moved("tree.gone", "tree")?;
        } else if path("tree.new").exists() {
            moved("tree.new", "tree")?;
        }
    }
    Ok(())
}

/// Days of a stream directory, oldest first.
pub fn day_files(dir: &Path) -> Result<Vec<NaiveDate>, String> {
    let mut days = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(days),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    for entry in entries {
        let name = entry.map_err(|e| e.to_string())?.file_name();
        let name = name.to_string_lossy();
        if let Some(stem) = name.strip_suffix(".jsonl")
            && let Ok(day) = NaiveDate::parse_from_str(stem, "%Y-%m-%d")
        {
            days.push(day);
        }
    }
    days.sort();
    Ok(days)
}

/// Holds `dir/lock` for the life of the process: a second process that can
/// connect to it exits; a socket that refuses connections is stale (its
/// owner died) and is taken over. No PID files, no timeouts.
pub fn take_lock(dir: &Path) -> Result<UnixListener, String> {
    let path = dir.join("lock");
    match UnixListener::bind(&path) {
        Ok(l) => Ok(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(&path).is_ok() {
                return Err(format!("another hippo holds {}", path.display()));
            }
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

impl Store {
    /// Opens the store. `lock` takes the single-writer lock; a reader that
    /// never writes (tests, replay inspection) may skip it.
    pub fn open(dir: &Path, lock: bool) -> Result<Store, String> {
        Store::open_from(dir, lock, 0)
    }

    /// Opens a log whose first id is `base`.
    pub fn open_from(dir: &Path, lock: bool, base: u64) -> Result<Store, String> {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let lock = if lock { Some(take_lock(dir)?) } else { None };
        if lock.is_some() {
            settle_forget(dir)?;
        }
        // Starting empty here would look healthy while every word is gone,
        // and new ids would start over: the chat is better off without hippo.
        if !dir.join("main").exists() && dir.join("chats.jsonl").exists() {
            return Err(format!("{}: the message log is missing", dir.display()));
        }
        fs::create_dir_all(dir.join("main")).map_err(|e| format!("{}: {e}", dir.display()))?;
        fs::create_dir_all(dir.join("tree")).map_err(|e| e.to_string())?;
        fs::create_dir_all(dir.join("state")).map_err(|e| e.to_string())?;
        let mut store = Store {
            dir: dir.to_owned(),
            base,
            days: Vec::new(),
            index: Vec::new(),
            seen: HashSet::new(),
            out: None,
            chats: HashMap::new(),
            torn: Vec::new(),
            last: HashMap::new(),
            _lock: lock,
        };
        store.load()?;
        Ok(store)
    }

    fn load(&mut self) -> Result<(), String> {
        let mut torn = Vec::new();
        // A later line for a chat replaces its record: how a chat is
        // retitled when its assistant is renamed.
        read_jsonl::<ChatRec>(&self.dir.join("chats.jsonl"), &mut torn, |c, _, _| {
            self.chats.insert(chat_key(&c.harness, &c.session), c);
        })?;
        self.days = day_files(&self.dir.join("main"))?;
        for (f, day) in self.days.clone().into_iter().enumerate() {
            let path = self.dir.join("main").join(day_name(day));
            let mut rows = Vec::new();
            read_jsonl::<Msg>(&path, &mut torn, |m, off, len| rows.push((m, off, len)))?;
            for (m, off, len) in rows {
                let want = self.len();
                // As journald reads a damaged file: what can't be trusted is
                // set aside and the rest stays readable. An id repeats when
                // two processes wrote, and one this far ahead was damaged.
                if m.i < want || m.i - want > MOST_LOST {
                    torn.push(format!("{}: message {} set aside", path.display(), m.i));
                    continue;
                }
                if m.i > want {
                    torn.push(format!(
                        "{}: messages {want} to {} lost",
                        path.display(),
                        m.i - 1
                    ));
                }
                // The lost keep their ids: the tree's nodes are by id.
                let date = self.index.last().map_or(0, |l| l.date);
                for _ in want..m.i {
                    self.index.push(Meta {
                        file: f as u32,
                        off: 0,
                        len: 0,
                        date,
                        chat: None,
                    });
                }
                self.index_msg(&m, f as u32, off, len);
            }
        }
        self.torn = torn;
        Ok(())
    }

    /// Reads the log afresh from disk, keeping the writer's lock: after
    /// `forget` rewrote it.
    pub fn reload(&mut self) -> Result<(), String> {
        self.days.clear();
        self.index.clear();
        self.seen.clear();
        self.out = None;
        self.chats.clear();
        self.last.clear();
        self.load()
    }

    /// How many messages, from the first, were sent before `cutoff` (Unix
    /// seconds): what a memory that forgets lets go.
    pub fn older(&self, cutoff: i64) -> u64 {
        self.index.iter().take_while(|m| m.date < cutoff).count() as u64
    }

    fn index_msg(&mut self, m: &Msg, file: u32, off: u64, len: u32) {
        let date = parse_date(&m.date).map(|d| d.timestamp()).unwrap_or(0);
        if let Some(src) = &m.src {
            self.seen.insert(src.key());
            self.last.insert(src.h.clone(), (m.i, date));
        }
        self.index.push(Meta {
            file,
            off,
            len,
            date,
            chat: m.src.as_ref().map(|s| chat_key(&s.h, &s.s)),
        });
    }

    /// Holds the ids up to `end`, which the tree has summarized though the
    /// log was cut short: given to new words, they would sit under summaries
    /// of the old ones and never reach a view.
    pub fn reach(&mut self, end: u64) {
        let Some(short) = end
            .checked_sub(self.len())
            .filter(|n| (1..=MOST_LOST).contains(n))
        else {
            return;
        };
        self.torn
            .push(format!("log ends {short} messages before the tree"));
        let date = self.index.last().map_or(0, |l| l.date);
        let file = self.days.len().saturating_sub(1) as u32;
        for _ in 0..short {
            self.index.push(Meta {
                file,
                off: 0,
                len: 0,
                date,
                chat: None,
            });
        }
    }

    pub fn len(&self) -> u64 {
        self.base + self.index.len() as u64
    }

    fn meta(&self, i: u64) -> Option<&Meta> {
        self.index.get(i.checked_sub(self.base)? as usize)
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The lines in the log. Lost messages keep their ids but have no line.
    pub fn lines(&self) -> u64 {
        self.index.iter().filter(|m| m.len > 0).count() as u64
    }

    /// The bytes of every file the store keeps. A hundred-odd files, so
    /// cheap enough for `health`. Its links lead outside it and don't count.
    pub fn bytes(&self) -> u64 {
        fn under(dir: &Path) -> u64 {
            let Ok(entries) = fs::read_dir(dir) else {
                return 0;
            };
            entries
                .flatten()
                .filter_map(|e| Some((e.path(), e.metadata().ok()?)))
                .map(|(path, m)| if m.is_dir() { under(&path) } else { m.len() })
                .sum()
        }
        under(&self.dir)
    }

    pub fn seen(&self, key: &str) -> bool {
        self.seen.contains(key)
    }

    pub fn date(&self, i: u64) -> Option<DateTime<Local>> {
        let m = self.meta(i)?;
        DateTime::from_timestamp(m.date, 0).map(|d| d.with_timezone(&Local))
    }

    /// The chat message `i` came from, as its `harness:session` key.
    pub fn chat(&self, i: u64) -> Option<&str> {
        self.meta(i)?.chat.as_deref()
    }

    /// The ids of one chat's messages, by its `harness:session` key, newest
    /// first.
    pub fn of_chat<'a>(&'a self, key: &'a str) -> impl Iterator<Item = u64> + 'a {
        let ids = (0..self.index.len()).rev();
        ids.filter(move |&i| self.index[i].chat.as_deref() == Some(key))
            .map(|i| i as u64)
    }

    /// Message `i`, read back from its file.
    pub fn get(&self, i: u64) -> Result<Msg, String> {
        let m = self.meta(i).ok_or_else(|| format!("No message {i}."))?;
        if m.len == 0 {
            return Ok(lost(i, m.date));
        }
        let path = self
            .dir
            .join("main")
            .join(day_name(self.days[m.file as usize]));
        let mut file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        file.seek(SeekFrom::Start(m.off))
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0; m.len as usize];
        file.read_exact(&mut buf).map_err(|e| e.to_string())?;
        serde_json::from_slice(&buf).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Every message in order, read file by file (for search and audits).
    pub fn scan(&self, mut each: impl FnMut(Msg) -> bool) -> Result<(), String> {
        let mut torn = Vec::new();
        for day in &self.days {
            let path = self.dir.join("main").join(day_name(*day));
            let mut stop = false;
            read_jsonl::<Msg>(&path, &mut torn, |m, _, _| {
                if !stop {
                    stop = !each(m);
                }
            })?;
            if stop {
                break;
            }
        }
        Ok(())
    }

    /// Logs drafts in order, each its own fsynced line, a text longer than
    /// `PAGE` over several in a row, as UniiChat logs a message in pages. A
    /// draft whose source was logged before is skipped. Returns the id each
    /// draft's first page took.
    pub fn append(&mut self, drafts: Vec<Draft>) -> Result<Vec<u64>, String> {
        let mut ids = Vec::new();
        for d in drafts {
            if let Some(src) = &d.src
                && self.seen.contains(&src.key())
            {
                continue;
            }
            ids.push(self.len());
            for text in pages(&d.text) {
                self.log(&d, text.to_owned())?;
            }
        }
        Ok(ids)
    }

    /// Logs one page of draft `d`.
    fn log(&mut self, d: &Draft, text: String) -> Result<(), String> {
        let i = self.len();
        let size = render(d.kind, &text).len() as u64;
        let msg = Msg {
            display: d.display.clone(),
            i,
            kind: d.kind,
            chat: d.chat.clone(),
            text,
            size,
            date: fmt_date(&d.date),
            src: d.src.clone(),
        };
        // Files split by day only to stay manageable; a message never
        // goes to a day before the last written one, so ids stay in
        // file order.
        let mut day = d.date.date_naive();
        if let Some(&last) = self.days.last() {
            day = day.max(last);
        }
        let line = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
        let file = self.writer(day)?;
        let (_, out, end) = self.out.as_mut().unwrap();
        let off = *end;
        append_line(out, &line)?;
        *end += line.len() as u64 + 1;
        self.index_msg(&msg, file, off, line.len() as u32 + 1);
        Ok(())
    }

    fn writer(&mut self, day: NaiveDate) -> Result<u32, String> {
        if self.out.as_ref().is_none_or(|(d, _, _)| *d != day) {
            let path = self.dir.join("main").join(day_name(day));
            heal(&path)?;
            let file = open_append(&path)?;
            let end = file.metadata().map_err(|e| e.to_string())?.len();
            if self.days.last() != Some(&day) {
                self.days.push(day);
            }
            self.out = Some((day, file, end));
        }
        Ok(self.days.len() as u32 - 1)
    }

    pub fn register(&mut self, rec: ChatRec) -> Result<(), String> {
        let key = chat_key(&rec.harness, &rec.session);
        if self.chats.contains_key(&key) {
            return Ok(());
        }
        let line = serde_json::to_string(&rec).map_err(|e| e.to_string())?;
        append_line(&mut open_append(&self.dir.join("chats.jsonl"))?, &line)?;
        self.chats.insert(key, rec);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_text_is_logged_in_pages() {
        let line = "x".repeat(99) + "\n";
        let text = line.repeat(700);
        let got = pages(&text);
        assert_eq!(got.len(), 3);
        assert!(
            got.iter()
                .all(|p| p.chars().count() <= PAGE && p.ends_with('\n'))
        );
        assert_eq!(got.concat(), text);
        let flat = "é".repeat(PAGE + 5);
        let got = pages(&flat);
        assert_eq!(
            got.iter().map(|p| p.chars().count()).collect::<Vec<_>>(),
            [PAGE, 5]
        );
        assert_eq!(pages("short"), ["short"]);
    }

    #[test]
    fn a_write_cut_short_leaves_nothing_behind() {
        let path = std::env::temp_dir().join(format!("hippo-short-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut file = open_append(&path).unwrap();
        append_line(&mut file, "whole").unwrap();
        let mut shut = File::open(&path).unwrap();
        assert!(append_line(&mut shut, "lost").is_err());
        append_line(&mut file, "next").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "whole\nnext\n");
        let _ = fs::remove_file(&path);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hippo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn filled(dir: &Path, n: usize) -> PathBuf {
        let mut store = Store::open(dir, false).unwrap();
        let drafts = (0..n).map(|k| Draft {
            display: None,
            kind: Kind::User,
            chat: None,
            text: format!("words {k}"),
            date: Local::now(),
            src: None,
        });
        store.append(drafts.collect()).unwrap();
        day_files(&dir.join("main"))
            .unwrap()
            .last()
            .map(|d| dir.join("main").join(day_name(*d)))
            .unwrap()
    }

    fn lines(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// A store as `forget` leaves it stopped after `renames` of its renames.
    fn forgetting(name: &str, renames: usize) -> PathBuf {
        let dir = scratch(name);
        filled(&dir, 5);
        fs::write(dir.join("tree/2026-01-01.jsonl"), "").unwrap();
        fs::create_dir_all(dir.join("main.new")).unwrap();
        fs::create_dir_all(dir.join("tree.new")).unwrap();
        let day = fs::read_dir(dir.join("main")).unwrap().next().unwrap();
        let one = lines(&day.unwrap().path())[..1].join("\n") + "\n";
        fs::write(dir.join("main.new/2026-01-01.jsonl"), one).unwrap();
        let steps = [
            ("tree", "tree.gone"),
            ("main", "main.gone"),
            ("main.new", "main"),
        ];
        for (from, to) in &steps[..renames] {
            fs::rename(dir.join(from), dir.join(to)).unwrap();
        }
        dir
    }

    #[test]
    fn a_forget_stopped_before_the_log_moved_never_happened() {
        let dir = forgetting("forget-1", 1);
        let store = Store::open(&dir, true).unwrap();
        assert_eq!(store.len(), 5);
        assert!(dir.join("tree/2026-01-01.jsonl").exists());
    }

    #[test]
    fn a_forget_stopped_between_the_logs_is_finished() {
        for renames in [2, 3] {
            let dir = forgetting(&format!("forget-{renames}"), renames);
            let store = Store::open(&dir, true).unwrap();
            assert_eq!(store.len(), 1, "after rename {renames}");
            assert!(dir.join("tree").exists() && !dir.join("tree.new").exists());
        }
    }

    #[test]
    fn a_store_that_lost_its_log_will_not_start_empty() {
        let dir = scratch("no-log");
        filled(&dir, 5);
        fs::write(dir.join("chats.jsonl"), "{}\n").unwrap();
        fs::remove_dir_all(dir.join("main")).unwrap();
        assert!(Store::open(&dir, true).is_err());
    }

    #[test]
    fn ids_summarized_once_are_never_given_to_new_words() {
        let dir = scratch("cut");
        filled(&dir, 5);
        let mut store = Store::open(&dir, false).unwrap();
        store.reach(9);
        assert_eq!(store.len(), 9);
        assert!(store.get(7).unwrap().text.contains("lost"));
        store
            .append(vec![Draft {
                display: None,
                kind: Kind::Talk,
                chat: None,
                text: "new".into(),
                date: Local::now(),
                src: None,
            }])
            .unwrap();
        let store = Store::open(&dir, false).unwrap();
        assert_eq!(store.get(9).unwrap().text, "new");
        assert_eq!(store.len(), 10);
    }

    #[test]
    fn a_damaged_line_keeps_its_id_and_the_store_opens() {
        let dir = scratch("damaged");
        let path = filled(&dir, 5);
        let mut all = lines(&path);
        all[2] = "\u{0}garbage{{{".to_owned();
        fs::write(&path, all.join("\n") + "\n").unwrap();
        let mut store = Store::open(&dir, false).unwrap();
        assert_eq!(store.len(), 5);
        assert_eq!(store.get(1).unwrap().text, "words 1");
        assert_eq!(store.get(3).unwrap().text, "words 3");
        assert!(store.get(2).unwrap().text.contains("lost"));
        assert!(store.torn.iter().any(|t| t.contains("2")));
        let ids = store.append(vec![Draft {
            display: None,
            kind: Kind::User,
            chat: None,
            text: "after".to_owned(),
            date: Local::now(),
            src: None,
        }]);
        assert_eq!(ids.unwrap(), [5]);
        assert_eq!(store.get(5).unwrap().text, "after");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_repeated_id_is_skipped_and_the_first_stays() {
        let dir = scratch("repeated");
        let path = filled(&dir, 4);
        let mut all = lines(&path);
        all.insert(2, all[1].replace("words 1", "words again"));
        fs::write(&path, all.join("\n") + "\n").unwrap();
        let store = Store::open(&dir, false).unwrap();
        assert_eq!(store.len(), 4);
        assert_eq!(store.get(1).unwrap().text, "words 1");
        assert_eq!(store.get(2).unwrap().text, "words 2");
        assert_eq!(store.torn.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_id_beyond_all_reason_is_damage_not_a_gap() {
        let dir = scratch("beyond");
        let path = filled(&dir, 3);
        let mut all = lines(&path);
        all[1] = all[1].replace("\"i\":1,", "\"i\":1000000000000,");
        fs::write(&path, all.join("\n") + "\n").unwrap();
        let store = Store::open(&dir, false).unwrap();
        assert_eq!(store.len(), 3);
        assert_eq!(store.get(2).unwrap().text, "words 2");
        let _ = fs::remove_dir_all(&dir);
    }
}
