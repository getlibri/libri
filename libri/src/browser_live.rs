//! The Browser tile's side of Libri's browser: the live view the person
//! browses in, shared with the agents that browse the same tabs.
//!
//! The pattern is the hosted agent browsers' live view (Browserbase, Steel)
//! and Chrome's own remote screen: the page is laid out at the size of the
//! screen that shows it (`Emulation.setDeviceMetricsOverride`), DevTools
//! casts it as JPEG frames only when it changes (`Page.startScreencast`),
//! and the person's mouse, wheel, keys and text go back as DevTools input
//! events. One WebSocket carries both ways: frames out as binary messages,
//! the browser's state and one-off news (text copied, a file asked for, a
//! download done) out as text, the person's input in as text. A frame the
//! socket could not take yet is dropped for the next, so a slow link shows
//! the newest picture rather than a backlog.
//!
//! Frames rather than video (WebRTC) because a page is still most of the
//! time: the cast sends nothing while nothing moves, needs no encoder
//! process on the host, and DevTools' acknowledgement already paces it to
//! what the viewer takes. Video would win for a page that plays video.
//!
//! The person's input takes the browser from the agents (`Holder::Person`),
//! as the card's Take control does; a hold taken by input alone lapses
//! after PERSON_LAPSE without input, unless an agent waits on the person
//! (`browser_hand_over`), so a tile left open never keeps the agents out.

use super::{Browser, Cdp, HEIGHT, Holder, TabInfo, WIDTH, attach, plain_url};
use axum::extract::ws::{Message, WebSocket};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A hold the person took by browsing alone ends this long after their
/// last input, so the agents get the browser back.
const PERSON_LAPSE: Duration = Duration::from_secs(90);

/// The largest picture cast, in device pixels, whatever the screen; the
/// virtual display has room for a window this size (browser.rs).
const MOST: f64 = 2400.0;

/// What the live views share with the browser.
#[derive(Default)]
pub struct Shared {
    /// Each open view's screen: CSS width and height, pixel ratio, and
    /// whether it is a phone. The newest to report its size lays the page
    /// out.
    views: HashMap<u64, Screen>,
    newest: Option<u64>,
    next: u64,
    /// The context agents browse in without the person's logins.
    pub throwaway: Option<String>,
    /// When the person's last input came, for a hold taken by input.
    pub person_at: Option<Instant>,
    /// The tabs agents opened with the person's logins, where they open
    /// pages while they have them (browser.rs `ask`).
    pub signed_in: HashSet<String>,
    /// Downloads under way: DevTools' guid to the name the site gave.
    downloads: HashMap<String, String>,
    /// The shown tab's page size as `fit` laid it out, in CSS pixels. The
    /// window can be wider than the page (Chromium keeps it 500 points wide
    /// at least), and the cast takes in all of it.
    pub page: Option<(f64, f64)>,
    /// The tab session and screen the cast was last set for: set again
    /// unchanged, a cast restarts and may wait for a change to paint.
    applied: Option<(String, Screen)>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
struct Screen {
    width: u32,
    height: u32,
    #[serde(default = "one")]
    scale: f64,
    #[serde(default)]
    mobile: bool,
}

fn one() -> f64 {
    1.0
}

impl Shared {
    fn screen(&self) -> Option<Screen> {
        self.newest.and_then(|n| self.views.get(&n)).copied()
    }
}

/// Lays the shown tab out for the live views' screen and casts it at that
/// size; with no view open, at the browser's own size.
pub(super) async fn fit(b: &Browser, cdp: &Cdp, session: &str) -> Result<(), String> {
    let s = Some(session);
    // A tab in the person's own browser keeps their window as it is.
    if b.status().yours {
        b.live.lock().unwrap().page = None;
        let _ = cdp.call("Page.stopScreencast", json!({}), s).await;
        cdp.call(
            "Page.startScreencast",
            json!({"format": "jpeg", "quality": 60, "maxWidth": WIDTH, "maxHeight": HEIGHT}),
            s,
        )
        .await?;
        return Ok(());
    }
    let screen = b.live.lock().unwrap().screen().unwrap_or(Screen {
        width: WIDTH,
        height: HEIGHT,
        scale: 1.0,
        mobile: false,
    });
    {
        let mut l = b.live.lock().unwrap();
        let now = Some((session.to_owned(), screen));
        if l.applied == now {
            return Ok(());
        }
        l.applied = now;
    }
    let _ = cdp.call("Page.stopScreencast", json!({}), s).await;
    let (width, height) = (
        screen.width.clamp(200, 4000),
        screen.height.clamp(200, 4000),
    );
    // No finer than the browser draws (2x, browser.rs): a 3x phone gets 2x.
    let scale = screen.scale.clamp(1.0, 2.0);
    cdp.call(
        "Emulation.setDeviceMetricsOverride",
        json!({"width": width, "height": height, "deviceScaleFactor": scale, "mobile": screen.mobile}),
        s,
    )
    .await?;
    let fit = (MOST / (width as f64 * scale).max(height as f64 * scale)).min(1.0);
    let (mut w, mut h) = (
        (width as f64 * scale * fit) as u32,
        (height as f64 * scale * fit) as u32,
    );
    // Chromium casts no more of the page than its window shows: the window
    // grows to the page's size, and the toolbar's room above it.
    let target = b.tab.lock().unwrap().as_ref().map(|(_, t)| t.clone());
    if let Some(t) = target
        && let Ok(win) = cdp
            .call("Browser.getWindowForTarget", json!({"targetId": t}), None)
            .await
    {
        // In the display's points: the browser draws at 2x (browser.rs), so
        // the window holds the page's CSS size and the toolbar.
        let bounds = json!({"left": 0, "top": 0, "width": width.max(400), "height": height + 120});
        let _ = cdp
            .call(
                "Browser.setWindowBounds",
                json!({"windowId": win["windowId"], "bounds": bounds}),
                None,
            )
            .await;
        // Chromium keeps a window 500 points wide at least, and casts all of
        // it: a narrower page shows at its left, cut out by the views.
        let drawn = cdp
            .call(
                "Browser.getWindowBounds",
                json!({"windowId": win["windowId"]}),
                None,
            )
            .await
            .ok()
            .and_then(|r| r["bounds"]["width"].as_f64())
            .unwrap_or(width as f64);
        b.live.lock().unwrap().page = Some((width as f64, height as f64));
        // Room in the picture for the whole window, so the page's part of
        // it keeps its pixels.
        let r = drawn.max(width as f64) / width as f64;
        (w, h) = ((w as f64 * r) as u32, (h as f64 * r) as u32);
    }
    cdp.call(
        "Page.startScreencast",
        json!({"format": "jpeg", "quality": 70, "maxWidth": w, "maxHeight": h}),
        s,
    )
    .await?;
    seed(b, cdp, session).await;
    Ok(())
}

/// Notes a tab DevTools told of, made or changed.
pub(super) fn tab_seen(b: &Browser, info: &Value) {
    let id = info["targetId"].as_str().unwrap_or_default();
    let throwaway = b.live.lock().unwrap().throwaway.clone();
    let logins = info["browserContextId"].as_str() != throwaway.as_deref() || throwaway.is_none();
    let tab = TabInfo {
        id: id.to_owned(),
        url: info["url"].as_str().unwrap_or_default().to_owned(),
        title: info["title"].as_str().unwrap_or_default().to_owned(),
        logins,
    };
    let mut s = b.status.lock().unwrap();
    match s.tabs.iter_mut().find(|t| t.id == id) {
        Some(t) => *t = tab,
        None => s.tabs.push(tab),
    }
}

/// Forgets a closed tab; answers whether it was the shown one.
pub(super) fn tab_gone(b: &Browser, id: &str) -> bool {
    let mut s = b.status.lock().unwrap();
    s.tabs.retain(|t| t.id != id);
    let shown = s.shown == id;
    drop(s);
    b.live.lock().unwrap().signed_in.remove(id);
    b.bump();
    shown
}

/// Sends the views the shown tab as it is now. The cast sends a picture
/// only when the page paints again: a page that painted before the cast
/// started, or before its load ended, and stays still, would leave the
/// views on what they showed before.
pub(super) async fn seed(b: &Browser, cdp: &Cdp, session: &str) {
    let Ok(shot) = cdp
        .call(
            "Page.captureScreenshot",
            json!({"format": "jpeg", "quality": 70}),
            Some(session),
        )
        .await
    else {
        return;
    };
    let Some(data) = shot["data"].as_str() else {
        return;
    };
    // Only for the tab still shown: another may have taken its place.
    let shown = b.tab.lock().unwrap().as_ref().map(|(s, _)| s.clone());
    if shown.as_deref() != Some(session) {
        return;
    }
    let size = b.live.lock().unwrap().screen().map(|s| (s.width, s.height));
    let (width, height) = size.unwrap_or((WIDTH, HEIGHT));
    {
        let mut f = b.frame.lock().unwrap();
        f.seq += 1;
        f.data = data.to_owned();
        f.width = width as f64;
        f.height = height as f64;
        f.page = None;
    }
    b.bump();
}

/// Has the browser save downloads where `download` keeps them.
pub(super) async fn downloads(cdp: &Cdp, dir: &Path, context: Option<&str>) {
    let to = dir.join("downloads");
    let _ = std::fs::create_dir_all(&to);
    let mut how = json!({"behavior": "allowAndName", "downloadPath": to, "eventsEnabled": true});
    if let Some(c) = context {
        how["browserContextId"] = json!(c);
    }
    if let Err(e) = cdp.call("Browser.setDownloadBehavior", how, None).await {
        eprintln!("libri: browser: downloads: {e}");
    }
}

/// A download's news: its name when it begins, and when it completes, the
/// file moved into the person's files under that name.
pub(super) fn download(b: &Browser, method: &str, params: &Value) {
    let guid = params["guid"].as_str().unwrap_or_default().to_owned();
    if method == "Browser.downloadWillBegin" {
        let name = params["suggestedFilename"]
            .as_str()
            .unwrap_or("download")
            .to_owned();
        b.live.lock().unwrap().downloads.insert(guid, name);
        return;
    }
    let state = params["state"].as_str().unwrap_or_default();
    if state != "completed" && state != "canceled" {
        return;
    }
    let Some(name) = b.live.lock().unwrap().downloads.remove(&guid) else {
        return;
    };
    let path = b.dir.join("downloads").join(&guid);
    if state == "canceled" {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let files = crate::files::Library::new(b.dir.parent().unwrap_or(&b.dir));
    let kept = std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| files.keep(&name, &bytes).map_err(|e| e.to_string()));
    let _ = std::fs::remove_file(&path);
    match kept {
        Ok(kept) => {
            let _ = b.notes.send(json!({"download": kept}));
        }
        Err(e) => eprintln!("libri: browser: download {name}: {e}"),
    }
}

/// What the person does in the tile.
#[derive(Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Act {
    /// The view's screen, in CSS pixels.
    View {
        width: u32,
        height: u32,
        #[serde(default = "one")]
        scale: f64,
        #[serde(default)]
        mobile: bool,
    },
    /// The view went out of sight (a hidden page, another place shown):
    /// it lays nothing out until it reports its screen again.
    Away,
    /// A mouse event, at CSS pixels of the page, as DevTools takes it.
    Mouse {
        r#type: String,
        x: f64,
        y: f64,
        #[serde(default)]
        button: Option<String>,
        #[serde(default)]
        buttons: u32,
        #[serde(default)]
        clicks: u32,
        #[serde(default)]
        modifiers: u32,
    },
    Wheel {
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
        #[serde(default)]
        modifiers: u32,
    },
    /// A key, as DevTools' `Input.dispatchKeyEvent` takes it.
    Key {
        r#type: String,
        key: String,
        code: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        key_code: u32,
        #[serde(default)]
        modifiers: u32,
    },
    /// Text as typed by a phone's keyboard or pasted.
    Text {
        text: String,
    },
    /// Copies the page's selection to the person's clipboard.
    Copy,
    Go {
        url: String,
    },
    Back,
    Forward,
    Reload,
    TabNew {
        #[serde(default)]
        url: String,
    },
    TabShow {
        id: String,
    },
    TabClose {
        id: String,
    },
    Take,
    Give,
}

/// Where the address bar's words go: an address as it is (with https://
/// when it has none), anything else to a search.
fn address(words: &str) -> Result<String, String> {
    let w = words.trim();
    if w.is_empty() {
        return Err("Type an address or words to search.".into());
    }
    let looks = !w.contains(char::is_whitespace)
        && (w.contains("://") || w.contains('.') || w.starts_with("localhost"));
    if looks {
        let url = if w.contains("://") {
            w.to_owned()
        } else {
            format!("https://{w}")
        };
        return plain_url(&url);
    }
    let mut q = String::new();
    for b in w.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                q.push(b as char)
            }
            b' ' => q.push('+'),
            _ => q.push_str(&format!("%{b:02X}")),
        }
    }
    Ok(format!("https://duckduckgo.com/?q={q}"))
}

impl Browser {
    /// Shows tab `id` in the views and makes it the one agents act in.
    pub(super) async fn show(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let (cdp, session) = {
            let mut held = self.running.lock().await;
            let r = held.as_mut().ok_or("The browser is not running.")?;
            if r.target == id {
                return Ok(());
            }
            let _ = r
                .cdp
                .call("Page.stopScreencast", json!({}), Some(&r.session))
                .await;
            let session = match r.sessions.get(id) {
                Some(s) => s.clone(),
                None => {
                    let s = attach(&r.cdp, id).await?;
                    r.sessions.insert(id.to_owned(), s.clone());
                    s
                }
            };
            r.target = id.to_owned();
            r.session = session.clone();
            let _ = r
                .cdp
                .call("Target.activateTarget", json!({"targetId": id}), None)
                .await;
            // In front of its window, so it paints (a page a link opened
            // shares its opener's window).
            let _ = r
                .cdp
                .call("Page.bringToFront", json!({}), Some(&session))
                .await;
            (r.cdp.clone(), session)
        };
        self.watch_tab(&cdp, &session, id).await?;
        {
            let mut s = self.status.lock().unwrap();
            if let Some(t) = s.tabs.iter().find(|t| t.id == id).cloned() {
                s.url = t.url;
                s.title = t.title;
            }
        }
        self.bump();
        Ok(())
    }

    /// Opens a tab and shows it: in the profile with the person's sign-ins
    /// (`logins`), the person's own tabs, or in the agents' throwaway
    /// context.
    pub(super) async fn tab_new(
        self: &Arc<Self>,
        url: &str,
        logins: bool,
    ) -> Result<String, String> {
        // A browser started for this tab starts with a blank one of its
        // own, which this tab takes the place of.
        let fresh = !self.status().running;
        self.up().await?;
        let first = fresh.then(|| self.status().shown).filter(|s| !s.is_empty());
        let (cdp, mut context) = {
            let held = self.running.lock().await;
            let r = held.as_ref().ok_or("The browser is not running.")?;
            if r.own.is_none() {
                return Err("The browser runs in your own browser now, with one tab.".into());
            }
            (r.cdp.clone(), r.context.clone())
        };
        // A throwaway context whose last tab closed keeps no window, and a
        // tab made in it is never painted: the agents get a fresh one.
        if !logins && context.is_some() && !self.status().tabs.iter().any(|t| !t.logins) {
            let made = cdp
                .call(
                    "Target.createBrowserContext",
                    json!({"disposeOnDetach": true}),
                    None,
                )
                .await?;
            let fresh = made["browserContextId"].as_str().map(str::to_owned);
            if let Some(old) = context.replace(fresh.clone().unwrap_or_default()) {
                let _ = cdp
                    .call(
                        "Target.disposeBrowserContext",
                        json!({"browserContextId": old}),
                        None,
                    )
                    .await;
            }
            if let Some(r) = self.running.lock().await.as_mut() {
                r.context = fresh.clone();
            }
            self.live.lock().unwrap().throwaway = fresh.clone();
            if let Some(c) = &fresh {
                downloads(&cdp, &self.dir, Some(c)).await;
            }
        }
        let mut tab = json!({"url": if url.is_empty() { "about:blank" } else { url }});
        let throwaway = !logins && context.is_some();
        if let (true, Some(c)) = (throwaway, context) {
            tab["browserContextId"] = json!(c);
        }
        // Each tab a window of its own, which the cast sizes to the views'
        // screen (`fit`) without resizing another tab's.
        tab["newWindow"] = json!(true);
        let made = cdp.call("Target.createTarget", tab, None).await?;
        let id = made["targetId"]
            .as_str()
            .ok_or("Chromium made no tab")?
            .to_owned();
        self.show(&id).await?;
        if let Some(first) = first {
            let _ = cdp
                .call("Target.closeTarget", json!({"targetId": first}), None)
                .await;
        }
        Ok(id)
    }

    /// Reads every tab's title and address again.
    pub(super) async fn retitle(self: &Arc<Self>) {
        let cdp = match self.running.lock().await.as_ref() {
            Some(r) if r.own.is_some() => r.cdp.clone(),
            _ => return,
        };
        // Late enough for the page's title to be set.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let Ok(all) = cdp.call("Target.getTargets", json!({}), None).await else {
            return;
        };
        for info in all["targetInfos"].as_array().into_iter().flatten() {
            if info["type"] == "page" {
                tab_seen(self, info);
            }
        }
        {
            let mut s = self.status.lock().unwrap();
            if let Some(t) = s.tabs.iter().find(|t| t.id == s.shown).cloned() {
                s.url = t.url;
                s.title = t.title;
            }
        }
        self.bump();
    }

    /// Closes a tab; the last one closed leaves a blank one.
    async fn tab_close(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let cdp = {
            let held = self.running.lock().await;
            held.as_ref()
                .ok_or("The browser is not running.")?
                .cdp
                .clone()
        };
        cdp.call("Target.closeTarget", json!({"targetId": id}), None)
            .await
            .map(drop)
    }

    /// After the shown tab closed: shows the newest tab left, or a blank one.
    pub(super) async fn after_close(self: &Arc<Self>, id: &str) {
        // The browser closing takes every tab with it: nothing to show, and
        // a blank tab here would start it again.
        match self.running.lock().await.as_mut() {
            Some(r) => r.sessions.remove(id),
            None => return,
        };
        let next = self.status().tabs.last().map(|t| (t.id.clone(), t.logins));
        let shown = match next {
            Some((next, _)) => self.show(&next).await,
            None => {
                let logins = self.status().holder == Holder::Person;
                self.tab_new("", logins).await.map(drop)
            }
        };
        if let Err(e) = shown {
            eprintln!("libri: browser: after a tab closed: {e}");
        }
    }

    /// The person's input takes the browser from the agents, a hold that
    /// lapses after PERSON_LAPSE with no more input.
    fn person_acts(&self) {
        self.touch();
        self.live.lock().unwrap().person_at = Some(Instant::now());
        let took = {
            let mut s = self.status.lock().unwrap();
            let took = s.holder != Holder::Person;
            s.holder = Holder::Person;
            took
        };
        if took {
            self.bump();
        }
    }

    /// Ends a hold the person took by browsing, once they stopped.
    pub(super) fn lapse(&self) {
        let quiet = self
            .live
            .lock()
            .unwrap()
            .person_at
            .is_some_and(|t| t.elapsed() > PERSON_LAPSE);
        let mut s = self.status.lock().unwrap();
        if quiet && s.holder == Holder::Person && s.waiting.is_none() {
            s.holder = Holder::Agent;
            drop(s);
            self.live.lock().unwrap().person_at = None;
            self.bump();
        }
    }

    /// Puts files the person chose into the file input that asked.
    pub async fn choose(self: &Arc<Self>, node: i64, files: Vec<PathBuf>) -> Result<(), String> {
        let (cdp, session) = self.up().await?;
        cdp.call(
            "DOM.setFileInputFiles",
            json!({"files": files, "backendNodeId": node}),
            Some(&session),
        )
        .await
        .map(drop)
    }

    /// One act of the person's in the tile.
    async fn act(self: &Arc<Self>, act: Act) -> Result<(), String> {
        match act {
            Act::View {
                width,
                height,
                scale,
                mobile,
            } => {
                return self
                    .resize(
                        Screen {
                            width,
                            height,
                            scale,
                            mobile,
                        },
                        None,
                    )
                    .await;
            }
            Act::Take => {
                self.person_acts();
                return Ok(());
            }
            Act::Give => {
                self.control(false, None).await?;
                self.live.lock().unwrap().person_at = None;
                return Ok(());
            }
            _ => {}
        }
        self.person_acts();
        let running = self.status().running;
        match act {
            Act::TabNew { url } => {
                let url = if url.trim().is_empty() {
                    String::new()
                } else {
                    address(&url)?
                };
                // The person's own tabs keep their sign-ins, in the profile.
                return self.tab_new(&url, true).await.map(drop);
            }
            // A browser not running yet starts with the person's own tab.
            Act::Go { url } if !running => {
                return self.tab_new(&address(&url)?, true).await.map(drop);
            }
            _ if !running => return Ok(()),
            _ => {}
        }
        let (cdp, session) = self.up().await?;
        let s = Some(session.as_str());
        match act {
            Act::Mouse {
                r#type,
                x,
                y,
                button,
                buttons,
                clicks,
                modifiers,
            } => {
                cdp.call(
                    "Input.dispatchMouseEvent",
                    json!({"type": r#type, "x": x, "y": y, "button": button.unwrap_or_else(|| "none".into()),
                        "buttons": buttons, "clickCount": clicks, "modifiers": modifiers}),
                    s,
                )
                .await?;
            }
            Act::Wheel {
                x,
                y,
                dx,
                dy,
                modifiers,
            } => {
                cdp.call(
                    "Input.dispatchMouseEvent",
                    json!({"type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy, "modifiers": modifiers}),
                    s,
                )
                .await?;
            }
            Act::Key {
                r#type,
                key,
                code,
                text,
                key_code,
                modifiers,
            } => {
                let mut e = json!({"type": r#type, "key": key, "code": code,
                    "windowsVirtualKeyCode": key_code, "nativeVirtualKeyCode": key_code, "modifiers": modifiers});
                if let Some(t) = text {
                    e["text"] = json!(t);
                    e["unmodifiedText"] = json!(t);
                }
                cdp.call("Input.dispatchKeyEvent", e, s).await?;
            }
            Act::Text { text } => {
                cdp.call("Input.insertText", json!({"text": text}), s)
                    .await?;
            }
            Act::Copy => {
                let v = super::evaluate(
                    &cdp,
                    s,
                    "(()=>{const e=document.activeElement;if(e&&(e.tagName==='INPUT'||e.tagName==='TEXTAREA')&&e.selectionStart!=null)return e.value.slice(e.selectionStart,e.selectionEnd);return String(getSelection())})()",
                )
                .await?;
                let text = v.as_str().unwrap_or_default();
                if !text.is_empty() {
                    let _ = self.notes.send(json!({"clip": text}));
                }
            }
            Act::Go { url } => {
                let url = address(&url)?;
                cdp.call("Page.navigate", json!({"url": url}), s).await?;
            }
            Act::Back | Act::Forward => {
                let h = cdp.call("Page.getNavigationHistory", json!({}), s).await?;
                let at = h["currentIndex"].as_i64().unwrap_or(0);
                let to = if matches!(act, Act::Back) {
                    at - 1
                } else {
                    at + 1
                };
                if let Some(e) = h["entries"].get(to.max(0) as usize).filter(|_| to >= 0) {
                    cdp.call(
                        "Page.navigateToHistoryEntry",
                        json!({"entryId": e["id"]}),
                        s,
                    )
                    .await?;
                }
            }
            Act::Reload => {
                cdp.call("Page.reload", json!({}), s).await?;
            }
            Act::TabNew { .. } => {}
            Act::TabShow { id } => self.show(&id).await?,
            Act::TabClose { id } => self.tab_close(&id).await?,
            Act::View { .. } | Act::Away | Act::Take | Act::Give => {}
        }
        Ok(())
    }

    /// A view's screen size: `gone` drops view `n`; else `n` (or a new view)
    /// reports `screen`. Lays the shown tab out again when the size in use
    /// changed.
    async fn resize(self: &Arc<Self>, screen: Screen, n: Option<u64>) -> Result<(), String> {
        let changed = {
            let mut l = self.live.lock().unwrap();
            let before = l.screen();
            let n = n.or(l.newest).unwrap_or(0);
            l.views.insert(n, screen);
            l.newest = Some(n);
            before != l.screen()
        };
        if changed {
            self.refit().await;
        }
        Ok(())
    }

    async fn refit(self: &Arc<Self>) {
        let link = self
            .running
            .lock()
            .await
            .as_ref()
            .map(|r| (r.cdp.clone(), r.session.clone()));
        if let Some((cdp, session)) = link
            && let Err(e) = fit(self, &cdp, &session).await
        {
            eprintln!("libri: browser: fit the view: {e}");
        }
    }

    /// Serves one live view on its WebSocket until it closes.
    pub async fn live(self: Arc<Self>, mut ws: WebSocket) {
        let n = {
            let mut l = self.live.lock().unwrap();
            l.next += 1;
            l.next
        };
        let mut changed = self.changed.subscribe();
        let mut notes = self.notes.subscribe();
        let (mut sent_status, mut sent_frame) = (String::new(), 0u64);
        let mut tick = tokio::time::interval(Duration::from_secs(20));
        loop {
            // What changed goes out: the state when it differs, the newest
            // frame when there is one the view has not had.
            let status =
                serde_json::to_string(&json!({"status": self.status()})).unwrap_or_default();
            if status != sent_status {
                if ws.send(Message::Text(status.clone().into())).await.is_err() {
                    break;
                }
                sent_status = status;
            }
            let frame = {
                let f = self.frame.lock().unwrap();
                (f.seq > sent_frame && !f.data.is_empty()).then(|| {
                    let (w, h) = f.page.unwrap_or((f.width, f.height));
                    (f.seq, f.data.clone(), w, h, f.width)
                })
            };
            if let Some((seq, data, w, h, view)) = frame {
                if let Ok(jpeg) = base64::engine::general_purpose::STANDARD.decode(data) {
                    // The page's width and height and the picture's width,
                    // in CSS pixels, then the picture: the page is at its
                    // top left.
                    let mut out = Vec::with_capacity(jpeg.len() + 12);
                    out.extend_from_slice(&(w.round() as u32).to_le_bytes());
                    out.extend_from_slice(&(h.round() as u32).to_le_bytes());
                    out.extend_from_slice(&(view.round() as u32).to_le_bytes());
                    out.extend_from_slice(&jpeg);
                    if ws.send(Message::Binary(out.into())).await.is_err() {
                        break;
                    }
                }
                sent_frame = seq;
                // Being watched counts as use: the browser stays up.
                self.touch();
            }
            tokio::select! {
                r = changed.changed() => if r.is_err() { break },
                n = notes.recv() => match n {
                    Ok(note) => {
                        if ws.send(Message::Text(json!({"note": note}).to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                },
                m = ws.recv() => match m {
                    Some(Ok(Message::Text(t))) => match serde_json::from_str::<Act>(&t) {
                        Ok(Act::View { width, height, scale, mobile }) => {
                            let _ = self.resize(Screen { width, height, scale, mobile }, Some(n)).await;
                        }
                        Ok(Act::Away) => self.away(n).await,
                        Ok(act) => {
                            if let Err(e) = self.act(act).await {
                                let _ = ws.send(Message::Text(json!({"note": {"error": e}}).to_string().into())).await;
                            }
                        }
                        Err(e) => eprintln!("libri: browser: a live view sent {t}: {e}"),
                    },
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                },
                _ = tick.tick() => {
                    if ws.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        self.away(n).await;
    }

    /// View `n` closed or went out of sight: the page lays out for the
    /// newest view left, or the browser's own size with none.
    async fn away(self: &Arc<Self>, n: u64) {
        let changed = {
            let mut l = self.live.lock().unwrap();
            let before = l.screen();
            l.views.remove(&n);
            if l.newest == Some(n) {
                l.newest = l.views.keys().max().copied();
            }
            before != l.screen()
        };
        if changed {
            self.refit().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_bar_opens_addresses_and_searches_words() {
        assert_eq!(address("example.com").unwrap(), "https://example.com");
        assert_eq!(
            address("http://localhost:8080/x").unwrap(),
            "http://localhost:8080/x"
        );
        assert_eq!(
            address("rust borrow checker").unwrap(),
            "https://duckduckgo.com/?q=rust+borrow+checker"
        );
        assert!(address("  ").is_err());
    }

    #[test]
    fn the_newest_view_lays_the_page_out() {
        let mut l = Shared::default();
        let a = Screen {
            width: 1200,
            height: 800,
            scale: 1.0,
            mobile: false,
        };
        let b = Screen {
            width: 390,
            height: 700,
            scale: 3.0,
            mobile: true,
        };
        l.views.insert(1, a);
        l.newest = Some(1);
        l.views.insert(2, b);
        l.newest = Some(2);
        assert_eq!(l.screen(), Some(b));
    }
}
