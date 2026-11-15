//! Keeps the server answering while its harnesses work.
//!
//! Where systemd delegates Libri its cgroup (`Delegate=yes`,
//! `DelegateSubgroup=server`), the daemon runs in the `server` leaf, and
//! each harness, with all it starts, and the terminal's shell move into a
//! `harness` cgroup beside it. `LIBRI_HARNESS_CGROUP` gives that cgroup its
//! limits as `file=value` pairs, e.g. `cpu.weight=20 io.weight=20
//! memory.high=8G`, so under load the agents' compiles slow down and give
//! up memory before the page does. `LIBRI_SERVER_CGROUP` does the same for
//! the daemon's own leaf (`memory.low=512M`): the service's protection
//! would otherwise go to its children by their use, mostly the harnesses'.
//! The agents' browser gets a `browser` cgroup of its own with a hard cap
//! (`LIBRI_BROWSER_CGROUP`, else `BROWSER_LIMITS`): a runaway page is
//! killed there, with the whole browser, before it can starve the host.
//! This is systemd's delegation model: a delegated service manages the
//! subtree below its own cgroup.
//!
//! Without the variables nothing moves, as on a host that delegates nothing.

use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The harness cgroup's `cgroup.procs`, open for the moves.
static HARNESS: OnceLock<File> = OnceLock::new();
/// The browser cgroup's `cgroup.procs`.
static BROWSER: OnceLock<File> = OnceLock::new();

/// The browser's limits unless `LIBRI_BROWSER_CGROUP` names others: one
/// Chromium with one tab sits near 1.5G; past 4G it is killed as a group.
const BROWSER_LIMITS: &str =
    "cpu.weight=20 memory.high=3G memory.max=4G memory.oom.group=1 pids.max=2048";

/// Makes the harness cgroup beside this process's own and applies
/// `LIBRI_HARNESS_CGROUP` to it, and `LIBRI_SERVER_CGROUP` to its own.
pub fn setup() -> Result<(), String> {
    let Ok(settings) = std::env::var("LIBRI_HARNESS_CGROUP") else {
        return Ok(());
    };
    let own = fs::read_to_string("/proc/self/cgroup").map_err(|e| format!("cgroup: {e}"))?;
    let leaf = leaf(&own)?;
    let parent = leaf.parent().ok_or("cgroup: at the root")?;
    let harness = parent.join("harness");
    let pairs = parse(&settings)?;
    let server = parse(&std::env::var("LIBRI_SERVER_CGROUP").unwrap_or_default())?;
    let browser =
        parse(&std::env::var("LIBRI_BROWSER_CGROUP").unwrap_or_else(|_| BROWSER_LIMITS.into()))?;
    // A limit takes effect only once the parent hands its controller down;
    // the parent holds no process (the daemon is in `server`), so it may.
    let mut controllers: Vec<&str> = pairs
        .iter()
        .chain(&server)
        .chain(&browser)
        .filter_map(|(file, _)| file.split_once('.').map(|(c, _)| c))
        .collect();
    controllers.sort_unstable();
    controllers.dedup();
    for c in controllers {
        write(&parent.join("cgroup.subtree_control"), &format!("+{c}"))?;
    }
    for (file, value) in &server {
        write(&leaf.join(file), value)?;
    }
    let _ = HARNESS.set(make(&harness, &pairs)?);
    let _ = BROWSER.set(make(&parent.join("browser"), &browser)?);
    Ok(())
}

/// Makes a cgroup if it is missing, applies its limits, and answers its
/// `cgroup.procs` open for moves.
fn make(dir: &Path, limits: &[(String, String)]) -> Result<File, String> {
    match fs::create_dir(dir) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => {
            return Err(format!("cgroup: {}: {e}", dir.display()));
        }
        _ => {}
    }
    for (file, value) in limits {
        write(&dir.join(file), value)?;
    }
    fs::OpenOptions::new()
        .write(true)
        .open(dir.join("cgroup.procs"))
        .map_err(|e| format!("cgroup: {}: {e}", dir.display()))
}

/// Starts `cmd` in the harness cgroup, where there is one: the child moves
/// itself before it execs, so nothing it starts is left behind in `server`.
pub fn harness(cmd: &mut Command) {
    enter(cmd, &HARNESS);
}

/// Starts Chromium (or its display) in the browser cgroup, where there is
/// one, and keeps it there: given the session bus, Chromium asks systemd
/// for a scope of its own under the user's manager and moves out of every
/// limit Libri set (Chromium 157).
pub fn browser(cmd: &mut Command) {
    cmd.env_remove("DBUS_SESSION_BUS_ADDRESS");
    enter(cmd, &BROWSER);
}

fn enter(cmd: &mut Command, group: &'static OnceLock<File>) {
    let Some(procs) = group.get() else {
        return;
    };
    let fd = procs.as_raw_fd();
    // SAFETY: one write(2), async-signal-safe, between fork and exec. "0"
    // moves the writer. A failed move leaves the child where it was, which
    // costs priority, not the run, so it is not an error.
    unsafe {
        cmd.pre_exec(move || {
            libc::write(fd, b"0".as_ptr().cast(), 1);
            Ok(())
        });
    }
}

/// Moves a process already started, for spawners that take no hook (the
/// terminal's pty). What it started in its first instant stays behind.
pub fn adopt(pid: u32) {
    if let Some(mut procs) = HARNESS.get() {
        let _ = io::Write::write_all(&mut procs, pid.to_string().as_bytes());
    }
}

/// This process's own cgroup directory, from `/proc/self/cgroup`, below
/// the root.
fn leaf(own: &str) -> Result<PathBuf, String> {
    // cgroup v2 has one line, `0::/system.slice/libri.service/server`.
    let path = own
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .ok_or("cgroup: not on the unified hierarchy")?;
    let leaf = Path::new("/sys/fs/cgroup").join(path.trim().trim_start_matches('/'));
    if leaf.parent() == Some(Path::new("/sys/fs/cgroup")) || leaf == Path::new("/sys/fs/cgroup") {
        return Err("cgroup: at the root".into());
    }
    Ok(leaf)
}

fn parse(settings: &str) -> Result<Vec<(String, String)>, String> {
    settings
        .split_whitespace()
        .map(|pair| {
            let (file, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("LIBRI_HARNESS_CGROUP: {pair}: not file=value"))?;
            // Files of the cgroup itself only: no path out of it.
            if file.contains('/') || !file.contains('.') {
                return Err(format!("LIBRI_HARNESS_CGROUP: {file}: not a cgroup file"));
            }
            Ok((file.to_string(), value.to_string()))
        })
        .collect()
}

fn write(path: &Path, value: &str) -> Result<(), String> {
    fs::write(path, value).map_err(|e| format!("cgroup: {} = {value}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemons_leaf_is_read_from_its_cgroup_line() {
        assert_eq!(
            leaf("0::/system.slice/libri.service/server\n").unwrap(),
            Path::new("/sys/fs/cgroup/system.slice/libri.service/server")
        );
        assert!(leaf("0::/\n").is_err());
        assert!(leaf("0::/libri.service\n").is_err());
        assert!(leaf("1:name=systemd:/x\n").is_err());
    }

    /// A harness cgroup's hard cap, its swap and the daemon's guarantee are
    /// written as given: past `memory.max` the kernel kills inside the
    /// harnesses, so their builds can't push the slice the daemon shares
    /// into throttling and swap (a 61G slice once froze the page).
    #[test]
    fn the_harness_cap_its_swap_and_the_daemons_guarantee_are_applied() {
        let dir = std::env::temp_dir().join(format!("libri-cgroup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("cgroup.procs"), "").unwrap();
        let caps =
            parse("cpu.weight=20 memory.high=28G memory.max=32G memory.swap.max=2G").unwrap();
        make(&dir, &caps).unwrap();
        for (file, value) in [
            ("memory.high", "28G"),
            ("memory.max", "32G"),
            ("memory.swap.max", "2G"),
        ] {
            assert_eq!(fs::read_to_string(dir.join(file)).unwrap(), value, "{file}");
        }
        assert_eq!(
            parse("memory.min=1G memory.low=1G").unwrap(),
            vec![
                ("memory.min".to_string(), "1G".to_string()),
                ("memory.low".to_string(), "1G".to_string())
            ]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_name_files_of_the_cgroup_only() {
        assert_eq!(
            parse("cpu.weight=20  memory.high=8G").unwrap(),
            vec![
                ("cpu.weight".to_string(), "20".to_string()),
                ("memory.high".to_string(), "8G".to_string())
            ]
        );
        assert!(parse("../x.y=1").is_err());
        assert!(parse("weight=1").is_err());
        assert!(parse("cpu.weight").is_err());
    }
}
