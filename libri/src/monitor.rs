//! Says when a turn has stopped making progress. The rule: no clock kills
//! a turn, as long real work looks like a hang to a clock. So this only tells
//! the person, once for each silence, and Stop stays theirs to press.
//!
//! The signal is the harness's own stream going quiet. Silence means nothing
//! while the turn waits on the person or on a background job, which the page
//! shows already. With no tool running, the model owes the next word, and a
//! stream dead that long is what a hang looks like from here. With a tool
//! running, silence is normal, so the wait is longer and the process tree must
//! be idle as well: a build that runs for an hour is using the CPU.

use crate::model::Step;
use std::collections::HashMap;
use std::fs;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long the model may give no word with no tool running. A notice wrongly
/// given costs one line, one missing costs the person's waiting on nothing.
const SILENCE: Duration = Duration::from_secs(120);
/// How many times longer a running tool may be quiet.
const TOOL_FACTOR: u32 = 5;
/// Of one core, the share at or below which a tool's processes count as idle:
/// an idle Node harness still ticks over.
const IDLE_SHARE: f64 = 0.1;
const TICK: Duration = Duration::from_secs(1);

/// What one turn is doing, as far as the harness has shown.
pub struct Monitor {
    began: Instant,
    /// Milliseconds after `began` that the harness last gave a word.
    heard: AtomicU64,
    /// The harness's process, 0 until it says.
    pid: AtomicU32,
    /// The person's open questions.
    asking: AtomicU64,
    /// The background jobs running.
    jobs: AtomicU64,
    /// The tools running, by call id.
    tools: Mutex<HashMap<String, String>>,
    /// What the harness has to say to the person, for the next tick.
    told: Mutex<Vec<String>>,
    ended: (Mutex<bool>, Condvar),
}

impl Monitor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            began: Instant::now(),
            heard: AtomicU64::new(0),
            pid: AtomicU32::new(0),
            asking: AtomicU64::new(0),
            jobs: AtomicU64::new(0),
            tools: Mutex::default(),
            told: Mutex::default(),
            ended: (Mutex::new(false), Condvar::new()),
        })
    }

    /// The harness gave a word.
    pub fn heard(&self) {
        let at = self.began.elapsed().as_millis() as u64;
        self.heard.store(at, Ordering::Relaxed);
    }

    /// Names the process the harness runs in.
    pub fn process(&self, pid: u32) {
        self.pid.store(pid, Ordering::Relaxed);
    }

    /// Follows one step of the turn.
    pub fn step(&self, step: &Step) {
        // A responsive statistics endpoint does not mean the model is progressing.
        if matches!(step, Step::Metrics { .. }) {
            return;
        }
        self.heard();
        let mut tools = self.tools.lock().unwrap();
        match step {
            Step::Call { id, name, .. } | Step::Agent { id, name, .. } => {
                tools.insert(id.clone(), name.clone());
            }
            Step::Result { id, .. } | Step::Done { id, .. } => {
                tools.remove(id);
            }
            Step::Jobs { names } => self.jobs.store(names.len() as u64, Ordering::Relaxed),
            _ => {}
        }
    }

    /// A question went to the person, or came back.
    pub fn asking(&self, open: bool) {
        if open {
            self.asking.fetch_add(1, Ordering::Relaxed);
        } else {
            self.asking.fetch_sub(1, Ordering::Relaxed);
            // The time they took was not the model's silence.
            self.heard();
        }
    }

    /// Passes a word to the person: the harness's thread has no door.
    pub fn tell(&self, text: String) {
        self.told.lock().unwrap().push(text);
    }

    /// Watches until `finish`, giving `notify` each silence once.
    pub fn watch(self: &Arc<Self>, notify: impl Fn(String) + Send + 'static) {
        let me = self.clone();
        let limit = silence();
        thread::spawn(move || me.run(limit, notify));
    }

    /// The turn is over.
    pub fn finish(&self) {
        *self.ended.0.lock().unwrap() = true;
        self.ended.1.notify_all();
    }

    fn run(&self, limit: Duration, notify: impl Fn(String)) {
        // The silence last told of, by when it began.
        let mut told = None;
        let mut sample: Option<(Instant, u64)> = None;
        loop {
            let ended = self.ended.0.lock().unwrap();
            let (ended, _) = self.ended.1.wait_timeout(ended, TICK).unwrap();
            // Ahead of the end: a short turn is over within the tick.
            self.told.lock().unwrap().drain(..).for_each(&notify);
            if *ended {
                return;
            }
            drop(ended);
            let heard = self.heard.load(Ordering::Relaxed);
            let silent = self
                .began
                .elapsed()
                .saturating_sub(Duration::from_millis(heard));
            let waiting =
                self.asking.load(Ordering::Relaxed) > 0 || self.jobs.load(Ordering::Relaxed) > 0;
            let tool = self.tools.lock().unwrap().values().next().cloned();
            let due = tool.as_ref().map_or(limit, |_| limit * TOOL_FACTOR);
            if waiting || told == Some(heard) || silent < due {
                sample = None;
                continue;
            }
            let Some(tool) = tool else {
                told = Some(heard);
                notify(format!(
                    "(No word from the model for {}. It may be stuck. Stop ends \
                     the turn; what you sent is kept.)",
                    spoken(silent)
                ));
                continue;
            };
            // Sampled at the window's two ends only: a sample reads every
            // process on the host, and a quiet build can run for an hour.
            let Some((since, before)) = sample else {
                sample = Some((
                    Instant::now(),
                    process_ticks(self.pid.load(Ordering::Relaxed)),
                ));
                continue;
            };
            let window = since.elapsed();
            if window < due / 10 {
                continue;
            }
            let now = process_ticks(self.pid.load(Ordering::Relaxed));
            sample = None;
            let used = now.saturating_sub(before) as f64 / ticks_per_second();
            if used <= window.as_secs_f64() * IDLE_SHARE {
                told = Some(heard);
                notify(format!(
                    "({tool} has run for {} with no output and no CPU use. It may \
                     be stuck. Stop ends the turn; what you sent is kept.)",
                    spoken(silent)
                ));
            }
        }
    }
}

/// How long the model may be silent; LIBRI_SILENCE_SECS for a check that
/// can't wait minutes.
fn silence() -> Duration {
    std::env::var("LIBRI_SILENCE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map_or(SILENCE, Duration::from_secs)
}

fn spoken(d: Duration) -> String {
    match d.as_secs() {
        s if s < 90 => format!("{s} seconds"),
        s => format!("{} minutes", (s + 30) / 60),
    }
}

fn ticks_per_second() -> f64 {
    // SAFETY: sysconf only reads a limit.
    unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f64 }
}

/// The parent and the CPU ticks of a /proc/<pid>/stat line, counting the
/// children it has reaped.
fn parse_stat(line: &str) -> Option<(u32, u64)> {
    // The name, in parentheses, may hold spaces and parentheses of its own.
    let rest = &line[line.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let ticks = fields.get(11..15)?.iter().map(|f| f.parse::<u64>().ok());
    Some((fields.get(1)?.parse().ok()?, ticks.sum::<Option<u64>>()?))
}

/// The CPU ticks of `root` and all below it: the tools it started may leave
/// its process group, and they stay its children.
fn process_ticks(root: u32) -> u64 {
    let Ok(entries) = fs::read_dir("/proc") else {
        return 0;
    };
    let all: Vec<(u32, u32, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let pid = e.file_name().to_str()?.parse().ok()?;
            let (parent, ticks) = parse_stat(&fs::read_to_string(e.path().join("stat")).ok()?)?;
            Some((pid, parent, ticks))
        })
        .collect();
    let mut tree = vec![root];
    let mut total = 0;
    while let Some(parent) = tree.pop() {
        for (pid, up, ticks) in &all {
            if *pid == parent {
                total += ticks;
            }
            if *up == parent {
                tree.push(*pid);
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;

    #[test]
    fn reads_a_stat_line_whatever_the_name() {
        let tail = " S 7 1 1 0 -1 0 0 0 0 0 11 22 33 44 20 0 1 0 5 0 0";
        assert_eq!(parse_stat(&format!("9 (a) b) c){tail}")), Some((7, 110)));
        assert_eq!(parse_stat("9 (gone) S 7"), None);
    }

    #[test]
    fn what_the_harness_said_reaches_the_person_when_the_turn_ends_at_once() {
        let m = Monitor::new();
        let (tx, rx) = mpsc::channel();
        m.tell("the tools are down".into());
        m.finish();
        m.run(Duration::from_secs(60), move |text| tx.send(text).unwrap());
        assert_eq!(rx.try_recv().unwrap(), "the tools are down");
    }

    fn tool(id: &str) -> Step {
        Step::Call {
            id: id.into(),
            name: "Bash".into(),
            input: json!({}),
        }
    }

    fn watched(m: &Arc<Monitor>) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel();
        let me = m.clone();
        thread::spawn(move || {
            me.run(Duration::from_millis(1500), move |t| {
                tx.send(t).unwrap();
            })
        });
        rx
    }

    const LONG: Duration = Duration::from_secs(4);

    #[test]
    fn tells_of_a_silence_once() {
        let m = Monitor::new();
        let told = watched(&m);
        assert!(told.recv_timeout(LONG).unwrap().contains("No word"));
        assert!(told.recv_timeout(Duration::from_millis(2500)).is_err());
        m.finish();
    }

    #[test]
    fn a_question_or_a_job_is_not_a_silence() {
        for hold in [true, false] {
            let m = Monitor::new();
            if hold {
                m.asking(true);
            } else {
                m.step(&Step::Jobs {
                    names: vec!["a build".into()],
                });
            }
            let told = watched(&m);
            assert!(told.recv_timeout(Duration::from_millis(2500)).is_err());
            m.finish();
        }
    }

    #[test]
    fn a_tool_gets_longer_and_its_end_clears_it() {
        let m = Monitor::new();
        m.step(&tool("t1"));
        let told = watched(&m);
        assert!(told.recv_timeout(Duration::from_millis(2500)).is_err());
        m.step(&Step::Result {
            id: "t1".into(),
            text: String::new(),
            error: false,
        });
        assert!(told.recv_timeout(LONG).unwrap().contains("No word"));
        m.finish();
    }
}
