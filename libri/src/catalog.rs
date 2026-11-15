//! The models the person picks from, each with the harness that runs it, so
//! the page offers models rather than harnesses: Claude's through Claude
//! Code, Meta subscriptions through Muse Code, and other models by default
//! through OpenCode, omp or Pi, whichever the person prefers. Each list is
//! the harness's own.

use crate::model::{Kept, list};
use crate::rpc::Session;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// The harnesses that run every other maker's models; the person picks one.
pub const OTHERS: &[&str] = &["omp", "opencode", "pi"];

/// The one the person prefers among OTHERS.
pub const OTHER: Kept = Kept("others");

/// The model picked and the harness that runs it, as JSON.
pub const PICK: Kept = Kept("pick");

/// Temp's own pick and thinking; until it has them it follows the main chat.
pub const TEMP_PICK: Kept = Kept("temp-pick");
pub const TEMP_EFFORT: Kept = Kept("temp-effort");

/// The model that writes workers' titles and summaries; until it has one
/// the cheapest default on hippo's tested list does.
pub const SYNOPSIS_PICK: Kept = Kept("synopsis-pick");

/// The model new workers start on; until it has one they take the main
/// chat's.
pub const WORKER_PICK: Kept = Kept("worker-pick");

/// The models last listed, for the tool server, which has no harness of its
/// own to ask.
const LISTED: Kept = Kept("offers");

/// How long one harness may take to list its models.
const LISTING: Duration = Duration::from_secs(60);

/// A model on offer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Offer {
    pub harness: String,
    /// As the harness takes it.
    pub model: String,
    pub name: String,
    /// Who serves it, by the harness's own id ("anthropic", "opencode-go").
    pub provider: String,
    /// The thinking levels it takes; none when the harness sets none.
    pub efforts: Vec<String>,
    /// The harness's own default.
    pub default: bool,
    /// The subscription in the usage report it draws on (usage::account).
    #[serde(default)]
    pub usage: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pick {
    pub harness: String,
    pub model: String,
}

/// One turn's model, thinking and access, apart from the main chat's.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TurnPick {
    pub pick: Pick,
    /// None for the harness's own default.
    #[serde(default)]
    pub effort: Option<String>,
    /// The harness's own mode; None for its default in Settings.
    #[serde(default)]
    pub access: Option<String>,
    /// The harness's own sandbox setting; None for its default in Settings.
    #[serde(default)]
    pub sandbox: Option<String>,
}

impl TurnPick {
    pub fn picks(&self) -> crate::access::Picks {
        crate::access::Picks {
            access: self.access.clone(),
            sandbox: self.sandbox.clone(),
        }
    }
}

/// The harnesses switched off in Settings, one a line.
pub const OFF: Kept = Kept("harnesses-off");

/// The harness a chat runs on until the person picks a model, from Settings.
pub const DEFAULT: Kept = Kept("default-harness");

/// Each harness's default model from Settings, as a JSON map.
pub const DEFAULT_MODELS: Kept = Kept("default-models");

/// The harnesses installed on this host (LIBRI_HARNESSES, set from host.rs
/// `harnesses`), else the one configured, whether or not switched on.
pub fn configured() -> Vec<String> {
    let all = list("LIBRI_HARNESSES");
    if all.is_empty() {
        vec![configured_default()]
    } else {
        all
    }
}

/// The harness a new chat runs on when the install names none: omp, whose
/// RPC stream carries the most of a turn (live output, questions, cards).
pub const PREFERRED: &str = "omp";

/// The install's default (LIBRI_BACKEND); else PREFERRED, when it is found
/// or nothing is; else the first found.
pub fn configured_default() -> String {
    if let Some(b) = std::env::var("LIBRI_BACKEND")
        .ok()
        .filter(|v| !v.is_empty())
    {
        return b;
    }
    let found = list("LIBRI_HARNESSES");
    if found.is_empty() || found.iter().any(|h| h == PREFERRED) {
        PREFERRED.into()
    } else {
        found[0].clone()
    }
}

/// Harnesses off until the person switches them on: Pi is offered but
/// rarely wanted. The host's configured default is never among them.
const OFF_AT_FIRST: &[&str] = &["pi"];

/// Those switched off in Settings; before the person has switched any,
/// those off at first.
pub fn off(state: &Path) -> Vec<String> {
    match std::fs::read_to_string(state.join(OFF.0)) {
        Ok(t) => t
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect(),
        Err(_) => {
            let default = configured_default();
            OFF_AT_FIRST
                .iter()
                .filter(|h| **h != default)
                .map(|h| (*h).to_owned())
                .collect()
        }
    }
}

/// The harnesses offered: those configured and switched on. Never none:
/// Settings won't switch off the last.
pub fn harnesses() -> Vec<String> {
    let all = configured();
    let off = state().map(|s| off(&s)).unwrap_or_default();
    let on: Vec<String> = all.iter().filter(|h| !off.contains(h)).cloned().collect();
    if on.is_empty() { all } else { on }
}

/// Switches `harness` on or off. The default harness moves to another on
/// when its own goes off.
pub fn switch(state: &Path, harness: &str, on: bool) -> Result<(), String> {
    if !configured().iter().any(|h| h == harness) {
        return Err(format!("{harness} is not a harness on this host"));
    }
    let mut off = off(state);
    off.retain(|h| h != harness);
    if !on {
        off.push(harness.to_owned());
        if configured().iter().all(|h| off.contains(h)) {
            return Err("One harness has to stay on.".into());
        }
    }
    // Written even when empty, so a harness off at first stays switched on.
    OFF.write(state, &off.join("\n"))?;
    if !on && DEFAULT.read(state).as_deref() == Some(harness) {
        DEFAULT.clear(state);
    }
    Ok(())
}

/// The harness picked in Settings to run a chat with no model picked.
pub fn default_picked(state: &Path) -> Option<String> {
    DEFAULT.read(state)
}

/// Holds `harness` as the default; it must be on.
pub fn set_default(state: &Path, harness: &str) -> Result<(), String> {
    if !harnesses().iter().any(|h| h == harness) {
        return Err(format!("{harness} is switched off"));
    }
    DEFAULT.write(state, harness)
}

/// Each harness's default model picked in Settings.
pub fn default_models(state: &Path) -> serde_json::Map<String, Value> {
    DEFAULT_MODELS
        .read(state)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// `harness`'s default model from Settings; None for its own.
pub fn default_model(harness: &str) -> Option<String> {
    let state = state()?;
    default_models(&state)
        .get(harness)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Holds `model` as `harness`'s default; empty goes back to its own.
pub fn set_default_model(state: &Path, harness: &str, model: &str) -> Result<(), String> {
    let mut all = default_models(state);
    if model.is_empty() {
        all.remove(harness);
    } else {
        all.insert(harness.to_owned(), model.into());
    }
    DEFAULT_MODELS.write(state, &Value::Object(all).to_string())
}

/// The harness a chat with no model picked runs on: the Settings default,
/// else the configured one, while switched on; else the first on.
pub fn default_harness() -> String {
    let on = harnesses();
    state()
        .and_then(|s| default_picked(&s))
        .into_iter()
        .chain([configured_default()])
        .find(|h| on.contains(h))
        .unwrap_or_else(|| on[0].clone())
}

fn state() -> Option<PathBuf> {
    std::env::var_os("STATE_DIRECTORY").map(PathBuf::from)
}

fn kept(state: &Path, file: &Kept) -> Option<Pick> {
    let p: Pick = serde_json::from_str(&file.read(state)?).ok()?;
    harnesses().contains(&p.harness).then_some(p)
}

fn keep(state: &Path, file: &Kept, p: &Pick) -> Result<(), String> {
    file.write(state, &serde_json::to_string(p).map_err(|e| e.to_string())?)
}

/// The person's pick, while its harness is still offered.
pub fn picked(state: &Path) -> Option<Pick> {
    kept(state, &PICK)
}

pub fn pick(state: &Path, p: &Pick) -> Result<(), String> {
    keep(state, &PICK, p)
}

/// Temp's own pick, while its harness is still offered.
pub fn temp_picked(state: &Path) -> Option<Pick> {
    kept(state, &TEMP_PICK)
}

pub fn temp_pick(state: &Path, p: &Pick) -> Result<(), String> {
    keep(state, &TEMP_PICK, p)
}

/// The pick for titles and summaries, while its harness is still offered.
pub fn synopsis_picked(state: &Path) -> Option<Pick> {
    kept(state, &SYNOPSIS_PICK)
}

/// Holds `p` for titles and summaries; None goes back to the default.
pub fn synopsis_pick(state: &Path, p: Option<&Pick>) -> Result<(), String> {
    match p {
        Some(p) => keep(state, &SYNOPSIS_PICK, p),
        None => {
            SYNOPSIS_PICK.clear(state);
            Ok(())
        }
    }
}

/// What new workers start on, whether or not it is on offer today: a model
/// that comes back takes up again. None follows the main chat.
pub fn worker_picked(state: &Path) -> Option<Pick> {
    serde_json::from_str(&WORKER_PICK.read(state)?).ok()
}

/// Holds `p` for new workers; None goes back to following the main chat.
pub fn worker_pick(state: &Path, p: Option<&Pick>) -> Result<(), String> {
    match p {
        Some(p) => keep(state, &WORKER_PICK, p),
        None => {
            WORKER_PICK.clear(state);
            Ok(())
        }
    }
}

/// The harness the next turn runs on.
pub fn harness_now() -> String {
    state()
        .and_then(|s| picked(&s))
        .map(|p| p.harness)
        .unwrap_or_else(default_harness)
}

/// The model picked for `harness`, else its default in Settings; None
/// for the harness's own.
pub fn model_for(harness: &str) -> Option<String> {
    state()
        .and_then(|s| picked(&s))
        .filter(|p| p.harness == harness && !p.model.is_empty())
        .map(|p| p.model)
        .or_else(|| default_model(harness))
}

/// What the main chat runs on now, which a worker with no other model takes
/// at its start and holds. An empty model is its harness's own default. Access isn't held: a
/// worker starts on its harness's default in Settings, so changing that takes
/// every worker with it, unless the person picked otherwise on its page.
pub fn main_choice(state: &Path) -> TurnPick {
    TurnPick {
        pick: picked(state).unwrap_or_else(|| Pick {
            harness: harness_now(),
            model: String::new(),
        }),
        effort: crate::model::EFFORT
            .read(state)
            .or_else(|| std::env::var("LIBRI_EFFORT").ok()),
        access: None,
        sandbox: None,
    }
}

/// Keeps the models just listed for `worker_choice`.
pub fn remember(state: &Path, offers: &[Offer]) {
    let kept = serde_json::to_string(offers).map_err(|e| e.to_string());
    if let Err(e) = kept.and_then(|text| LISTED.write(state, &text)) {
        eprintln!("libri: keeping the model list: {e}");
    }
}

/// What a new worker runs on, on its harness's default access: `named`, by
/// any name `resolve` takes; else the Workers default in Settings; else the
/// main chat's model. Its thinking is `effort` when named, which the model
/// must take; else the main chat's, then the install's (LIBRI_EFFORT), when
/// the model has that level; else the harness's own default. A default no
/// longer on offer gives way to the main chat's model, with the words saying
/// so; a named model or level not on offer is refused with what is, since
/// main can name another. Nothing is guessed: no fallback stands in for a
/// name that matches nothing or more than one model.
pub fn worker_choice(
    state: &Path,
    named: Option<&str>,
    effort: Option<&str>,
) -> Result<(TurnPick, Option<String>), String> {
    let mut choice = main_choice(state);
    let offers = listed(state);
    let mut lost = None;
    let offer = if let Some(named) = named {
        let found = resolve(&offers, named)?;
        choice.pick = Pick {
            harness: found.harness.clone(),
            model: found.model.clone(),
        };
        Some(found)
    } else if let Some(default) = worker_picked(state) {
        match offers
            .iter()
            .find(|o| o.harness == default.harness && o.model == default.model)
            .filter(|o| harnesses().contains(&o.harness))
        {
            Some(found) => {
                choice.pick = default;
                Some(found)
            }
            None => {
                lost = Some(format!(
                    "The Workers default in Settings, {}, is not on offer, so they run on your model.",
                    default.model
                ));
                offer_of(&offers, &choice.pick)
            }
        }
    } else {
        offer_of(&offers, &choice.pick)
    };
    choice.effort = match (effort, offer) {
        (Some(asked), Some(o)) => Some(level(o, asked)?),
        // A model not on the list (the main chat's own default, unnamed)
        // can't be checked, so a level asked for it is refused, not dropped.
        (Some(asked), None) => {
            return Err(format!(
                "Thinking {asked} can't be checked: name the model too ({}).",
                choice.pick.harness
            ));
        }
        (None, Some(o)) => choice
            .effort
            .clone()
            .into_iter()
            .chain(std::env::var("LIBRI_EFFORT").ok())
            .find(|e| o.efforts.contains(e)),
        (None, None) => choice.effort.clone(),
    };
    Ok((choice, lost))
}

/// The models last listed.
pub fn listed(state: &Path) -> Vec<Offer> {
    LISTED
        .read(state)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn offer_of<'a>(offers: &'a [Offer], p: &Pick) -> Option<&'a Offer> {
    offers
        .iter()
        .find(|o| o.harness == p.harness && o.model == p.model)
}

/// `asked` as `o` spells it, when `o` takes it; else why not, with the
/// levels it does take.
pub fn level(o: &Offer, asked: &str) -> Result<String, String> {
    let asked = asked.trim().to_lowercase();
    if o.efforts.is_empty() {
        return Err(format!(
            "{} ({}) takes no thinking level: {} sets none for it. Leave effort out.",
            o.name, o.model, o.harness
        ));
    }
    o.efforts
        .iter()
        .find(|e| **e == asked)
        .cloned()
        .ok_or_else(|| {
            format!(
                "{} ({}) takes no thinking level {asked}; it takes {}.",
                o.name,
                o.model,
                o.efforts.join(", ")
            )
        })
}

/// The harnesses that run their own maker's models, as against those that
/// route to many makers' (OTHERS): a name both offer means the former.
const OWN_MAKER: &[&str] = &["claude", "codex", "muse"];

/// The one offer `named` means, by, in this order of strength: its id as
/// the harness takes it ("opus", "gpt-6.1-sol", "opencode-zen/glm-5.3"); its
/// display name ("Fable 5.2") or full id ("claude-fable-5-2"); the id with
/// no provider in front; then its short name, the id without version numbers
/// ("wonder-full", "muse-wonder-full", "sol", "gpt-sol"). Case, spaces, dots
/// and dashes are alike. Several at the strongest level found go to the
/// dedicated harness over a router (Claude Code over omp for an Opus), then
/// to the newest version ("sol" is gpt-6.1-sol while that is newest); a tie
/// after that is refused with the candidates. Matching nothing is refused
/// with all that is on offer.
pub fn resolve<'a>(offers: &'a [Offer], named: &str) -> Result<&'a Offer, String> {
    let want = norm(named);
    if want.is_empty() {
        return Err("Name a model.".into());
    }
    let strength = |o: &Offer| aliases(o).iter().position(|tier| tier.contains(&want));
    let Some(best) = offers.iter().filter_map(strength).min() else {
        return Err(format!(
            "{named} is not a model on offer. On offer, by harness:\n{}",
            on_offer(offers)
        ));
    };
    let mut found: Vec<&Offer> = offers
        .iter()
        .filter(|o| strength(o) == Some(best))
        .collect();
    if found
        .iter()
        .any(|o| OWN_MAKER.contains(&o.harness.as_str()))
    {
        found.retain(|o| OWN_MAKER.contains(&o.harness.as_str()));
    }
    let newest = found
        .iter()
        .map(|o| version(&o.model))
        .max()
        .unwrap_or_default();
    found.retain(|o| version(&o.model) == newest);
    match found[..] {
        [one] => Ok(one),
        _ => Err(format!(
            "{named} could be any of {}; name one by its id.",
            found
                .iter()
                .map(|o| format!("{} ({}, {})", o.model, o.name, o.harness))
                .collect::<Vec<_>>()
                .join("; ")
        )),
    }
}

/// Lowercase, with spaces, dots, underscores and slashes as dashes.
fn norm(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-' | '/'))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// A version part of an id: "6", "5", "v4", "k2". Lone letters stay words.
fn is_version(part: &str) -> bool {
    let digits = part.strip_prefix(['v', 'k']).unwrap_or(part);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// The numbers in an id, in order, to compare versions by.
fn version(model: &str) -> Vec<u64> {
    norm(model)
        .split('-')
        .filter(|p| is_version(p))
        .filter_map(|p| p.trim_start_matches(['v', 'k']).parse().ok())
        .collect()
}

/// The names an offer answers to, strongest first.
fn aliases(o: &Offer) -> [Vec<String>; 4] {
    let id = norm(&o.model);
    let bare = o
        .model
        .split_once('/')
        .map_or_else(|| id.clone(), |(_, rest)| norm(rest));
    let name = norm(&o.name);
    let mut named = vec![name.clone()];
    // Claude Code lists aliases ("fable") under names with the version
    // ("Fable 5.2"), so its full id is "claude-" and the name.
    if o.harness == "claude" {
        named.push(format!("claude-{name}"));
    }
    let words: Vec<&str> = bare.split('-').filter(|p| !is_version(p)).collect();
    let mut short = vec![words.join("-")];
    if words.len() > 1 {
        short.push(words[1..].join("-"));
    }
    [vec![id], named, vec![bare], short]
}

/// Every offer, a line per harness.
fn on_offer(offers: &[Offer]) -> String {
    let mut by: Vec<(&str, Vec<&str>)> = Vec::new();
    for o in offers {
        match by.iter_mut().find(|(h, _)| *h == o.harness) {
            Some((_, ids)) => ids.push(&o.model),
            None => by.push((&o.harness, vec![&o.model])),
        }
    }
    if by.is_empty() {
        return "none listed yet: open the model picker once so Libri asks each harness.".into();
    }
    by.iter()
        .map(|(h, ids)| format!("{h}: {}", ids.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Which of OTHERS are offered.
pub fn others_offered() -> Vec<String> {
    let all = harnesses();
    OTHERS
        .iter()
        .filter(|o| all.iter().any(|h| h == *o))
        .map(|o| (*o).to_owned())
        .collect()
}

/// Which harnesses switched on aren't signed in, by their own credentials:
/// offered greyed, with how to sign them in, rather than failing a turn.
pub fn signed_out() -> Vec<String> {
    let checks: Vec<_> = harnesses()
        .into_iter()
        .map(|h| thread::spawn(move || (!crate::signin::signed(&h)).then_some(h)))
        .collect();
    checks
        .into_iter()
        .filter_map(|c| c.join().ok().flatten())
        .collect()
}

/// The harness that runs the models neither Claude Code nor Codex does.
pub fn others(state: &Path) -> Option<String> {
    let offered = others_offered();
    OTHER
        .read(state)
        .filter(|o| offered.contains(o))
        .or_else(|| offered.contains(&default_harness()).then(default_harness))
        .or_else(|| offered.first().cloned())
}

/// Switches the other models to `harness`; a pick on the one it replaces
/// moves with it, or falls back to the default model. Returns the models
/// now on offer.
pub fn set_others(state: &Path, harness: &str) -> Result<Vec<Offer>, String> {
    if !others_offered().iter().any(|o| o == harness) {
        return Err(format!("{harness} does not run other models here"));
    }
    if !crate::signin::signed(harness) {
        return Err(format!(
            "{harness} is not signed in: {}",
            crate::signin::remedy(harness)
        ));
    }
    OTHER.write(state, harness)?;
    let offers = offers(state);
    if !offers.is_empty() {
        remember(state, &offers);
    }
    for file in [&PICK, &TEMP_PICK, &WORKER_PICK, &SYNOPSIS_PICK] {
        if let Some(p) = kept(state, file).filter(|p| OTHERS.contains(&p.harness.as_str())) {
            let moved = Pick {
                harness: harness.to_owned(),
                model: p.model,
            };
            if offers
                .iter()
                .any(|o| o.harness == moved.harness && o.model == moved.model)
            {
                keep(state, file, &moved)?;
            } else {
                file.clear(state);
            }
        }
    }
    Ok(offers)
}

/// Every model on offer: each harness asked in parallel, the other
/// models' harness leaving out the makers Claude Code and Codex serve.
pub fn offers(state: &Path) -> Vec<Offer> {
    let all = harnesses();
    let has = |h: &str| all.iter().any(|a| a == h);
    let mut lists = Vec::new();
    if has("claude") {
        lists.push(thread::spawn(|| Ok(claude())));
    }
    if has("codex") {
        lists.push(thread::spawn(codex));
    }
    if has("muse") {
        lists.push(thread::spawn(muse));
    }
    let other = others(state);
    match other.as_deref() {
        Some("opencode") => lists.push(thread::spawn(opencode)),
        Some("omp") => lists.push(thread::spawn(omp)),
        Some("pi") => lists.push(thread::spawn(pi)),
        _ => {}
    }
    let mut found = Vec::new();
    for l in lists {
        match l.join() {
            Ok(Ok(o)) => found.extend(o),
            Ok(Err(e)) => eprintln!("libri: listing models: {e}"),
            Err(_) => eprintln!("libri: listing models panicked"),
        }
    }
    found.retain(|o| provider_allowed(o, has("claude"), has("codex"), has("muse")));

    for o in &mut found {
        o.usage = crate::usage::account(&o.harness, &o.provider).map(str::to_owned);
    }
    found
}

/// omp owns the default path for OpenAI too. Dedicated harnesses remain
/// explicit choices, while subscription providers restricted to their own
/// harness never appear behind omp. Google's API-key provider is separate.
fn provider_allowed(o: &Offer, claude: bool, codex: bool, muse: bool) -> bool {
    if o.harness == "omp" {
        return !matches!(
            o.provider.as_str(),
            "anthropic" | "google-gemini-cli" | "google-antigravity" | "meta"
        );
    }
    !OTHERS.contains(&o.harness.as_str())
        || !((claude && o.provider == "anthropic")
            || (codex && matches!(o.provider.as_str(), "openai" | "openai-codex"))
            || (muse && o.provider == "meta"))
}

/// Claude Code's own list, from its stream-json `initialize`: its aliases
/// (opus, fable, sonnet, haiku), so a new model appears with a Claude Code
/// update. If it can't be asked, the aliases it has long had, unnamed.
fn claude() -> Vec<Offer> {
    let mut offers = match claude_models().map(|v| claude_offers(&v)) {
        Ok(o) if !o.is_empty() => o,
        listed => {
            let why = listed.err().unwrap_or_else(|| "it listed none".into());
            eprintln!("libri: listing Claude Code's models: {why}");
            ["opus", "fable", "sonnet", "haiku"]
                .map(|m| Offer {
                    harness: "claude".into(),
                    model: m.into(),
                    name: claude_name(m),
                    provider: "anthropic".into(),
                    efforts: Vec::new(),
                    default: false,
                    usage: None,
                })
                .to_vec()
        }
    };
    let default = std::env::var("LIBRI_MODEL").unwrap_or_else(|_| "sonnet".into());
    for o in &mut offers {
        if o.efforts.is_empty() {
            o.efforts = list("LIBRI_EFFORTS");
        }
        // The default may be a full id ("claude-opus-5-5") for an alias.
        o.default = o.model == default
            || default
                .strip_prefix("claude-")
                .is_some_and(|r| r.starts_with(&format!("{}-", o.model)));
    }
    offers
}

/// Claude Code's answer to `initialize`, the control request the Agent SDK
/// opens with. Safe mode: listing loads none of the person's hooks or
/// plugins.
fn claude_models() -> Result<Value, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    let command = std::env::var("LIBRI_CLAUDE").unwrap_or_else(|_| "claude".into());
    let mut child = Command::new(&command)
        .args([
            "-p",
            "--safe-mode",
            "--no-session-persistence",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{command}: {e}"))?;
    let ask = json!({"type": "control_request", "request_id": "models",
        "request": {"subtype": "initialize"}});
    // Closing its input ends it once it has answered.
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    writeln!(stdin, "{ask}").map_err(|e| format!("{command}: {e}"))?;
    drop(stdin);
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let answer = BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
            .find(|v| v["type"] == "control_response" && v["response"]["request_id"] == "models");
        let _ = tx.send(answer);
    });
    let answer = rx.recv_timeout(LISTING);
    let _ = child.kill();
    let _ = child.wait();
    let r = answer
        .map_err(|_| format!("{command} gave no model list in {}s", LISTING.as_secs()))?
        .ok_or_else(|| format!("{command} ended without a model list"))?;
    if r["response"]["subtype"] == "error" {
        return Err(format!(
            "{command} refused initialize: {}",
            r["response"]["error"].as_str().unwrap_or_default()
        ));
    }
    Ok(r["response"]["response"].clone())
}

/// The aliases among Claude Code's models: neither its "default" nor the
/// older ids it pins by full name.
fn claude_offers(init: &Value) -> Vec<Offer> {
    init["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let model = m["value"].as_str()?;
            (model != "default" && !model.starts_with("claude-")).then(|| Offer {
                harness: "claude".into(),
                model: model.to_owned(),
                name: m["displayName"]
                    .as_str()
                    .map_or_else(|| claude_name(model), str::to_owned),
                provider: "anthropic".into(),
                efforts: m["supportedEffortLevels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str().map(str::to_owned))
                    .collect(),
                default: false,
                usage: None,
            })
        })
        .collect()
}

/// "claude-opus-5-5" reads "Opus 5.5"; an alias such as "opus" carries no
/// version, so it reads "Opus".
fn claude_name(id: &str) -> String {
    let id = id.strip_prefix("claude-").unwrap_or(id);
    let (words, version): (Vec<&str>, Vec<&str>) = id
        .split('-')
        .partition(|p| !p.chars().all(|c| c.is_ascii_digit()));
    let mut name = words
        .iter()
        .map(|w| titled(w))
        .collect::<Vec<_>>()
        .join(" ");
    if !version.is_empty() {
        name = format!("{name} {}", version.join("."));
    }
    name
}

fn titled(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Codex's own list, from its app-server's `model/list`.
fn codex() -> Result<Vec<Offer>, String> {
    let codex = std::env::var("LIBRI_CODEX").unwrap_or_else(|_| "codex".into());
    let mut cmd = Command::new(codex);
    cmd.arg("app-server");
    let s = Session::spawn(cmd, false)?;
    let mut aside = Vec::new();
    let listed = (|| {
        s.call(
            "initialize",
            json!({"clientInfo": {"name": "libri", "version": env!("CARGO_PKG_VERSION")}}),
            LISTING,
            &mut aside,
        )?;
        s.peer.notify("initialized", json!({}))?;
        let mut all = Vec::new();
        let mut cursor = Value::Null;
        loop {
            let page = s.call("model/list", json!({"cursor": cursor}), LISTING, &mut aside)?;
            all.extend(codex_offers(&page));
            cursor = page["nextCursor"].clone();
            if !cursor.is_string() {
                return Ok(all);
            }
        }
    })();
    s.end();
    listed
}

fn codex_offers(page: &Value) -> Vec<Offer> {
    let Some(data) = page["data"].as_array() else {
        return Vec::new();
    };
    data.iter()
        .filter(|m| m["hidden"] != true)
        .filter_map(|m| {
            let model = m["model"].as_str().or(m["id"].as_str())?;
            Some(Offer {
                harness: "codex".into(),
                model: model.to_owned(),
                name: m["displayName"].as_str().unwrap_or(model).to_owned(),
                provider: "openai".into(),
                efforts: m["supportedReasoningEfforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e["reasoningEffort"].as_str().map(str::to_owned))
                    .collect(),
                default: m["isDefault"] == true,
                usage: None,
            })
        })
        .collect()
}

/// A lister's output. Each harness reads its own credentials: Libri holds
/// no keys.
fn run(command: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(command);
    cmd.args(args).stdin(std::process::Stdio::null());
    let out = cmd.output().map_err(|e| format!("{command}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        // Its own words, not the stack trace under them.
        let said = err
            .lines()
            .find(|l| l.contains("Error"))
            .or_else(|| err.lines().last())
            .unwrap_or("");
        return Err(format!("{command} {}: {said}", args.join(" ")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// OpenCode's own list, `provider/model` a line.
fn opencode() -> Result<Vec<Offer>, String> {
    // Through Libri's own server: a private one would bind a port the
    // unit may not allow.
    let out = crate::opencode::OpenCode::from_env()?.models()?;
    Ok(opencode_offers(&out))
}

fn opencode_offers(out: &str) -> Vec<Offer> {
    out.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (provider, id) = l.split_once('/')?;
            (!provider.contains(char::is_whitespace)).then(|| Offer {
                harness: "opencode".into(),
                model: l.to_owned(),
                name: id.to_owned(),
                provider: provider.to_owned(),
                efforts: Vec::new(),
                default: false,
                usage: None,
            })
        })
        .collect()
}

/// omp's own list, the chat models of the providers it can reach.
/// Whether `harness` reaches any model with the credentials it holds.
pub fn reaches(harness: &str) -> bool {
    match harness {
        "opencode" => opencode(),
        "omp" => omp(),
        "pi" => pi(),
        "muse" => muse(),
        _ => return true,
    }
    .is_ok_and(|o| !o.is_empty())
}

/// Pi's own list, from its RPC `get_available_models`.
fn pi() -> Result<Vec<Offer>, String> {
    Ok(pi_offers(&crate::pi::Pi::from_env()?.models()?))
}

/// The thinking levels offered on a Pi model that reasons: those every
/// provider of Pi's takes. Pi has more (`--thinking`), per model.
const PI_EFFORTS: &[&str] = &["low", "medium", "high"];

fn pi_offers(models: &[Value]) -> Vec<Offer> {
    models
        .iter()
        .filter_map(|m| {
            let (provider, id) = (m["provider"].as_str()?, m["id"].as_str()?);
            Some(Offer {
                harness: "pi".into(),
                model: format!("{provider}/{id}"),
                name: m["name"].as_str().unwrap_or(id).to_owned(),
                provider: provider.to_owned(),
                efforts: if m["reasoning"] == true {
                    PI_EFFORTS.iter().map(|e| (*e).to_owned()).collect()
                } else {
                    Vec::new()
                },
                default: false,
                usage: None,
            })
        })
        .collect()
}

/// Muse Code's own list, from its MSP host's `model/list`.
fn muse() -> Result<Vec<Offer>, String> {
    Ok(muse_offers(&crate::muse::Muse::from_env()?.models()?))
}

fn muse_offers(listed: &Value) -> Vec<Offer> {
    listed["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let model = m["modelId"].as_str()?;
            Some(Offer {
                harness: "muse".into(),
                model: model.to_owned(),
                name: m["displayLabel"].as_str().unwrap_or(model).to_owned(),
                provider: m["providerId"].as_str().unwrap_or("meta").to_owned(),
                // "unknown" when Muse doesn't say: its tiers, as Paseo
                // offers them then.
                efforts: match m["variants"].as_array() {
                    Some(v) => v
                        .iter()
                        .filter_map(|e| e.as_str().map(str::to_owned))
                        .collect(),
                    None => crate::muse::EFFORTS
                        .iter()
                        .map(|e| (*e).to_owned())
                        .collect(),
                },
                default: m["isDefault"] == true,
                usage: None,
            })
        })
        .collect()
}

fn omp() -> Result<Vec<Offer>, String> {
    let command = std::env::var("LIBRI_OMP").unwrap_or_else(|_| "omp".into());
    let out = run(&command, &["models", "--json"])?;
    let v: Value = serde_json::from_str(&out).map_err(|e| format!("omp models: {e}"))?;
    Ok(omp_offers(&v))
}

fn omp_offers(v: &Value) -> Vec<Offer> {
    v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["kind"] == "chat")
        .filter_map(|m| {
            let model = m["selector"].as_str()?;
            Some(Offer {
                harness: "omp".into(),
                model: model.to_owned(),
                name: m["name"].as_str().unwrap_or(model).to_owned(),
                provider: m["provider"].as_str().unwrap_or("").to_owned(),
                efforts: m["thinking"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str().map(str::to_owned))
                    .collect(),
                default: false,
                usage: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omp_routes_openai_and_api_providers_but_excludes_restricted_logins() {
        for provider in [
            "openai",
            "openai-codex",
            "deepseek",
            "xai",
            "google",
            "zai",
            "kimi-code",
            "opencode-go",
        ] {
            let mut o = offer("model", &[]);
            o.harness = "omp".into();
            o.provider = provider.into();
            assert!(provider_allowed(&o, true, true, true), "{provider}");
        }
        for provider in [
            "anthropic",
            "google-gemini-cli",
            "google-antigravity",
            "meta",
        ] {
            let mut o = offer("model", &[]);
            o.harness = "omp".into();
            o.provider = provider.into();
            assert!(!provider_allowed(&o, false, false, false), "{provider}");
        }
        assert_eq!(OTHERS[0], "omp");
    }

    fn offer(model: &str, efforts: &[&str]) -> Offer {
        Offer {
            harness: default_harness(),
            model: model.into(),
            name: model.into(),
            provider: "anthropic".into(),
            efforts: efforts.iter().map(|e| (*e).to_owned()).collect(),
            default: false,
            usage: None,
        }
    }

    #[test]
    fn the_last_harness_on_stays_on() {
        let state = scratch("last-on");
        let only = configured_default();
        let before = off(&state);
        assert!(switch(&state, &only, false).is_err());
        assert_eq!(off(&state), before);
        assert!(switch(&state, "no-such-harness", false).is_err());
    }

    #[test]
    fn a_default_model_is_held_per_harness_and_cleared_by_empty() {
        let state = scratch("default-model");
        set_default_model(&state, "pi", "opencode-go/deepseek-v4-pro").unwrap();
        set_default_model(&state, "muse", "muse-1").unwrap();
        let all = default_models(&state);
        assert_eq!(all["pi"], "opencode-go/deepseek-v4-pro");
        set_default_model(&state, "pi", "").unwrap();
        let all = default_models(&state);
        assert!(!all.contains_key("pi"));
        assert_eq!(all["muse"], "muse-1");
    }

    #[test]
    fn harnesses_switched_off_are_read_one_a_line() {
        let state = scratch("off-lines");
        OFF.write(&state, "pi\nmuse\n").unwrap();
        assert_eq!(off(&state), ["pi", "muse"]);
    }

    #[test]
    fn pi_is_off_until_switched_on_and_stays_on() {
        let state = scratch("off-at-first");
        assert_eq!(off(&state), ["pi"]);
        OFF.write(&state, "").unwrap();
        assert!(off(&state).is_empty());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("catalog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pick_of(model: &str) -> Pick {
        Pick {
            harness: default_harness(),
            model: model.into(),
        }
    }

    fn on(harness: &str, model: &str, name: &str, efforts: &[&str]) -> Offer {
        Offer {
            harness: harness.into(),
            model: model.into(),
            name: name.into(),
            provider: String::new(),
            efforts: efforts.iter().map(|e| (*e).to_owned()).collect(),
            default: false,
            usage: None,
        }
    }

    /// As one install listed them on 2026-11-11, with the names that once
    /// had to be guessed.
    fn listed_once() -> Vec<Offer> {
        let five = ["low", "medium", "high", "xhigh", "max"];
        vec![
            on("claude", "opus", "Opus 5.5", &five),
            on("claude", "fable", "Fable 5.2", &five),
            on("claude", "haiku", "Haiku 5.5", &five),
            on("codex", "gpt-6.1-sol", "GPT-6.1-Sol", &five),
            on("codex", "gpt-6-sol", "GPT-6-Sol", &five),
            on("codex", "gpt-5.6-sol", "GPT-5.6-Sol", &five),
            on("codex", "gpt-6.1-terra", "GPT-6.1 Terra", &five),
            on("muse", "muse-wonder-1.0", "muse-wonder-1.0", &five),
            on(
                "muse",
                "muse-wonder-full-1.0",
                "muse-wonder-full-1.0",
                &five,
            ),
            on("muse", "muse-spark-1.3", "muse-spark-1.3", &five),
            on("omp", "openai-codex/gpt-6.1-sol", "GPT-6.1 Sol", &five),
            on(
                "omp",
                "opencode-zen/claude-opus-5-5",
                "Claude Opus 5.5",
                &five,
            ),
            on(
                "omp",
                "opencode-zen/claude-fable-5-2",
                "Claude Fable 5.2",
                &five,
            ),
            on(
                "omp",
                "opencode-zen/claude-haiku-5-5",
                "Claude Haiku 5.5",
                &five,
            ),
            on(
                "omp",
                "opencode-go/claude-haiku-5-5",
                "Claude Haiku 5.5",
                &five,
            ),
            on(
                "omp",
                "opencode-zen/glm-5.3",
                "GLM-5.3",
                &["low", "high", "max"],
            ),
            on("omp", "local/plain", "Plain", &[]),
        ]
    }

    #[test]
    fn each_model_answers_to_its_id_its_name_and_its_short_name() {
        let all = listed_once();
        let means = |named: &str| {
            resolve(&all, named)
                .map(|o| format!("{}:{}", o.harness, o.model))
                .unwrap_or_else(|e| e)
        };
        for (named, meant) in [
            ("opus", "claude:opus"),
            ("Opus 5.5", "claude:opus"),
            ("claude-opus-5-5", "claude:opus"),
            ("fable", "claude:fable"),
            ("Fable 5.2", "claude:fable"),
            ("claude-fable-5-2", "claude:fable"),
            ("haiku", "claude:haiku"),
            ("gpt-6.1-sol", "codex:gpt-6.1-sol"),
            ("GPT-6.1 Sol", "codex:gpt-6.1-sol"),
            ("sol", "codex:gpt-6.1-sol"),
            ("Sol", "codex:gpt-6.1-sol"),
            ("gpt-6-sol", "codex:gpt-6-sol"),
            ("muse-wonder-full-1.0", "muse:muse-wonder-full-1.0"),
            ("Muse Wonder Full", "muse:muse-wonder-full-1.0"),
            ("wonder-full", "muse:muse-wonder-full-1.0"),
            ("wonder", "muse:muse-wonder-1.0"),
            ("spark", "muse:muse-spark-1.3"),
            ("openai-codex/gpt-6.1-sol", "omp:openai-codex/gpt-6.1-sol"),
            (
                "opencode-zen/claude-opus-5-5",
                "omp:opencode-zen/claude-opus-5-5",
            ),
            ("glm", "omp:opencode-zen/glm-5.3"),
        ] {
            assert_eq!(means(named), meant, "{named}");
        }
    }

    #[test]
    fn a_name_that_fits_none_or_several_is_refused_with_what_is_on_offer() {
        let all = listed_once();
        let none = resolve(&all, "muse").unwrap_err();
        assert!(none.contains("not a model on offer"), "{none}");
        assert!(
            none.contains("muse: muse-wonder-1.0, muse-wonder-full-1.0"),
            "{none}"
        );
        assert!(none.contains("claude: opus, fable, haiku"), "{none}");
        // Two routers, one name and version: no dedicated harness to prefer.
        let mut routed = all.clone();
        routed.retain(|o| o.harness == "omp");
        let both = resolve(&routed, "Claude Haiku 5.5").unwrap_err();
        assert!(both.contains("opencode-zen/claude-haiku-5-5"), "{both}");
        assert!(both.contains("opencode-go/claude-haiku-5-5"), "{both}");
    }

    #[test]
    fn a_thinking_level_is_checked_against_the_model() {
        let all = listed_once();
        let sol = resolve(&all, "sol").unwrap();
        assert_eq!(level(sol, "High").unwrap(), "high");
        assert!(
            level(sol, "ultra")
                .unwrap_err()
                .contains("low, medium, high, xhigh, max")
        );
        let plain = resolve(&all, "local/plain").unwrap();
        assert!(
            level(plain, "high")
                .unwrap_err()
                .contains("takes no thinking level")
        );
    }

    #[test]
    fn a_named_worker_gets_the_level_asked_else_mains_when_it_has_it() {
        let dir = scratch("named-effort");
        remember(&dir, &listed_once());
        pick(
            &dir,
            &Pick {
                harness: "claude".into(),
                model: "opus".into(),
            },
        )
        .unwrap();
        crate::model::EFFORT.write(&dir, "xhigh").unwrap();
        let ask = |m: &str, e: Option<&str>| {
            worker_choice(&dir, Some(m), e).map(|(c, _)| (c.pick.model, c.effort))
        };
        assert_eq!(
            ask("Muse Wonder Full", Some("high")).unwrap(),
            ("muse-wonder-full-1.0".into(), Some("high".into()))
        );
        assert_eq!(
            ask("sol", None).unwrap(),
            ("gpt-6.1-sol".into(), Some("xhigh".into()))
        );
        assert_eq!(
            ask("glm", None).unwrap(),
            ("opencode-zen/glm-5.3".into(), None)
        );
        assert!(ask("glm", Some("medium")).is_err());
        assert!(ask("local/plain", Some("high")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_worker_takes_the_named_model_then_the_default_then_mains() {
        let dir = scratch("order");
        remember(
            &dir,
            &[
                offer("main", &["high"]),
                offer("cheap", &[]),
                offer("big", &[]),
            ],
        );
        pick(&dir, &pick_of("main")).unwrap();
        assert_eq!(
            worker_choice(&dir, None, None).unwrap().0.pick.model,
            "main"
        );
        worker_pick(&dir, Some(&pick_of("cheap"))).unwrap();
        let (chosen, words) = worker_choice(&dir, None, None).unwrap();
        assert_eq!((chosen.pick.model.as_str(), words), ("cheap", None));
        assert_eq!(
            worker_choice(&dir, Some("BIG"), None).unwrap().0.pick.model,
            "big"
        );
        assert!(worker_choice(&dir, Some("gone"), None).is_err());
        worker_pick(&dir, None).unwrap();
        assert_eq!(
            worker_choice(&dir, None, None).unwrap().0.pick.model,
            "main"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_default_takes_mains_thinking_only_when_it_has_that_level() {
        let dir = scratch("effort");
        remember(
            &dir,
            &[
                offer("main", &["high"]),
                offer("same", &["low", "high"]),
                offer("plain", &[]),
            ],
        );
        pick(&dir, &pick_of("main")).unwrap();
        crate::model::EFFORT.write(&dir, "high").unwrap();
        worker_pick(&dir, Some(&pick_of("same"))).unwrap();
        let held = worker_choice(&dir, None, None).unwrap().0;
        assert_eq!(held.effort.as_deref(), Some("high"));
        worker_pick(&dir, Some(&pick_of("plain"))).unwrap();
        assert_eq!(worker_choice(&dir, None, None).unwrap().0.effort, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_default_off_offer_gives_way_to_mains_model_and_says_so() {
        let dir = scratch("lost");
        remember(&dir, &[offer("main", &[])]);
        pick(&dir, &pick_of("main")).unwrap();
        worker_pick(&dir, Some(&pick_of("gone"))).unwrap();
        let (chosen, words) = worker_choice(&dir, None, None).unwrap();
        assert_eq!(chosen.pick.model, "main");
        assert!(words.unwrap().contains("gone"));
        // Still the pick, and a named model does not hear about it.
        assert_eq!(worker_picked(&dir), Some(pick_of("gone")));
        assert_eq!(worker_choice(&dir, Some("main"), None).unwrap().1, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn claude_names_carry_the_version() {
        assert_eq!(claude_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(claude_name("claude-fable-5-1"), "Fable 5.1");
        assert_eq!(claude_name("opus"), "Opus");
    }

    #[test]
    fn claude_offers_its_aliases_with_their_efforts() {
        let init = json!({"models": [
            {"value": "default", "displayName": "Default (recommended)"},
            {"value": "opus", "displayName": "Opus 5.5", "supportedEffortLevels": ["low", "max"]},
            {"value": "fable", "displayName": "Fable 5.2", "supportedEffortLevels": ["high"]},
            {"value": "haiku"},
            {"value": "claude-fable-5-1", "displayName": "Fable 5.1"}
        ]});
        let o = claude_offers(&init);
        let names: Vec<_> = o
            .iter()
            .map(|o| (o.model.as_str(), o.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("opus", "Opus 5.5"),
                ("fable", "Fable 5.2"),
                ("haiku", "Haiku")
            ]
        );
        assert_eq!(o[0].efforts, ["low", "max"]);
        assert!(o[2].efforts.is_empty());
    }

    #[test]
    fn codex_lists_shown_models_with_their_efforts() {
        let page = json!({"data": [
            {"id": "gpt-a", "model": "gpt-a", "displayName": "GPT-A", "hidden": false, "isDefault": true,
             "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "ultra"}]},
            {"id": "gpt-old", "model": "gpt-old", "hidden": true}
        ], "nextCursor": null});
        let o = codex_offers(&page);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].name, "GPT-A");
        assert_eq!(o[0].efforts, ["low", "ultra"]);
        assert!(o[0].default);
    }

    #[test]
    fn opencode_lines_become_offers() {
        let o = opencode_offers("opencode-go/glm-5.3\nnoise line\ndeepseek/deepseek-v4-pro\n");
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].provider, "opencode-go");
        assert_eq!(o[0].name, "glm-5.3");
    }

    #[test]
    fn omp_keeps_chat_models() {
        let v = json!({"models": [
            {"provider": "deepseek", "kind": "chat", "selector": "deepseek/flash", "name": "Flash", "thinking": ["minimal", "high"]},
            {"provider": "x", "kind": "image", "selector": "x/img", "name": "Img"}
        ]});
        let o = omp_offers(&v);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].efforts, ["minimal", "high"]);
    }

    #[test]
    fn pi_models_go_by_provider_and_id() {
        let models = [
            json!({"id": "deepseek-v4.1-flash", "name": "DeepSeek V4.1 Flash",
                "provider": "opencode-go", "reasoning": true}),
            json!({"id": "plain", "provider": "local", "reasoning": false}),
        ];
        let o = pi_offers(&models);
        assert_eq!(o[0].model, "opencode-go/deepseek-v4.1-flash");
        assert_eq!(o[0].efforts, PI_EFFORTS);
        assert_eq!(o[1].name, "plain");
        assert!(o[1].efforts.is_empty());
    }

    #[test]
    fn muse_lists_its_models_with_its_default() {
        let v = json!({"models": [
            {"modelId": "muse-wonder-1.0", "displayLabel": "muse-wonder-1.0",
             "providerId": "meta", "isDefault": false, "variants": "unknown"},
            {"modelId": "muse-spark", "displayLabel": "Spark", "providerId": "meta",
             "isDefault": true, "variants": ["low", "high"]}
        ]});
        let o = muse_offers(&v);
        assert_eq!(o[0].efforts.len(), crate::muse::EFFORTS.len());
        assert_eq!((o[1].name.as_str(), o[1].default), ("Spark", true));
        assert_eq!(o[1].efforts, ["low", "high"]);
    }
}
