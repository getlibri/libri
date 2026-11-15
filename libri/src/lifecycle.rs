//! Where a worker's job stands: one record, `state.json` in its folder,
//! changed only by `transition` under the folder's lock.
//!
//! It replaces the marker files a worker's state used to be (`go`,
//! `running`, `cancel`, `archived`, `awaited/`, `surfaced`, `reported`,
//! `read`, `began`), whose sequences no lock covered: archiving a paused
//! worker left it waiting on a reply that had gone elsewhere, for good once
//! it was restored. A folder from before is read into a record the first time
//! it is locked; its markers stay, written from the record on each change,
//! so a build rolled back to still reads the truth. What is data rather than
//! state stays in
//! files of its own: the log, `told/`, `asked/`, `attached/`, `route.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const RECORD: &str = "state.json";
const LOCK: &str = "lock";

/// The phase of a worker's job. What it waits on, a nap set while it works
/// and what the person has read are beside it in `Record`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Phase {
    /// Nothing to do: its job is over, or it never had one.
    #[default]
    Idle,
    /// A run waits to start.
    Queued,
    /// A run is at work; `cancel` once a stop has been asked of it.
    Running {
        #[serde(default)]
        cancel: bool,
    },
    /// Its run ended with replies awaited: the job goes on when they come.
    Paused,
    /// Put down by the person until `until`, seconds since the epoch: out of
    /// Waiting until then, back in it after.
    Napping { until: u64 },
    /// Put away by the person at `at`, seconds since the epoch.
    Archived { at: u64 },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub phase: Phase,
    /// The workers it has written to and had no reply from, each with when.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub awaited: BTreeMap<String, u64>,
    /// A nap the person set while it was at work, taken when the job stops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nap_after: Option<u64>,
    /// The main chat has heard that its wait has run long.
    #[serde(default)]
    pub surfaced: bool,
    /// The main chat has had a report of it.
    #[serde(default)]
    pub reported: bool,
    /// When it last came to work, in nanoseconds: the order of Working.
    #[serde(default)]
    pub began: u128,
    /// The length of the log the person last read. A length, not a time:
    /// a write in the same second as the read is still news.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<u64>,
    /// What the worker said it needs of the person. It holds the card in
    /// Waiting, read or not, until the person or the main chat tells it
    /// something or the person clears it: opening the thread to read the
    /// question is not answering it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need: Option<Need>,
    /// The length of the log when it was last merged into the main chat:
    /// while the log is no longer, it has nothing new to merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged: Option<u64>,
    /// The folder's marker files as this record last read them. A build
    /// from before records still writes markers, and after a rollback to
    /// one and a roll forward again they say more than the record does;
    /// while they match it, the record is the newer. Goes with the markers,
    /// once no build that writes them can come back.
    #[serde(default)]
    pub markers: String,
}

/// What a worker needs of the person, as its Waiting card says it. A finished
/// job with its report unread is the third kind, "For review", which the
/// record needn't hold: reading it is the whole of acting on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Need {
    pub kind: NeedKind,
    /// The worker's one-line ask.
    pub ask: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NeedKind {
    /// A merge, a deploy or a spend that is the person's to allow.
    Approval,
    /// A question only the person can settle.
    Answer,
}

/// What happens to a worker's job.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A run is wanted: it is started, told something or answered. `at` is
    /// the time it comes to work, kept unless it never left Working.
    Go {
        at: u128,
    },
    /// The tick claims the waiting run.
    Take,
    /// The run is over; `late` when words came too late for it, which it
    /// goes on to answer.
    Ended {
        late: bool,
    },
    /// The person or the main chat asks the run at work to stop.
    StopAsked,
    /// A run that waited to start won't.
    Unqueue,
    /// A restart cut the run short: it waits to start again.
    Resume,
    Archive {
        at: u64,
    },
    Restore,
    /// It has written to `peer` and waits on the reply.
    Awaits {
        peer: String,
        at: u64,
    },
    /// `peer`'s reply came, or never will.
    Answered {
        peer: String,
    },
    /// Its run failed or stopped: it waits on no one.
    WaitsOnNone,
    /// The person puts it down until `until`; `now` to refuse a past time.
    Nap {
        until: u64,
        now: u64,
    },
    /// Its nap ends: the person woke it, or it needs someone now.
    Wake,
}

/// Applies `event` to `r`, or says why it can't be.
pub fn transition(r: &mut Record, event: Event) -> Result<(), String> {
    use Phase::*;
    match event {
        Event::Go { at } => match r.phase {
            Running { .. } => return Err("at work".into()),
            Queued => {}
            Paused => r.phase = Queued,
            Napping { until } => {
                r.nap_after = Some(until);
                r.began = at;
                r.phase = Queued;
            }
            Idle | Archived { .. } => {
                r.began = at;
                r.phase = Queued;
            }
        },
        Event::Take => match r.phase {
            Queued => r.phase = Running { cancel: false },
            _ => return Err("no run waits".into()),
        },
        Event::Ended { late } => match r.phase {
            Running { .. } if late => r.phase = Queued,
            Running { .. } => r.phase = stopped(r),
            _ => return Err("not at work".into()),
        },
        Event::StopAsked => match r.phase {
            Running { .. } => r.phase = Running { cancel: true },
            _ => return Err("not at work".into()),
        },
        Event::Unqueue => match r.phase {
            Queued => r.phase = stopped(r),
            _ => return Err("not at work".into()),
        },
        Event::Resume => {
            if let Running { .. } = r.phase {
                r.phase = Queued;
            }
        }
        Event::Archive { at } => match r.phase {
            Queued | Running { .. } => return Err("still at work".into()),
            Paused => return Err("waiting on a reply; stop it first".into()),
            Archived { .. } => {}
            Idle | Napping { .. } => {
                r.nap_after = None;
                r.need = None;
                r.phase = Archived { at };
            }
        },
        Event::Restore => match r.phase {
            Archived { .. } => r.phase = Idle,
            _ => return Err("not archived".into()),
        },
        Event::Awaits { peer, at } => {
            match r.phase {
                Archived { .. } => return Err("archived".into()),
                Idle => r.phase = Paused,
                Napping { until } => {
                    r.nap_after = Some(until);
                    r.phase = Paused;
                }
                Queued | Running { .. } | Paused => {}
            }
            r.awaited.insert(peer, at);
            r.surfaced = false;
        }
        Event::Answered { peer } => {
            r.awaited.remove(&peer);
            r.surfaced = false;
            if r.phase == Paused && r.awaited.is_empty() {
                r.phase = stopped(r);
            }
        }
        Event::WaitsOnNone => {
            r.awaited.clear();
            r.surfaced = false;
            if r.phase == Paused {
                r.phase = stopped(r);
            }
        }
        Event::Nap { until, now } => {
            if until <= now {
                return Err("That wake time has passed. Choose another time.".into());
            }
            match r.phase {
                Archived { .. } => return Err("archived".into()),
                Idle | Napping { .. } => r.phase = Napping { until },
                Queued | Running { .. } | Paused => r.nap_after = Some(until),
            }
        }
        Event::Wake => {
            r.nap_after = None;
            if let Napping { .. } = r.phase {
                // Back in Waiting, as a snoozed thread comes back to T3
                // Code's inbox, whatever was read meanwhile.
                r.read = None;
                r.phase = Idle;
            }
        }
    }
    Ok(())
}

/// Where a job goes when its run stops for good: paused while replies are
/// owed it, else down for the nap set meanwhile, else idle.
fn stopped(r: &mut Record) -> Phase {
    if !r.awaited.is_empty() {
        return Phase::Paused;
    }
    match r.nap_after.take() {
        Some(until) => Phase::Napping { until },
        None => Phase::Idle,
    }
}

/// Where a worker stands as every screen shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shown {
    /// At work, waiting to be, or paused on a reply: its job isn't over.
    Working,
    Archived,
    Napping,
    /// Its job stopped and the person hasn't read how.
    Waiting,
    /// Its job stopped and the person has read how.
    Sleeping,
}

impl Shown {
    pub fn word(self) -> &'static str {
        match self {
            Shown::Working => "working",
            Shown::Archived => "archived",
            Shown::Napping => "napping",
            Shown::Waiting => "waiting",
            Shown::Sleeping => "sleeping",
        }
    }
}

impl Record {
    /// Whether the person has read a log of `len` bytes.
    pub fn is_read(&self, len: u64) -> bool {
        self.read.is_some_and(|r| r >= len)
    }

    /// Where it stands at `now` with a log of `len` bytes. A nap whose time
    /// has come is Waiting at once, read or not, before the tick tidies it.
    pub fn shown(&self, len: u64, now: u64) -> Shown {
        match self.phase {
            Phase::Queued | Phase::Running { .. } | Phase::Paused => Shown::Working,
            Phase::Archived { .. } => Shown::Archived,
            Phase::Napping { until } if until > now => Shown::Napping,
            Phase::Napping { .. } => Shown::Waiting,
            Phase::Idle if self.need.is_some() => Shown::Waiting,
            Phase::Idle if self.is_read(len) => Shown::Sleeping,
            Phase::Idle => Shown::Waiting,
        }
    }

    /// Whether a run is at work or waits to start.
    pub fn at_work(&self) -> bool {
        matches!(self.phase, Phase::Queued | Phase::Running { .. })
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |t| t.as_secs())
}

/// The record in folder `d` as it stands, for a reader: written whole by a
/// rename, so it needs no lock. A folder from before records, or whose
/// markers a build from before records has changed since, reads from its
/// markers until its next change writes the record.
pub fn read(d: &Path) -> Record {
    load(d).0
}

/// The record in folder `d`, and whether it comes from the markers and so
/// is not yet kept.
fn load(d: &Path) -> (Record, bool) {
    let markers = snapshot(d);
    let kept = fs::read(d.join(RECORD)).ok().and_then(|body| {
        serde_json::from_slice::<Record>(&body)
            .map_err(|e| eprintln!("libri: {}: {e}", d.join(RECORD).display()))
            .ok()
    });
    match kept {
        Some(r) if r.markers == markers => (r, false),
        _ => (
            Record {
                markers,
                ..from_markers(d)
            },
            true,
        ),
    }
}

/// Changes the record in folder `d` by `f` under the folder's lock, and
/// keeps it if `f` succeeds. Anything else that has to agree with the
/// record (the route, the log) is changed inside `f`. The markers are
/// written from the record, so a build from before records still reads the
/// folder rightly after a rollback.
pub fn change<T>(d: &Path, f: impl FnOnce(&mut Record) -> Result<T, String>) -> Result<T, String> {
    let _held = lock(d)?;
    let (mut r, fresh) = load(d);
    let was = r.clone();
    let answer = f(&mut r)?;
    if fresh || r != was {
        save(d, &r)?;
    }
    Ok(answer)
}

/// Applies `event` to the record in folder `d`.
pub fn apply(d: &Path, event: Event) -> Result<(), String> {
    change(d, |r| transition(r, event))
}

/// Writes the markers from `r`, then the record with their snapshot: a
/// build from before records reads what this one did, and a roll forward
/// finds the markers changed only where that build really changed them.
fn save(d: &Path, r: &Record) -> Result<(), String> {
    write_markers(d, r).map_err(|e| format!("{}: {e}", d.display()))?;
    let r = Record {
        markers: snapshot(d),
        ..r.clone()
    };
    let body = serde_json::to_vec(&r).map_err(|e| e.to_string())?;
    let p = d.join(RECORD);
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, body)
        .and_then(|()| fs::rename(&tmp, &p))
        .map_err(|e| format!("{}: {e}", p.display()))
}

/// The markers a build from before records would have left for `r`, the
/// inverse of `from_markers`. Only what differs is touched, so the times an
/// old build reads (when it was archived, when a wait began) hold. A nap has
/// no marker: an old build reads it as idle.
fn write_markers(d: &Path, r: &Record) -> std::io::Result<()> {
    let set = |m: &str, on: bool| -> std::io::Result<()> {
        let p = d.join(m);
        match (on, p.exists()) {
            (true, false) => fs::write(&p, ""),
            (false, true) => fs::remove_file(&p),
            _ => Ok(()),
        }
    };
    let at = |p: &Path, secs: u64| {
        fs::File::options()
            .write(true)
            .open(p)
            .and_then(|f| f.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(secs)))
    };
    let body = |m: &str, text: Option<String>| -> std::io::Result<()> {
        let p = d.join(m);
        match text {
            Some(t) if fs::read_to_string(&p).ok().as_deref() != Some(t.as_str()) => {
                fs::write(&p, t)
            }
            None if p.exists() => fs::remove_file(&p),
            _ => Ok(()),
        }
    };
    let (running, cancel) = match r.phase {
        Phase::Running { cancel } => (true, cancel),
        _ => (false, false),
    };
    set("running", running)?;
    set("cancel", cancel)?;
    set("go", r.phase == Phase::Queued)?;
    match r.phase {
        Phase::Archived { at: when } if !d.join("archived").exists() => {
            fs::write(d.join("archived"), "")?;
            at(&d.join("archived"), when)?;
        }
        Phase::Archived { .. } => {}
        _ => set("archived", false)?,
    }
    set("surfaced", r.surfaced)?;
    set("reported", r.reported)?;
    body("began", (r.began != 0).then(|| r.began.to_string()))?;
    // `read` held the log's time: written only while the whole log is read.
    let log = fs::metadata(d.join("log.jsonl")).ok();
    let read = match (r.read, &log) {
        (Some(len), Some(m)) if len >= m.len() => {
            modified(&d.join("log.jsonl")).map(|t| t.to_string())
        }
        _ => None,
    };
    body("read", read)?;
    let waits = d.join("awaited");
    for e in fs::read_dir(&waits).into_iter().flatten().flatten() {
        if !r.awaited.contains_key(&*e.file_name().to_string_lossy()) {
            fs::remove_file(e.path())?;
        }
    }
    for (peer, when) in &r.awaited {
        let p = waits.join(peer);
        if !p.exists() {
            fs::create_dir_all(&waits)?;
            fs::write(&p, "")?;
            at(&p, *when)?;
        }
    }
    Ok(())
}

/// The folder's lock, let go when dropped. flock holds across processes
/// (the tool servers) and between threads, each opening the file anew.
struct Held {
    _file: fs::File,
}

fn lock(d: &Path) -> Result<Held, String> {
    let p = d.join(LOCK);
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&p)
        .map_err(|e| format!("{}: {e}", p.display()))?;
    // SAFETY: flock on a descriptor this function owns, held by `Held`.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!(
            "{}: {}",
            p.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(Held { _file: f })
}

const MARKERS: [&str; 8] = [
    "go", "running", "cancel", "archived", "surfaced", "reported", "read", "began",
];

fn modified(p: &Path) -> Option<u64> {
    let t = fs::metadata(p).and_then(|m| m.modified()).ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

/// The record a folder's markers make, as `threads` used to read them. An
/// archived worker that still waits on a reply is the old bug: the reply went
/// to the main chat as unheard, so it waits on no one.
fn from_markers(d: &Path) -> Record {
    let has = |f: &str| d.join(f).exists();
    let mut awaited: BTreeMap<String, u64> = fs::read_dir(d.join("awaited"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let peer = e.file_name().to_string_lossy().into_owned();
            (peer, modified(&e.path()).unwrap_or(0))
        })
        .collect();
    let phase = if has("running") {
        Phase::Running {
            cancel: has("cancel"),
        }
    } else if has("go") {
        Phase::Queued
    } else if has("archived") {
        awaited.clear();
        Phase::Archived {
            at: modified(&d.join("archived")).unwrap_or(0),
        }
    } else if !awaited.is_empty() {
        Phase::Paused
    } else {
        Phase::Idle
    };
    // `read` held the log's time in seconds; read then means read now.
    let log = fs::metadata(d.join("log.jsonl")).ok();
    let read_at = fs::read_to_string(d.join("read"))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok());
    let read = match (read_at, &log) {
        (Some(at), Some(m)) if at >= modified(&d.join("log.jsonl")).unwrap_or(u64::MAX) => {
            Some(m.len())
        }
        _ => None,
    };
    Record {
        phase,
        awaited,
        nap_after: None,
        surfaced: has("surfaced"),
        reported: has("reported"),
        began: fs::read_to_string(d.join("began"))
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0),
        read,
        need: None,
        merged: None,
        markers: String::new(),
    }
}

/// What the folder's markers are: each present, with its time and what it
/// holds. A build from before records changes it; this one never does.
fn snapshot(d: &Path) -> String {
    let stamp = |p: &Path| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |t| t.as_nanos())
    };
    let mut out = String::new();
    for m in MARKERS {
        let p = d.join(m);
        if let Ok(body) = fs::read_to_string(&p) {
            out.push_str(&format!("{m} {} {}\n", stamp(&p), body.trim()));
        }
    }
    let mut peers: Vec<_> = fs::read_dir(d.join("awaited"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                stamp(&e.path()),
            )
        })
        .collect();
    peers.sort();
    for (peer, at) in peers {
        out.push_str(&format!("awaited/{peer} {at}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(phase: Phase) -> Record {
        Record {
            phase,
            ..Record::default()
        }
    }

    #[test]
    fn a_paused_worker_cant_be_archived_and_an_archived_one_waits_on_no_one() {
        let mut r = at(Phase::Running { cancel: false });
        transition(
            &mut r,
            Event::Awaits {
                peer: "kit".into(),
                at: 1,
            },
        )
        .unwrap();
        transition(&mut r, Event::Ended { late: false }).unwrap();
        assert_eq!(r.phase, Phase::Paused);
        assert!(transition(&mut r, Event::Archive { at: 2 }).is_err());
        assert_eq!(r.phase, Phase::Paused);
        transition(&mut r, Event::Answered { peer: "kit".into() }).unwrap();
        assert_eq!(r.phase, Phase::Idle);
        transition(&mut r, Event::Archive { at: 2 }).unwrap();
        assert!(
            transition(
                &mut r,
                Event::Awaits {
                    peer: "kit".into(),
                    at: 3
                }
            )
            .is_err()
        );
        transition(&mut r, Event::Restore).unwrap();
        assert_eq!((r.phase, r.awaited.len()), (Phase::Idle, 0));
    }

    #[test]
    fn only_a_waiting_run_is_taken_and_only_a_running_one_ends() {
        let mut r = Record::default();
        assert!(transition(&mut r, Event::Take).is_err());
        assert!(transition(&mut r, Event::Ended { late: false }).is_err());
        transition(&mut r, Event::Go { at: 5 }).unwrap();
        assert_eq!((r.phase, r.began), (Phase::Queued, 5));
        transition(&mut r, Event::Take).unwrap();
        assert!(transition(&mut r, Event::Go { at: 6 }).is_err());
        assert!(transition(&mut r, Event::Archive { at: 6 }).is_err());
        transition(&mut r, Event::StopAsked).unwrap();
        assert_eq!(r.phase, Phase::Running { cancel: true });
        transition(&mut r, Event::Resume).unwrap();
        assert_eq!(r.phase, Phase::Queued);
        transition(&mut r, Event::Take).unwrap();
        transition(&mut r, Event::Ended { late: true }).unwrap();
        assert_eq!(r.phase, Phase::Queued);
        // A paused job told more never left Working, so keeps its place.
        transition(&mut r, Event::Take).unwrap();
        transition(
            &mut r,
            Event::Awaits {
                peer: "p".into(),
                at: 1,
            },
        )
        .unwrap();
        transition(&mut r, Event::Ended { late: false }).unwrap();
        transition(&mut r, Event::Go { at: 9 }).unwrap();
        assert_eq!((r.phase, r.began), (Phase::Queued, 5));
    }

    #[test]
    fn a_nap_set_at_work_waits_for_the_job_to_stop_and_an_expired_one_is_waiting() {
        let mut r = at(Phase::Running { cancel: false });
        transition(&mut r, Event::Nap { until: 50, now: 10 }).unwrap();
        assert_eq!(r.shown(0, 10), Shown::Working);
        transition(&mut r, Event::Ended { late: false }).unwrap();
        assert_eq!(r.phase, Phase::Napping { until: 50 });
        r.read = Some(100);
        assert_eq!(r.shown(100, 10), Shown::Napping);
        assert_eq!(
            r.shown(100, 50),
            Shown::Waiting,
            "an expired nap reads as read"
        );
        // Told more, it works, then naps on.
        transition(&mut r, Event::Go { at: 1 }).unwrap();
        transition(&mut r, Event::Take).unwrap();
        transition(&mut r, Event::Ended { late: false }).unwrap();
        assert_eq!(r.phase, Phase::Napping { until: 50 });
        transition(&mut r, Event::Wake).unwrap();
        assert_eq!((r.phase, r.read), (Phase::Idle, None));
        // Woken while at work, the nap is cancelled, not taken early.
        r.phase = Phase::Queued;
        transition(&mut r, Event::Nap { until: 50, now: 10 }).unwrap();
        transition(&mut r, Event::Wake).unwrap();
        assert_eq!((r.phase, r.nap_after), (Phase::Queued, None));
        assert!(transition(&mut r, Event::Nap { until: 5, now: 10 }).is_err());
        r.phase = Phase::Napping { until: 50 };
        transition(&mut r, Event::Archive { at: 1 }).unwrap();
        assert!(transition(&mut r, Event::Nap { until: 50, now: 10 }).is_err());
    }

    #[test]
    fn a_folder_of_markers_becomes_a_record_and_keeps_its_markers() {
        let d = std::env::temp_dir().join(format!("libri-lifecycle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("awaited")).unwrap();
        fs::write(d.join("log.jsonl"), "{}\n").unwrap();
        fs::write(d.join("awaited/kit"), "").unwrap();
        fs::write(d.join("archived"), "").unwrap();
        fs::write(d.join("reported"), "").unwrap();
        let log_at = modified(&d.join("log.jsonl")).unwrap();
        fs::write(d.join("read"), log_at.to_string()).unwrap();
        let r = read(&d);
        assert!(matches!(r.phase, Phase::Archived { .. }));
        assert!(r.awaited.is_empty(), "an archived worker kept its wait");
        assert_eq!((r.reported, r.read), (true, Some(3)));
        apply(&d, Event::Restore).unwrap();
        // The markers say what the record does, for a build from before.
        assert!(
            !d.join("archived").exists(),
            "a restored worker reads archived"
        );
        assert!(
            !d.join("awaited/kit").exists(),
            "the cleared wait came back"
        );
        for m in ["reported", "read"] {
            assert!(d.join(m).exists(), "{m} gone");
        }
        // The record wins over markers it has read, and migrating again
        // (a second resume) changes nothing.
        assert_eq!(read(&d).phase, Phase::Idle);
        let kept = fs::read(d.join(RECORD)).unwrap();
        change(&d, |_| Ok(())).unwrap();
        assert_eq!(read(&d).phase, Phase::Idle);
        assert_eq!(fs::read(d.join(RECORD)).unwrap(), kept);
        // Rolled back, the old build queues it; rolled forward, the markers
        // it changed say more than the record.
        fs::write(d.join("go"), "").unwrap();
        assert_eq!(read(&d).phase, Phase::Queued);
        apply(&d, Event::Take).unwrap();
        assert!(matches!(read(&d).phase, Phase::Running { .. }));
        apply(&d, Event::Ended { late: false }).unwrap();
        // A paused one migrates paused.
        fs::remove_file(d.join(RECORD)).unwrap();
        fs::create_dir_all(d.join("awaited")).unwrap();
        fs::write(d.join("awaited/kit"), "").unwrap();
        assert_eq!(read(&d).phase, Phase::Paused);
        let _ = fs::remove_dir_all(&d);
    }

    /// Found in review, 11-02: markers left as they were made a worker archived under
    /// this build look live to a build rolled back to, and the next roll
    /// forward took that stale view for the newer one.
    #[test]
    fn a_rollback_reads_what_this_build_did() {
        let d = std::env::temp_dir().join(format!("libri-rollback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("log.jsonl"), "{}\n").unwrap();
        apply(&d, Event::Archive { at: 1_000 }).unwrap();
        // What a build from before records reads.
        assert_eq!(from_markers(&d).phase, Phase::Archived { at: 1_000 });
        // Untouched by the old build, the record still wins on return.
        let before = fs::read(d.join(RECORD)).unwrap();
        assert_eq!(read(&d).phase, Phase::Archived { at: 1_000 });
        change(&d, |_| Ok(())).unwrap();
        assert_eq!(fs::read(d.join(RECORD)).unwrap(), before);
        apply(&d, Event::Restore).unwrap();
        assert_eq!(from_markers(&d).phase, Phase::Idle);
        apply(
            &d,
            Event::Awaits {
                peer: "kit".into(),
                at: 50,
            },
        )
        .unwrap();
        let old = from_markers(&d);
        assert_eq!(
            (old.phase, old.awaited.get("kit")),
            (Phase::Paused, Some(&50))
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn hippo_reads_archived_where_the_record_says_it() {
        // hippo's code/source `archived` reads phase.state; keep the two
        // in step.
        let r = at(Phase::Archived { at: 7 });
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["phase"]["state"], "archived");
        assert_eq!(v["phase"]["at"], 7);
    }

    #[test]
    fn changes_under_the_lock_land_one_at_a_time() {
        let d = std::env::temp_dir().join(format!("libri-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let d = d.clone();
                std::thread::spawn(move || {
                    for j in 0..25 {
                        change(&d, |r| {
                            r.awaited.insert(format!("{i}-{j}"), 0);
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert_eq!(read(&d).awaited.len(), 200);
        let _ = fs::remove_dir_all(&d);
    }
}
