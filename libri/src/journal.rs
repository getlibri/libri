//! What a restart must not lose: the person's words from the moment they are
//! accepted until the memory holds them, and which turn was running.
//!
//! The site says 202 and keeps the words in memory, for as long as a turn in
//! front takes, so each goes to a file of its own first. Once saved to hippo a
//! file is cleared and the turn's record takes over: the memory has the words
//! and each step, and only the fact that the turn was running would be lost.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::sync::watch;

/// Whose turn was cut off, which says what can be done about it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cut {
    Person,
    /// The reading of a worker's report: the report is on the Work page.
    Work,
    /// Watches and messages: each is marked done only after
    /// its turn, so it comes round again by itself.
    Own,
}

/// The turn under way.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Open {
    pub cut: Cut,
    pub words: String,
    /// The messages folded into it.
    pub ids: Vec<String>,
    /// The person's words that went into it as it ran: the memory has them,
    /// but only this turn's answer would have taken them up.
    #[serde(default)]
    pub joined: Vec<String>,
    /// How many times running it has been picked up after a restart.
    pub tries: u32,
    /// Its questions waiting for the person's answer.
    pub asking: u32,
    /// How many files came with it. They are not kept, only said to be lost.
    pub files: usize,
    /// The key hippo keeps the person's words under once it has them: the
    /// memory's rows after it say how far the turn got.
    #[serde(default)]
    pub kept: Option<String>,
    /// Whether background jobs of the turn's run on after its answer, so an
    /// answer kept does not end it.
    #[serde(default)]
    pub working: bool,
}

/// Words accepted and not yet in the memory.
#[derive(Debug, Serialize, Deserialize)]
pub struct Waiting {
    pub id: String,
    pub text: String,
    /// When it came, in nanoseconds: the order they are run in again.
    pub at: u128,
    /// How many files came with it. They are not kept, only said to be lost.
    pub files: usize,
}

fn inbox(state: &Path) -> PathBuf {
    state.join("site/inbox")
}

fn record(state: &Path) -> PathBuf {
    state.join("open-turn.json")
}

/// The file of a message: ids come from the client, so never its path as is.
fn entry(state: &Path, id: &str) -> PathBuf {
    let name: String = id.bytes().map(|b| format!("{b:02x}")).collect();
    inbox(state).join(name)
}

/// Writes `file` whole or not at all, and on disk before returning.
fn write(file: &Path, body: &[u8]) -> std::io::Result<()> {
    let dir = file.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(dir)?;
    let part = file.with_extension("part");
    let mut f = fs::File::create(&part)?;
    f.write_all(body)?;
    f.sync_all()?;
    fs::rename(&part, file)?;
    fs::File::open(dir)?.sync_all()
}

/// Keeps a message until `clear`.
pub fn keep(state: &Path, id: &str, text: &str, files: usize, at: u128) -> Result<(), String> {
    let body = serde_json::to_vec(&Waiting {
        id: id.to_owned(),
        text: text.to_owned(),
        at,
        files,
    })
    .map_err(|e| e.to_string())?;
    write(&entry(state, id), &body).map_err(|e| format!("journal: {e}"))
}

/// The memory has these, or the person has heard it could not take them.
pub fn clear<S: AsRef<str>>(state: &Path, ids: &[S]) {
    for id in ids {
        let _ = fs::remove_file(entry(state, id.as_ref()));
    }
}

/// All that waits, oldest first.
pub fn waiting(state: &Path) -> Vec<Waiting> {
    let Ok(files) = fs::read_dir(inbox(state)) else {
        return Vec::new();
    };
    let mut all: Vec<Waiting> = files
        .flatten()
        .filter_map(|f| serde_json::from_slice(&fs::read(f.path()).ok()?).ok())
        .collect();
    all.sort_by_key(|w| w.at);
    all
}

fn read(state: &Path) -> Option<Open> {
    serde_json::from_slice(&fs::read(record(state)).ok()?).ok()
}

/// Records the turn that begins now, or the change in it.
pub fn open(state: &Path, turn: &Open) {
    let written = serde_json::to_vec(turn)
        .map_err(|e| e.to_string())
        .and_then(|body| write(&record(state), &body).map_err(|e| e.to_string()));
    if let Err(e) = written {
        eprintln!("libri: open turn: {e}");
    }
}

/// The person's words went into the turn under way.
pub fn joined(state: &Path, words: &str, files: usize) {
    let Some(mut turn) = read(state) else {
        return;
    };
    turn.joined.push(words.to_owned());
    turn.files += files;
    open(state, &turn);
}

/// A question of the turn's went to the person, or came back.
pub fn asking(state: &Path, asked: bool) {
    let Some(mut turn) = read(state) else {
        return;
    };
    turn.asking = if asked {
        turn.asking + 1
    } else {
        turn.asking.saturating_sub(1)
    };
    open(state, &turn);
}

/// The memory has the turn's words, under `key`.
pub fn kept(state: &Path, key: &str) {
    let Some(mut turn) = read(state) else {
        return;
    };
    turn.kept = Some(key.to_owned());
    open(state, &turn);
}

/// The turn's background jobs started, or all ended.
pub fn working(state: &Path, working: bool) {
    let Some(mut turn) = read(state) else {
        return;
    };
    if turn.working != working {
        turn.working = working;
        open(state, &turn);
    }
}

/// The turn is over.
pub fn close(state: &Path) {
    let _ = fs::remove_file(record(state));
}

/// The turn the last run left open. Its record stays until a word of it has
/// reached the person, as another restart may come first.
pub fn left_open(state: &Path) -> Option<Open> {
    let turn = read(state)?;
    // The memory holds these, so running them too would answer them twice.
    clear(state, &turn.ids);
    Some(turn)
}

/// How many message ids are remembered as taken; a client sends again only
/// what waited in its queue, a few at most.
const TAKEN_KEPT: usize = 64;

/// The ids of the latest messages taken. A client that lost the answer to a
/// send (a 504, a reconnect, a restart) sends it again, and the second copy
/// must not run the turn twice. Kept on disk, since a deploy's restart is when
/// clients resend. A copy that comes while the first is still being taken
/// waits for the first's outcome: answered 202 before then, it would drop the
/// words if the first then failed.
pub struct Taken {
    file: PathBuf,
    ids: Mutex<Ids>,
}

#[derive(Default)]
struct Ids {
    /// Taken for good, oldest first.
    done: VecDeque<String>,
    /// Being taken now; the sender drops when that send ends either way.
    going: HashMap<String, watch::Sender<()>>,
}

/// One send's hold on a message id. Dropped without `accept` (a failed send,
/// or a request its client hung up on), it lets the id go for a copy to run.
pub struct Claim<'a> {
    taken: &'a Taken,
    id: String,
    accepted: bool,
}

impl Claim<'_> {
    /// The message is this daemon's now: copies of it are no-ops from here.
    pub fn accept(mut self) {
        self.accepted = true;
        let mut ids = self.taken.ids.lock().unwrap();
        if ids.done.len() == TAKEN_KEPT {
            ids.done.pop_front();
        }
        ids.done.push_back(self.id.clone());
        self.taken.save(&ids.done);
        ids.going.remove(&self.id);
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if !self.accepted {
            self.taken.ids.lock().unwrap().going.remove(&self.id);
        }
    }
}

impl Taken {
    pub fn load(state: &Path) -> Self {
        let file = state.join("site/taken.json");
        let done = fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Taken {
            file,
            ids: Mutex::new(Ids {
                done,
                going: HashMap::new(),
            }),
        }
    }

    /// Claims `id` for one send: None when a copy of it was taken already.
    /// While another copy is being taken, waits for its outcome first.
    pub async fn claim(&self, id: &str) -> Option<Claim<'_>> {
        loop {
            let mut first = {
                let mut ids = self.ids.lock().unwrap();
                if ids.done.iter().any(|t| t == id) {
                    return None;
                }
                match ids.going.get(id) {
                    Some(going) => going.subscribe(),
                    None => {
                        ids.going.insert(id.to_owned(), watch::channel(()).0);
                        return Some(Claim {
                            taken: self,
                            id: id.to_owned(),
                            accepted: false,
                        });
                    }
                }
            };
            // Nothing is ever sent: this ends when the first's sender drops.
            while first.changed().await.is_ok() {}
        }
    }

    /// Whole or not at all, but not synced: a power cut loses only the
    /// guard against a resend, never words.
    fn save(&self, ids: &VecDeque<String>) {
        let saved = serde_json::to_vec(ids)
            .map_err(|e| e.to_string())
            .and_then(|body| {
                let part = self.file.with_extension("part");
                fs::create_dir_all(self.file.parent().unwrap_or(Path::new(".")))
                    .and_then(|()| fs::write(&part, body))
                    .and_then(|()| fs::rename(&part, &self.file))
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = saved {
            eprintln!("libri: taken: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("libri-journal-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn keeps_words_in_order_until_cleared() {
        let d = dir("order");
        keep(&d, "../b", "second", 0, 2).unwrap();
        keep(&d, "a", "first", 1, 1).unwrap();
        let all = waiting(&d);
        assert_eq!(
            all.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(all[0].files, 1);
        clear(&d, &["a", "../b"]);
        assert!(waiting(&d).is_empty());
    }

    #[test]
    fn a_turn_left_open_takes_its_words_from_the_inbox() {
        let d = dir("open");
        keep(&d, "a", "ran", 0, 1).unwrap();
        keep(&d, "b", "behind", 0, 2).unwrap();
        open(
            &d,
            &Open {
                cut: Cut::Person,
                words: "ran".into(),
                ids: vec!["a".into()],
                joined: Vec::new(),
                tries: 1,
                asking: 0,
                files: 0,
                kept: None,
                working: false,
            },
        );
        asking(&d, true);
        kept(&d, "spool-1-0");
        working(&d, true);
        joined(&d, "and this", 2);
        let turn = left_open(&d).unwrap();
        assert_eq!((turn.joined[0].as_str(), turn.files), ("and this", 2));
        assert_eq!((turn.cut, turn.tries, turn.asking), (Cut::Person, 1, 1));
        assert_eq!(
            (turn.kept.as_deref(), turn.working),
            (Some("spool-1-0"), true)
        );
        assert_eq!(waiting(&d)[0].id, "b");
        // Still there for a restart before anyone was told.
        assert!(left_open(&d).is_some());
        close(&d);
        assert!(left_open(&d).is_none());
    }

    #[tokio::test]
    async fn a_message_id_runs_once_across_copies_and_restarts() {
        let d = dir("taken");
        let taken = Taken::load(&d);
        taken.claim("m1").await.unwrap().accept();
        assert!(taken.claim("m1").await.is_none(), "a copy ran again");
        let again = Taken::load(&d);
        assert!(
            again.claim("m1").await.is_none(),
            "a restart forgot what ran"
        );
        drop(again.claim("m2").await.unwrap());
        assert!(
            again.claim("m2").await.is_some(),
            "a failed send can't be tried again"
        );
        for i in 0..TAKEN_KEPT {
            again.claim(&format!("x{i}")).await.unwrap().accept();
        }
        assert!(again.claim("m1").await.is_some(), "the oldest is let go");
    }

    /// Found in review, 11-02: a copy sent while the first was still being taken got
    /// 202, and the words were lost when the first then failed.
    #[tokio::test]
    async fn a_copy_in_flight_waits_for_the_first_and_runs_if_it_fails() {
        let taken = std::sync::Arc::new(Taken::load(&dir("inflight")));
        let first = taken.claim("m1").await.unwrap();
        let copy = {
            let taken = taken.clone();
            tokio::spawn(async move { taken.claim("m1").await.map(|c| c.accept()).is_some() })
        };
        tokio::task::yield_now().await;
        assert!(
            !copy.is_finished(),
            "the copy was answered before the first ended"
        );
        drop(first);
        assert!(copy.await.unwrap(), "the copy of a failed send didn't run");
        assert!(taken.claim("m1").await.is_none());

        let first = taken.claim("m2").await.unwrap();
        let copy = {
            let taken = taken.clone();
            tokio::spawn(async move { taken.claim("m2").await.is_some() })
        };
        tokio::task::yield_now().await;
        first.accept();
        assert!(!copy.await.unwrap(), "the copy of a taken send ran too");
    }
}
