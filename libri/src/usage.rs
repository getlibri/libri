//! How full each subscription is, read by Libri itself from each provider
//! with the sign-in its harness already keeps: the composer's rings,
//! Settings' Usage page and the `usage` tool all read this.
//!
//! - Claude: Claude Code's usage endpoint (`/api/oauth/usage`, the one its
//!   `/usage` reads) with Claude Code's own login, read only. Libri never
//!   renews that login: a renewal rotates the refresh token, which signs
//!   Claude Code out. A lapsed one waits for Claude Code's next turn.
//! - ChatGPT: `codex app-server`'s `account/rateLimits/read`, Codex's own
//!   protocol, so Codex keeps its login fresh itself. It starts no thread, so
//!   it leaves no session on disk.
//! - OpenCode Go: Go's usage endpoint with the key OpenCode keeps.
//!
//! The endpoints but Codex's are undocumented; expect them to change. Each
//! account is asked at most every FRESH, and a failed read keeps the last
//! good one, marked as of when it was read.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use serde::Serialize;
use serde_json::{Value, json};

use crate::model::Kept;

/// Whether the composers show the rings; any value but "off" shows them.
pub const SHOWN: Kept = Kept("usage-shown");

pub fn shown(state: &Path) -> bool {
    SHOWN.read(state).as_deref() != Some("off")
}

pub fn show(state: &Path, on: bool) -> Result<(), String> {
    if on {
        SHOWN.clear(state);
        Ok(())
    } else {
        SHOWN.write(state, "off")
    }
}

/// The account in the usage report whose limits a model draws on: the
/// subscription its harness signs in with. None for one paid by key, which
/// has no window to fill.
pub fn account(harness: &str, provider: &str) -> Option<&'static str> {
    match (harness, provider) {
        ("claude", _) => Some("claude"),
        ("codex", _) => Some("codex"),
        (_, "opencode-go") => Some("go"),
        _ => None,
    }
}

/// One subscription: its windows, shortest first, and whatever else the
/// provider says of it.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Account {
    pub account: String,
    pub name: String,
    /// The plan, as the provider names it ("Max 20x", "Pro").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub windows: Vec<Window>,
    pub notes: Vec<String>,
    /// When the provider was last read, Unix seconds.
    pub read_at: i64,
    /// The last read failed: these windows are from `read_at`.
    pub stale: bool,
}

/// One limit window: how much of it is used and when it starts over.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Window {
    /// Its id ("5h", "week", "week-fable").
    pub window: String,
    pub name: String,
    /// Its length, by which the rings order it and the pace mark is placed.
    pub hours: Option<f64>,
    pub pct: u32,
    /// As people read it: "Sun Nov 8 17:00".
    pub resets: String,
    /// Unix seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    /// The one model a window counts, when it counts one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// The whole report: every account, and the lines that belong to none.
#[derive(Clone, Serialize, Debug, PartialEq, Default)]
pub struct Report {
    pub accounts: Vec<Account>,
    pub notes: Vec<String>,
}

/// One window as clients up to APK 0.9.17 read `GET /api/usage`: a flat
/// list, every account's windows in turn. Remove once no phone runs 0.9.17
/// or older; newer clients read the whole report from `/api/usage/report`.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Limit {
    pub account: String,
    pub window: String,
    pub pct: u32,
    pub resets: String,
}

pub fn limits(r: &Report) -> Vec<Limit> {
    r.accounts
        .iter()
        .flat_map(|a| {
            a.windows.iter().map(|w| Limit {
                account: a.account.clone(),
                window: w.window.clone(),
                pct: w.pct,
                resets: w.resets.clone(),
            })
        })
        .collect()
}

/// How long one reading of an account holds: T3 Code's five minutes, which
/// keeps well under the providers' own rate limits.
const FRESH: Duration = Duration::from_secs(300);
/// How long one provider may take to answer.
const TIMEOUT: Duration = Duration::from_secs(20);

/// The last reading of each account, and when it was taken.
static READINGS: Mutex<Vec<(Instant, Account)>> = Mutex::new(Vec::new());

/// The subscriptions this host can read, each asked afresh when its reading
/// is older than FRESH, all at once.
pub fn report() -> Report {
    let sources = sources();
    let due: Vec<Source> = {
        let kept = READINGS.lock().unwrap();
        sources
            .iter()
            .copied()
            .filter(|s| {
                !kept
                    .iter()
                    .any(|(at, a)| a.account == s.id() && at.elapsed() < FRESH)
            })
            .collect()
    };
    let asked: Vec<_> = due
        .into_iter()
        .map(|s| (s, thread::spawn(move || s.read())))
        .collect();
    let mut kept = READINGS.lock().unwrap();
    for (s, t) in asked {
        let got = t
            .join()
            .unwrap_or_else(|_| Err("the reader panicked".into()));
        let now = Instant::now();
        let fresh = match got {
            Ok(mut a) => {
                sort(&mut a);
                a
            }
            Err(e) => {
                eprintln!("libri: usage {}: {e}", s.id());
                match kept.iter().find(|(_, a)| a.account == s.id()) {
                    // The last good windows, marked old; asked again at FRESH.
                    Some((_, last)) if !last.windows.is_empty() => Account {
                        stale: true,
                        notes: vec![format!("Couldn't read just now: {e}")],
                        ..last.clone()
                    },
                    _ => Account {
                        account: s.id().into(),
                        name: s.name().into(),
                        plan: None,
                        windows: Vec::new(),
                        notes: vec![e],
                        read_at: Local::now().timestamp(),
                        stale: false,
                    },
                }
            }
        };
        kept.retain(|(_, a)| a.account != s.id());
        kept.push((now, fresh));
    }
    Report {
        accounts: sources
            .iter()
            .filter_map(|s| kept.iter().find(|(_, a)| a.account == s.id()))
            .map(|(_, a)| a.clone())
            .collect(),
        notes: Vec::new(),
    }
}

/// The report in a line per window, for the usage tool.
pub fn text(r: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if r.accounts.is_empty() {
        out.push_str("No subscription signed in here has limits to read.\n");
    }
    for a in &r.accounts {
        let plan = a
            .plan
            .as_deref()
            .map(|p| format!(" ({p})"))
            .unwrap_or_default();
        let _ = writeln!(out, "{}{plan}:", a.name);
        for w in &a.windows {
            let _ = writeln!(out, "  {} {}% used, resets {}", w.name, w.pct, w.resets);
        }
        for n in &a.notes {
            let _ = writeln!(out, "  {n}");
        }
    }
    out
}

/// A provider Libri can read limits from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    Claude,
    Codex,
    Go,
}

impl Source {
    fn id(self) -> &'static str {
        match self {
            Source::Claude => "claude",
            Source::Codex => "codex",
            Source::Go => "go",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Source::Claude => "Claude",
            Source::Codex => "ChatGPT",
            Source::Go => "OpenCode Go",
        }
    }

    fn read(self) -> Result<Account, String> {
        match self {
            Source::Claude => claude(),
            Source::Codex => codex(),
            Source::Go => go(),
        }
    }
}

/// The providers whose sign-in a harness here keeps.
fn sources() -> Vec<Source> {
    let harnesses = crate::catalog::harnesses();
    let has = |h: &str| harnesses.iter().any(|x| x == h);
    let mut out = Vec::new();
    if has("claude") && claude_login().is_file() {
        out.push(Source::Claude);
    }
    if has("codex") && codex_login().is_file() {
        out.push(Source::Codex);
    }
    if (has("opencode") || has("omp")) && go_key().is_ok() {
        out.push(Source::Go);
    }
    out
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Where Claude Code keeps its login on Linux; macOS keeps it in the
/// Keychain, which Libri does not read.
fn claude_login() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".claude"))
        .join(".credentials.json")
}

/// Where Codex keeps its login; one signed in with an API key has no
/// windows, and Codex says so.
fn codex_login() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".codex"))
        .join("auth.json")
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build(),
    )
}

/// GETs `url` as JSON with the bearer `token`; the token never goes into
/// an error.
fn get(url: &str, token: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let mut req = agent()
        .get(url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("User-Agent", "libri");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut resp = req.call().map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    match status {
        200 => resp.body_mut().read_json().map_err(|e| e.to_string()),
        401 | 403 => Err(format!("the sign-in was refused ({status})")),
        429 => Err("the provider is rate limiting usage reads".into()),
        s => Err(format!("the provider answered {s}")),
    }
}

fn claude() -> Result<Account, String> {
    let text = std::fs::read_to_string(claude_login()).map_err(|e| e.to_string())?;
    let login: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let o = &login["claudeAiOauth"];
    let plan = claude_plan(o);
    let expires = o["expiresAt"].as_i64().unwrap_or(0) / 1000;
    let mut a = Account {
        account: "claude".into(),
        name: "Claude".into(),
        plan,
        windows: Vec::new(),
        notes: Vec::new(),
        read_at: Local::now().timestamp(),
        stale: false,
    };
    if expires < Local::now().timestamp() {
        a.notes
            .push("Claude Code's sign-in has lapsed; it renews on the next Claude turn.".into());
        return Ok(a);
    }
    let token = o["accessToken"]
        .as_str()
        .ok_or("Claude Code keeps no sign-in")?;
    let out = get(
        "https://api.anthropic.com/api/oauth/usage",
        token,
        &[("anthropic-beta", "oauth-2025-04-20")],
    )?;
    a.windows = claude_windows(&out);
    if let Some(rows) = out["seven_day_breakdown"]["rows"].as_array() {
        let used: Vec<String> = rows
            .iter()
            .filter(|r| r["percent"].as_u64().unwrap_or(0) > 0)
            .filter_map(|r| Some(format!("{} {}%", r["display_name"].as_str()?, r["percent"])))
            .collect();
        if !used.is_empty() {
            a.notes
                .push(format!("This week by app: {}", used.join(" · ")));
        }
    }
    if out["extra_usage"]["is_enabled"] == true
        && let Some(u) = out["extra_usage"]["utilization"].as_f64()
    {
        a.notes
            .push(format!("Extra usage {u:.0}% of its monthly limit"));
    }
    Ok(a)
}

/// "Max 20x" from `max` and `default_claude_max_20x`.
fn claude_plan(o: &Value) -> Option<String> {
    let kind = o["subscriptionType"].as_str()?;
    let mut name = capitalized(kind);
    if let Some(x) = o["rateLimitTier"]
        .as_str()
        .and_then(|t| t.rsplit('_').next())
        .filter(|t| t.ends_with('x') && t[..t.len() - 1].parse::<u32>().is_ok())
    {
        name = format!("{name} {x}");
    }
    Some(name)
}

/// The windows in Claude's answer: its `limits` list where it gives one
/// (the whole week, and each model's own week), else the older named
/// fields.
fn claude_windows(out: &Value) -> Vec<Window> {
    if let Some(list) = out["limits"].as_array().filter(|l| !l.is_empty()) {
        return list
            .iter()
            .filter_map(|l| {
                let pct = l["percent"].as_f64()?;
                let at = stamp(&l["resets_at"])?;
                let model = l["scope"]["model"]["display_name"]
                    .as_str()
                    .map(str::to_owned);
                let (id, name, hours) = match l["group"].as_str().unwrap_or_default() {
                    "weekly" => ("week", "Week", 168.0),
                    "session" | "five_hour" => ("5h", "5 hours", 5.0),
                    "monthly" => ("month", "Month", 720.0),
                    _ => return None,
                };
                Some(window(id, name, hours, pct, at, model))
            })
            .collect();
    }
    [
        ("five_hour", "5h", "5 hours", 5.0, None),
        ("seven_day", "week", "Week", 168.0, None),
        ("seven_day_opus", "week", "Week", 168.0, Some("Opus")),
        ("seven_day_sonnet", "week", "Week", 168.0, Some("Sonnet")),
    ]
    .into_iter()
    .filter_map(|(k, id, name, hours, model)| {
        let w = &out[k];
        let at = stamp(&w["resets_at"])?;
        Some(window(
            id,
            name,
            hours,
            w["utilization"].as_f64()?,
            at,
            model.map(str::to_owned),
        ))
    })
    .collect()
}

fn window(id: &str, name: &str, hours: f64, pct: f64, at: i64, model: Option<String>) -> Window {
    let (window, name) = match &model {
        Some(m) => (
            format!("{id}-{}", m.to_ascii_lowercase()),
            format!("{name} · {m}"),
        ),
        None => (id.to_owned(), name.to_owned()),
    };
    Window {
        window,
        name,
        hours: Some(hours),
        pct: pct.round().clamp(0.0, 100.0) as u32,
        resets: when(at),
        resets_at: Some(at),
        model,
    }
}

/// Unix seconds from a number of them or an RFC 3339 time.
fn stamp(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| {
        chrono::DateTime::parse_from_rfc3339(v.as_str()?)
            .ok()
            .map(|t| t.timestamp())
    })
}

fn when(at: i64) -> String {
    Local
        .timestamp_opt(at, 0)
        .single()
        .map(|t| t.format("%a %b %-d %H:%M").to_string())
        .unwrap_or_default()
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Asks a `codex app-server` of its own for the account's limits: the
/// handshake, then `account/rateLimits/read`, as Codex's `/status` does.
fn codex() -> Result<Account, String> {
    let command = std::env::var("LIBRI_CODEX")
        .ok()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "codex".into());
    let mut child = Command::new(&command)
        .arg("app-server")
        .current_dir(crate::model::bare_dir()?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{command}: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line)
                && tx.send(v).is_err()
            {
                break;
            }
        }
    });
    let mut send =
        |v: Value| -> Result<(), String> { writeln!(stdin, "{v}").map_err(|e| e.to_string()) };
    let deadline = Instant::now() + TIMEOUT;
    let reply = |id: u64| -> Result<Value, String> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let m = rx
                .recv_timeout(left)
                .map_err(|_| "codex app-server didn't answer")?;
            if m["id"] == id {
                if let Some(e) = m.get("error") {
                    return Err(e["message"].as_str().unwrap_or("failed").to_owned());
                }
                return Ok(m["result"].clone());
            }
        }
    };
    let got = (|| {
        send(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"clientInfo": {"name": "libri", "version": env!("CARGO_PKG_VERSION")}}}))?;
        reply(1)?;
        send(json!({"jsonrpc": "2.0", "method": "initialized"}))?;
        send(json!({"jsonrpc": "2.0", "id": 2, "method": "account/rateLimits/read"}))?;
        reply(2)
    })();
    let _ = child.kill();
    let _ = child.wait();
    Ok(codex_account(&got?))
}

fn codex_account(out: &Value) -> Account {
    let limits: Vec<&Value> = match out["rateLimitsByLimitId"].as_object() {
        Some(m) if !m.is_empty() => m.values().collect(),
        _ => vec![&out["rateLimits"]],
    };
    let mut windows = Vec::new();
    let mut plan = None;
    for l in limits {
        plan = plan.or_else(|| l["planType"].as_str().map(codex_plan));
        // A limit with a name of its own counts one model.
        let model = l["limitName"].as_str().map(str::to_owned);
        for w in [&l["primary"], &l["secondary"]] {
            let (Some(pct), Some(mins), Some(at)) = (
                w["usedPercent"].as_f64(),
                w["windowDurationMins"].as_u64(),
                w["resetsAt"].as_i64(),
            ) else {
                continue;
            };
            let (id, name) = match mins {
                300 => ("5h".to_owned(), "5 hours".to_owned()),
                10080 => ("week".to_owned(), "Week".to_owned()),
                m => (format!("{m}m"), format!("{} hours", m / 60)),
            };
            windows.push(window(
                &id,
                &name,
                mins as f64 / 60.0,
                pct,
                at,
                model.clone(),
            ));
        }
    }
    let mut notes = Vec::new();
    let resets = out["rateLimitResetCredits"]["credits"]
        .as_array()
        .map_or(0, |c| {
            c.iter().filter(|c| c["status"] == "available").count()
        });
    if resets > 0 {
        notes.push(format!(
            "{resets} free full reset{} unused",
            if resets == 1 { "" } else { "s" }
        ));
    }
    Account {
        account: "codex".into(),
        name: "ChatGPT".into(),
        plan,
        windows,
        notes,
        read_at: Local::now().timestamp(),
        stale: false,
    }
}

/// "Pro Lite" from `prolite`.
fn codex_plan(t: &str) -> String {
    match t {
        "prolite" => "Pro Lite".into(),
        other => capitalized(other),
    }
}

/// The OpenCode Go key OpenCode keeps.
fn go_key() -> Result<String, String> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"));
    let text =
        std::fs::read_to_string(data.join("opencode/auth.json")).map_err(|e| e.to_string())?;
    let auth: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    auth["opencode-go"]["key"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "OpenCode keeps no Go key".into())
}

fn go() -> Result<Account, String> {
    let out = get("https://opencode.ai/zen/go/v1/usage", &go_key()?, &[])?;
    Ok(go_account(&out))
}

fn go_account(out: &Value) -> Account {
    let u = &out["usage"];
    let windows = [
        ("rolling", "5h", "5 hours", 5.0),
        ("weekly", "week", "Week", 168.0),
        ("monthly", "month", "Month", 720.0),
    ]
    .into_iter()
    .filter_map(|(k, id, name, hours)| {
        let w = &u[k];
        Some(window(
            id,
            name,
            hours,
            w["percent"].as_f64()?,
            stamp(&w["resetsAt"])?,
            None,
        ))
    })
    .collect();
    Account {
        account: "go".into(),
        name: "OpenCode Go".into(),
        plan: None,
        windows,
        notes: Vec::new(),
        read_at: Local::now().timestamp(),
        stale: false,
    }
}

/// Shortest first; a model's own window after the whole account's.
fn sort(a: &mut Account) {
    a.windows.sort_by(|x, y| {
        x.hours
            .unwrap_or(f64::MAX)
            .total_cmp(&y.hours.unwrap_or(f64::MAX))
            .then(x.model.is_some().cmp(&y.model.is_some()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claudes_limits_list_gives_the_week_and_each_models_week() {
        let out = json!({
            "five_hour": null,
            "seven_day": {"utilization": 77.0, "resets_at": "2026-11-08T22:00:00.66+00:00"},
            "limits": [
                {"kind": "weekly_all", "group": "weekly", "percent": 77,
                 "resets_at": "2026-11-08T22:00:00.66+00:00", "scope": null},
                {"kind": "weekly_scoped", "group": "weekly", "percent": 0,
                 "resets_at": "2026-11-08T22:00:00+00:00",
                 "scope": {"model": {"id": null, "display_name": "Fable"}}}
            ]
        });
        let w = claude_windows(&out);
        let ids: Vec<_> = w
            .iter()
            .map(|w| (w.window.as_str(), w.name.as_str(), w.pct))
            .collect();
        assert_eq!(
            ids,
            [("week", "Week", 77), ("week-fable", "Week · Fable", 0)]
        );
        assert_eq!(w[0].resets_at, Some(1794175200));
    }

    #[test]
    fn claudes_older_fields_still_read() {
        let out = json!({
            "five_hour": {"utilization": 12.4, "resets_at": "2026-11-06T23:00:00+00:00"},
            "seven_day": {"utilization": 30.0, "resets_at": "2026-11-08T22:00:00+00:00"},
            "seven_day_opus": null,
        });
        let w = claude_windows(&out);
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0].window.as_str(), w[0].pct, w[0].hours),
            ("5h", 12, Some(5.0))
        );
    }

    #[test]
    fn the_plan_reads_as_claude_names_it() {
        let o = json!({"subscriptionType": "max", "rateLimitTier": "default_claude_max_20x"});
        assert_eq!(claude_plan(&o).as_deref(), Some("Max 20x"));
        let o = json!({"subscriptionType": "pro", "rateLimitTier": "default_claude_ai"});
        assert_eq!(claude_plan(&o).as_deref(), Some("Pro"));
    }

    #[test]
    fn codex_windows_go_by_their_length_not_their_slot() {
        let out = json!({
            "rateLimits": {},
            "rateLimitsByLimitId": {"codex": {"limitId": "codex", "limitName": null,
                "primary": {"usedPercent": 2, "windowDurationMins": 10080, "resetsAt": 1794403804},
                "secondary": null, "planType": "prolite"}},
            "rateLimitResetCredits": {"credits": [{"status": "available"}, {"status": "used"}]}
        });
        let mut a = codex_account(&out);
        sort(&mut a);
        assert_eq!(a.plan.as_deref(), Some("Pro Lite"));
        assert_eq!(a.windows.len(), 1);
        assert_eq!(
            (a.windows[0].window.as_str(), a.windows[0].pct),
            ("week", 2)
        );
        assert_eq!(a.notes, ["1 free full reset unused"]);
    }

    #[test]
    fn go_has_three_windows() {
        let out = json!({"usage": {
            "rolling": {"status": "ok", "percent": 0, "resetsAt": "2026-11-07T02:36:47.000Z"},
            "weekly": {"status": "ok", "percent": 8, "resetsAt": "2026-11-09T00:00:00.000Z"},
            "monthly": {"status": "ok", "percent": 12, "resetsAt": "2026-11-24T13:28:54.000Z"}}});
        let a = go_account(&out);
        let w: Vec<_> = a
            .windows
            .iter()
            .map(|w| (w.window.as_str(), w.pct))
            .collect();
        assert_eq!(w, [("5h", 0), ("week", 8), ("month", 12)]);
    }

    #[test]
    fn the_old_list_still_reads_as_the_0_9_17_client_reads_it() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct OldLimit {
            account: String,
            window: String,
            pct: f64,
            resets: String,
        }
        let r = Report {
            accounts: vec![go_account(&json!({"usage": {
                "weekly": {"percent": 8, "resetsAt": 1794403804}}}))],
            notes: Vec::new(),
        };
        let json = serde_json::to_string(&limits(&r)).unwrap();
        let old: Vec<OldLimit> = serde_json::from_str(&json).unwrap();
        assert_eq!(
            (old[0].account.as_str(), old[0].window.as_str(), old[0].pct),
            ("go", "week", 8.0)
        );
        assert!(!old[0].resets.is_empty());
    }

    #[test]
    fn a_model_draws_on_its_harness_subscription() {
        assert_eq!(account("claude", "anthropic"), Some("claude"));
        assert_eq!(account("codex", "openai"), Some("codex"));
        assert_eq!(account("opencode", "opencode-go"), Some("go"));
        assert_eq!(account("omp", "deepseek"), None);
    }
}
