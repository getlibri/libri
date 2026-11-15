//! Skills: Agent Skills folders (agentskills.io), each a `SKILL.md` with
//! `name` and `description` frontmatter and whatever files it brings.
//!
//! The person's own skills are theirs: Libri lists them and never writes,
//! moves or deletes one. Libri's skills, made in Settings or by a model, live
//! in Libri's state (`skills/`), and only while the person has switched skill
//! management on are they installed where the harnesses look: once into
//! `~/.agents/skills`, which Codex, OpenCode, Muse, omp and Pi read, and as a
//! link in Claude Code's folder, which is the one harness that doesn't. Each
//! installed folder carries a manifest of the files Libri wrote and their
//! hashes (as Paseo installs its own skills), so Libri removes only those, and
//! only unchanged: a file the person edited or added stays. A name the person
//! already uses in any of the harnesses' folders is theirs; Libri's skill of
//! that name is left out and the clash shown. Workers run the same harnesses
//! in the same home, so they see the same skills.
//!
//! Libri's skills come from two places: those made in Settings or by a model,
//! in its store, and those of a git repository the person names as the
//! source, whose top-level folders are skills. Libri keeps a shallow checkout
//! of the source in its state, pulls it when the daemon starts, when the
//! person asks, and before a turn once an hour has passed, and installs its
//! skills as it does the store's. The source's are edited in the repository,
//! not in Settings; a store skill by a name the source has is left out.

use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The manifest in each folder Libri installed: the files it wrote, by path
/// within the folder, with their SHA-256.
const MANIFEST: &str = ".libri-managed.json";
const OWNER: &str = "libri";
/// A folder being written beside `skills/`, renamed into place once whole.
const STAGE: &str = ".libri-stage-";
/// The folder an update replaces, kept until the new one is in place.
const OLD: &str = ".libri-old-";

/// A folder as the person knows it: under the home, from `~`.
fn shown(dir: &Path) -> String {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from);
    match home.as_deref().and_then(|h| dir.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    }
}

/// Whether skill management is on: absent, it is off, and Libri writes
/// nothing where the harnesses look.
const MANAGED: crate::model::Kept = crate::model::Kept("skills-managed");

/// The standard's limit on a description.
const LONGEST_DESCRIPTION: usize = 1024;
/// Far above any instructions worth loading into a turn, and below what a
/// harness would choke on.
const LONGEST: usize = 256 * 1024;

/// The standard's name rule: 1-64 of a-z, 0-9 and single hyphens, neither
/// first nor last.
pub fn valid_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--");
    if ok {
        Ok(())
    } else {
        Err(
            "A skill's name is up to 64 lowercase letters, digits and single hyphens, \
             such as weekly-report."
                .into(),
        )
    }
}

/// A frontmatter scalar as YAML writes it: plain, quoted, or a `|`/`>` block
/// over the indented lines after it. Enough for name and description, the
/// two fields read here; the rest of the frontmatter is kept as written.
fn scalar(first: &str, rest: &[&str]) -> String {
    let v = first.trim();
    if let Some(style) = v.chars().next().filter(|c| *c == '|' || *c == '>') {
        let lines: Vec<&str> = rest.iter().map(|l| l.trim()).collect();
        return if style == '|' {
            lines.join("\n")
        } else {
            lines.join(" ")
        };
    }
    let mut out = v.to_string();
    for more in rest {
        out.push(' ');
        out.push_str(more.trim());
    }
    if out.len() >= 2 && out.starts_with('"') && out.ends_with('"') {
        return serde_json::from_str(&out).unwrap_or_else(|_| out[1..out.len() - 1].into());
    }
    if out.len() >= 2 && out.starts_with('\'') && out.ends_with('\'') {
        return out[1..out.len() - 1].replace("''", "'");
    }
    out
}

/// A SKILL.md split into its frontmatter's top-level entries (key and raw
/// lines) and its body.
/// A frontmatter entry: its key and the lines it is written over.
type Entry<'a> = (String, Vec<&'a str>);

fn split(text: &str) -> Option<(Vec<Entry<'_>>, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    // The frontmatter ends at the first line that is only ---.
    let mut at = 0;
    let (head, body) = loop {
        let line_end = rest[at..]
            .find('\n')
            .map(|i| at + i + 1)
            .unwrap_or(rest.len());
        if rest[at..line_end].trim_end() == "---" {
            break (&rest[..at], &rest[line_end..]);
        }
        if line_end == rest.len() {
            return None;
        }
        at = line_end;
    };
    let mut entries: Vec<Entry> = Vec::new();
    for line in head.lines() {
        let top = !line.starts_with([' ', '\t']) && !line.trim().is_empty();
        match line.split_once(':').filter(|_| top) {
            Some((key, _)) => entries.push((key.trim().to_string(), vec![line])),
            None => match entries.last_mut() {
                Some(e) => e.1.push(line),
                None if line.trim().is_empty() => {}
                None => entries.push((String::new(), vec![line])),
            },
        }
    }
    Some((entries, body))
}

fn field(entries: &[Entry], key: &str) -> Option<String> {
    let (_, lines) = entries.iter().find(|(k, _)| k == key)?;
    let first = lines[0].split_once(':').map(|(_, v)| v).unwrap_or("");
    Some(scalar(first, &lines[1..]))
}

/// The name, description and body of a SKILL.md, or what is wrong with it.
pub fn parse(text: &str) -> Result<(String, String, String), String> {
    let (entries, body) =
        split(text).ok_or("SKILL.md doesn't start with frontmatter between --- lines.")?;
    let name = field(&entries, "name").ok_or("SKILL.md has no name.")?;
    let description = field(&entries, "description").ok_or("SKILL.md has no description.")?;
    Ok((
        name,
        description,
        body.trim_start_matches(['\n', '\r']).to_string(),
    ))
}

/// A SKILL.md with this name, description and body, keeping any other
/// frontmatter of `old` (license, allowed-tools, metadata) as written.
pub fn compose(old: Option<&str>, name: &str, description: &str, body: &str) -> String {
    let kept: Vec<String> = old
        .and_then(split)
        .map(|(entries, _)| {
            entries
                .into_iter()
                .filter(|(k, _)| k != "name" && k != "description")
                .flat_map(|(_, lines)| lines.into_iter().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    // A JSON string is a YAML double-quoted scalar: no character of the
    // person's can end it early.
    let quoted = serde_json::to_string(description.trim()).unwrap_or_default();
    let mut out = format!("---\nname: {name}\ndescription: {quoted}\n");
    for line in kept {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("---\n\n");
    out.push_str(body.trim_end());
    out.push('\n');
    out
}

fn check(name: &str, description: &str, body: &str) -> Result<(), String> {
    valid_name(name)?;
    let d = description.trim();
    if d.is_empty() {
        return Err(
            "Say when the skill is for: the models pick a skill by its description.".into(),
        );
    }
    if d.chars().count() > LONGEST_DESCRIPTION {
        return Err(format!(
            "A description is up to {LONGEST_DESCRIPTION} characters."
        ));
    }
    if body.len() > LONGEST {
        return Err(format!(
            "A skill's instructions are up to {} KB.",
            LONGEST / 1024
        ));
    }
    Ok(())
}

/// Where skills are, for one home.
pub struct Places {
    /// Libri's own skills, which only Libri writes.
    pub store: PathBuf,
    /// The checkout of the source repository, whose folders are skills too.
    pub source: PathBuf,
    /// The open standard's user folder, where Libri installs its skills.
    pub agents: PathBuf,
    /// Claude Code's folder, by its own rule; None without a home for it.
    pub claude: Option<PathBuf>,
    /// Folders Libri reads and never writes: Codex's own, OpenCode's, Pi's.
    pub codex: PathBuf,
    pub opencode: PathBuf,
    pub pi: PathBuf,
}

impl Places {
    /// Libri's state, which holds the store.
    fn state(&self) -> &Path {
        self.store.parent().unwrap_or(&self.store)
    }

    /// This daemon's, from its state directory and home.
    pub fn here() -> Option<Places> {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = var("HOME")?;
        let state = var("STATE_DIRECTORY")?;
        let claude = crate::model::claude_home();
        Some(Places {
            store: state.join("skills"),
            source: state.join("skills-source"),
            agents: home.join(".agents/skills"),
            claude: (!claude.as_os_str().is_empty()).then(|| claude.join("skills")),
            codex: var("CODEX_HOME")
                .unwrap_or_else(|| home.join(".codex"))
                .join("skills"),
            opencode: var("XDG_CONFIG_HOME")
                .unwrap_or_else(|| home.join(".config"))
                .join("opencode/skills"),
            pi: home.join(".pi/agent/skills"),
        })
    }

    /// The user folders a harness reads, first one first, as each harness's
    /// own discovery finds them.
    fn read_by(&self, harness: &str) -> Vec<&Path> {
        let claude = self.claude.as_deref();
        let (a, c, x, o, p) = (
            Some(self.agents.as_path()),
            claude,
            Some(self.codex.as_path()),
            Some(self.opencode.as_path()),
            Some(self.pi.as_path()),
        );
        let order = match harness {
            "claude" => vec![c],
            "codex" => vec![a, x],
            "opencode" => vec![o, a, c],
            "muse" => vec![a, c, x],
            "pi" => vec![p, a],
            _ => vec![a],
        };
        order.into_iter().flatten().collect()
    }

    /// Every folder a person's skill may be in, for clashes and the list.
    fn theirs(&self) -> Vec<&Path> {
        let mut all = vec![self.agents.as_path()];
        all.extend(self.claude.as_deref());
        all.extend([
            self.codex.as_path(),
            self.opencode.as_path(),
            self.pi.as_path(),
        ]);
        all
    }
}

pub fn managed(state: &Path) -> bool {
    MANAGED.read(state).as_deref() == Some("on")
}

/// Switches skill management on or off, and brings the folders in line:
/// off, Libri's installed files are taken back and nothing else is touched.
pub fn set_managed(state: &Path, places: &Places, on: bool) -> Result<Report, String> {
    if on {
        MANAGED.write(state, "on")?;
    } else {
        MANAGED.clear(state);
    }
    sync(places, on)
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// What follows the frontmatter: the instructions a harness loads when
    /// the skill is used.
    pub body: String,
    /// Files beside SKILL.md, which Settings doesn't edit but keeps.
    pub files: usize,
    /// From the source repository, edited there rather than in Settings.
    pub sourced: bool,
}

/// A name in use both by one of Libri's skills and by the person.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Clash {
    pub name: String,
    /// The folders the person's own is in.
    pub places: Vec<String>,
}

/// What a sync left as it was, for Settings to say.
#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Report {
    pub on: bool,
    /// Libri's skills not installed, because the person has one by the name.
    pub clashes: Vec<Clash>,
    /// Libri's installed skills the person has changed by hand, which Libri
    /// no longer updates.
    pub edited: Vec<String>,
}

/// A skill of the person's own, outside Libri.
#[derive(Serialize, Debug, PartialEq)]
pub struct Theirs {
    pub name: String,
    pub description: String,
    pub places: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct Shelf {
    /// Where Libri's skills are, for a model asked to make one.
    pub dir: String,
    pub skills: Vec<Skill>,
    /// Folders in Libri's store that aren't a valid skill, and why.
    pub broken: Vec<(String, String)>,
    #[serde(flatten)]
    pub report: Report,
    pub theirs: Vec<Theirs>,
    pub source: Source,
}

/// One entry of the composer's skill menu.
#[derive(Serialize, Debug, PartialEq)]
pub struct Offer {
    pub name: String,
    pub description: String,
    /// "libri" for one Libri installed, else "yours".
    pub source: &'static str,
}

/// The skills each harness has, and how its person names one in a message:
/// Claude Code takes a skill as `/name`, the others as `$name`, Codex's own.
#[derive(Serialize, Debug)]
pub struct Menu {
    pub mention: &'static str,
    pub skills: Vec<Offer>,
}

pub fn mention(harness: &str) -> &'static str {
    if harness == "claude" { "/" } else { "$" }
}

/// A SKILL.md read: name, description and body, or what is wrong with it.
type Read = Result<(String, String, String), String>;

/// The skill folders in `dir`, by name, each read; links followed, as the
/// harnesses follow them.
fn folders(dir: &Path) -> Vec<(String, PathBuf, Read)> {
    let mut out: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(n, _)| !n.starts_with('.'))
        .map(|(n, p)| {
            let read = fs::read_to_string(p.join("SKILL.md"))
                .map_err(|_| "No SKILL.md in it.".to_string())
                .and_then(|t| parse(&t));
            (n, p, read)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Libri's skills, and every folder in its store that isn't one.
pub fn shelf(places: &Places, report: Report) -> Shelf {
    let mut skills = Vec::new();
    let mut broken = Vec::new();
    for (sourced, folder, path, read) in libris(places) {
        match read {
            Ok((name, _, _)) if name != folder => {
                broken.push((folder, format!("Its SKILL.md is named {name}.")))
            }
            Ok(_) if !sourced && sourced_names(places).contains(&folder) => broken.push((
                folder,
                "The source has a skill by this name, so this one is left out.".into(),
            )),
            Ok((name, description, body)) => skills.push(Skill {
                name,
                description,
                body,
                files: hashes(&path)
                    .map(|h| h.len())
                    .unwrap_or(1)
                    .saturating_sub(1),
                sourced,
            }),
            Err(why) => broken.push((folder, why)),
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    let mut theirs: Vec<Theirs> = Vec::new();
    for dir in places.theirs() {
        for (folder, path, read) in folders(dir) {
            if ours(&path, places) {
                continue;
            }
            let Ok((name, description, _)) = read else {
                continue;
            };
            let place = shown(dir);
            match theirs.iter_mut().find(|t| t.name == folder) {
                Some(t) if !t.places.contains(&place) => t.places.push(place),
                Some(_) => {}
                None => theirs.push(Theirs {
                    name,
                    description,
                    places: vec![place],
                }),
            }
        }
    }
    theirs.sort_by(|a, b| a.name.cmp(&b.name));
    Shelf {
        dir: places.store.display().to_string(),
        skills,
        broken,
        report,
        theirs,
        source: source(places),
    }
}

/// What a harness finds, as it finds it: the first folder by a name wins.
pub fn menu(places: &Places, harness: &str) -> Menu {
    let mut skills: Vec<Offer> = Vec::new();
    for dir in places.read_by(harness) {
        for (folder, path, read) in folders(dir) {
            let Ok((name, description, _)) = read else {
                continue;
            };
            if skills.iter().any(|s| s.name == folder) {
                continue;
            }
            let source = if ours(&path, places) {
                "libri"
            } else {
                "yours"
            };
            skills.push(Offer {
                name,
                description,
                source,
            });
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Menu {
        mention: mention(harness),
        skills,
    }
}

/// Saves one of Libri's skills: a new one is made, an existing one's
/// SKILL.md rewritten, its other files untouched.
pub fn save(places: &Places, name: &str, description: &str, body: &str) -> Result<(), String> {
    check(name, description, body)?;
    if sourced_names(places).iter().any(|n| n == name) {
        return Err(format!(
            "The source has a skill named {name}; change it there, or pick another name."
        ));
    }
    let folder = places.store.join(name);
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let file = folder.join("SKILL.md");
    let old = fs::read_to_string(&file).ok();
    let text = compose(old.as_deref(), name, description, body);
    let tmp = folder.join(format!(".SKILL.md.{}.tmp", std::process::id()));
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))
}

/// Deletes one of Libri's skills from its store; the next sync takes back
/// what was installed of it.
pub fn delete(store: &Path, name: &str) -> Result<(), String> {
    valid_name(name)?;
    let folder = store.join(name);
    if !folder.join("SKILL.md").is_file() {
        return Err(format!("There is no skill named {name}."));
    }
    fs::remove_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The regular files under `dir` by path within it, with their SHA-256; the
/// manifest itself left out, links not followed.
fn hashes(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<(), String> {
        let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for e in entries.flatten() {
            let path = e.path();
            let kind = e.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                if rel == MANIFEST {
                    continue;
                }
                let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let sum = ring::digest::digest(&ring::digest::SHA256, &bytes);
                out.insert(rel, hex(sum.as_ref()));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// The files a folder's manifest says Libri wrote; None for a folder Libri
/// didn't install.
fn manifest(dir: &Path) -> Option<BTreeMap<String, String>> {
    // A link is never Libri's install: it writes real folders.
    if fs::symlink_metadata(dir).ok()?.file_type().is_symlink() {
        return None;
    }
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join(MANIFEST)).ok()?).ok()?;
    (v["owner"] == OWNER && v["version"] == 1).then_some(())?;
    serde_json::from_value(v["files"].clone()).ok()
}

/// Whether `path`, in any of the folders, is Libri's: a folder it installed,
/// or the link it made to one.
fn ours(path: &Path, places: &Places) -> bool {
    if manifest(path).is_some() {
        return true;
    }
    let Some(name) = path.file_name() else {
        return false;
    };
    let installed = places.agents.join(name);
    fs::read_link(path).is_ok_and(|to| to == installed) && manifest(&installed).is_some()
}

/// Whether two paths are one folder: a person who links Claude Code's
/// folder to the standard's gets no second link per skill.
fn same(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

enum Installed {
    Done,
    /// Changed by hand since Libri wrote it: left as it is.
    Edited,
}

/// Writes `src` to `dst` with its manifest: whole in a staged folder first,
/// then renamed into place, the old one kept until the new one stands and
/// put back if it can't.
fn install(src: &Path, dst: &Path, name: &str) -> Result<Installed, String> {
    let want = hashes(src)?;
    let there = fs::symlink_metadata(dst).is_ok();
    if there {
        let have = manifest(dst).ok_or_else(|| format!("{} isn't Libri's.", dst.display()))?;
        if hashes(dst)? != have {
            return Ok(Installed::Edited);
        }
        if have == want {
            return Ok(Installed::Done);
        }
    }
    let skills = dst.parent().ok_or("No folder for skills.")?;
    let root = skills.parent().ok_or("No folder for skills.")?;
    fs::create_dir_all(skills).map_err(|e| format!("{}: {e}", skills.display()))?;
    let stage = root.join(format!("{STAGE}{name}"));
    let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| err(&stage, e))?;
    }
    for rel in want.keys() {
        let to = stage.join(rel);
        if let Some(up) = to.parent() {
            fs::create_dir_all(up).map_err(|e| err(up, e))?;
        }
        fs::copy(src.join(rel), &to).map_err(|e| err(&to, e))?;
    }
    let record = serde_json::json!({ "owner": OWNER, "version": 1, "files": want });
    fs::write(stage.join(MANIFEST), format!("{record:#}\n")).map_err(|e| err(&stage, e))?;
    if there {
        let old = root.join(format!("{OLD}{name}"));
        if old.exists() {
            fs::remove_dir_all(&old).map_err(|e| err(&old, e))?;
        }
        fs::rename(dst, &old).map_err(|e| err(dst, e))?;
        if let Err(e) = fs::rename(&stage, dst) {
            fs::rename(&old, dst).ok();
            return Err(err(dst, e));
        }
        fs::remove_dir_all(&old).map_err(|e| err(&old, e))?;
    } else {
        fs::rename(&stage, dst).map_err(|e| err(dst, e))?;
    }
    Ok(Installed::Done)
}

/// Takes back an installed folder: the files Libri wrote and nobody has
/// changed, then the manifest, then the folder if nothing else is in it.
fn retire(dst: &Path) -> Result<(), String> {
    let Some(files) = manifest(dst) else {
        return Ok(());
    };
    let now = hashes(dst)?;
    for (rel, sum) in &files {
        if now.get(rel) == Some(sum) {
            let file = dst.join(rel);
            fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            // Folders the file was in go once empty; the person's stay.
            let mut up = file.parent();
            while let Some(d) = up.filter(|d| *d != dst) {
                if fs::remove_dir(d).is_err() {
                    break;
                }
                up = d.parent();
            }
        }
    }
    fs::remove_file(dst.join(MANIFEST)).map_err(|e| format!("{}: {e}", dst.display()))?;
    fs::remove_dir(dst).ok();
    Ok(())
}

/// The folders, by name, where the person has a skill called `name` of
/// their own.
fn clash(places: &Places, name: &str) -> Vec<String> {
    places
        .theirs()
        .into_iter()
        .filter(|dir| {
            let at = dir.join(name);
            fs::symlink_metadata(&at).is_ok() && !ours(&at, places)
        })
        .map(shown)
        .collect()
}

/// The link in Claude Code's folder to an installed skill, if Libri made it.
fn unlink(places: &Places, name: &str) -> Result<(), String> {
    let Some(claude) = &places.claude else {
        return Ok(());
    };
    let at = claude.join(name);
    if fs::read_link(&at).is_ok_and(|to| to == places.agents.join(name)) {
        fs::remove_file(&at).map_err(|e| format!("{}: {e}", at.display()))?;
    }
    Ok(())
}

fn link(places: &Places, name: &str) -> Result<(), String> {
    let Some(claude) = &places.claude else {
        return Ok(());
    };
    // Without Claude Code's own folder above it, there's no Claude Code.
    if !claude.parent().is_some_and(Path::is_dir) || same(claude, &places.agents) {
        return Ok(());
    }
    let at = claude.join(name);
    if fs::symlink_metadata(&at).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(claude).map_err(|e| format!("{}: {e}", claude.display()))?;
    std::os::unix::fs::symlink(places.agents.join(name), &at)
        .map_err(|e| format!("{}: {e}", at.display()))
}

/// Folders a sync stopped in the middle of: a staged one is dropped, an old
/// one is put back where its new one never arrived.
fn recover(places: &Places) {
    let Some(root) = places.agents.parent() else {
        return;
    };
    for e in fs::read_dir(root).into_iter().flatten().flatten() {
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if name.starts_with(STAGE) {
            fs::remove_dir_all(e.path()).ok();
        } else if let Some(skill) = name.strip_prefix(OLD) {
            let dst = places.agents.join(skill);
            if fs::symlink_metadata(&dst).is_err() {
                fs::rename(e.path(), &dst).ok();
            } else {
                fs::remove_dir_all(e.path()).ok();
            }
        }
    }
}

/// The git repository Libri takes skills from, as the person named it.
const SOURCE: crate::model::Kept = crate::model::Kept("skills-source-url");
/// How long a pulled source stands before a turn pulls it again.
const PULL_EVERY: Duration = Duration::from_secs(60 * 60);

/// Where the source stands, for Settings.
#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Source {
    /// The repository as the person named it; None without one.
    pub url: Option<String>,
    /// The checkout's commit, short.
    pub commit: Option<String>,
    /// Seconds since the last pull that worked, in this daemon's life.
    pub pulled_ago: Option<u64>,
    /// Why the last pull failed, until one works.
    pub failed: Option<String>,
    pub pulling: bool,
}

#[derive(Default)]
struct Pulls {
    /// The last pull begun, which a turn waits an hour after.
    begun: Option<Instant>,
    worked: Option<Instant>,
    failed: Option<String>,
    pulling: bool,
}

/// Each state's pulls; one daemon has one, tests several.
static PULLS: Mutex<BTreeMap<PathBuf, Pulls>> = Mutex::new(BTreeMap::new());

fn pulls<T>(state: &Path, f: impl FnOnce(&mut Pulls) -> T) -> T {
    let mut all = PULLS.lock().unwrap_or_else(|e| e.into_inner());
    f(all.entry(state.to_owned()).or_default())
}

/// Libri's skill folders, the source's first: (from the source, folder,
/// path, read).
fn libris(places: &Places) -> Vec<(bool, String, PathBuf, Read)> {
    let sourced = folders(&places.source)
        .into_iter()
        .map(|(f, p, r)| (true, f, p, r));
    let stored = folders(&places.store)
        .into_iter()
        .map(|(f, p, r)| (false, f, p, r));
    sourced.chain(stored).collect()
}

/// The names the source's folders take.
fn sourced_names(places: &Places) -> Vec<String> {
    folders(&places.source)
        .into_iter()
        .map(|(f, _, _)| f)
        .collect()
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    // Pulled with no one to answer: a prompt for a password or a host key
    // fails the pull rather than holding it, and a stalled transfer ends.
    cmd.args([
        "-c",
        "http.lowSpeedLimit=1000",
        "-c",
        "http.lowSpeedTime=30",
    ])
    .args(args)
    .env("GIT_TERMINAL_PROMPT", "0")
    .stdin(std::process::Stdio::null());
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=20 -o ServerAliveInterval=15 -o ServerAliveCountMax=2",
        );
    }
    let out = cmd.output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("git failed")
            .trim()
            .to_owned())
    }
}

/// Where the source stands now.
pub fn source(places: &Places) -> Source {
    let state = places.state();
    let (pulled_ago, failed, pulling) = pulls(state, |p| {
        (
            p.worked.map(|t| t.elapsed().as_secs()),
            p.failed.clone(),
            p.pulling,
        )
    });
    Source {
        url: SOURCE.read(state),
        commit: git(Some(&places.source), &["rev-parse", "--short", "HEAD"]).ok(),
        pulled_ago,
        failed,
        pulling,
    }
}

/// Brings the checkout to the source's newest commit on its default
/// branch, cloning it afresh when there is none or it is of another
/// repository; without a source, the checkout goes.
fn fetch(places: &Places, url: Option<&str>) -> Result<(), String> {
    let dir = &places.source;
    let Some(url) = url else {
        if dir.exists() {
            fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        return Ok(());
    };
    let same = git(Some(dir), &["remote", "get-url", "origin"]).is_ok_and(|u| u == url);
    if same {
        git(
            Some(dir),
            &["fetch", "--quiet", "--depth", "1", "origin", "HEAD"],
        )?;
        git(Some(dir), &["reset", "--quiet", "--hard", "FETCH_HEAD"])?;
        git(Some(dir), &["clean", "--quiet", "-ffdx"])?;
        return Ok(());
    }
    // Cloned beside and renamed over, so a failed clone leaves the last
    // checkout in place.
    let stage = dir.with_file_name(".skills-source-clone");
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
    }
    let to = stage.to_string_lossy().into_owned();
    git(None, &["clone", "--quiet", "--depth", "1", "--", url, &to])?;
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::rename(&stage, dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Pulls the source and brings the harnesses' folders in line with it.
/// One pull at a time: a second asked meanwhile returns at once.
pub fn pull(state: &Path, places: &Places) -> Result<Report, String> {
    let busy = pulls(state, |p| {
        let busy = p.pulling;
        if !busy {
            p.pulling = true;
            p.begun = Some(Instant::now());
        }
        busy
    });
    if busy {
        return sync(places, managed(state));
    }
    let fetched = fetch(places, SOURCE.read(state).as_deref());
    pulls(state, |p| {
        p.pulling = false;
        match &fetched {
            Ok(()) => {
                p.worked = Some(Instant::now());
                p.failed = None;
            }
            Err(e) => p.failed = Some(e.clone()),
        }
    });
    // A failed pull leaves the last checkout, whose skills still stand.
    sync(places, managed(state))
}

/// Names the source, or with None forgets it and its skills; then pulls.
pub fn set_source(state: &Path, places: &Places, url: Option<&str>) -> Result<Report, String> {
    match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => {
            if u.starts_with('-') || u.chars().any(char::is_whitespace) {
                return Err("A source is a git repository's address, such as git@example.com:me/skills.git.".into());
            }
            SOURCE.write(state, u)?;
        }
        None => SOURCE.clear(state),
    }
    pulls(state, |p| p.failed = None);
    pull(state, places)
}

/// A pull in the background when the source is due one: never pulled in
/// this daemon's life, or an hour since.
fn pull_if_due(state: &Path) {
    if SOURCE.read(state).is_none() {
        return;
    }
    if pulls(state, |p| {
        p.pulling || p.begun.is_some_and(|t| t.elapsed() < PULL_EVERY)
    }) {
        return;
    }
    let state = state.to_owned();
    std::thread::spawn(move || {
        if let Some(places) = Places::here()
            && let Err(e) = pull(&state, &places)
        {
            eprintln!("libri: skills: {e}");
        }
    });
}

/// One sync at a time: a turn and Settings may both start one.
static SYNCING: Mutex<()> = Mutex::new(());

/// Brings the harnesses' folders in line with Libri's skills: on, each one
/// without a clash installed and linked for Claude Code; off, or for one
/// gone or clashing, what Libri installed taken back. Nothing that isn't
/// Libri's is written, moved or deleted.
pub fn sync(places: &Places, on: bool) -> Result<Report, String> {
    let _held = SYNCING.lock().unwrap_or_else(|e| e.into_inner());
    recover(places);
    let mut report = Report {
        on,
        ..Report::default()
    };
    let mut wanted = Vec::new();
    if on {
        let sourced = sourced_names(places);
        for (from_source, folder, path, read) in libris(places) {
            if !matches!(&read, Ok((name, _, _)) if *name == folder)
                || valid_name(&folder).is_err()
                || (!from_source && sourced.contains(&folder))
            {
                continue;
            }
            let theirs = clash(places, &folder);
            if !theirs.is_empty() {
                report.clashes.push(Clash {
                    name: folder,
                    places: theirs,
                });
                continue;
            }
            let dst = places.agents.join(&folder);
            match install(&path, &dst, &folder)? {
                Installed::Edited => report.edited.push(folder.clone()),
                Installed::Done => {}
            }
            link(places, &folder)?;
            wanted.push(folder);
        }
    }
    for (folder, path, _) in folders(&places.agents) {
        if wanted.contains(&folder) || manifest(&path).is_none() {
            continue;
        }
        unlink(places, &folder)?;
        retire(&path)?;
    }
    Ok(report)
}

/// The sync for this daemon's home, before a turn, which then sees a skill
/// a model just made in Libri's store.
pub fn sync_here() -> Option<Report> {
    let places = Places::here()?;
    let state = PathBuf::from(std::env::var_os("STATE_DIRECTORY")?);
    pull_if_due(&state);
    sync(&places, managed(&state))
        .map_err(|e| eprintln!("libri: skills: {e}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_standard() {
        for ok in ["a", "weekly-report", "pdf2", &"x".repeat(64)] {
            assert!(valid_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "Weekly",
            "-a",
            "a-",
            "a--b",
            "a_b",
            "a b",
            "../x",
            &"x".repeat(65),
        ] {
            assert!(valid_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_the_frontmatter_as_yaml_writes_it() {
        let text = "---\nname: pdf\ndescription: >\n  Fill PDF forms.\n  Use for PDFs.\nlicense: MIT\n---\n\n# PDF\n\nSteps.\n";
        let (name, description, body) = parse(text).unwrap();
        assert_eq!(name, "pdf");
        assert_eq!(description, "Fill PDF forms. Use for PDFs.");
        assert_eq!(body, "# PDF\n\nSteps.\n");
        let quoted = "---\nname: q\ndescription: \"Say \\\"hi\\\": then go\"\n---\nBody";
        assert_eq!(parse(quoted).unwrap().1, "Say \"hi\": then go");
        let single = "---\nname: q\ndescription: 'It''s here'\n---\n";
        assert_eq!(parse(single).unwrap().1, "It's here");
        assert!(parse("# No frontmatter").is_err());
        assert!(parse("---\nname: x\n---\nbody").is_err());
    }

    #[test]
    fn a_rewrite_keeps_the_other_fields() {
        let old = "---\nname: pdf\ndescription: old\nlicense: MIT\nmetadata:\n  author: Ada\n---\nOld body\n";
        let new = compose(Some(old), "pdf", "When: a form", "New body");
        assert_eq!(
            new,
            "---\nname: pdf\ndescription: \"When: a form\"\nlicense: MIT\nmetadata:\n  author: Ada\n---\n\nNew body\n"
        );
        let (name, description, body) = parse(&new).unwrap();
        assert_eq!(
            (name.as_str(), description.as_str(), body.as_str()),
            ("pdf", "When: a form", "New body\n")
        );
    }

    fn places(name: &str) -> Places {
        let p = std::env::temp_dir().join(format!("libri-skills-{}-{name}", std::process::id()));
        fs::remove_dir_all(&p).ok();
        fs::create_dir_all(p.join("home/.claude")).unwrap();
        Places {
            store: p.join("state/skills"),
            source: p.join("state/skills-source"),
            agents: p.join("home/.agents/skills"),
            claude: Some(p.join("home/.claude/skills")),
            codex: p.join("home/.codex/skills"),
            opencode: p.join("home/.config/opencode/skills"),
            pi: p.join("home/.pi/agent/skills"),
        }
    }

    fn mine(dir: &Path, name: &str) {
        fs::create_dir_all(dir.join(name)).unwrap();
        fs::write(
            dir.join(name).join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Mine.\n---\nMine.\n"),
        )
        .unwrap();
    }

    #[test]
    fn off_writes_nothing() {
        let p = places("off");
        save(&p, "weekly-report", "Fridays.", "Write it.").unwrap();
        assert_eq!(sync(&p, false).unwrap(), Report::default());
        assert!(!p.agents.exists());
        assert!(!p.claude.as_ref().unwrap().exists());
    }

    #[test]
    fn on_installs_once_and_links_for_claude_code() {
        let p = places("on");
        save(&p, "weekly-report", "Fridays.", "Write it.").unwrap();
        fs::write(p.store.join("weekly-report/template.md"), "x").unwrap();
        sync(&p, true).unwrap();
        let dst = p.agents.join("weekly-report");
        assert!(dst.join("template.md").is_file());
        assert_eq!(manifest(&dst).unwrap().len(), 2);
        let claude = p.claude.as_ref().unwrap();
        assert_eq!(fs::read_link(claude.join("weekly-report")).unwrap(), dst);
        // Codex reads the standard's folder: nothing is copied to its own.
        assert!(!p.codex.exists());
        // An edit in Libri reaches the installed copy.
        save(&p, "weekly-report", "Fridays.", "Write it well.").unwrap();
        sync(&p, true).unwrap();
        assert!(
            fs::read_to_string(dst.join("SKILL.md"))
                .unwrap()
                .contains("well")
        );
        let m = menu(&p, "claude");
        assert_eq!(m.mention, "/");
        assert_eq!(m.skills[0].source, "libri");
        assert_eq!(menu(&p, "codex").mention, "$");
    }

    #[test]
    fn off_takes_back_only_what_libri_wrote() {
        let p = places("back");
        save(&p, "weekly-report", "Fridays.", "Write it.").unwrap();
        mine(&p.agents, "notes");
        sync(&p, true).unwrap();
        let dst = p.agents.join("weekly-report");
        // A file the person adds to Libri's install is theirs.
        fs::write(dst.join("theirs.md"), "x").unwrap();
        let r = sync(&p, true).unwrap();
        assert_eq!(r.edited, ["weekly-report"]);
        sync(&p, false).unwrap();
        assert!(dst.join("theirs.md").is_file());
        assert!(!dst.join("SKILL.md").exists());
        assert!(!dst.join(MANIFEST).exists());
        assert!(fs::symlink_metadata(p.claude.as_ref().unwrap().join("weekly-report")).is_err());
        assert!(p.agents.join("notes/SKILL.md").is_file());
        // Libri's store keeps its skills for when it's on again.
        assert!(p.store.join("weekly-report/SKILL.md").is_file());
    }

    #[test]
    fn a_name_the_person_uses_is_theirs() {
        let p = places("clash");
        let claude = p.claude.clone().unwrap();
        for (dir, name) in [(&p.agents, "a"), (&claude, "b"), (&p.codex, "c")] {
            mine(dir, name);
            save(&p, name, "Libri's.", "Body").unwrap();
        }
        let r = sync(&p, true).unwrap();
        let names: Vec<&str> = r.clashes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        for (dir, name) in [(&p.agents, "a"), (&claude, "b"), (&p.codex, "c")] {
            assert!(
                fs::read_to_string(dir.join(name).join("SKILL.md"))
                    .unwrap()
                    .contains("Mine.")
            );
        }
        assert!(!p.agents.join("b").exists() && !p.agents.join("c").exists());
        // One the person makes after Libri installed its own wins, and Libri's
        // goes.
        save(&p, "d", "Libri's.", "Body").unwrap();
        sync(&p, true).unwrap();
        assert!(p.agents.join("d").is_dir());
        fs::remove_file(claude.join("d")).unwrap();
        mine(&claude, "d");
        sync(&p, true).unwrap();
        assert!(!p.agents.join("d").exists());
        assert!(claude.join("d/SKILL.md").is_file());
        let shelf = shelf(&p, Report::default());
        let theirs: Vec<&str> = shelf.theirs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(theirs, ["a", "b", "c", "d"]);
    }

    #[test]
    fn a_deleted_skill_is_taken_back() {
        let p = places("gone");
        save(&p, "x", "y", "z").unwrap();
        sync(&p, true).unwrap();
        delete(&p.store, "x").unwrap();
        sync(&p, true).unwrap();
        assert!(!p.agents.join("x").exists());
        assert!(fs::symlink_metadata(p.claude.as_ref().unwrap().join("x")).is_err());
    }

    #[test]
    fn a_sync_cut_short_is_put_right() {
        let p = places("cut");
        save(&p, "x", "y", "z").unwrap();
        sync(&p, true).unwrap();
        let root = p.agents.parent().unwrap();
        fs::rename(p.agents.join("x"), root.join(format!("{OLD}x"))).unwrap();
        fs::create_dir_all(root.join(format!("{STAGE}x"))).unwrap();
        sync(&p, true).unwrap();
        assert!(manifest(&p.agents.join("x")).is_some());
        assert!(!root.join(format!("{OLD}x")).exists() && !root.join(format!("{STAGE}x")).exists());
    }

    #[test]
    fn a_host_without_claude_code_gets_no_folder_for_it() {
        let mut p = places("none");
        p.claude = Some(p.store.parent().unwrap().join("no-claude/skills"));
        save(&p, "x", "y", "z").unwrap();
        sync(&p, true).unwrap();
        assert!(!p.claude.unwrap().parent().unwrap().exists());
    }

    #[test]
    fn refuses_what_the_standard_does() {
        let p = places("refuse");
        assert!(save(&p, "Bad Name", "d", "b").is_err());
        assert!(save(&p, "ok", "  ", "b").is_err());
        assert!(save(&p, "ok", &"d".repeat(1025), "b").is_err());
        assert!(delete(&p.store, "../skills").is_err());
        assert!(delete(&p.store, "missing").is_err());
    }

    /// A git repository of skills, as a source, with one commit.
    fn repo(at: &Path, names: &[&str]) -> String {
        fs::create_dir_all(at).unwrap();
        let g = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(at)
                .args(["-c", "user.name=Ada", "-c", "user.email=ada@example.com"])
                .args(args)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        g(&["init", "--quiet", "-b", "main"]);
        for n in names {
            mine(at, n);
        }
        fs::write(at.join("README.md"), "Skills.\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "--quiet", "-m", "Skills"]);
        at.display().to_string()
    }

    #[test]
    fn a_source_reaches_the_harnesses_and_goes_with_it() {
        let p = places("source");
        let state = p.state().to_owned();
        fs::create_dir_all(&state).unwrap();
        let root = state.parent().unwrap().to_owned();
        let url = repo(&root.join("shared-skills"), &["tutor", "council"]);
        set_managed(&state, &p, true).unwrap();
        set_source(&state, &p, Some(&url)).unwrap();
        assert_eq!(source(&p).failed, None);
        for n in ["tutor", "council"] {
            assert!(manifest(&p.agents.join(n)).is_some(), "{n} installed");
            assert!(
                fs::read_link(p.claude.as_ref().unwrap().join(n)).is_ok(),
                "{n} linked"
            );
        }
        let shelf = shelf(&p, Report::default());
        assert!(shelf.skills.iter().all(|s| s.sourced));
        assert_eq!(shelf.source.url.as_deref(), Some(url.as_str()));
        assert!(shelf.source.commit.is_some() && shelf.source.failed.is_none());

        // A new skill in the repository arrives with the next pull, one
        // taken out of it goes.
        let g = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(root.join("shared-skills"))
                    .args(["-c", "user.name=Ada", "-c", "user.email=ada@example.com"])
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        mine(&root.join("shared-skills"), "architect");
        g(&["rm", "-rq", "council"]);
        g(&["add", "-A"]);
        g(&["commit", "--quiet", "-m", "More"]);
        pull(&state, &p).unwrap();
        assert!(p.agents.join("architect/SKILL.md").is_file());
        assert!(!p.agents.join("council").exists());
        assert!(fs::symlink_metadata(p.claude.as_ref().unwrap().join("council")).is_err());

        // A skill of the store's by a source name is left out, and Settings
        // can't make one.
        assert!(save(&p, "tutor", "Mine.", "Body").is_err());
        mine(&p.store, "tutor");
        let shelf = super::shelf(&p, sync(&p, true).unwrap());
        assert!(shelf.broken.iter().any(|(n, _)| n == "tutor"));

        // Forgetting the source takes back its skills and its checkout.
        set_source(&state, &p, None).unwrap();
        assert!(!p.source.exists());
        assert!(!p.agents.join("architect").exists());
        // The store's tutor, no longer behind the source's, takes its place.
        let shelf = super::shelf(&p, Report::default());
        assert!(shelf.skills.iter().any(|s| s.name == "tutor" && !s.sourced));
        assert!(manifest(&p.agents.join("tutor")).is_some());
    }

    #[test]
    fn a_failed_pull_keeps_the_last_checkout() {
        let p = places("source-fail");
        let state = p.state().to_owned();
        fs::create_dir_all(&state).unwrap();
        let root = state.parent().unwrap().to_owned();
        let url = repo(&root.join("shared-skills"), &["tutor"]);
        set_managed(&state, &p, true).unwrap();
        set_source(&state, &p, Some(&url)).unwrap();
        fs::remove_dir_all(root.join("shared-skills")).unwrap();
        pull(&state, &p).unwrap();
        assert!(source(&p).failed.is_some());
        assert!(p.agents.join("tutor/SKILL.md").is_file());
        assert!(set_source(&state, &p, Some("--upload-pack=x")).is_err());
    }
}
