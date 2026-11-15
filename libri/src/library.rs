//! What the Library and Tasks pages show of the assistant's own keeping:
//! the pages show_page published, the lists in the book, and the watches.
//!
//! A page is an MCP App view (`text/html;profile=mcp-app`, the
//! `io.modelcontextprotocol/ui` extension): one HTML file the Library runs
//! sandboxed, as the chat runs a tool's card, which talks to Libri only by
//! the extension's JSON-RPC over postMessage. Through it the page reads and
//! changes the data it keeps (page_data.rs), the same data the agent's
//! page_data tools reach, and puts a message in the person's composer.
//! The pages read and change these directly, with no model call: a snooze or
//! a delete is an edit of the data, as a phone's reminders app makes one.

use crate::book::{self, Book};
use crate::watch::{self, Op, Watch};
use chrono::{Local, NaiveDateTime, TimeZone};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// A published page, as the Library lists it.
#[derive(Debug, PartialEq, Serialize)]
pub struct Page {
    /// Its name, which is also its path on the site: `/{name}/`.
    pub name: String,
    /// The document's `<title>`, when it has one.
    pub title: Option<String>,
    /// When its latest version was published, in seconds since the epoch.
    pub changed: u64,
}

/// The longest stretch of a page read for its title: a title belongs in the
/// head, and a page can be megabytes.
const TITLE_WITHIN: u64 = 8 * 1024;

/// Every published page, the latest changed first.
pub fn pages(dir: &Path) -> Vec<Page> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut pages: Vec<Page> = entries
        .filter_map(Result::ok)
        .filter_map(|d| {
            let index = d.path().join("index.html");
            let changed = fs::metadata(&index).ok()?.modified().ok()?;
            Some(Page {
                name: d.file_name().into_string().ok()?,
                title: title_of(&index),
                changed: changed.duration_since(UNIX_EPOCH).ok()?.as_secs(),
            })
        })
        .collect();
    pages.sort_by(|a, b| b.changed.cmp(&a.changed).then(a.name.cmp(&b.name)));
    pages
}

fn title_of(index: &Path) -> Option<String> {
    let mut head = String::new();
    fs::File::open(index)
        .ok()?
        .take(TITLE_WITHIN)
        .read_to_string(&mut head)
        .ok()?;
    let lower = head.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let title = unescape(
        head[start..end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    );
    (!title.is_empty()).then_some(title)
}

/// The few entities a title uses; the rest show as written.
fn unescape(s: String) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// The address MCP Apps names a page's view by.
pub const PAGE_URI: &str = "ui://libri/pages/";

/// Where a page's outside origins are kept, beside its versions.
const CSP_FILE: &str = ".csp.json";
/// The connections a page reads from, as show_page named them.
const READS_FILE: &str = ".connections.json";

/// show_page's description: how a page talks to Libri, which the model
/// needs whole to write one.
pub const SHOW_PAGE: &str = "Publish an interactive page in the person's Library, at a name, and get its link to send; the same name again replaces it, and older versions are kept. Use it when a chat message would be too long or too flat: a chart, a table, a plan, an explainer with pictures in it, or something to use together, as a todo list both of you check off. The page is one HTML document (styles and scripts inline) that Libri runs sandboxed as an MCP App: no cookies, storage or network, and no outside files unless listed in resource_domains. It keeps a JSON object of data that you and the person both change: page_data reads it, page_data_change merges a patch into it. Keep anything both of you edit in objects keyed by id (a patch replaces an array whole). The page reaches the same data by MCP Apps JSON-RPC over postMessage to window.parent: send request ui/initialize {appInfo:{name,version}, appCapabilities:{}, protocolVersion:\"2026-01-26\"}, and on its answer the notification ui/notifications/initialized; then request tools/call {name, arguments} with page_data {}, page_data_change {patch}, or page_data_watch {since: version}, which answers once the data is past that version or after 25 s, so call it in a loop to stay live. Each answers with structuredContent {version, data, by} (by: person or agent). A page that names connections in `connections` may also call their read tools by the names you know them by (mcp__NAME__tool), e.g. a calendar's search for an agenda; it can read, never send, change or delete. Request ui/message {role:\"user\", content:[{type:\"text\", text}]} to put a message in the person's composer. The initialize answer's hostContext.styles.variables holds the chat's colours and type (--color-background-primary, --color-text-primary, --color-text-secondary, --color-border-primary, --font-sans, ...), and hostContext.styles.css.fonts the @font-face rules for its fonts, to add in a <style>; use them so the page matches. Libri shows the page's title above it, so leave the title out of the page.";

/// The origins a page may reach outside Libri, as MCP Apps' `ui.csp`
/// names them; none by default.
#[derive(Debug, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Csp {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connect_domains: Vec<String>,
}

/// A page's view: its HTML, and the origins it declared.
pub struct View {
    pub html: String,
    pub csp: Csp,
}

/// The one path a model chooses, held to a plain label.
fn plain(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 40
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-');
    if !ok {
        return Err("The name is lowercase letters, digits and dashes, 40 at most.".into());
    }
    Ok(())
}

/// Whether `name` is a published page; an error saying so if not.
pub fn held(dir: &Path, name: &str) -> Result<(), String> {
    plain(name)?;
    if !dir.join(name).join("index.html").is_file() {
        return Err(format!("There is no page {name}; show_page publishes one."));
    }
    Ok(())
}

/// Where the person opens a page: in the Library, on the chat's own site.
pub fn link(url: &str, name: &str) -> String {
    format!("{}/#page={name}", url.trim_end_matches('/'))
}

/// An origin as CSP takes one: a scheme and a host, nothing after.
fn origin(o: &str) -> Result<(), String> {
    let rest = o
        .strip_prefix("https://")
        .or_else(|| o.strip_prefix("http://"));
    let ok = rest.is_some_and(|h| {
        !h.is_empty()
            && h.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-.:*".contains(&b))
    });
    if !ok {
        return Err(format!(
            "{o} is not an origin: give a scheme and host, as https://cdn.example.com."
        ));
    }
    Ok(())
}

/// Writes a page under its name: a dated copy that stays, and index.html
/// swapped in whole, with the origins it declared beside them.
pub fn publish(
    dir: &Path,
    url: &str,
    name: &str,
    html: &str,
    csp: &Csp,
    reads: &[String],
) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    plain(name)?;
    // The web chat shares the host and owns this path.
    if name == "api" {
        return Err(format!("{name} is taken; choose another name."));
    }
    for o in csp.resource_domains.iter().chain(&csp.connect_domains) {
        origin(o)?;
    }
    for r in reads {
        let ok = !r.is_empty()
            && r.len() <= 40
            && r.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !ok {
            return Err(format!("{r} is not a connection's name."));
        }
    }
    // The site directory is the assistant's and the proxy's alone, so
    // inside it everything is world-readable: the sandbox's uid map
    // cannot hand files to the proxy's group.
    let page = dir.join(name);
    fs::create_dir_all(&page).map_err(|e| e.to_string())?;
    fs::set_permissions(&page, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let readable = fs::Permissions::from_mode(0o644);
    let write = |to: &Path, text: &str| -> Result<(), String> {
        let tmp = to.with_extension("tmp");
        fs::write(&tmp, text).map_err(|e| e.to_string())?;
        fs::set_permissions(&tmp, readable.clone()).map_err(|e| e.to_string())?;
        fs::rename(&tmp, to).map_err(|e| e.to_string())
    };
    write(&page.join(format!("{stamp}.html")), html)?;
    let declared = page.join(CSP_FILE);
    if *csp == Csp::default() {
        let _ = fs::remove_file(&declared);
    } else {
        write(
            &declared,
            &serde_json::to_string(csp).map_err(|e| e.to_string())?,
        )?;
    }
    let named = page.join(READS_FILE);
    if reads.is_empty() {
        let _ = fs::remove_file(&named);
    } else {
        write(
            &named,
            &serde_json::to_string(reads).map_err(|e| e.to_string())?,
        )?;
    }
    write(&page.join("index.html"), html)?;
    Ok(format!(
        "Published: {} (this version is kept as {stamp})",
        link(url, name)
    ))
}

/// The connections a published page may read from, by name.
pub fn reads(dir: &Path, name: &str) -> Vec<String> {
    fs::read_to_string(dir.join(name).join(READS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// The view of the page an MCP Apps `ui://libri/pages/<name>` names.
pub fn view(dir: &Path, uri: &str) -> Result<View, String> {
    let name = uri
        .strip_prefix(PAGE_URI)
        .ok_or_else(|| format!("{uri} is not a page."))?;
    held(dir, name)?;
    let page = dir.join(name);
    let html = fs::read_to_string(page.join("index.html")).map_err(|e| e.to_string())?;
    let csp = fs::read_to_string(page.join(CSP_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    Ok(View { html, csp })
}

/// A list, as the Library lists it.
#[derive(Debug, PartialEq, Serialize)]
pub struct List {
    pub name: String,
    pub items: Vec<String>,
    /// Always false: apps before 0.9.31 need the field to read the
    /// Library. Remove once no app before 0.9.31 is in use.
    pub shared: bool,
}

/// The person's lists.
pub fn lists(state: &Path) -> Result<Vec<List>, String> {
    let on = Book::open(state)?.read(book::lists)?;
    Ok(on
        .into_iter()
        .map(|(name, items)| List {
            name,
            items,
            shared: false,
        })
        .collect())
}

/// A watch, as the Tasks page lists it.
#[derive(Debug, PartialEq, Serialize)]
pub struct Shown {
    pub id: u64,
    /// When it next fires, in seconds since the epoch; None for one a push
    /// wakes.
    pub at: Option<u64>,
    /// How often it repeats, as the watch tool names it (`daily`, `30m`).
    pub every: Option<String>,
    /// `note`, sent as worded, or `ask`, carried out by the model.
    pub kind: &'static str,
    pub text: String,
    /// The name of the worker whose thread takes it; None is the main chat.
    pub worker: Option<String>,
    pub active: bool,
    /// The server and tool it reads before firing, when it looks.
    pub looks: Option<String>,
    /// Why its last look failed, while it is failing.
    pub failing: Option<String>,
    /// The look's arguments, as JSON, when it has any.
    pub arguments: Option<String>,
    /// The tests an item must pass, each as `field op value`.
    pub tests: Vec<String>,
    /// The fields of an item its turn sees.
    pub fields: Vec<String>,
    /// Who else a turn's answer goes to.
    pub shared: Vec<String>,
    /// A push wakes it, at its secret address (not shown).
    pub pushed: bool,
}

fn shown(state: &Path, w: Watch) -> Shown {
    Shown {
        id: w.id,
        at: w.hook.is_none().then_some(w.at).flatten().and_then(epoch),
        every: w.every.map(|e| e.name()),
        kind: if w.turn { "ask" } else { "note" },
        looks: w.look.as_ref().map(|l| format!("{} {}", l.server, l.tool)),
        arguments: w
            .look
            .as_ref()
            .filter(|l| !l.arguments.is_empty())
            .map(|l| Value::Object(l.arguments.clone()).to_string()),
        tests: w
            .tests
            .iter()
            .map(|t| {
                let op = match t.op {
                    Op::Is => "is",
                    Op::Has => "has",
                    Op::Over => "over",
                    Op::Under => "under",
                };
                format!("{} {op} {}", t.field, t.value)
            })
            .collect(),
        fields: w.fields,
        shared: w.share_with,
        pushed: w.hook.is_some(),
        text: w.text,
        // The book keeps the worker's id; people know it by its name.
        worker: w.worker.map(|id| crate::task::shown(state, &id)),
        active: w.active,
        failing: w.error,
    }
}

fn epoch(t: NaiveDateTime) -> Option<u64> {
    Local
        .from_local_datetime(&t)
        .earliest()
        .and_then(|t| u64::try_from(t.timestamp()).ok())
}

fn local(secs: u64) -> Option<NaiveDateTime> {
    Local
        .timestamp_opt(i64::try_from(secs).ok()?, 0)
        .single()
        .map(|t| t.naive_local())
}

/// Every watch, soonest first, those a push wakes last.
pub fn watches(state: &Path) -> Result<Vec<Shown>, String> {
    let all = Book::open(state)?.read(watch::all)?;
    Ok(all.into_iter().map(|w| shown(state, w)).collect())
}

/// Stops and forgets a watch. False when there was none by `id`.
pub fn delete(state: &Path, id: u64) -> Result<bool, String> {
    Book::open(state)?.change(|db| watch::remove(db, id))
}

/// Moves a timed watch's next firing to `until` (seconds since the epoch);
/// a repeating one goes on from there. A push has no time to move.
pub fn snooze(state: &Path, id: u64, until: u64) -> Result<(), String> {
    let at = local(until).ok_or("That time is out of range.")?;
    if at <= Local::now().naive_local() {
        return Err("A snooze lies ahead.".into());
    }
    Book::open(state)?.change(|db| {
        let Some(mut w) = watch::get(db, id)? else {
            return Ok(Err(format!("There is no watch {id}.")));
        };
        if w.hook.is_some() {
            return Ok(Err("A push has no time to move.".into()));
        }
        w.at = Some(at);
        watch::change(db, &w)?;
        Ok(Ok(()))
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("libri-library-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn pages_list_newest_first_with_their_titles() {
        let d = dir("pages");
        for (name, html) in [
            (
                "old",
                "<html><head><TITLE>Old &amp; kept</TITLE></head></html>",
            ),
            ("new", "<p>no title</p>"),
        ] {
            fs::create_dir_all(d.join(name)).unwrap();
            fs::write(d.join(name).join("index.html"), html).unwrap();
        }
        // A folder with no page in it is no page.
        fs::create_dir_all(d.join("empty")).unwrap();
        let old = fs::File::options()
            .write(true)
            .open(d.join("old/index.html"))
            .unwrap();
        old.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_000))
            .unwrap();
        let got = pages(&d);
        assert_eq!(got.len(), 2);
        assert_eq!(
            (got[0].name.as_str(), got[0].title.as_deref()),
            ("new", None)
        );
        assert_eq!(
            (got[1].name.as_str(), got[1].title.as_deref()),
            ("old", Some("Old & kept"))
        );
        assert_eq!(got[1].changed, 1_000);
    }

    #[test]
    fn a_watch_snoozes_and_deletes_without_the_model() {
        let d = dir("watches");
        let at = Local::now().naive_local() + chrono::Duration::hours(1);
        let id = Book::open(&d)
            .unwrap()
            .change(|db| {
                watch::add(
                    db,
                    &Watch {
                        at: Some(at),
                        text: "Call the dentist".into(),
                        active: true,
                        ..Watch::default()
                    },
                )
            })
            .unwrap();
        let listed = watches(&d).unwrap();
        assert_eq!(
            (listed[0].kind, listed[0].text.as_str()),
            ("note", "Call the dentist")
        );
        let later = listed[0].at.unwrap() + 3 * 3600;
        snooze(&d, id, later).unwrap();
        assert_eq!(watches(&d).unwrap()[0].at, Some(later));
        assert!(snooze(&d, id, 1).is_err(), "a snooze into the past");
        assert!(delete(&d, id).unwrap());
        assert!(watches(&d).unwrap().is_empty());
        assert!(!delete(&d, id).unwrap());
    }

    #[test]
    fn a_watch_lists_everything_it_holds() {
        let d = dir("watch-details");
        Book::open(&d)
            .unwrap()
            .change(|db| {
                watch::add(
                    db,
                    &Watch {
                        at: Some(Local::now().naive_local() + chrono::Duration::hours(1)),
                        every: watch::Every::parse("30m"),
                        turn: true,
                        text: "Tell me of cheap flights".into(),
                        worker: Some("Wren".into()),
                        share_with: vec!["Ada".into()],
                        active: true,
                        look: Some(crate::look::Look {
                            server: "travel".into(),
                            tool: "fares".into(),
                            arguments: serde_json::json!({"to": "LIS"})
                                .as_object()
                                .unwrap()
                                .clone(),
                            items: None,
                            key: "id".into(),
                        }),
                        tests: vec![watch::Test {
                            field: "price".into(),
                            op: Op::Under,
                            value: "200".into(),
                        }],
                        fields: vec!["price".into()],
                        ..Watch::default()
                    },
                )
            })
            .unwrap();
        let w = &watches(&d).unwrap()[0];
        assert_eq!(w.every.as_deref(), Some("30m"));
        assert_eq!(w.looks.as_deref(), Some("travel fares"));
        assert_eq!(w.arguments.as_deref(), Some(r#"{"to":"LIS"}"#));
        assert_eq!(w.tests, ["price under 200"]);
        assert_eq!(w.fields, ["price"]);
        assert_eq!(w.shared, ["Ada"]);
        assert!(!w.pushed);
    }

    #[test]
    fn a_page_is_published_under_a_plain_name_only() {
        let d = dir("publish");
        let url = "https://ada.example";
        let none = Csp::default();
        let said = publish(&d, url, "trip-plan", "<p>hi</p>", &none, &[]).unwrap();
        assert!(said.starts_with("Published: https://ada.example/#page=trip-plan "));
        assert!(d.join("trip-plan/index.html").is_file());
        assert_eq!(fs::read_dir(d.join("trip-plan")).unwrap().count(), 2);
        assert!(publish(&d, url, "trip-plan", "<p>again</p>", &none, &[]).is_ok());
        assert_eq!(
            view(&d, "ui://libri/pages/trip-plan").unwrap().html,
            "<p>again</p>"
        );
        for bad in ["../etc", "Trip", "", "-x", "api"] {
            assert!(publish(&d, url, bad, "x", &none, &[]).is_err(), "{bad}");
        }
        assert!(view(&d, "ui://libri/pages/../etc").is_err());
        assert!(view(&d, "ui://libri/pages/missing").is_err());
        assert!(view(&d, "ui://libri/browser").is_err());
    }

    #[test]
    fn a_page_keeps_the_origins_it_declares() {
        let d = dir("csp");
        let url = "https://ada.example";
        let cdn = Csp {
            resource_domains: vec!["https://cdn.example.com".into()],
            connect_domains: vec![],
        };
        publish(&d, url, "chart", "<p>c</p>", &cdn, &[]).unwrap();
        assert_eq!(view(&d, "ui://libri/pages/chart").unwrap().csp, cdn);
        // Published again with none, it declares none.
        publish(&d, url, "chart", "<p>c</p>", &Csp::default(), &[]).unwrap();
        assert_eq!(
            view(&d, "ui://libri/pages/chart").unwrap().csp,
            Csp::default()
        );
        let bad = Csp {
            resource_domains: vec!["https://x.com; script-src *".into()],
            connect_domains: vec![],
        };
        assert!(publish(&d, url, "chart", "x", &bad, &[]).is_err());
    }
}
