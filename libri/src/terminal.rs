//! The terminal: a shell on the host, as the daemon's user, drawn on the
//! page. The shell runs in a PTY here and keeps running while no page
//! looks at it; Ghostty's terminal library keeps its screen and scrollback
//! on the host as well, so a page that opens or reconnects is sent the
//! screen as it is (as Paseo sends its headless xterm's), not a replay of
//! raw output that full-screen programs garble. The page draws with
//! ghostty-web, the same library built for the browser.
//!
//! Over the WebSocket: the page's first frame is its size as JSON
//! (`{"cols":80,"rows":24}`), as is any later resize; its keystrokes come
//! as binary frames. The host answers with the screen, then the shell's
//! output, both binary. The last page to size the terminal sizes it, as
//! tmux sizes a window to its latest client.
//!
//! Agents share the same shell, as tmux's `send-keys` and `capture-pane`
//! share a session with whoever is attached: what an agent types goes in
//! through the PTY, so its echo and output reach every open page, and what
//! it reads is the host's screen as plain text. Their tools reach the
//! daemon by SOCKET in the private state folder, as the browser's do. As in
//! Codex's `write_stdin`, a send answers with the output that came after
//! it, once the output settles or the awaited text appears.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};

/// The socket agents' tools reach the terminal by, in the state folder.
pub const SOCKET: &str = "terminal.sock";

/// Lines of history the host keeps above the screen.
const SCROLLBACK: usize = 5_000;
/// Output chunks a page may fall behind by before it is sent the screen
/// afresh instead.
const BACKLOG: usize = 1_024;
/// The size a shell an agent starts opens at, until a page sizes it.
const AGENT_SIZE: Size = Size {
    cols: 120,
    rows: 32,
};
/// How often a wait looks at the screen again.
const POLL: Duration = Duration::from_millis(100);
/// How quiet the output must stay after a send for it to count as settled.
const SETTLE: Duration = Duration::from_millis(500);
/// How long a send waits by default, and any wait at the most.
const WAIT: Duration = Duration::from_secs(10);
const WAIT_MOST: Duration = Duration::from_secs(120);
/// Lines of history a read may ask for above the screen.
const LINES_MOST: usize = 2_000;
/// The most text one answer carries; past it the oldest lines go.
const TEXT_MOST: usize = 16 * 1024;

/// The daemon's shells by name, each started when a page first opens its
/// terminal and again after it exits. `MAIN` is the one every terminal
/// opened without a name shares, and the one the agents' tools reach; a
/// terminal tile opened beside another names its own (the page's `?id=`),
/// so each is a shell of its own, as each tab of a terminal app is.
#[derive(Default)]
pub struct Terminals(Mutex<std::collections::HashMap<String, Arc<Shell>>>);

/// The shared shell's name.
const MAIN: &str = "main";
/// The most named shells kept at once; past it, opening another fails.
const SHELLS_MOST: usize = 32;

impl Terminals {
    fn get(&self, id: &str, size: Size) -> Result<Arc<Shell>, String> {
        let mut all = self.0.lock().unwrap();
        // A shell that exited frees its name.
        all.retain(|_, s| !s.vt.is_closed());
        if let Some(s) = all.get(id) {
            return Ok(s.clone());
        }
        if all.len() >= SHELLS_MOST {
            return Err(format!("{SHELLS_MOST} shells are open; close one first."));
        }
        let s = Arc::new(Shell::start(size)?);
        all.insert(id.to_owned(), s.clone());
        Ok(s)
    }
}

impl Terminals {
    /// Hangs up each named shell older than `grace` that no tile names in
    /// `kept` (every workspace's terminals, open or closed and kept: see
    /// layout::shells). A closed tile's shell ends with it; a workspace
    /// closed into Recently closed keeps its shells until it is deleted or
    /// ages out. The shared shell is never ended.
    pub fn end_unnamed(&self, kept: &HashSet<String>, grace: Duration) {
        let mut all = self.0.lock().unwrap();
        all.retain(|id, s| {
            let keep = id == MAIN || kept.contains(id) || s.born.elapsed() < grace;
            if !keep {
                s.hang_up();
            }
            keep
        });
    }
}

/// A terminal's name as a page gives it: letters, digits, `-` and `_`, at
/// most 40 of them; anything else, or none, is the shared shell.
pub fn shell_name(given: Option<&str>) -> String {
    given
        .filter(|n| {
            !n.is_empty()
                && n.len() <= 40
                && n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .unwrap_or(MAIN)
        .to_owned()
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
struct Size {
    cols: u16,
    rows: u16,
}

impl Size {
    fn pty(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// A page joining: the screen as VT sequences, and the output after it.
type Joined = (Vec<u8>, broadcast::Receiver<Arc<[u8]>>);

enum Work {
    Output(Vec<u8>),
    Resize(Size),
    Attach(oneshot::Sender<Joined>),
    Look(oneshot::Sender<Look>),
    /// The shell has exited.
    Ended,
}

struct Shell {
    /// The shell's own process, to tell it from a program it runs.
    pid: Option<u32>,
    input: Mutex<Box<dyn Write + Send>>,
    pty: Mutex<Box<dyn MasterPty + Send>>,
    /// To the thread that owns the screen; Ghostty's types stay on the
    /// thread that made them.
    vt: tokio::sync::mpsc::UnboundedSender<Work>,
    /// When it started, so one opened a moment ago outlives a layout that
    /// does not name it yet.
    born: Instant,
}

impl Shell {
    fn start(size: Size) -> Result<Self, String> {
        let pair = native_pty_system()
            .openpty(size.pty())
            .map_err(|e| format!("terminal: {e}"))?;
        let (shell, home) = account();
        let mut cmd = CommandBuilder::new(&shell);
        // A login shell, so the profile sets PATH; the daemon's own is the
        // service's.
        cmd.arg("-l");
        cmd.cwd(home);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("terminal: {shell}: {e}"))?;
        let pid = child.process_id();
        if let Some(pid) = pid {
            crate::cgroup::adopt(pid);
        }
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("terminal: {e}"))?;
        let input = pair
            .master
            .take_writer()
            .map_err(|e| format!("terminal: {e}"))?;
        let (vt, mut work) = tokio::sync::mpsc::unbounded_channel();
        let out = vt.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0; 64 * 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || out.send(Work::Output(buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            let _ = child.wait();
            let _ = out.send(Work::Ended);
        });
        let (ready, started) = std_mpsc::channel();
        std::thread::spawn(move || {
            let mut screen = match Screen::new(size) {
                Ok(s) => {
                    let _ = ready.send(Ok(()));
                    s
                }
                Err(e) => {
                    let _ = ready.send(Err(e));
                    return;
                }
            };
            // Leaving drops the pages' sender, which closes their streams.
            while let Some(w) = work.blocking_recv() {
                if matches!(w, Work::Ended) {
                    break;
                }
                screen.handle(w);
            }
        });
        started
            .recv()
            .map_err(|_| "terminal: the screen thread died".to_string())??;
        Ok(Shell {
            pid,
            input: Mutex::new(input),
            pty: Mutex::new(pair.master),
            vt,
            born: Instant::now(),
        })
    }

    /// Hangs the shell up (SIGHUP to its process group), as closing a
    /// terminal window does; it and what it runs end.
    fn hang_up(&self) {
        if let Some(pid) = self.pid {
            // SAFETY: a signal to a process group this shell leads.
            unsafe { libc::killpg(pid as i32, libc::SIGHUP) };
        }
    }

    fn resize(&self, size: Size) {
        let _ = self.pty.lock().unwrap().resize(size.pty());
        let _ = self.vt.send(Work::Resize(size));
    }

    async fn attach(&self) -> Option<Joined> {
        let (tx, rx) = oneshot::channel();
        self.vt.send(Work::Attach(tx)).ok()?;
        rx.await.ok()
    }

    fn write(&self, data: &[u8]) {
        let mut input = self.input.lock().unwrap();
        let _ = input.write_all(data).and_then(|()| input.flush());
    }

    async fn look(&self) -> Result<Look, String> {
        let (tx, rx) = oneshot::channel();
        self.vt
            .send(Work::Look(tx))
            .map_err(|_| "The terminal's shell has exited.".to_string())?;
        rx.await
            .map_err(|_| "The terminal's shell has exited.".to_string())
    }

    /// What runs in the foreground: None while the shell waits at its
    /// prompt, else the program's command line.
    fn foreground(&self) -> Option<String> {
        let leader = self.pty.lock().unwrap().process_group_leader()?;
        if Some(leader as u32) == self.pid {
            return None;
        }
        let line = std::fs::read(format!("/proc/{leader}/cmdline")).ok()?;
        let words: Vec<String> = line
            .split(|b| *b == 0)
            .filter(|w| !w.is_empty())
            .map(|w| String::from_utf8_lossy(w).into_owned())
            .collect();
        Some(if words.is_empty() {
            format!("process {leader}")
        } else {
            words.join(" ")
        })
    }

    /// The working directory of what runs in the foreground, or the
    /// shell's.
    fn cwd(&self, pwd: &str) -> Option<String> {
        // OSC 7's `file://host/path`, when the shell reports it.
        if let Some(rest) = pwd.strip_prefix("file://") {
            return rest.find('/').map(|i| rest[i..].to_owned());
        }
        let pid = self
            .pty
            .lock()
            .unwrap()
            .process_group_leader()
            .map(|p| p as u32)
            .or(self.pid)?;
        std::fs::read_link(format!("/proc/{pid}/cwd"))
            .ok()
            .map(|p| p.display().to_string())
    }
}

/// The screen as an agent reads it.
struct Look {
    /// Output chunks taken so far, to tell when output stops.
    seq: u64,
    /// The screen and its history as plain text, a wrapped line whole.
    text: String,
    title: String,
    pwd: String,
    cols: u16,
    rows: u16,
    alternate: bool,
}

/// The screen, on its own thread: Ghostty's terminal fed the shell's
/// output, and the pages following it.
struct Screen {
    term: libghostty_vt::Terminal<'static, 'static>,
    pages: broadcast::Sender<Arc<[u8]>>,
    seq: u64,
}

impl Screen {
    fn new(size: Size) -> Result<Self, String> {
        let term = libghostty_vt::Terminal::new(libghostty_vt::terminal::Options {
            cols: size.cols,
            rows: size.rows,
            max_scrollback: SCROLLBACK,
        })
        .map_err(|e| format!("terminal: {e:?}"))?;
        Ok(Screen {
            term,
            pages: broadcast::channel(BACKLOG).0,
            seq: 0,
        })
    }

    fn handle(&mut self, w: Work) {
        match w {
            Work::Output(data) => {
                // Queries the shell's programs make are answered by the
                // page's terminal; this one installs no reply handler, so
                // they are not answered twice.
                self.term.vt_write(&data);
                self.seq += 1;
                let _ = self.pages.send(data.into());
            }
            Work::Resize(size) => {
                let _ = self.term.resize(size.cols, size.rows, 0, 0);
            }
            Work::Attach(reply) => {
                let _ = reply.send((self.snapshot(), self.pages.subscribe()));
            }
            Work::Look(reply) => {
                let _ = reply.send(self.look());
            }
            Work::Ended => {}
        }
    }

    fn look(&self) -> Look {
        let opts = FormatterOptions::new()
            .with_format(Format::Plain)
            .with_unwrap(true);
        let text = Formatter::new(&self.term, opts)
            .and_then(|mut f| f.format_alloc(None).map(|b| b.to_vec()))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        Look {
            seq: self.seq,
            text,
            title: self.term.title().unwrap_or_default().to_owned(),
            pwd: self.term.pwd().unwrap_or_default().to_owned(),
            cols: self.term.cols().unwrap_or(0),
            rows: self.term.rows().unwrap_or(0),
            alternate: matches!(
                self.term.active_screen(),
                Ok(libghostty_vt::screen::Screen::Alternate)
            ),
        }
    }

    /// The screen and its history as VT sequences that redraw it on a fresh
    /// terminal, with the modes, cursor and pen to carry on from.
    fn snapshot(&self) -> Vec<u8> {
        let opts = FormatterOptions::new()
            .with_format(Format::Vt)
            // A wrapped line stays one line, so the page folds it at its
            // own width; a hard break here would stay in it for good.
            .with_unwrap(true)
            .with_modes(true)
            .with_scrolling_region(true)
            // Not tab stops: the formatter sets them after placing the
            // cursor, and setting them moves it (ghostty a887df4).
            .with_pwd(true)
            .with_keyboard(true)
            .with_cursor(true)
            .with_style(true)
            .with_hyperlink(true)
            .with_kitty_keyboard(true)
            .with_charsets(true);
        Formatter::new(&self.term, opts)
            .and_then(|mut f| f.format_alloc(None).map(|b| b.to_vec()))
            .unwrap_or_default()
    }
}

/// A request from an agent's tools, one JSON line on SOCKET.
#[derive(Deserialize, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Ask {
    /// Start the shell if none runs, and say how it stands.
    Open,
    /// The screen, with `lines` of history above it.
    Read {
        #[serde(default)]
        lines: usize,
        #[serde(flatten)]
        wait: Wait,
    },
    /// Type `text`, then press `keys`, and answer with the output after.
    Send {
        #[serde(default)]
        text: String,
        #[serde(default)]
        keys: Vec<String>,
        #[serde(flatten)]
        wait: Wait,
    },
}

/// What a request waits for before it answers.
#[derive(Deserialize, Debug, Default)]
pub struct Wait {
    /// Text to wait for: on a send, in the output after it; on a read,
    /// anywhere on the screen or in its history.
    #[serde(default)]
    pub wait_for: Option<String>,
    /// Seconds the output must stay quiet.
    #[serde(default)]
    pub settle: Option<f64>,
    /// Seconds to wait at the most.
    #[serde(default)]
    pub timeout: Option<f64>,
}

impl Wait {
    fn limit(&self, default: Duration) -> Duration {
        self.timeout
            .filter(|t| t.is_finite() && *t >= 0.0)
            .map_or(default, Duration::from_secs_f64)
            .min(WAIT_MOST)
    }
}

/// How a wait ended.
enum Waited {
    Not,
    Appeared(Duration),
    Settled(Duration),
    TimedOut(Duration),
}

impl Terminals {
    /// Answers one agent's request.
    pub async fn ask(&self, ask: Ask) -> Result<String, String> {
        let shell = self.get(MAIN, AGENT_SIZE)?;
        match ask {
            Ask::Open => {
                let now = shell.look().await?;
                Ok(answer(&shell, &now, &Waited::Not, &screen(&now, 0)))
            }
            Ask::Read { lines, wait } => {
                let settle = wait.settle.map(seconds);
                let (now, waited) = if wait.wait_for.is_none() && settle.is_none() {
                    (shell.look().await?, Waited::Not)
                } else {
                    let limit = wait.limit(WAIT);
                    watch(&shell, None, wait.wait_for.as_deref(), settle, limit).await?
                };
                Ok(answer(
                    &shell,
                    &now,
                    &waited,
                    &screen(&now, lines.min(LINES_MOST)),
                ))
            }
            Ask::Send { text, keys, wait } => {
                let mut bytes = text.into_bytes();
                for k in &keys {
                    bytes.extend(
                        key(k).ok_or_else(|| {
                            format!("No key is named {k:?}. Keys: {}.", KEY_NAMES)
                        })?,
                    );
                }
                if bytes.is_empty() {
                    return Err("Give text or keys to send.".into());
                }
                let before = shell.look().await?;
                shell.write(&bytes);
                // Awaited text alone ends the wait unless a settle is asked
                // for too; with neither, the output settling does.
                let settle = match (&wait.wait_for, wait.settle) {
                    (_, Some(s)) => Some(seconds(s)),
                    (Some(_), None) => None,
                    (None, None) => Some(SETTLE),
                };
                let (now, waited) = watch(
                    &shell,
                    Some(&before.text),
                    wait.wait_for.as_deref(),
                    settle,
                    wait.limit(WAIT),
                )
                .await?;
                let body = if now.alternate {
                    // A full-screen program redraws rather than prints:
                    // its screen is what there is to read.
                    screen(&now, 0)
                } else {
                    after(&before.text, &now.text)
                };
                Ok(answer(&shell, &now, &waited, &body))
            }
        }
    }

    /// Serves agents' tools on SOCKET in the state folder, which only the
    /// daemon's user can enter.
    pub async fn serve(self: Arc<Self>, state: &Path) -> Result<(), String> {
        let path = state.join(SOCKET);
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        loop {
            let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
            let me = self.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
                let (r, mut w) = stream.into_split();
                let mut line = String::new();
                if tokio::io::BufReader::new(r)
                    .read_line(&mut line)
                    .await
                    .is_err()
                {
                    return;
                }
                let answer = match serde_json::from_str::<Ask>(&line) {
                    Ok(ask) => me.ask(ask).await,
                    Err(e) => Err(format!("terminal: {e}")),
                };
                let out = match answer {
                    Ok(text) => json!({"text": text}),
                    Err(e) => json!({"error": e}),
                };
                let _ = w.write_all(format!("{out}\n").as_bytes()).await;
            });
        }
    }
}

/// An agent's request through the daemon's socket, from the tools server.
pub fn request(state: &Path, ask: Value) -> Result<String, String> {
    let path = state.join(SOCKET);
    let mut s = std::os::unix::net::UnixStream::connect(&path)
        .map_err(|_| "The terminal is not running on this host.".to_string())?;
    s.set_read_timeout(Some(WAIT_MOST + Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    writeln!(s, "{ask}").map_err(|e| e.to_string())?;
    let mut out = String::new();
    s.read_to_string(&mut out)
        .map_err(|e| format!("terminal: {e}"))?;
    let v: Value = serde_json::from_str(&out).map_err(|e| format!("terminal: {e}"))?;
    match (v["text"].as_str(), v["error"].as_str()) {
        (Some(t), _) => Ok(t.to_owned()),
        (_, Some(e)) => Err(e.to_owned()),
        _ => Err("terminal: no answer".into()),
    }
}

/// Whether this host's daemon serves the terminal to agents.
pub fn here(state: &Path) -> bool {
    state.join(SOCKET).exists()
}

fn seconds(s: f64) -> Duration {
    if s.is_finite() && s >= 0.0 {
        Duration::from_secs_f64(s).min(WAIT_MOST)
    } else {
        SETTLE
    }
}

/// Looks at the screen until `text` appears (after `since`, when given),
/// or the output stays quiet for `settle`, or `limit` passes.
async fn watch(
    shell: &Shell,
    since: Option<&str>,
    text: Option<&str>,
    settle: Option<Duration>,
    limit: Duration,
) -> Result<(Look, Waited), String> {
    let start = Instant::now();
    let mut now = shell.look().await?;
    let (mut seq, mut quiet) = (now.seq, Instant::now());
    loop {
        let seen = match since {
            Some(before) => after(before, &now.text),
            None => now.text.clone(),
        };
        if let Some(t) = text
            && seen.contains(t)
        {
            return Ok((now, Waited::Appeared(start.elapsed())));
        }
        if let Some(s) = settle
            && quiet.elapsed() >= s
        {
            return Ok((now, Waited::Settled(start.elapsed())));
        }
        if start.elapsed() >= limit {
            return Ok((now, Waited::TimedOut(start.elapsed())));
        }
        tokio::time::sleep(POLL).await;
        now = shell.look().await?;
        if now.seq != seq {
            (seq, quiet) = (now.seq, Instant::now());
        }
    }
}

/// The lines of `now` from the first that differs from `before`: the line
/// typed at the prompt and all that followed it.
fn after(before: &str, now: &str) -> String {
    let (b, n): (Vec<&str>, Vec<&str>) = (before.lines().collect(), now.lines().collect());
    let same = b.iter().zip(&n).take_while(|(x, y)| x == y).count();
    trimmed(&n[same..].join("\n"))
}

/// The last `rows + lines` lines: the screen and that much history.
fn screen(look: &Look, lines: usize) -> String {
    let text = look.text.trim_end();
    let all: Vec<&str> = text.lines().collect();
    let keep = (look.rows as usize).max(1) + lines;
    trimmed(&all[all.len().saturating_sub(keep)..].join("\n"))
}

/// Without blank lines at either end or trailing spaces, and at most
/// TEXT_MOST, the newest kept.
fn trimmed(text: &str) -> String {
    let text: Vec<&str> = text.lines().map(str::trim_end).collect();
    let text = text.join("\n");
    let text = text.trim_end().trim_start_matches('\n');
    if text.len() <= TEXT_MOST {
        return text.to_owned();
    }
    let mut cut = text.len() - TEXT_MOST;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    let cut = text[cut..].find('\n').map_or(cut, |i| cut + i + 1);
    format!("[earlier output cut]\n{}", &text[cut..])
}

/// The answer an agent reads: how the terminal stands, how the wait ended,
/// then the text.
fn answer(shell: &Shell, look: &Look, waited: &Waited, body: &str) -> String {
    let mut out = format!("Terminal {}x{}", look.cols, look.rows);
    if let Some(cwd) = shell.cwd(&look.pwd) {
        out += &format!(", in {cwd}");
    }
    match shell.foreground() {
        Some(cmd) => out += &format!(", running `{cmd}`"),
        None => out += ", at the shell's prompt",
    }
    if look.alternate {
        out += ", full-screen";
    }
    out.push('.');
    if !look.title.is_empty() {
        out += &format!(" Title: {}.", look.title);
    }
    let s = |d: &Duration| format!("{:.1}s", d.as_secs_f64());
    match waited {
        Waited::Not => {}
        Waited::Appeared(d) => out += &format!(" The text appeared after {}.", s(d)),
        Waited::Settled(d) => out += &format!(" Output settled after {}.", s(d)),
        Waited::TimedOut(d) => {
            out += &format!(" Still waiting after {}: read again to see more.", s(d))
        }
    }
    out.push_str("\n\n");
    out.push_str(if body.is_empty() { "(no output)" } else { body });
    out
}

/// The key names `key` knows, for the agent told of one it doesn't.
const KEY_NAMES: &str = "Enter, Tab, Escape, Backspace, Space, Up, Down, Left, Right, Home, End, PageUp, PageDown, Insert, Delete, F1-F12, Ctrl-<letter> (Ctrl-C), Alt-<key>";

/// The bytes a named key sends, as xterm sends them; tmux's names (C-c,
/// M-x, BSpace) are taken as well.
fn key(name: &str) -> Option<Vec<u8>> {
    let n = name.trim().to_ascii_lowercase().replace('_', "-");
    let n = n.replace('+', "-");
    for alt in ["alt-", "meta-", "m-"] {
        if let Some(rest) = n.strip_prefix(alt) {
            let mut b = vec![0x1b];
            b.extend(key(rest).or_else(|| (rest.len() == 1).then(|| rest.as_bytes().to_vec()))?);
            return Some(b);
        }
    }
    for ctrl in ["ctrl-", "control-", "c-"] {
        if let Some(rest) = n.strip_prefix(ctrl) {
            let c = match rest.as_bytes() {
                [c @ b'a'..=b'z'] => c & 0x1f,
                [b'@'] | [b' '] => 0,
                [b'['] => 0x1b,
                [b'\\'] => 0x1c,
                [b']'] => 0x1d,
                _ => return None,
            };
            return Some(vec![c]);
        }
    }
    // Arrows and Home/End as in normal cursor mode, which shells and most
    // programs read in either mode.
    let b: &[u8] = match n.as_str() {
        "enter" | "return" => b"\r",
        "tab" => b"\t",
        "escape" | "esc" => b"\x1b",
        "backspace" | "bspace" => b"\x7f",
        "space" => b" ",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "pageup" | "pgup" | "ppage" => b"\x1b[5~",
        "pagedown" | "pgdn" | "npage" => b"\x1b[6~",
        "insert" | "ic" => b"\x1b[2~",
        "delete" | "dc" => b"\x1b[3~",
        "f1" => b"\x1bOP",
        "f2" => b"\x1bOQ",
        "f3" => b"\x1bOR",
        "f4" => b"\x1bOS",
        "f5" => b"\x1b[15~",
        "f6" => b"\x1b[17~",
        "f7" => b"\x1b[18~",
        "f8" => b"\x1b[19~",
        "f9" => b"\x1b[20~",
        "f10" => b"\x1b[21~",
        "f11" => b"\x1b[23~",
        "f12" => b"\x1b[24~",
        _ => return None,
    };
    Some(b.to_vec())
}

/// The daemon user's shell and home, from the password database: the
/// service's environment may name neither.
fn account() -> (String, String) {
    // SAFETY: getpwuid returns a pointer into static storage or null; it is
    // read at once, before any other call could reuse it.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return ("/bin/sh".into(), "/".into());
        }
        let text =
            |p: *const libc::c_char| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
        (text((*pw).pw_shell), text((*pw).pw_dir))
    }
}

/// One page's connection to the shell, for as long as it stays open.
pub async fn serve(terminals: Arc<Terminals>, id: String, mut ws: WebSocket) {
    let Some(size) = next_size(&mut ws).await else {
        return;
    };
    let shell = match terminals.get(&id, size) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("libri: {e}");
            let _ = ws.send(Message::Binary(format!("{e}\r\n").into())).await;
            return;
        }
    };
    shell.resize(size);
    'attach: loop {
        let Some((screen, mut output)) = shell.attach().await else {
            break;
        };
        if ws.send(Message::Binary(screen.into())).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                out = output.recv() => match out {
                    Ok(data) => {
                        if ws.send(Message::Binary(data.to_vec().into())).await.is_err() {
                            return;
                        }
                    }
                    // Too far behind to catch up byte by byte: clear the
                    // page's terminal and send the screen as it is now.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if ws.send(Message::Binary(b"\x1bc"[..].into())).await.is_err() {
                            return;
                        }
                        continue 'attach;
                    }
                    Err(broadcast::error::RecvError::Closed) => break 'attach,
                },
                from = ws.recv() => match from {
                    Some(Ok(Message::Binary(data))) => shell.write(&data),
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(size) = serde_json::from_str::<Size>(&text) {
                            shell.resize(size);
                        } else if text.trim() == RESTART {
                            // The tile menu's Restart shell: the shell is hung
                            // up as a closed terminal window hangs it up; it
                            // ends, every page is told so, and the page that
                            // asked opens a new one.
                            shell.hang_up();
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return,
                },
            }
        }
    }
    // The shell exited: a clean close, which the page tells from a lost
    // connection, so it offers a new shell rather than reconnecting.
    let _ = ws
        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
            code: axum::extract::ws::close_code::NORMAL,
            reason: "exited".into(),
        })))
        .await;
}

/// A page's word to end its shell, so that it can open a new one.
const RESTART: &str = r#"{"op":"restart"}"#;

/// Waits for the page to say its size; anything sent before then is
/// dropped.
async fn next_size(ws: &mut WebSocket) -> Option<Size> {
    while let Some(Ok(m)) = ws.recv().await {
        if let Message::Text(text) = m
            && let Ok(size) = serde_json::from_str::<Size>(&text)
            && size.cols > 0
            && size.rows > 0
        {
            return Some(size);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full-screen program's screen survives as a snapshot: redrawn on a
    /// fresh terminal it reads as it did, cursor and all, where replaying
    /// the history a reconnect missed could not.
    #[test]
    fn a_snapshot_redraws_the_screen() {
        let size = Size { cols: 20, rows: 4 };
        let mut screen = Screen::new(size).unwrap();
        for line in 0..10 {
            screen.handle(Work::Output(format!("line {line}\r\n").into_bytes()));
        }
        // An alternate-screen program draws over the top.
        screen.handle(Work::Output(b"\x1b[?1049h\x1b[2J\x1b[Hvim here".to_vec()));
        let shot = screen.snapshot();
        let mut fresh = Screen::new(size).unwrap();
        fresh.handle(Work::Output(shot));
        let plain = |s: &Screen| {
            let opts = FormatterOptions::new().with_format(Format::Plain);
            let mut f = Formatter::new(&s.term, opts).unwrap();
            String::from_utf8(f.format_alloc(None).unwrap().to_vec()).unwrap()
        };
        assert_eq!(plain(&fresh), plain(&screen));
        let at = |s: &Screen| (s.term.cursor_x().unwrap(), s.term.cursor_y().unwrap());
        assert_eq!(at(&fresh), at(&screen));
        assert!(plain(&fresh).contains("vim here"));
    }

    /// A line the screen wrapped is one line in the snapshot: a terminal
    /// that narrows afterwards folds it once, at its new width.
    #[test]
    fn a_snapshot_keeps_a_wrapped_line_whole() {
        let size = Size { cols: 20, rows: 6 };
        let mut screen = Screen::new(size).unwrap();
        let line = "0123456789".repeat(3);
        screen.handle(Work::Output(line.clone().into_bytes()));
        let shot = screen.snapshot();
        let mut fresh = Screen::new(size).unwrap();
        fresh.handle(Work::Output(shot));
        fresh.handle(Work::Resize(Size { cols: 14, rows: 6 }));
        let opts = FormatterOptions::new()
            .with_format(Format::Plain)
            .with_unwrap(true);
        let mut f = Formatter::new(&fresh.term, opts).unwrap();
        let plain = String::from_utf8(f.format_alloc(None).unwrap().to_vec()).unwrap();
        assert_eq!(plain.trim_end(), line);
    }

    /// A read is the screen and its history as plain text, a wrapped line
    /// whole, and the last screenful by default.
    #[test]
    fn a_look_reads_the_screen_and_history_as_text() {
        let mut screen = Screen::new(Size { cols: 20, rows: 4 }).unwrap();
        for line in 0..10 {
            screen.handle(Work::Output(
                format!("\x1b[1mline {line}\x1b[0m\r\n").into_bytes(),
            ));
        }
        screen.handle(Work::Output(b"\x1b]2;make\x07$ ".to_vec()));
        let look = screen.look();
        assert_eq!(look.seq, 11);
        assert_eq!(look.title, "make");
        assert!(look.text.contains("line 0"), "{}", look.text);
        assert_eq!(super::screen(&look, 0), "line 7\nline 8\nline 9\n$");
        assert_eq!(super::screen(&look, 2).lines().next(), Some("line 5"));
    }

    /// What a send answers with is what came after it: the line typed at
    /// the prompt on, not the screen before.
    #[test]
    fn the_output_after_a_send_starts_at_the_line_that_changed() {
        let before = "old 1\nold 2\n$ ";
        let now = "old 1\nold 2\n$ ls\na  b\n$ ";
        assert_eq!(after(before, now), "$ ls\na  b\n$");
        assert_eq!(after(now, now), "");
    }

    #[test]
    fn long_output_keeps_its_newest_lines() {
        let text: String = (0..5_000).map(|i| format!("line {i}\n")).collect();
        let t = trimmed(&text);
        assert!(t.len() <= TEXT_MOST + 30);
        assert!(t.starts_with("[earlier output cut]\nline "));
        assert!(t.ends_with("line 4999"));
    }

    #[test]
    fn keys_send_what_xterm_sends() {
        assert_eq!(key("Enter").unwrap(), b"\r");
        assert_eq!(key("Ctrl-C").unwrap(), [3]);
        assert_eq!(key("C-c").unwrap(), [3]);
        assert_eq!(key("ctrl+d").unwrap(), [4]);
        assert_eq!(key("Up").unwrap(), b"\x1b[A");
        assert_eq!(key("Alt-b").unwrap(), b"\x1bb");
        assert_eq!(key("M-Left").unwrap(), b"\x1b\x1b[D");
        assert_eq!(key("F5").unwrap(), b"\x1b[15~");
        assert!(key("Hyper").is_none());
        assert!(key("Ctrl-1").is_none());
    }
}
