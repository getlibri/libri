//! `libri harness-check`: runs one fixed exchange through the harness the
//! environment names (LIBRI_BACKEND and its settings) and reports which
//! of the events the page shows it carries. Two turns: one that thinks,
//! runs a shell command the harness must ask leave for, and answers; one
//! stopped as soon as it starts writing. Both run as a subagent's do, so
//! the harness saves no session for them.

use crate::model::{self, Ask, Asker, Run, Step};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SYSTEM: &str = "You are being tested by the program that drives you. Do exactly what the message asks, briefly.";
const WORK: &str = "Think briefly first. Then run this shell command with your shell tool: touch libri-check.tmp && echo libri-check\nThen reply with its output and nothing else.";
const LONG: &str = "Write the numbers from one to two hundred in words, one per line.";

/// What a harness was seen to carry.
#[derive(Default)]
struct Seen {
    text: bool,
    thinking: bool,
    calls: bool,
    results: bool,
    subagents: bool,
    asks: Vec<String>,
    /// A call ran after its ask was answered: the turn went on.
    resumed: bool,
    usage: Option<(u64, u64)>,
    answer: Option<Result<String, String>>,
    /// The second turn ended on the stop, and how soon.
    cancel: Option<Result<Duration, String>>,
}

struct Allow(Arc<Mutex<Seen>>);

impl Asker for Allow {
    fn put(&self, ask: Ask) -> Option<Value> {
        self.0.lock().unwrap().asks.push(ask.tool.clone());
        Some(ask.input)
    }
    fn withdraw(&self, _: &str) {}
}

pub fn run() -> Result<(), String> {
    let backend = crate::catalog::configured_default();
    let m = model::from_env()?;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let s = seen.clone();
    let steps: model::Steps = Arc::new(move |step| {
        let mut s = s.lock().unwrap();
        match step {
            Step::Text { .. } => s.text = true,
            Step::Thinking { .. } => s.thinking = true,
            Step::Call { .. } => s.calls = true,
            Step::Result { .. } => {
                s.results = true;
                s.resumed |= !s.asks.is_empty();
            }
            Step::Agent { .. } | Step::Inner { .. } | Step::Done { .. } => s.subagents = true,
            Step::Row { .. }
            | Step::Stored { .. }
            | Step::Structured { .. }
            | Step::Progress { .. }
            | Step::Metrics { .. }
            | Step::Runs { .. } => {}
            Step::Usage { input, output, .. } => s.usage = Some((input, output)),
            Step::Answer { .. } | Step::Jobs { .. } => {}
        }
    });
    let run = Run {
        by: crate::ledger::Caller::by(crate::ledger::By::Check),
        acts: true,
        sub: Some("harness-check".into()),
        steps: Some(steps.clone()),
        ask: Some(Arc::new(Allow(seen.clone()))),
        ..Run::default()
    };
    let answer = m.answer(SYSTEM, "", WORK, &[], run);
    seen.lock().unwrap().answer = Some(answer);

    // The stop: set once the second turn first writes or thinks.
    let stop = Arc::new(AtomicBool::new(false));
    let (flag, started) = (stop.clone(), Instant::now());
    let first: Arc<Mutex<Option<Instant>>> = Arc::default();
    let at = first.clone();
    let steps: model::Steps = Arc::new(move |step| {
        if matches!(step, Step::Text { .. } | Step::Thinking { .. }) {
            at.lock().unwrap().get_or_insert_with(Instant::now);
            flag.store(true, Ordering::Relaxed);
        }
    });
    let run = Run {
        by: crate::ledger::Caller::by(crate::ledger::By::Check),
        acts: true,
        sub: Some("harness-check".into()),
        stop: Some(stop),
        steps: Some(steps),
        ..Run::default()
    };
    let r = m.answer(SYSTEM, "", LONG, &[], run);
    let since = first.lock().unwrap().unwrap_or(started);
    seen.lock().unwrap().cancel = Some(match r {
        Err(e) if e == "stopped" => Ok(since.elapsed()),
        Err(e) => Err(e),
        Ok(t) => Err(format!("answered {} chars, never stopped", t.len())),
    });

    let s = seen.lock().unwrap();
    let yes = |b: bool| if b { "yes" } else { "no" };
    println!(
        "harness      {backend} {}",
        std::env::var("LIBRI_ACP").unwrap_or_default()
    );
    println!("text         {}", yes(s.text));
    println!("thinking     {}", yes(s.thinking));
    println!("tool calls   {}", yes(s.calls));
    println!("results      {}", yes(s.results));
    println!("subagents    {} (the prompt starts none)", yes(s.subagents));
    println!(
        "approvals    {}",
        if s.asks.is_empty() {
            "no".into()
        } else {
            format!("yes: {}", s.asks.join(", "))
        }
    );
    println!("resume       {}", yes(s.resumed));
    match &s.cancel {
        Some(Ok(d)) => println!(
            "cancel       yes, {:.1}s after the first words",
            d.as_secs_f64()
        ),
        Some(Err(e)) => println!("cancel       no: {e}"),
        None => println!("cancel       not tried"),
    }
    match s.usage {
        Some((i, o)) => println!("usage        yes: {i} in, {o} out"),
        None => println!("usage        no"),
    }
    match &s.answer {
        Some(Ok(a)) => println!("answer       {a:?}"),
        Some(Err(e)) => println!("answer       failed: {e}"),
        None => {}
    }
    Ok(())
}
