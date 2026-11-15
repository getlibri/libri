//! What every model call used, and who asked for it, kept by Libri itself:
//! one line per model per call, in `usage/YYYY-MM.jsonl` under the state
//! directory, with the tokens the provider reported. Every harness Libri
//! makes is wrapped once (`model::harness`), so each turn of the main chat,
//! a worker, Temp, the memory's summarizer and the workers' cards lands here
//! whichever harness runs it; calls Libri makes to an API itself (Grok)
//! write their own line. Nothing is read from other apps' transcripts.
//!
//! Lines are appended whole, so the tools process and the daemon can both
//! write. What a line costs is worked out when it is read, from
//! `prices.json`, so a corrected price applies to the past too.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

/// Who asked for a call.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(rename_all = "kebab-case")]
pub enum By {
    /// A call nothing named: counted, never dropped.
    #[default]
    Other,
    /// The main chat, answering the person or a worker's report.
    Main,
    /// The main chat, on a turn a watch or a routine started.
    Watch,
    /// The main chat, answering another agent over A2A.
    Message,
    Temp,
    Worker,
    /// hippo's summarizer, compacting the chat's memory or a worker's.
    Memory,
    /// The title and status line on a worker's card.
    Card,
    /// Settings' check that a harness answers.
    Check,
    /// Grok's X search.
    Search,
    /// Pictures from Grok Imagine.
    Image,
    /// Text read aloud.
    Speech,
}

impl By {
    pub fn name(self) -> &'static str {
        match self {
            By::Other => "Other",
            By::Main => "Main chat",
            By::Watch => "Watches",
            By::Message => "Messages",
            By::Temp => "Temp",
            By::Worker => "Workers",
            By::Memory => "Memory",
            By::Card => "Worker cards",
            By::Check => "Checks",
            By::Search => "X search",
            By::Image => "Pictures",
            By::Speech => "Speech",
        }
    }
}

/// Who asked, and for which worker when it was for one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Caller {
    pub by: By,
    pub worker: Option<String>,
}

impl Caller {
    pub fn by(by: By) -> Self {
        Caller { by, worker: None }
    }

    pub fn worker(by: By, name: &str) -> Self {
        Caller {
            by,
            worker: Some(name.to_owned()),
        }
    }
}

/// One model's share of one call.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Unix seconds.
    pub at: i64,
    pub by: By,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    pub harness: String,
    /// Who served it, by the harness's id ("anthropic", "opencode-go").
    pub provider: String,
    pub model: String,
    /// Input sent at the full rate: the cache's shares are apart.
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    /// "subscription", "key" or "local"; worked out from the harness and
    /// provider when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid: Option<String>,
}

fn dir(state: &Path) -> PathBuf {
    state.join("usage")
}

/// The state directory of this process, where the daemon and its tools
/// both keep it.
pub fn state() -> Option<PathBuf> {
    std::env::var_os("STATE_DIRECTORY").map(PathBuf::from)
}

/// Appends `e` to its month's file. A failure is told, not raised: a
/// call's answer is worth more than its record.
pub fn record(state: &Path, e: &Entry) {
    if e.input + e.cache_read + e.cache_write + e.output == 0 {
        return;
    }
    let at = Local
        .timestamp_opt(e.at, 0)
        .single()
        .unwrap_or_else(Local::now);
    let file = dir(state).join(format!("{}.jsonl", at.format("%Y-%m")));
    let mut line = match serde_json::to_string(e) {
        Ok(l) => l,
        Err(err) => return eprintln!("libri: usage ledger: {err}"),
    };
    line.push('\n');
    // One write of a whole line on an O_APPEND file: lines from two
    // processes never interleave.
    let written = fs::create_dir_all(dir(state)).and_then(|()| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)?
            .write_all(line.as_bytes())
    });
    if let Err(err) = written {
        eprintln!("libri: usage ledger {}: {err}", file.display());
    }
}

/// Records one harness's token report for `caller`. `input` counts every
/// token sent, the cache's shares among them, as `Step::Usage` does.
pub fn harness_used(
    caller: &Caller,
    harness: &str,
    model: &str,
    (input, cache_read, cache_write, output): (u64, u64, u64, u64),
) {
    let Some(state) = state() else { return };
    let (provider, model) = provider_of(harness, model);
    record(
        &state,
        &Entry {
            at: Local::now().timestamp(),
            by: caller.by,
            worker: caller.worker.clone(),
            harness: harness.into(),
            provider,
            model,
            input: input.saturating_sub(cache_read + cache_write),
            cache_read,
            cache_write,
            output,
            paid: None,
        },
    );
}

/// The provider a harness's model id names, and the model without it:
/// "opencode-go/deepseek-v4.1-flash" on OpenCode is DeepSeek served by
/// OpenCode Go. Claude Code and Codex serve their own makers' models.
fn provider_of(harness: &str, model: &str) -> (String, String) {
    match harness {
        "claude" => ("anthropic".into(), model.into()),
        "codex" => ("openai".into(), model.into()),
        "muse" => ("meta".into(), model.into()),
        _ => match model.split_once('/') {
            Some((p, m)) => (p.into(), m.into()),
            None => (harness.into(), model.into()),
        },
    }
}

/// How a line was paid for.
pub fn paid(e: &Entry) -> &str {
    if let Some(p) = &e.paid {
        return p;
    }
    if matches!(
        e.provider.as_str(),
        "local" | "llama" | "llama-swap" | "ollama"
    ) {
        "local"
    } else if crate::usage::account(&e.harness, &e.provider).is_some() {
        "subscription"
    } else {
        "key"
    }
}

#[derive(Deserialize)]
struct Prices {
    checked: String,
    models: Vec<Price>,
    companies: Vec<Maker>,
    names: BTreeMap<String, String>,
}

/// List prices in USD per million tokens.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Price {
    #[serde(rename = "match")]
    pub matches: String,
    pub company: String,
    pub input: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub output: f64,
    pub source: String,
}

#[derive(Deserialize)]
struct Maker {
    #[serde(rename = "match")]
    matches: String,
    company: String,
}

static PRICES: LazyLock<Prices> = LazyLock::new(|| {
    serde_json::from_str(include_str!("prices.json")).expect("prices.json is well formed")
});

pub fn price(model: &str) -> Option<&'static Price> {
    let m = model.to_ascii_lowercase();
    PRICES.models.iter().find(|p| m.contains(&p.matches))
}

/// The company that made `model`, by id ("anthropic").
pub fn company(model: &str) -> String {
    let m = model.to_ascii_lowercase();
    price(&m)
        .map(|p| p.company.clone())
        .or_else(|| {
            PRICES
                .companies
                .iter()
                .find(|c| m.contains(&c.matches))
                .map(|c| c.company.clone())
        })
        .unwrap_or_else(|| "other".into())
}

pub fn company_name(id: &str) -> String {
    PRICES.names.get(id).cloned().unwrap_or_else(|| id.into())
}

/// What the line's tokens cost at list API prices; None for a model with
/// no price, Some(0) for a local one.
pub fn cost(e: &Entry) -> Option<f64> {
    if paid(e) == "local" {
        return Some(0.0);
    }
    let p = price(&e.model)?;
    Some(
        (e.input as f64 * p.input
            + e.cache_read as f64 * p.cache_read
            + e.cache_write as f64 * p.cache_write
            + e.output as f64 * p.output)
            / 1e6,
    )
}

/// Every line from `since` on, oldest first.
pub fn read(state: &Path, since: DateTime<Local>) -> Vec<Entry> {
    let mut months = Vec::new();
    let mut m = NaiveDate::from_ymd_opt(since.year(), since.month(), 1).unwrap();
    let today = Local::now().date_naive();
    while m <= today {
        months.push(m);
        m = m.checked_add_months(chrono::Months::new(1)).unwrap();
    }
    let from = since.timestamp();
    let mut out = Vec::new();
    for m in months {
        let file = dir(state).join(format!("{}.jsonl", m.format("%Y-%m")));
        let Ok(f) = fs::File::open(&file) else {
            continue;
        };
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            // A torn last line from a crash is skipped, not fatal.
            if let Ok(e) = serde_json::from_str::<Entry>(&line)
                && e.at >= from
            {
                out.push(e);
            }
        }
    }
    out
}

/// The spans Settings offers.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Span {
    Day,
    Week,
    Month,
}

impl Span {
    /// Today from midnight; the last 7 days; the last 30 days.
    pub fn since(self, now: DateTime<Local>) -> DateTime<Local> {
        let midnight = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|t| Local.from_local_datetime(&t).earliest())
            .unwrap_or(now);
        match self {
            Span::Day => midnight,
            Span::Week => midnight - Duration::days(6),
            Span::Month => midnight - Duration::days(29),
        }
    }
}

/// A group's sums.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Sum {
    pub key: String,
    pub name: String,
    pub calls: u64,
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    /// At list API prices, the priced lines only.
    pub cost: f64,
    /// Lines whose model has no price, left out of `cost`.
    pub unpriced: u64,
}

impl Sum {
    fn add(&mut self, e: &Entry) {
        self.calls += 1;
        self.input += e.input;
        self.cache_read += e.cache_read;
        self.cache_write += e.cache_write;
        self.output += e.output;
        match cost(e) {
            Some(c) => self.cost += c,
            None => self.unpriced += 1,
        }
    }
}

/// One day's cost, for the strip of bars.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Day {
    pub day: String,
    pub cost: f64,
}

/// Spend over a span, every way it is cut.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Spend {
    pub since: i64,
    pub total: Sum,
    pub by_caller: Vec<Sum>,
    pub by_model: Vec<Sum>,
    pub by_company: Vec<Sum>,
    pub by_harness: Vec<Sum>,
    pub by_paid: Vec<Sum>,
    pub days: Vec<Day>,
    pub prices_checked: String,
    /// The table every cost here comes from, with each price's source.
    pub prices: Vec<Price>,
}

fn groups(entries: &[Entry], key: impl Fn(&Entry) -> (String, String)) -> Vec<Sum> {
    let mut m: BTreeMap<String, Sum> = BTreeMap::new();
    for e in entries {
        let (k, name) = key(e);
        m.entry(k.clone())
            .or_insert_with(|| Sum {
                key: k,
                name,
                ..Sum::default()
            })
            .add(e);
    }
    let mut v: Vec<Sum> = m.into_values().collect();
    // Dearest first; among the unpriced, the most used.
    v.sort_by(|a, b| {
        b.cost
            .total_cmp(&a.cost)
            .then((b.input + b.output).cmp(&(a.input + a.output)))
    });
    v
}

fn harness_name(h: &str) -> String {
    match h {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        "omp" => "omp",
        "pi" => "Pi",
        "muse" => "Muse",
        "grok" => "Grok",
        "xai" => "xAI API",
        other => other,
    }
    .into()
}

/// Cuts `entries` (already within the span from `since`) every way.
pub fn spend(entries: &[Entry], since: DateTime<Local>) -> Spend {
    let mut total = Sum {
        key: "total".into(),
        name: "Total".into(),
        ..Sum::default()
    };
    for e in entries {
        total.add(e);
    }
    // A card's worker stays out of its key: all cards are one row.
    let caller = |e: &Entry| match (&e.worker, e.by) {
        (Some(w), By::Worker) => (format!("worker:{w}"), w.clone()),
        (Some(w), By::Memory) => (format!("memory:{w}"), format!("Memory · {w}")),
        (_, by) => (
            serde_json::to_value(by)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            by.name().into(),
        ),
    };
    let mut days: BTreeMap<String, f64> = BTreeMap::new();
    let mut d = since.date_naive();
    let today = Local::now().date_naive();
    while d <= today {
        days.insert(d.format("%Y-%m-%d").to_string(), 0.0);
        d += Duration::days(1);
    }
    for e in entries {
        if let Some(t) = Local.timestamp_opt(e.at, 0).single() {
            *days.entry(t.format("%Y-%m-%d").to_string()).or_default() += cost(e).unwrap_or(0.0);
        }
    }
    Spend {
        since: since.timestamp(),
        total,
        by_caller: groups(entries, caller),
        by_model: groups(entries, |e| (e.model.clone(), e.model.clone())),
        by_company: groups(entries, |e| {
            let c = company(&e.model);
            (c.clone(), company_name(&c))
        }),
        by_harness: groups(entries, |e| (e.harness.clone(), harness_name(&e.harness))),
        by_paid: groups(entries, |e| {
            let p = paid(e).to_owned();
            let name = match p.as_str() {
                "subscription" => "Subscriptions",
                "key" => "Paid by key",
                "local" => "Local",
                _ => "Other",
            };
            (p, name.into())
        }),
        days: days
            .into_iter()
            .map(|(day, cost)| Day { day, cost })
            .collect(),
        prices_checked: PRICES.checked.clone(),
        prices: PRICES.models.clone(),
    }
}

/// The ledger over `span`, as the person reads it.
pub fn report(state: &Path, span: Span) -> Spend {
    let since = span.since(Local::now());
    spend(&read(state, since), since)
}

/// The spend over `span` in a few lines, for the usage tool.
pub fn text(s: &Spend, span: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Use this {span}: ${:.2} at API prices over {} calls ({} tokens in, {} out){}.",
        s.total.cost,
        s.total.calls,
        tokens(s.total.input + s.total.cache_read + s.total.cache_write),
        tokens(s.total.output),
        if s.total.unpriced > 0 {
            format!(
                ", {} calls on models with no price left out",
                s.total.unpriced
            )
        } else {
            String::new()
        }
    );
    for (title, rows) in [
        ("By caller", &s.by_caller),
        ("By model", &s.by_model),
        ("By company", &s.by_company),
        ("By harness", &s.by_harness),
        ("By payment", &s.by_paid),
    ] {
        let line: Vec<String> = rows
            .iter()
            .take(8)
            .map(|r| format!("{} ${:.2}", r.name, r.cost))
            .collect();
        if !line.is_empty() {
            let _ = writeln!(out, "{title}: {}", line.join(" · "));
        }
    }
    let _ = writeln!(out, "Prices checked {}.", s.prices_checked);
    out
}

/// "1.2M", "34k", "512".
pub fn tokens(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{}k", n / 1000),
        n => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(by: By, worker: Option<&str>, harness: &str, provider: &str, model: &str) -> Entry {
        Entry {
            at: Local::now().timestamp(),
            by,
            worker: worker.map(str::to_owned),
            harness: harness.into(),
            provider: provider.into(),
            model: model.into(),
            input: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 0,
            output: 100_000,
            paid: None,
        }
    }

    #[test]
    fn a_line_costs_its_tokens_at_list_price() {
        let e = entry(By::Main, None, "claude", "anthropic", "claude-opus-5-5");
        // 1M in at $4, 1M cache reads at $0.20, 0.1M out at $20.
        assert!((cost(&e).unwrap() - 6.2).abs() < 1e-9);
        assert_eq!(paid(&e), "subscription");
        let local = entry(By::Main, None, "omp", "local", "arete-27b");
        assert_eq!(cost(&local), Some(0.0));
        assert_eq!(paid(&local), "local");
        let odd = entry(By::Main, None, "omp", "somewhere", "mystery-1");
        assert_eq!(cost(&odd), None);
        assert_eq!(paid(&odd), "key");
    }

    #[test]
    fn models_find_their_maker_and_server() {
        assert_eq!(company("claude-haiku-5-5"), "anthropic");
        assert_eq!(company("gpt-6.1-sol"), "openai");
        assert_eq!(company("deepseek-v4.1-flash"), "deepseek");
        assert_eq!(company("qwen3.8-27b"), "alibaba");
        assert_eq!(company("zzz"), "other");
        assert_eq!(
            provider_of("opencode", "opencode-go/deepseek-v4.1-flash"),
            ("opencode-go".into(), "deepseek-v4.1-flash".into())
        );
        assert_eq!(provider_of("claude", "claude-opus-5-5").0, "anthropic");
    }

    #[test]
    fn lines_survive_the_file_and_group_by_who_asked() {
        let d = std::env::temp_dir().join(format!("ledger-lines-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let state = d.as_path();
        record(
            state,
            &entry(By::Main, None, "claude", "anthropic", "claude-opus-5-5"),
        );
        record(
            state,
            &entry(
                By::Worker,
                Some("Ruth"),
                "claude",
                "anthropic",
                "claude-opus-5-5",
            ),
        );
        record(
            state,
            &entry(By::Memory, None, "claude", "anthropic", "claude-haiku-5-5"),
        );
        record(
            state,
            &entry(
                By::Worker,
                Some("Ruth"),
                "omp",
                "opencode-go",
                "deepseek-v4.1-flash",
            ),
        );
        // Nothing used is nothing written.
        record(state, &Entry::default());
        let s = report(state, Span::Day);
        assert_eq!(s.total.calls, 4);
        let ruth = s.by_caller.iter().find(|r| r.key == "worker:Ruth").unwrap();
        assert_eq!((ruth.name.as_str(), ruth.calls), ("Ruth", 2));
        assert!(
            s.by_caller
                .iter()
                .any(|r| r.key == "memory" && r.name == "Memory")
        );
        let companies: Vec<_> = s.by_company.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(companies, ["Anthropic", "DeepSeek"]);
        assert_eq!(s.by_paid[0].key, "subscription");
        assert_eq!(s.days.len(), 1);
        assert!((s.days[0].cost - s.total.cost).abs() < 1e-9);
    }

    #[test]
    fn every_price_names_a_known_company() {
        for p in &PRICES.models {
            assert!(PRICES.names.contains_key(&p.company), "{}", p.matches);
        }
    }
}
