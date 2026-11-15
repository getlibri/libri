//! Search for people: a full-text index over every message of the log and
//! every line of the workers' logs, in `find.db` beside them. The log stays
//! the truth; the index is built from it and kept up by following the
//! files as `tail -f` would, so it can be deleted at any time and is built
//! again (a few seconds for a year of messages).
//!
//! `search` (a regex over the log, newest first) stays the agent's tool:
//! it answers exactly what it is asked. This answers what a person meant:
//! words stemmed ("deploying" finds "deployed"), the last word read as a
//! prefix while it is typed, ranked by how well each message matches, who
//! said it and how recent it is.

use crate::memory::store::Msg;
use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

/// Bumped when the schema or what a row holds changes: the index is then
/// built again from nothing.
const SCHEMA: i64 = 1;

/// The most hits one search gives.
pub const MAX: usize = 100;

/// The longest text a context line carries; tool output runs to megabytes.
const LINE_MAX: usize = 4000;

/// Main's messages carry this as their source; a worker's, its folder name.
pub const MAIN: &str = "";

/// The memory this service keeps an index for, and the handle its searches
/// share: one at a time is a few milliseconds each.
static SERVED: OnceLock<(PathBuf, Mutex<Option<Index>>)> = OnceLock::new();

/// Keeps the index of the memory in `dir` up from a thread of its own, a
/// second behind the log at most. A failure is reported and tried again:
/// search waits, nothing else does.
pub fn serve(dir: &Path) {
    let _ = SERVED.set((dir.to_owned(), Mutex::new(None)));
    let dir = dir.to_owned();
    let spawned = thread::Builder::new().name("find".into()).spawn(move || {
        let mut index = None;
        let mut said = String::new();
        loop {
            let step = match index.as_mut() {
                Some(ix) => {
                    let t = std::time::Instant::now();
                    Index::catch_up(ix).map(|n| {
                        if n > 10_000 {
                            eprintln!("hippo: search index: {n} lines in {:.1?}", t.elapsed());
                        }
                    })
                }
                None => Index::open(&dir).map(|ix| index = Some(ix)),
            };
            if let Err(e) = step
                && e != said
            {
                eprintln!("hippo: {e}");
                said = e;
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
    if let Err(e) = spawned {
        eprintln!("hippo: no thread for the search index: {e}");
    }
}

/// Runs `f` on the served index, opened for reading on first use.
pub fn with<T>(f: impl FnOnce(&Index) -> Result<T, String>) -> Result<T, String> {
    let (dir, reader) = SERVED.get().ok_or("This memory keeps no search index.")?;
    let mut held = reader.lock().unwrap_or_else(|p| p.into_inner());
    if held.is_none() {
        if !dir.join("find.db").exists() {
            return Err("The search index is still being built.".into());
        }
        *held = Some(Index::reader(dir)?);
    }
    f(held.as_ref().unwrap())
}

pub struct Index {
    db: Connection,
    dir: PathBuf,
}

/// One message found.
#[derive(Debug, Serialize)]
pub struct Hit {
    /// `""` for main's chat, else the worker's folder.
    pub src: String,
    /// The message's id in main; the line of the worker's log.
    pub at: u64,
    pub kind: String,
    /// Unix seconds; for an old worker's line, when it began.
    pub time: i64,
    /// The words around the match, each matched word between STX and ETX
    /// (U+0002, U+0003), characters no message types.
    pub text: String,
}

/// One line of a source, for reading a hit in its place.
#[derive(Debug, Serialize)]
pub struct Line {
    pub at: u64,
    pub kind: String,
    pub time: i64,
    pub text: String,
    /// The text was cut to `LINE_MAX`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
}

/// A worker, named as the person knows it.
#[derive(Debug, Serialize)]
pub struct Source {
    pub src: String,
    pub name: String,
    pub began: i64,
    pub archived: bool,
}

/// How hits are ordered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// By match, who spoke and recency together.
    Best,
    /// Newest first.
    New,
}

fn err(e: rusqlite::Error) -> String {
    format!("find: {e}")
}

impl Index {
    /// The index of the memory in `dir`, made or remade as needed.
    pub fn open(dir: &Path) -> Result<Index, String> {
        let db = Connection::open(dir.join("find.db")).map_err(err)?;
        db.pragma_update(None, "journal_mode", "wal").map_err(err)?;
        db.pragma_update(None, "synchronous", "normal")
            .map_err(err)?;
        // A first build is one transaction the size of the index: its
        // journal shrinks back once written through.
        db.pragma_update(None, "journal_size_limit", 16 << 20)
            .map_err(err)?;
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(err)?;
        if version != SCHEMA {
            db.execute_batch(
                "DROP TABLE IF EXISTS f; DROP TABLE IF EXISTS files;
                 -- porter: a word finds its other forms; prefix: the word
                 -- being typed finds its endings without a scan.
                 CREATE VIRTUAL TABLE f USING fts5(
                     text, kind UNINDEXED, src UNINDEXED, at UNINDEXED,
                     time UNINDEXED, path UNINDEXED,
                     tokenize = 'porter unicode61 remove_diacritics 2',
                     prefix = '2 3');
                 -- How far each file is read: a file replaced (its inode)
                 -- or cut short is read again from the start.
                 CREATE TABLE files(path TEXT PRIMARY KEY, ino INTEGER,
                     off INTEGER, line INTEGER);",
            )
            .map_err(err)?;
            db.pragma_update(None, "user_version", SCHEMA)
                .map_err(err)?;
        }
        Ok(Index {
            db,
            dir: dir.to_owned(),
        })
    }

    /// A second handle on the same index, for reading while this one writes.
    pub fn reader(dir: &Path) -> Result<Index, String> {
        let db = Connection::open_with_flags(
            dir.join("find.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(err)?;
        Ok(Index {
            db,
            dir: dir.to_owned(),
        })
    }

    /// Reads what was written since the last call into the index, and
    /// drops what went with files gone or replaced. Returns lines added.
    pub fn catch_up(&mut self) -> Result<usize, String> {
        let mut files = Vec::new();
        for day in listed(&self.dir.join("main"))? {
            files.push((day, MAIN.to_owned(), 0));
        }
        let own = memory_of(&self.dir);
        for w in listed(&self.dir.join("agents"))? {
            let log = w.join("log.jsonl");
            if log.is_file() && memory_of(&w) == own {
                let src = name_of(&w);
                files.push((log, src, began(&w)));
            }
        }
        let tx = self.db.transaction().map_err(err)?;
        let mut added = 0;
        let mut known: Vec<String> = {
            let mut q = tx.prepare("SELECT path FROM files").map_err(err)?;
            q.query_map([], |r| r.get(0))
                .map_err(err)?
                .collect::<Result<_, _>>()
                .map_err(err)?
        };
        for (path, src, began) in &files {
            let key = path.to_string_lossy().into_owned();
            known.retain(|k| k != &key);
            added += follow(&tx, path, &key, src, *began)?;
        }
        for gone in known {
            tx.execute("DELETE FROM f WHERE path = ?1", [&gone])
                .map_err(err)?;
            tx.execute("DELETE FROM files WHERE path = ?1", [&gone])
                .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(added)
    }

    /// Messages matching `words`, best `limit` first. Every word must
    /// match; when none holds all, any word will, and `loose` says so.
    pub fn find(
        &self,
        words: &str,
        order: Order,
        limit: usize,
    ) -> Result<(Vec<Hit>, bool), String> {
        let Some((all, any)) = query(words) else {
            return Ok((Vec::new(), false));
        };
        let limit = limit.clamp(1, MAX);
        let hits = self.ranked(&all, order, limit)?;
        if !hits.is_empty() || all == any {
            return Ok((hits, false));
        }
        Ok((self.ranked(&any, order, limit)?, true))
    }

    fn ranked(&self, q: &str, order: Order, limit: usize) -> Result<Vec<Hit>, String> {
        let now = chrono::Local::now().timestamp();
        // Ranked first, then snippets made for the few kept: a snippet for
        // each of thousands of matches is most of a search's time.
        let by = match order {
            // bm25 is negative, better more so. Words said outweigh tools'
            // output; this fortnight outweighs last year, at most twice.
            Order::Best => {
                "bm25(f) * (CASE kind WHEN 'user' THEN 1.6 WHEN 'talk' THEN 1.4
                     WHEN 'work' THEN 1.3 WHEN 'note' THEN 1.0 WHEN 'tool' THEN 0.7
                     ELSE 0.5 END)
                 * (1.0 + 1.0 / (1.0 + max(0, ?2 - time) / 1209600.0))"
            }
            Order::New => "-time * 1e6 - at",
        };
        let sql = format!("SELECT rowid FROM f WHERE f MATCH ?1 ORDER BY {by} LIMIT ?3");
        let mut rank = self.db.prepare_cached(&sql).map_err(err)?;
        let ids: Vec<i64> = rank
            .query_map(params![q, now, limit as i64], |r| r.get(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        let mut snip = self
            .db
            .prepare_cached(
                "SELECT src, at, kind, time, snippet(f, 0, char(2), char(3), '…', 24)
                 FROM f WHERE f MATCH ?1 AND rowid = ?2",
            )
            .map_err(err)?;
        let mut hits = Vec::with_capacity(ids.len());
        for id in ids {
            let hit = snip
                .query_row(params![q, id], |r| {
                    Ok(Hit {
                        src: r.get(0)?,
                        at: r.get::<_, i64>(1)? as u64,
                        kind: r.get(2)?,
                        time: r.get(3)?,
                        text: r.get(4)?,
                    })
                })
                .map_err(err)?;
            hits.push(hit);
        }
        Ok(hits)
    }

    /// Lines `before` ahead of `at` to `after` behind it in `src`, from the
    /// index (main's are its messages, a worker's its log), oldest first.
    pub fn around(&self, src: &str, at: u64, before: u64, after: u64) -> Result<Vec<Line>, String> {
        let from = at.saturating_sub(before) as i64;
        let to = at.saturating_add(after) as i64;
        let mut q = self
            .db
            .prepare_cached(
                "SELECT at, kind, time, substr(text, 1, ?4), length(text) > ?4 FROM f
                 WHERE src = ?1 AND at BETWEEN ?2 AND ?3 ORDER BY at",
            )
            .map_err(err)?;
        q.query_map(params![src, from, to, LINE_MAX as i64], |r| {
            Ok(Line {
                at: r.get::<_, i64>(0)? as u64,
                kind: r.get(1)?,
                time: r.get(2)?,
                text: r.get(3)?,
                cut: r.get(4)?,
            })
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)
    }

    /// Every worker with a log, newest first.
    pub fn sources(&self) -> Result<Vec<Source>, String> {
        let mut out = Vec::new();
        let own = memory_of(&self.dir);
        for w in listed(&self.dir.join("agents"))? {
            if !w.join("log.jsonl").is_file() || memory_of(&w) != own {
                continue;
            }
            let src = name_of(&w);
            let name = fs::read_to_string(w.join("name"))
                .map(|n| n.trim().to_owned())
                .unwrap_or_else(|_| src.clone());
            out.push(Source {
                name,
                began: began(&w),
                archived: crate::code::source::archived(&w),
                src,
            });
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.began));
        Ok(out)
    }
}

/// The entries of a folder, by name; none when it is missing.
fn listed(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut all: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    all.sort();
    Ok(all)
}

/// The named memory a store or a worker's folder belongs to, from its
/// `memory` file; None is the main one. A store reads only its own memory's
/// workers: Libri keeps every worker in one folder and names the memory of
/// those a named memory's chat started, so neither memory finds the other's.
fn memory_of(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("memory")).ok()?;
    Some(text.trim().to_owned()).filter(|t| !t.is_empty())
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// When a worker began, in Unix seconds: its `began` file holds
/// nanoseconds; an old worker without one, when its folder was made.
fn began(w: &Path) -> i64 {
    if let Some(ns) = fs::read_to_string(w.join("began"))
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
    {
        return ns / 1_000_000_000;
    }
    fs::metadata(w).map(|m| m.mtime()).unwrap_or(0)
}

/// Reads `path` on from where the index left it, a whole line at a time.
fn follow(
    tx: &rusqlite::Transaction,
    path: &Path,
    key: &str,
    src: &str,
    began: i64,
) -> Result<usize, String> {
    let Ok(meta) = fs::metadata(path) else {
        return Ok(0);
    };
    let ino = meta.ino() as i64;
    let had: Option<(i64, i64, i64)> = tx
        .query_row(
            "SELECT ino, off, line FROM files WHERE path = ?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let (mut off, mut line) = match had {
        Some((i, off, line)) if i == ino && off as u64 <= meta.len() => (off as u64, line),
        Some(_) => {
            tx.execute("DELETE FROM f WHERE path = ?1", [key])
                .map_err(err)?;
            (0, 0)
        }
        None => (0, 0),
    };
    if off == meta.len() {
        return Ok(0);
    }
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    // A line still being written waits for its newline.
    let Some(end) = buf.iter().rposition(|&b| b == b'\n') else {
        return Ok(0);
    };
    let now = chrono::Local::now().timestamp();
    let mut put = tx
        .prepare_cached(
            "INSERT INTO f(text, kind, src, at, time, path) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(err)?;
    let mut added = 0;
    for raw in buf[..=end].split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
        let row = if src == MAIN {
            serde_json::from_slice::<Msg>(raw).ok().map(|m| {
                let time = crate::memory::store::parse_date(&m.date).map_or(0, |d| d.timestamp());
                (m.text, m.kind.name().to_owned(), m.i, time)
            })
        } else {
            serde_json::from_slice::<serde_json::Value>(raw)
                .ok()
                .map(|v| {
                    // A worker's log keeps no times: what was there at first
                    // dates from its start, what comes after from when it came.
                    let time = if had.is_some() { now } else { began };
                    let s = |k| v[k].as_str().unwrap_or("").to_owned();
                    (s("text"), s("kind"), line as u64, time)
                })
        };
        line += 1;
        if let Some((text, kind, at, time)) = row {
            put.execute(params![text, kind, src, at as i64, time, key])
                .map_err(err)?;
            added += 1;
        }
    }
    off += end as u64 + 1;
    tx.execute(
        "INSERT INTO files(path, ino, off, line) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET ino = ?2, off = ?3, line = ?4",
        params![key, ino, off as i64, line],
    )
    .map_err(err)?;
    Ok(added)
}

/// What a person typed as FTS5 queries: every word, and any word. Words
/// in quotes stay a phrase; the last word, while still being typed (no
/// space after it), also finds the words it begins. None when nothing is
/// left to look for.
pub fn query(typed: &str) -> Option<(String, String)> {
    let mut terms: Vec<String> = Vec::new();
    for (k, part) in typed.split('"').enumerate() {
        if k % 2 == 1 {
            let words = words(part);
            if !words.is_empty() {
                terms.push(format!("\"{}\"", words.join(" ")));
            }
        } else {
            terms.extend(words(part).into_iter().map(|w| format!("\"{w}\"")));
        }
    }
    // A question asked as one would ask a person: its filler words match
    // nearly every message and bury the few that matter. They stay when
    // nothing else is left, or in quotes, or while still being typed.
    let typing = !typed.ends_with(|c: char| c.is_whitespace() || c == '"');
    let filler = |t: &String| {
        t.trim_matches('"').split(' ').count() == 1
            && FILLER.contains(&t.trim_matches('"').to_lowercase().as_str())
    };
    if terms.iter().any(|t| !filler(t)) {
        let last = terms.len() - 1;
        let kept: Vec<String> = terms
            .iter()
            .enumerate()
            .filter(|(k, t)| !filler(t) || (typing && *k == last))
            .map(|(_, t)| t.clone())
            .collect();
        terms = kept;
    }
    if typing && let Some(t) = terms.last_mut() {
        // One letter would find nearly every message: it waits for a second.
        if t.chars().count() > 3 {
            t.push('*');
        }
    }
    if terms.is_empty() {
        return None;
    }
    Some((terms.join(" "), terms.join(" OR ")))
}

/// English words too common to tell messages apart.
const FILLER: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "but", "by", "can", "did", "do", "does",
    "for", "from", "had", "has", "have", "how", "i", "if", "in", "is", "it", "its", "me", "my",
    "of", "on", "or", "our", "say", "said", "so", "that", "the", "their", "them", "then", "there",
    "they", "this", "to", "us", "was", "we", "were", "what", "when", "where", "which", "who",
    "why", "will", "with", "would", "you", "your",
];

/// The words of a phrase as the tokenizer cuts them: letters and digits.
fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn line(i: u64, kind: &str, text: &str, date: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({"i": i, "kind": kind, "text": text, "size": 1, "date": date})
        )
    }

    /// A store of three messages and one worker, in a folder of its own.
    fn store(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hippo-find-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("main")).unwrap();
        fs::write(
            d.join("main/2026-10-01.jsonl"),
            line(
                0,
                "user",
                "Can we deploy the tunnel to Attic?",
                "2026-10-01T09:00:00-04:00",
            ) + &line(
                1,
                "talk",
                "Deployed; the tunnel is up.",
                "2026-10-01T09:01:00-04:00",
            ) + &line(
                2,
                "echo",
                "deploy log: 400 lines of output",
                "2026-10-01T09:02:00-04:00",
            ),
        )
        .unwrap();
        let w = d.join("agents/finn-2026-11-03-0905");
        fs::create_dir_all(&w).unwrap();
        fs::write(w.join("name"), "Finn").unwrap();
        fs::write(w.join("began"), "1793700000000000000").unwrap();
        fs::write(
            w.join("log.jsonl"),
            "{\"kind\":\"user\",\"text\":\"Add a logout button to Juniper\"}\n{\"kind\":\"talk\",\"text\":\"Logout added.\"}\n",
        )
        .unwrap();
        d
    }

    #[test]
    fn typed_words_become_queries() {
        assert_eq!(query("tunnel a").unwrap().0, "\"tunnel\" \"a\"");
        assert_eq!(query("tunnel attic").unwrap().0, "\"tunnel\" \"attic\"*");
        assert_eq!(query("tunnel attic ").unwrap().1, "\"tunnel\" OR \"attic\"");
        assert_eq!(query("\"log out\" end").unwrap().0, "\"log out\" \"end\"*");
        assert_eq!(
            query("staff \"log out").unwrap().0,
            "\"staff\" \"log out\"*"
        );
        assert_eq!(query("C++ (x)").unwrap().0, "\"C\" \"x\"");
        assert!(query(" \"\" ?! ").is_none());
        // Filler goes unless it is all there is, quoted, or being typed.
        assert_eq!(
            query("what did bob say about the frame ").unwrap().0,
            "\"bob\" \"frame\""
        );
        assert_eq!(query("what is the").unwrap().0, "\"what\" \"is\" \"the\"*");
        assert_eq!(
            query("\"the frame\" of").unwrap().0,
            "\"the frame\" \"of\"*"
        );
    }

    #[test]
    fn finds_main_and_workers_by_stem_and_prefix() {
        let d = store("stem");
        let mut ix = Index::open(&d).unwrap();
        assert_eq!(ix.catch_up().unwrap(), 5);
        assert_eq!(ix.catch_up().unwrap(), 0);
        // "deploying" finds "deploy" and "Deployed"; what was said ranks
        // above a tool's output.
        let (hits, loose) = ix.find("deploying ", Order::Best, 10).unwrap();
        assert!(!loose);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits.last().unwrap().kind, "echo");
        assert!(hits[0].text.contains('\u{2}'));
        // The word being typed is a prefix.
        let (hits, _) = ix.find("logo", Order::Best, 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.src == "finn-2026-11-03-0905"));
        // No message holds both: either does.
        let (hits, loose) = ix.find("tunnel juniper ", Order::Best, 10).unwrap();
        assert!(loose);
        assert_eq!(hits.len(), 3);
        // Newest first.
        let (hits, _) = ix.find("tunnel ", Order::New, 10).unwrap();
        assert_eq!(hits.iter().map(|h| h.at).collect::<Vec<_>>(), [1, 0]);
    }

    #[test]
    fn a_store_finds_only_its_own_memorys_workers() {
        let d = store("memories");
        let w = d.join("agents/milo-2026-11-09-1300");
        fs::create_dir_all(&w).unwrap();
        fs::write(w.join("memory"), "jobs\n").unwrap();
        fs::write(
            w.join("log.jsonl"),
            "{\"kind\":\"user\",\"text\":\"Pull the Greenhouse boards\"}\n",
        )
        .unwrap();
        let mut ix = Index::open(&d).unwrap();
        ix.catch_up().unwrap();
        assert!(ix.find("greenhouse", Order::Best, 10).unwrap().0.is_empty());
        assert_eq!(ix.find("logout", Order::Best, 10).unwrap().0.len(), 2);
        // The same folder, read as the Jobs memory's store.
        let _ = fs::remove_file(d.join("find.db"));
        fs::write(d.join("memory"), "jobs").unwrap();
        let mut ix = Index::open(&d).unwrap();
        ix.catch_up().unwrap();
        assert_eq!(ix.find("greenhouse", Order::Best, 10).unwrap().0.len(), 1);
        assert!(ix.find("logout", Order::Best, 10).unwrap().0.is_empty());
        let sources: Vec<String> = ix.sources().unwrap().into_iter().map(|s| s.src).collect();
        assert_eq!(sources, ["milo-2026-11-09-1300"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn follows_appends_and_replaced_files() {
        let d = store("follow");
        let mut ix = Index::open(&d).unwrap();
        ix.catch_up().unwrap();
        let day = d.join("main/2026-10-01.jsonl");
        let mut f = fs::OpenOptions::new().append(true).open(&day).unwrap();
        // Half a line waits for the rest.
        let next = line(3, "user", "and the pinecone", "2026-10-01T10:00:00-04:00");
        f.write_all(&next.as_bytes()[..10]).unwrap();
        assert_eq!(ix.catch_up().unwrap(), 0);
        f.write_all(&next.as_bytes()[10..]).unwrap();
        assert_eq!(ix.catch_up().unwrap(), 1);
        assert_eq!(ix.find("pinecone ", Order::Best, 5).unwrap().0[0].at, 3);
        // Forgetting rewrites the log and renumbers: the file is new.
        fs::remove_file(&day).unwrap();
        fs::write(
            &day,
            line(0, "user", "pinecone only", "2026-10-01T10:00:00-04:00"),
        )
        .unwrap();
        assert_eq!(ix.catch_up().unwrap(), 1);
        let (hits, _) = ix.find("tunnel ", Order::Best, 5).unwrap();
        assert!(hits.is_empty());
        // A worker's folder gone takes its lines with it.
        fs::remove_dir_all(d.join("agents/finn-2026-11-03-0905")).unwrap();
        ix.catch_up().unwrap();
        assert!(ix.find("logout ", Order::Best, 5).unwrap().0.is_empty());
    }

    #[test]
    fn reads_a_hit_in_its_place() {
        let d = store("around");
        let mut ix = Index::open(&d).unwrap();
        ix.catch_up().unwrap();
        let r = Index::reader(&d).unwrap();
        let lines = r.around(MAIN, 1, 1, 5).unwrap();
        assert_eq!(lines.iter().map(|l| l.at).collect::<Vec<_>>(), [0, 1, 2]);
        let lines = r.around("finn-2026-11-03-0905", 0, 3, 0).unwrap();
        assert_eq!(lines[0].text, "Add a logout button to Juniper");
        assert_eq!(lines[0].time, 1793700000);
        let s = r.sources().unwrap();
        assert_eq!((s[0].name.as_str(), s[0].archived), ("Finn", false));
    }
}
