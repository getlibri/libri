//! The vault: a folder of Markdown notes, kept by Obsidian's own
//! conventions, so the app opens it as it is. A note is `Name.md` in the
//! vault or a folder of it, and goes by its name alone: links are
//! `[[Name]]`, `[[Name#Heading]]` or `[[Name|alias]]` and resolve by name,
//! whatever folder the note sits in, so no two notes share one (Obsidian
//! matches names without regard to case). Properties are YAML frontmatter.
//! Attachments sit in `attachments/`, embedded as `![[file.png]]`. Obsidian's
//! own `.obsidian/` folder is the app's: no note goes there.

use regex::{Regex, RegexBuilder};
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

/// The most a note may hold: one past it is a sign to split it.
pub const NOTE_MAX: usize = 256 * 1024;

/// Where attachments go, as Obsidian's "Default location for new
/// attachments" set to a folder names it.
pub const ATTACHMENTS: &str = "attachments";

/// The most a search answers with, so one broad pattern can't flood a turn.
const HITS_MAX: usize = 200;
const ANSWER_MAX: usize = 32 * 1024;

/// A Markdown link to a note, `[text](Some note.md)`, which Obsidian's
/// wikilink setting leaves out: its links go by name, and survive a move.
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\]\(\s*(<[^>]*\.md(#[^>]*)?>|[^)\s]*\.md(#[^)\s]*)?)\s*\)").unwrap()
});

pub struct Vault {
    root: PathBuf,
}

/// A note asked for: its name, the folder it was named in if any, and a
/// heading within it.
struct Asked<'a> {
    name: &'a str,
    folder: Option<&'a str>,
    heading: Option<&'a str>,
}

/// Takes a note as a link names it: `Ada`, `People/Ada`, `Ada.md`,
/// `[[Ada#Work|her job]]`.
fn asked(text: &str) -> Result<Asked<'_>, String> {
    let t = text.trim();
    let t = t
        .strip_prefix("[[")
        .and_then(|t| t.strip_suffix("]]"))
        .unwrap_or(t);
    let t = t.split('|').next().unwrap_or(t);
    let (t, heading) = match t.split_once('#') {
        Some((n, h)) => (n, Some(h.trim()).filter(|h| !h.is_empty())),
        None => (t, None),
    };
    let t = t.trim();
    let t = t.strip_suffix(".md").unwrap_or(t);
    let bad =
        || format!("{text:?} is no note name: name it as a link does, such as Ada or People/Ada.");
    if t.is_empty() || t.contains('\\') || t.contains('\0') {
        return Err(bad());
    }
    for part in Path::new(t).components() {
        match part {
            Component::Normal(p) if !p.to_string_lossy().starts_with('.') => {}
            _ => return Err(bad()),
        }
    }
    let (folder, name) = match t.rsplit_once('/') {
        Some((f, n)) => (Some(f), n),
        None => (None, t),
    };
    if folder.is_some_and(|f| f == ATTACHMENTS || f.starts_with(&format!("{ATTACHMENTS}/"))) {
        return Err(format!("{ATTACHMENTS}/ holds attachments, not notes."));
    }
    // What Obsidian refuses in a file name, as a link could not name it.
    if name.contains(['[', ']', '#', '|', '^', ':']) {
        return Err(format!(
            "{name:?}: a note's name can't hold [ ] # | ^ or :."
        ));
    }
    Ok(Asked {
        name,
        folder,
        heading,
    })
}

impl Vault {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Every note by name and file.
    fn notes(&self) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out.sort();
        out
    }

    /// The note of that name, wherever it sits: the one in `folder` when a
    /// folder is named, else the one of the name, as a link finds it.
    fn find(&self, a: &Asked) -> Option<PathBuf> {
        if let Some(f) = a.folder {
            let file = self.root.join(f).join(format!("{}.md", a.name));
            return file.is_file().then_some(file);
        }
        self.notes()
            .into_iter()
            .find(|(_, p)| stem(p).eq_ignore_ascii_case(a.name))
            .map(|(_, p)| p)
    }

    /// A note whole, or under `#Heading` its section alone, as a link to it
    /// opens.
    pub fn read(&self, note: &str) -> Result<String, String> {
        let a = asked(note)?;
        let file = self
            .find(&a)
            .ok_or_else(|| format!("No note {}.", a.name))?;
        let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", a.name))?;
        match a.heading {
            None => Ok(text),
            Some(h) => section(&text, h).ok_or_else(|| format!("{} has no heading {h}.", a.name)),
        }
    }

    /// Writes a note whole, through a file renamed into place so a reader
    /// never sees half of it; empty text removes it. A note that exists is
    /// written where it is; a new one goes in the folder named, else the
    /// vault's top. A name already taken elsewhere is refused, as the
    /// links to it would no longer know which note they mean.
    pub fn write(&self, note: &str, text: &str) -> Result<String, String> {
        let a = asked(note)?;
        if a.heading.is_some() {
            return Err("Write a note whole: name it without #heading.".into());
        }
        let found = self.find(&Asked { folder: None, ..a });
        let file = match (&found, a.folder) {
            (Some(p), Some(f)) if p != &self.root.join(f).join(format!("{}.md", a.name)) => {
                return Err(format!(
                    "A note named {} is already at {}: notes go by name, so each name is one note's.",
                    a.name,
                    shown(&self.root, p)
                ));
            }
            (Some(p), _) => p.clone(),
            (None, f) => self
                .root
                .join(f.unwrap_or(""))
                .join(format!("{}.md", a.name)),
        };
        if text.trim().is_empty() {
            return match found {
                Some(_) => fs::remove_file(&file)
                    .map(|()| format!("Removed {}.", a.name))
                    .map_err(|e| format!("{}: {e}", a.name)),
                None => Err(format!("No note {}.", a.name)),
            };
        }
        check(text)?;
        let dir = file.parent().expect("a note's file has a folder");
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", a.name))?;
        let part = dir.join(format!(".{}.writing", a.name));
        let mut body = text.to_owned();
        if !body.ends_with('\n') {
            body.push('\n');
        }
        fs::write(&part, body)
            .and_then(|()| fs::rename(&part, &file))
            .map_err(|e| format!("{}: {e}", a.name))?;
        let verb = if found.is_some() { "Wrote" } else { "Made" };
        Ok(format!("{verb} {}.", shown(&self.root, &file)))
    }

    /// Every note, by where it sits, with its size and when it last
    /// changed; within `folder` if one is given.
    pub fn list(&self, folder: Option<&str>) -> Result<String, String> {
        let folder = folder
            .map(|f| f.trim().trim_matches('/'))
            .filter(|f| !f.is_empty());
        let mut out = String::new();
        for (name, path) in self.notes() {
            if let Some(f) = folder
                && !name.starts_with(&format!("{f}/"))
            {
                continue;
            }
            let meta = fs::metadata(&path).ok();
            let size = meta.as_ref().map_or(0, |m| m.len());
            let changed = meta
                .and_then(|m| m.modified().ok())
                .map(day)
                .unwrap_or_default();
            let _ = writeln!(out, "{name}  {}  {changed}", bytes(size));
        }
        if out.is_empty() {
            return Ok(match folder {
                Some(f) => format!("No notes in {f}."),
                None => "The vault has no notes yet.".into(),
            });
        }
        Ok(out)
    }

    /// Every line of every note matching a case-insensitive regex, and
    /// every note whose name does, as `Note:line: text`.
    pub fn search(&self, pattern: &str) -> Result<String, String> {
        let re = RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .map_err(|e| format!("Bad regex: {e}"))?;
        let (mut out, mut hits) = (String::new(), 0);
        let full = |out: &String, hits| hits >= HITS_MAX || out.len() >= ANSWER_MAX;
        'notes: for (name, path) in self.notes() {
            if re.is_match(&name) {
                let _ = writeln!(out, "{name}: (name)");
                hits += 1;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for (i, line) in text.lines().enumerate() {
                if full(&out, hits) {
                    break 'notes;
                }
                if re.is_match(line) {
                    let _ = writeln!(out, "{name}:{}: {}", i + 1, cut(line, 300));
                    hits += 1;
                }
            }
        }
        if out.is_empty() {
            return Ok("No matches.".into());
        }
        if full(&out, hits) {
            out.push_str("(more matches past these: narrow the pattern)\n");
        }
        Ok(out)
    }
}

/// What a note must be for Obsidian to read it as written: frontmatter
/// closed, links by name.
fn check(text: &str) -> Result<(), String> {
    if text.len() > NOTE_MAX {
        return Err(format!(
            "The note would be {} KB; a note holds at most {} KB: split it into notes that link.",
            text.len() / 1024,
            NOTE_MAX / 1024
        ));
    }
    if let Some(rest) = text.strip_prefix("---\n")
        && !rest.lines().any(|l| l.trim_end() == "---")
    {
        return Err("The frontmatter opened with --- has no closing --- line.".into());
    }
    // A link to a page on the web is no note's.
    if let Some(m) = MARKDOWN_LINK
        .find_iter(text)
        .find(|m| !m.as_str().contains("://"))
    {
        return Err(format!(
            "{}: link notes by name, [[Note]] or [[Note|text]], never by file.",
            m.as_str()
        ));
    }
    Ok(())
}

/// The section under heading `h`: its heading line and all down to the next
/// heading of its level or higher.
fn section(text: &str, h: &str) -> Option<String> {
    let level = |l: &str| {
        let n = l.chars().take_while(|&c| c == '#').count();
        (n > 0 && l[n..].starts_with(' ')).then_some(n)
    };
    let mut lines = text.lines();
    let mut out = String::new();
    let depth = loop {
        let l = lines.next()?;
        if let Some(n) = level(l)
            && l[n..].trim().eq_ignore_ascii_case(h)
        {
            out.push_str(l);
            out.push('\n');
            break n;
        }
    };
    for l in lines {
        if level(l).is_some_and(|n| n <= depth) {
            break;
        }
        out.push_str(l);
        out.push('\n');
    }
    Some(out)
}

/// Every note under `dir`, by its path from the vault without `.md`,
/// skipping hidden folders and files (`.obsidian`, a write under way) and
/// the attachments.
fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || (dir == root && name == ATTACHMENTS) {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, out);
        } else if path.extension().is_some_and(|x| x == "md") {
            out.push((shown(root, &path), path));
        }
    }
}

/// A note's file as the list shows it: its path in the vault, without `.md`.
fn shown(root: &Path, file: &Path) -> String {
    file.with_extension("")
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn stem(file: &Path) -> String {
    file.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn cut(line: &str, max: usize) -> String {
    match line.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_owned(),
    }
}

fn bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else {
        format!("{} KB", n.div_ceil(1024))
    }
}

/// A file time as its UTC day, which is all a list needs to show.
fn day(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (y, m, d) = civil(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 as a calendar date (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> Vault {
        let dir = std::env::temp_dir().join(format!("wiki-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Vault::new(dir)
    }

    fn names(v: &Vault) -> Vec<String> {
        v.list(None)
            .unwrap()
            .lines()
            .map(|l| l.split("  ").next().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_note_goes_by_its_name_wherever_it_sits() {
        let v = scratch("names");
        assert_eq!(v.list(None).unwrap(), "The vault has no notes yet.");
        assert_eq!(
            v.write("People/Ada", "# Ada\nLikes [[Tea]].").unwrap(),
            "Made People/Ada."
        );
        // By name alone, as [[Ada]] finds it, case and all.
        assert_eq!(v.read("ada").unwrap(), "# Ada\nLikes [[Tea]].\n");
        assert_eq!(v.read("[[Ada|her]]").unwrap(), "# Ada\nLikes [[Tea]].\n");
        assert_eq!(
            v.write("Ada.md", "# Ada\nTea.").unwrap(),
            "Wrote People/Ada."
        );
        assert_eq!(v.write("Index", "[[Ada]]").unwrap(), "Made Index.");
        assert_eq!(names(&v), ["Index", "People/Ada"]);
        assert!(v.list(Some("People")).unwrap().starts_with("People/Ada  "));
        assert_eq!(v.list(Some("Projects")).unwrap(), "No notes in Projects.");
    }

    #[test]
    fn a_name_taken_in_another_folder_is_refused() {
        let v = scratch("unique");
        v.write("People/Ada", "x").unwrap();
        let err = v.write("Projects/ADA", "y").unwrap_err();
        assert!(err.contains("already at People/Ada"), "{err}");
        assert_eq!(v.write("People/Ada", "z").unwrap(), "Wrote People/Ada.");
    }

    #[test]
    fn a_heading_reads_its_section() {
        let v = scratch("heading");
        v.write(
            "Ada",
            "# Ada\nintro\n## Work\nbuilds\n### Now\nlibri\n## Home\ntea",
        )
        .unwrap();
        assert_eq!(
            v.read("Ada#Work").unwrap(),
            "## Work\nbuilds\n### Now\nlibri\n"
        );
        assert_eq!(v.read("[[Ada#home|at home]]").unwrap(), "## Home\ntea\n");
        assert_eq!(v.read("Ada#Pets").unwrap_err(), "Ada has no heading Pets.");
        assert!(v.write("Ada#Work", "x").is_err());
    }

    #[test]
    fn empty_text_removes_a_note() {
        let v = scratch("remove");
        v.write("Bob", "hi").unwrap();
        assert_eq!(v.write("Bob", " \n").unwrap(), "Removed Bob.");
        assert_eq!(v.read("Bob").unwrap_err(), "No note Bob.");
        assert_eq!(v.write("Bob", "").unwrap_err(), "No note Bob.");
    }

    #[test]
    fn no_name_leads_outside_the_vault_or_into_the_apps_folders() {
        let v = scratch("bad");
        for bad in [
            "",
            "../x",
            "/etc/passwd",
            "a/../../b",
            ".obsidian/app",
            "a/.git/x",
            "a\\b",
            "attachments/x",
            "a:b",
            "a^b",
        ] {
            assert!(v.write(bad, "x").is_err(), "{bad:?} was taken");
            assert!(v.read(bad).is_err(), "{bad:?} was read");
        }
        fs::create_dir_all(v.root.join(".obsidian")).unwrap();
        fs::write(v.root.join(".obsidian/hidden.md"), "secret").unwrap();
        fs::create_dir_all(v.root.join(ATTACHMENTS)).unwrap();
        fs::write(v.root.join("attachments/notes.md"), "secret").unwrap();
        assert_eq!(v.list(None).unwrap(), "The vault has no notes yet.");
        assert_eq!(v.search("secret").unwrap(), "No matches.");
    }

    #[test]
    fn a_note_keeps_to_obsidians_form() {
        let v = scratch("form");
        let ok = "---\ntags: [person]\naliases: [Ada L]\nsources: [hippo 92949]\n---\n# Ada\nSee [[Tea#Green|green]] and ![[attachments/tea.png]]. #person";
        assert!(v.write("Ada", ok).is_ok());
        let err = v.write("Bob", "---\ntags: [x]\n# Bob").unwrap_err();
        assert!(err.contains("closing ---"), "{err}");
        for link in [
            "[tea](Tea.md)",
            "[tea](../People/Tea.md#Green)",
            "[t](<My note.md>)",
        ] {
            let err = v.write("Bob", &format!("see {link}")).unwrap_err();
            assert!(err.contains("by name"), "{link}: {err}");
        }
        assert!(
            v.write("Bob", "see [site](https://example.com/a.md.html) and [readme](https://example.com/README.md)")
                .is_ok()
        );
    }

    #[test]
    fn search_finds_lines_and_names_case_blind() {
        let v = scratch("search");
        v.write("Projects/Libri", "Port 8951.\nShips on box.")
            .unwrap();
        v.write("Tea", "green").unwrap();
        assert_eq!(
            v.search("PORT \\d+").unwrap(),
            "Projects/Libri:1: Port 8951.\n"
        );
        assert_eq!(v.search("^tea$").unwrap(), "Tea: (name)\n");
        assert!(v.search("(").unwrap_err().starts_with("Bad regex"));
    }

    #[test]
    fn a_note_too_big_is_refused() {
        let v = scratch("big");
        let err = v.write("Big", &"x".repeat(NOTE_MAX + 1)).unwrap_err();
        assert!(err.contains("split it"), "{err}");
    }

    #[test]
    fn dates_are_calendar_days() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_764), (2026, 11, 7));
        assert_eq!(civil(-1), (1969, 12, 31));
    }
}
