//! A still of each published page, for the Library's cards: rendered once
//! per version by a headless browser and served as a small WebP image, as
//! Notion, Figma, Google Drive and Duet show a document's first screen in
//! their galleries. The cards cost an image each; the live page runs only
//! when it is opened.
//!
//! One loop keeps the stills: it renders what is new or changed, one page
//! at a time, and drops the stills of versions and pages that are gone.
//! It looks again when the Library is read and every half minute, so a
//! page show_page writes from any process, or one copied into the pages
//! directory by hand, gets its still. Stills are off unless `browser.chromium`
//! names a Chromium, the one the agents' browser uses: a browser is a large
//! install for a glimpse of each page, so a host opts in, and without one the
//! cards show each page's title.

use crate::library;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Notify;

/// The window a page is laid out in, a laptop's first screen, and the scale
/// it is drawn at: 512 by 384 pixels, sharp on a phone's card.
const WINDOW: &str = "1024,768";
const SCALE: &str = "0.5";
/// How long a page's scripts may run, in the browser's own virtual time,
/// before the still is taken: enough for a chart to draw.
const SETTLE_MS: &str = "3000";
/// How long a render may take before it is given up, and the page tried
/// again at its next version.
const LIMIT: Duration = Duration::from_secs(30);
/// How often the loop looks for new pages unprompted.
const AGAIN: Duration = Duration::from_secs(30);

/// Where the stills are kept: in the state, out of the pages the site serves.
fn dir(state: &Path) -> PathBuf {
    state.join("snapshots")
}

/// A version's still: the page's name and when it changed.
pub fn file(state: &Path, name: &str, changed: u64) -> PathBuf {
    dir(state).join(format!("{name}.{changed}.webp"))
}

/// Whether this host renders stills, so a card asks for one only then.
pub fn on() -> bool {
    browser().is_some()
}

/// The browser named in the config, if any.
fn browser() -> Option<String> {
    named(|k| std::env::var(k).ok())
}

/// The browser `get` names, as the environment holds the config.
fn named(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    get("LIBRI_CHROMIUM").filter(|b| !b.is_empty())
}

/// Keeps a still of every page, until the daemon stops. `kick` asks it to
/// look now.
pub async fn keep(state: PathBuf, pages: PathBuf, kick: Arc<Notify>) {
    let Some(browser) = browser() else {
        return;
    };
    let mut failed_once = false;
    loop {
        let (s, p) = (state.clone(), pages.clone());
        let todo = tokio::task::spawn_blocking(move || stale(&s, &p))
            .await
            .unwrap_or_default();
        for (name, changed) in todo {
            match render(&browser, &state, &pages, &name, changed).await {
                Ok(()) => {}
                // A machine with no browser says so once, not per page.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if !failed_once {
                        eprintln!(
                            "libri: no {browser} to render the Library's stills; the cards show their titles"
                        );
                        failed_once = true;
                    }
                    break;
                }
                Err(e) => eprintln!("libri: still of {name}: {e}"),
            }
        }
        tokio::select! {
            _ = kick.notified() => {}
            _ = tokio::time::sleep(AGAIN) => {}
        }
    }
}

/// The pages whose current version has no still, the latest first, after
/// dropping the stills of versions and pages that are gone. A render that
/// failed leaves a `.failed` mark, so a page that cannot render is not
/// tried again until it changes.
fn stale(state: &Path, pages: &Path) -> Vec<(String, u64)> {
    let dir = dir(state);
    if fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let current = library::pages(pages);
    let wanted: std::collections::HashSet<String> = current
        .iter()
        .flat_map(|p| {
            let base = format!("{}.{}", p.name, p.changed);
            [format!("{base}.webp"), format!("{base}.failed")]
        })
        .collect();
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // The browser's own profile stays; every still not wanted goes.
            if e.path().is_file() && !wanted.contains(&name) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    current
        .into_iter()
        .filter(|p| {
            let f = file(state, &p.name, p.changed);
            !f.exists() && !f.with_extension("failed").exists()
        })
        .map(|p| (p.name, p.changed))
        .collect()
}

/// Renders one version's still into place: written beside it first, so a
/// half-written image is never served.
async fn render(
    browser: &str,
    state: &Path,
    pages: &Path,
    name: &str,
    changed: u64,
) -> std::io::Result<()> {
    let index = pages.join(name).join("index.html");
    let to = file(state, name, changed);
    let part = to.with_extension("part.webp");
    let profile = dir(state).join(".browser");
    let mut run = Command::new(browser);
    crate::cgroup::browser(run.as_std_mut());
    let run = run
        .args([
            "--headless",
            "--disable-gpu",
            "--hide-scrollbars",
            "--mute-audio",
            "--no-first-run",
            "--disable-extensions",
            "--disable-sync",
            &format!("--user-data-dir={}", profile.display()),
            &format!("--window-size={WINDOW}"),
            &format!("--force-device-scale-factor={SCALE}"),
            &format!("--virtual-time-budget={SETTLE_MS}"),
            &format!("--screenshot={}", part.display()),
        ])
        .arg(format!("file://{}", index.display()))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status();
    let done = tokio::time::timeout(LIMIT, run).await;
    let ok = matches!(&done, Ok(Ok(s)) if s.success()) && part.exists();
    if ok {
        return fs::rename(&part, &to);
    }
    let _ = fs::remove_file(&part);
    if let Ok(Err(e)) = done {
        return Err(e);
    }
    fs::write(to.with_extension("failed"), b"")?;
    Err(std::io::Error::other(match done {
        Err(_) => "took too long".to_string(),
        Ok(Ok(s)) => format!("the browser ended with {s}"),
        Ok(Err(_)) => unreachable!(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(pages: &Path, name: &str) {
        fs::create_dir_all(pages.join(name)).unwrap();
        fs::write(pages.join(name).join("index.html"), "<title>T</title>").unwrap();
    }

    #[test]
    fn stale_lists_the_new_and_drops_the_gone() {
        let root = std::env::temp_dir().join(format!("libri-snapshot-{}", std::process::id()));
        let (state, pages) = (root.join("state"), root.join("pages"));
        page(&pages, "kept");
        page(&pages, "new");
        let kept = library::pages(&pages)
            .into_iter()
            .find(|p| p.name == "kept")
            .unwrap();
        fs::create_dir_all(dir(&state)).unwrap();
        fs::write(file(&state, "kept", kept.changed), b"img").unwrap();
        fs::write(file(&state, "kept", kept.changed - 1), b"old").unwrap();
        fs::write(file(&state, "gone", 5), b"img").unwrap();
        let todo = stale(&state, &pages);
        assert_eq!(
            todo.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(),
            ["new"]
        );
        assert!(file(&state, "kept", kept.changed).exists());
        assert!(!file(&state, "kept", kept.changed - 1).exists());
        assert!(!file(&state, "gone", 5).exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_failed_render_waits_for_the_next_version() {
        let root =
            std::env::temp_dir().join(format!("libri-snapshot-failed-{}", std::process::id()));
        let (state, pages) = (root.join("state"), root.join("pages"));
        page(&pages, "broken");
        let p = library::pages(&pages).remove(0);
        fs::create_dir_all(dir(&state)).unwrap();
        fs::write(
            file(&state, "broken", p.changed).with_extension("failed"),
            b"",
        )
        .unwrap();
        assert!(stale(&state, &pages).is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    /// A host that names its Chromium for agents (`browser.chromium`) has
    /// stills from it too: one browser on the host, not a second key.
    #[test]
    fn stills_use_the_hosts_one_chromium() {
        let env = |k: &str| (k == "LIBRI_CHROMIUM").then(|| "/bin/chromium".to_owned());
        assert_eq!(named(env).as_deref(), Some("/bin/chromium"));
        assert_eq!(named(|_| Some(String::new())), None);
        assert_eq!(named(|_| None), None);
    }

    /// The real render, with the browser LIBRI_CHROMIUM names: skipped where
    /// none is named, as in the build's sandbox, which has no browser.
    #[tokio::test]
    async fn renders_a_still_with_the_named_browser() {
        let Ok(browser) = std::env::var("LIBRI_CHROMIUM") else {
            return;
        };
        let root =
            std::env::temp_dir().join(format!("libri-snapshot-render-{}", std::process::id()));
        let (state, pages) = (root.join("state"), root.join("pages"));
        fs::create_dir_all(pages.join("hello")).unwrap();
        fs::write(
            pages.join("hello").join("index.html"),
            "<body style='background:#7a3b1d'><h1 id=h></h1><script>h.textContent='drawn'</script>",
        )
        .unwrap();
        let todo = stale(&state, &pages);
        let (name, changed) = todo[0].clone();
        render(&browser, &state, &pages, &name, changed)
            .await
            .unwrap();
        let image = fs::read(file(&state, &name, changed)).unwrap();
        assert_eq!(&image[..4], b"RIFF");
        assert_eq!(&image[8..12], b"WEBP");
        assert!(stale(&state, &pages).is_empty());
        fs::remove_dir_all(&root).unwrap();
    }
}
