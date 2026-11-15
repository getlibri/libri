//! The standing guidance every harness is given, in layers, through the
//! system prompt and nothing else, so each one takes it the same way and
//! exactly once. The product's own rules live here, true for anyone on any
//! host. The person's are one file in the state directory (AGENTS.md),
//! opening with an Identity section (who the assistant is, who it works for,
//! its role), which no other prompt text names; what is true of this host
//! alone is another (HOST.md). Each is edited in Settings, and a fresh
//! install has neither. The prompt takes the person's first and the host's
//! after, widest scope first.
//!
//! The files travel with the state directory, as Settings do. Each is read
//! for every prompt, so an edit reaches the next turn without a restart.

use crate::model::Kept;
use std::path::Path;

/// What a worker can rely on of its memory; the main chat's head says it
/// in UniiChat's words.
const MEMORY: &str = "\n\nEverything said here is recorded word for word on \
its own. To keep something, say it in a reply.";

/// The rest, for a worker with the tools to use it: its harness compacts
/// its thread, which keeps little of a tool's output, and a model without
/// tools has none of that.
const WORKER_TOOLS: &str = " Compaction keeps little tool output, so state \
what you learned that will matter later.

Before acting on or reporting a status (live, deployed, pushed, fixed, \
tested, answered), recall it or check the system itself.";

/// The person's rules, as Settings shows them.
pub const RULES: Kept = Kept("AGENTS.md");

/// What is true of this host alone: its tools, its commands, what the
/// assistant is for here. Each host has its own, in its own state.
pub const HOST: Kept = Kept("HOST.md");

/// Claude Code and omp take the prompt as one argument, and Linux refuses
/// one past 128 KiB: longer rules would fail every turn with nothing to say
/// why. This leaves the rest of the prompt room.
const LONGEST: usize = 64 * 1024;

/// How to use Libri and its memory, for a worker; the main chat's head
/// holds its own.
pub fn product(tools: bool, worker: bool) -> String {
    match (worker, tools) {
        (false, _) => String::new(),
        (true, false) => MEMORY.to_owned(),
        (true, true) => format!("{MEMORY}{WORKER_TOOLS}"),
    }
}

/// A host's guide as a worker reads it: its sections on the memory go, which
/// tell of the main view and how to bring it up to date. A worker has its
/// own, forked once, and must never refresh it, so these words can't reach
/// it, and the rest of the guide is as true for it as for the main chat.
pub fn without_memory(guide: &str) -> String {
    let mut kept = Vec::new();
    // The depth of the heading being cut.
    let mut cutting = None;
    for line in guide.lines() {
        let depth = line.bytes().take_while(|b| *b == b'#').count();
        let title = line[depth..].strip_prefix(' ').filter(|_| depth > 0);
        if let Some(title) = title {
            if cutting.is_some_and(|d| depth <= d) {
                cutting = None;
            }
            if cutting.is_none() && title.to_lowercase().starts_with("memory") {
                cutting = Some(depth);
            }
        }
        if cutting.is_none() {
            kept.push(line);
        }
    }
    kept.join("\n")
}

/// The person's rules, empty until they write some.
pub fn rules(state: &Path) -> String {
    upgrade(state);
    RULES.read(state).unwrap_or_default()
}

/// The heading of the rules' section on who the assistant is.
const IDENTITY: &str = "Identity";

/// Who the assistant is: the rules' Identity section, whole, else nothing.
/// What a prompt without the rules (a call) still says.
pub fn identity(state: &Path) -> String {
    section(&rules(state), IDENTITY).unwrap_or_default()
}

/// The section of `text` under the top-level heading `title`, heading
/// included, up to the next heading of any level: the person's own rules
/// may open at `##`.
fn section(text: &str, title: &str) -> Option<String> {
    let mut lines = text.lines().skip_while(|l| {
        !l.strip_prefix("# ")
            .is_some_and(|t| t.trim().eq_ignore_ascii_case(title))
    });
    let head = lines.next()?;
    let body: Vec<&str> = lines.take_while(|l| !l.starts_with('#')).collect();
    Some(format!("{head}\n{}", body.join("\n")).trim().to_owned())
}

/// Saving nothing leaves an empty file: on another host a file that is gone
/// can't be told from one that never came.
pub fn set_rules(state: &Path, text: &str) -> Result<(), String> {
    save(state, &RULES, text)
}

/// The host's facts, empty until some are written.
pub fn host(state: &Path) -> String {
    upgrade(state);
    HOST.read(state).unwrap_or_default()
}

pub fn set_host(state: &Path, text: &str) -> Result<(), String> {
    save(state, &HOST, text)
}

/// Takes a state from before 0.9.70 into these files, once: until then a
/// deployment named the assistant in its `harness.identity` file, put on
/// every prompt, and kept the host's facts in its `harness.instructions`
/// file. The first read opens the rules with an Identity section from the
/// one, unless they have one, and fills HOST.md from the other, empty if
/// there is none; HOST.md then exists, so it never runs again, and an edit
/// or an emptied file stands. A fresh install, with neither set, has nothing
/// taken. Goes once no deployment sets either.
fn upgrade(state: &Path) {
    upgrade_from(
        state,
        std::env::var_os("LIBRI_IDENTITY").as_deref(),
        std::env::var_os("LIBRI_INSTRUCTIONS").as_deref(),
    )
}

fn upgrade_from(state: &Path, identity: Option<&std::ffi::OsStr>, host: Option<&std::ffi::OsStr>) {
    if state.join(HOST.0).exists() || (identity.is_none() && host.is_none()) {
        return;
    }
    let read = |f: Option<&std::ffi::OsStr>| {
        f.and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| t.trim().to_owned())
            .unwrap_or_default()
    };
    let rules = RULES.read(state).unwrap_or_default();
    let who = read(identity);
    if !who.is_empty() && section(&rules, IDENTITY).is_none() {
        let text = match rules.trim() {
            "" => format!("# {IDENTITY}\n\n{who}\n"),
            r => format!("# {IDENTITY}\n\n{who}\n\n{r}\n"),
        };
        if let Err(e) = save(state, &RULES, &text) {
            // HOST.md unwritten, so the next read tries again.
            eprintln!("libri: AGENTS.md not given an Identity from harness.identity: {e}");
            return;
        }
    }
    if let Err(e) = save(state, &HOST, &read(host)) {
        eprintln!("libri: HOST.md not filled from harness.instructions: {e}");
    }
}

fn save(state: &Path, file: &Kept, text: &str) -> Result<(), String> {
    if text.len() > LONGEST {
        return Err(format!(
            "This is {} KB; {} can hold {} KB.",
            text.len() / 1024,
            file.0,
            LONGEST / 1024
        ));
    }
    crate::host::write_private(&state.join(file.0), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("libri-guide-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_worker_is_told_of_its_own_memory() {
        let words = product(true, true);
        assert!(words.contains("recall it or check the system itself"));
        assert!(!words.contains("<chat>"));
        assert!(!words.contains("zoom"));
    }

    #[test]
    fn a_worker_reads_the_guide_without_its_memory_section() {
        let guide = "Deploy with release.\n\n## Memory: hippo\n\nThe view, \
                     loaded with every message.\n\n### Refresh\n\n- `hippo view` \
                     refreshes it.\n\n## Notes\n\nA memory of a note stays.\n\n\
                     # Other\n\nThe end.";
        let kept = without_memory(guide);
        assert!(!kept.contains("hippo view"));
        assert!(!kept.contains("The view"));
        assert!(kept.starts_with("Deploy with release."));
        assert!(kept.contains("## Notes\n\nA memory of a note stays."));
        assert!(kept.ends_with("The end."));
        assert_eq!(without_memory("No headings."), "No headings.");
    }

    #[test]
    fn an_old_state_is_taken_in_once() {
        let s = state("upgrade");
        let (who, facts) = (s.join("old-identity.md"), s.join("old-guide.md"));
        std::fs::write(&who, "You are Iris, Ada's assistant.\n").unwrap();
        std::fs::write(&facts, "Deploy with `release`.\n").unwrap();
        set_rules(&s, "# Engineering\n\nBe plain.").unwrap();
        upgrade_from(&s, Some(who.as_os_str()), Some(facts.as_os_str()));
        assert_eq!(
            RULES.read(&s).unwrap(),
            "# Identity\n\nYou are Iris, Ada's assistant.\n\n# Engineering\n\nBe plain."
        );
        assert_eq!(HOST.read(&s).unwrap(), "Deploy with `release`.");
        // Once taken, each is the person's: edited or emptied, it stands.
        set_rules(&s, "# Engineering\n\nBe plain.").unwrap();
        set_host(&s, "").unwrap();
        upgrade_from(&s, Some(who.as_os_str()), Some(facts.as_os_str()));
        assert_eq!(RULES.read(&s).unwrap(), "# Engineering\n\nBe plain.");
        assert!(s.join(HOST.0).exists() && HOST.read(&s).is_none());
        // Rules with an Identity keep theirs; no old facts leave HOST.md empty.
        let named = state("upgrade-named");
        set_rules(&named, "# Identity\n\nYou are Rook.").unwrap();
        upgrade_from(&named, Some(who.as_os_str()), None);
        assert_eq!(
            section(&RULES.read(&named).unwrap(), IDENTITY).unwrap(),
            "# Identity\n\nYou are Rook."
        );
        assert!(named.join(HOST.0).exists() && HOST.read(&named).is_none());
        // A fresh install has nothing written.
        let bare = state("upgrade-bare");
        upgrade_from(&bare, None, None);
        assert!(!bare.join(RULES.0).exists() && !bare.join(HOST.0).exists());
    }

    #[test]
    fn a_section_runs_to_the_next_heading() {
        let text = "# Identity\n\nYou are Rook.\n\n## Engineering\n\nPlain.";
        assert_eq!(
            section(text, "identity").unwrap(),
            "# Identity\n\nYou are Rook."
        );
        assert_eq!(section("# Engineering\nPlain.", "Identity"), None);
    }

    #[test]
    fn the_host_file_is_its_own_and_starts_empty() {
        let dir = state("host");
        assert_eq!(HOST.read(&dir).unwrap_or_default(), "");
        set_host(&dir, "## Deploys\n\n`release` ships it.\n").unwrap();
        assert_eq!(
            HOST.read(&dir).unwrap(),
            "## Deploys\n\n`release` ships it."
        );
        assert_eq!(RULES.read(&dir).unwrap_or_default(), "");
        set_host(&dir, "").unwrap();
        assert_eq!(HOST.read(&dir), None);
        assert!(dir.join("HOST.md").exists());
        assert!(set_host(&dir, &"x".repeat(LONGEST + 1)).is_err());
    }

    #[test]
    fn nothing_written_reads_as_no_rules() {
        let dir = state("nothing");
        assert_eq!(rules(&dir), "");
    }

    #[test]
    fn a_saved_text_comes_back_and_can_be_emptied() {
        let dir = state("saved");
        set_rules(&dir, "## Naming\n\nPlain words.\n").unwrap();
        assert_eq!(rules(&dir), "## Naming\n\nPlain words.");
        set_rules(&dir, "").unwrap();
        assert_eq!(rules(&dir), "");
        assert!(dir.join("AGENTS.md").exists());
    }

    #[test]
    fn a_text_too_long_for_one_argument_is_refused_and_keeps_the_old() {
        let dir = state("long");
        set_rules(&dir, "kept").unwrap();
        assert!(set_rules(&dir, &"x".repeat(LONGEST + 1)).is_err());
        assert_eq!(rules(&dir), "kept");
    }

    #[test]
    fn only_a_worker_has_the_guide_and_without_tools_no_tool() {
        // The main chat's head says it in UniiChat's words.
        assert_eq!(product(true, false), "");
        assert!(!product(false, true).contains("recall"));
    }
}
