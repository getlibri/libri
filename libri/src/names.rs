//! Which names workers are given. By default that is the built-in list in
//! `task.rs` and any the person adds; the person may instead keep a whole
//! list of their own, switched on and off without losing either. Names they
//! exclude stay out of whichever is in use. The host holds all of it, so
//! every device shows the same list. A worker's name is settled when it
//! starts: nothing here reaches a worker that already has one.
//!
//! A name becomes the front of the worker's ID, and a bare name with a hyphen
//! reads as an ID (`task::dir`, hippo's `agent_log`), so a listed name is one
//! word of A to Z. With no listed name free a worker is numbered instead
//! (`task::numbered`): it is never refused.

use crate::model::Kept;
use crate::task::{MAIN, NAMES};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// What the person changed, as JSON; until they do it is the built-in list.
pub const LIST: Kept = Kept("worker-names");

/// Screens set worker names in small rows, and a call reads them aloud.
const SHORTEST: usize = 2;
const LONGEST: usize = 16;

/// The person's changes to the built-in list.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Names {
    #[serde(default)]
    added: Vec<String>,
    #[serde(default)]
    excluded: Vec<String>,
    /// Their own list, kept while it is off.
    #[serde(default)]
    own: Vec<String>,
    /// Whether their own list stands in for the built-in names and `added`.
    #[serde(default)]
    use_own: bool,
}

/// One change, as Settings sends it. A change already made answers as done,
/// so a second device a moment behind the first is not refused.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum Change {
    Add {
        name: String,
    },
    Remove {
        name: String,
    },
    Exclude {
        name: String,
    },
    Restore {
        name: String,
    },
    UseOwn {
        on: bool,
    },
    /// The whole of their own list, by commas or a name to a line.
    SetOwn {
        text: String,
    },
}

/// The list as Settings shows it: `allowed` counts the names new workers are
/// given, and none means they are numbered. `left_out` answers `SetOwn` with
/// the entries that can't be names.
#[derive(Serialize)]
pub struct Listed {
    builtin: Vec<&'static str>,
    added: Vec<String>,
    own: Vec<String>,
    use_own: bool,
    excluded: Vec<String>,
    allowed: usize,
    left_out: Vec<String>,
}

impl Names {
    /// The names new workers are given.
    pub fn allowed(&self) -> Vec<String> {
        let listed: Vec<String> = if self.use_own {
            self.own.clone()
        } else {
            NAMES
                .iter()
                .map(|n| (*n).to_owned())
                .chain(self.added.iter().cloned())
                .collect()
        };
        listed.into_iter().filter(|n| !self.excludes(n)).collect()
    }

    fn excludes(&self, name: &str) -> bool {
        same(&self.excluded, name).is_some()
    }
}

/// The entry of `names` that is `name`, whatever the case.
fn same<'a>(names: &'a [String], name: &str) -> Option<&'a String> {
    names.iter().find(|n| n.eq_ignore_ascii_case(name))
}

pub fn read(state: &Path) -> Names {
    LIST.read(state)
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn listed(state: &Path) -> Listed {
    let names = read(state);
    // A to Z to be scanned; draws are random, so the list's own order doesn't
    // matter to them.
    let mut builtin = NAMES.to_vec();
    builtin.sort_unstable();
    Listed {
        builtin,
        allowed: names.allowed().len(),
        added: names.added,
        own: names.own,
        use_own: names.use_own,
        excluded: names.excluded,
        left_out: Vec::new(),
    }
}

/// `name` as a list spells it, with its first letter capital as the built-in
/// ones have; the rest stays as typed, for a McCoy.
fn clean(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("Type a name first.".into());
    }
    if !(SHORTEST..=LONGEST).contains(&name.len()) || !name.chars().all(|c| c.is_ascii_alphabetic())
    {
        return Err(format!(
            "A worker's name is one word of {SHORTEST} to {LONGEST} letters, A to Z."
        ));
    }
    if name.eq_ignore_ascii_case(MAIN) {
        return Err("Workers use Main to reach the chat, so no worker can have it.".into());
    }
    let mut letters = name.chars();
    Ok(letters
        .next()
        .map(|c| c.to_ascii_uppercase())
        .into_iter()
        .chain(letters)
        .collect())
}

/// Reads a pasted list, split at commas and line ends: the names, once each,
/// and the entries that can't be names.
fn pasted(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut kept, mut left_out) = (Vec::new(), Vec::new());
    for entry in text
        .split([',', '\n', '\r'])
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        match clean(entry) {
            Ok(name) if same(&kept, &name).is_none() => kept.push(name),
            Ok(_) => {}
            Err(_) => left_out.push(entry.to_owned()),
        }
    }
    (kept, left_out)
}

fn builtin(name: &str) -> Option<&'static str> {
    NAMES.iter().find(|n| n.eq_ignore_ascii_case(name)).copied()
}

/// Applies `change`; the answer is what `SetOwn` left out.
pub fn change(state: &Path, change: &Change) -> Result<Vec<String>, String> {
    let mut names = read(state);
    let mut left_out = Vec::new();
    match change {
        Change::Add { name } => {
            let name = clean(name.trim())?;
            // Asked for by name, an excluded one is simply wanted back.
            let back = names.excludes(&name);
            names.excluded.retain(|n| !n.eq_ignore_ascii_case(&name));
            match builtin(&name).or(same(&names.added, &name).map(String::as_str)) {
                None => names.added.push(name),
                Some(n) if !back => return Err(format!("{n} is already on the list.")),
                Some(_) => {}
            }
        }
        Change::Remove { name } => {
            names.added.retain(|n| !n.eq_ignore_ascii_case(name.trim()));
        }
        Change::Exclude { name } => {
            let name = name.trim();
            let listed = builtin(name).or(same(&names.own, name).map(String::as_str));
            let Some(n) = listed else {
                return Err(format!(
                    "{name} can't be excluded: only a built-in name or one on your own list can."
                ));
            };
            if !names.excludes(n) {
                let n = n.to_owned();
                names.excluded.push(n);
            }
        }
        Change::Restore { name } => {
            let name = name.trim();
            names.excluded.retain(|n| !n.eq_ignore_ascii_case(name));
        }
        Change::UseOwn { on } => names.use_own = *on,
        Change::SetOwn { text } => {
            let (own, out) = pasted(text);
            // An empty box is an empty list; a paste of nothing usable is a
            // mistake, and must not cost the list already kept.
            if own.is_empty() && !out.is_empty() {
                return Err(format!(
                    "None of those can be a worker's name, which is one word of {SHORTEST} to {LONGEST} letters, A to Z."
                ));
            }
            names.own = own;
            left_out = out;
        }
    }
    let raw = serde_json::to_string(&names).map_err(|e| e.to_string())?;
    LIST.write(state, &raw)?;
    Ok(left_out)
}

/// The reply to a change: the list, and what `change` left out.
pub fn answer(state: &Path, left_out: Vec<String>) -> Listed {
    Listed {
        left_out,
        ..listed(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let state = std::env::temp_dir().join(format!("libri-names-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&state);
        fs::create_dir_all(&state).unwrap();
        state
    }

    fn add(name: &str) -> Change {
        Change::Add { name: name.into() }
    }

    fn exclude(name: &str) -> Change {
        Change::Exclude { name: name.into() }
    }

    fn set_own(text: &str) -> Change {
        Change::SetOwn { text: text.into() }
    }

    const BAD: [&str; 12] = [
        "J",
        "Thaddeusthaddeuss",
        "Mary Jane",
        "Mary-Jane",
        "O'Neil",
        "José",
        "Zo\u{200b}e",
        "Bo\u{7}b",
        "😀",
        "../x",
        "R2D2",
        "Worker 1",
    ];

    #[test]
    fn with_no_changes_the_list_is_the_built_in_one() {
        let state = scratch("plain");
        assert_eq!(read(&state).allowed().len(), NAMES.len());
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn settings_list_the_built_in_names_a_to_z_and_a_persons_own_as_written() {
        let state = scratch("order");
        assert!(listed(&state).builtin.is_sorted());
        assert_eq!(listed(&state).builtin.len(), NAMES.len());
        change(&state, &set_own("Zinnia, Alder, Moss")).unwrap();
        assert_eq!(listed(&state).own, ["Zinnia", "Alder", "Moss"]);
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn changes_survive_a_fresh_read_and_a_damaged_file_means_none() {
        let state = scratch("kept");
        change(&state, &add("Thaddeus")).unwrap();
        change(&state, &exclude("Kit")).unwrap();
        let names = read(&state);
        assert!(names.allowed().contains(&"Thaddeus".to_owned()));
        assert!(!names.allowed().contains(&"Kit".to_owned()));
        assert_eq!(names.allowed().len(), NAMES.len());
        fs::write(state.join(LIST.0), "not json").unwrap();
        assert_eq!(read(&state), Names::default());
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn a_blank_or_unshowable_name_is_refused() {
        let state = scratch("refused");
        assert_eq!(
            change(&state, &add("   ")).unwrap_err(),
            "Type a name first."
        );
        for bad in BAD {
            assert!(
                change(&state, &add(bad)).unwrap_err().contains("A to Z"),
                "{bad}"
            );
        }
        assert!(change(&state, &add("MAIN")).is_err());
        assert_eq!(read(&state), Names::default());
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn a_name_on_the_list_is_refused_however_it_is_cased() {
        let state = scratch("twice");
        change(&state, &add("  thaddeus ")).unwrap();
        assert_eq!(read(&state).added, ["Thaddeus"]);
        for again in ["Thaddeus", "THADDEUS", "thaddeus"] {
            assert_eq!(
                change(&state, &add(again)).unwrap_err(),
                "Thaddeus is already on the list."
            );
        }
        assert_eq!(
            change(&state, &add("KIT")).unwrap_err(),
            "Kit is already on the list."
        );
        change(&state, &add("mcCoy")).unwrap();
        assert_eq!(read(&state).added, ["Thaddeus", "McCoy"]);
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn adding_an_excluded_name_brings_it_back() {
        let state = scratch("back");
        change(&state, &exclude("kit")).unwrap();
        assert_eq!(read(&state).excluded, ["Kit"]);
        change(&state, &add("KIT")).unwrap();
        assert_eq!(read(&state), Names::default());
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn an_added_name_is_removed_not_excluded_and_a_change_made_twice_is_done() {
        let state = scratch("repeat");
        change(&state, &add("Thaddeus")).unwrap();
        assert!(change(&state, &exclude("Thaddeus")).is_err());
        change(&state, &exclude("Kit")).unwrap();
        change(&state, &exclude("Kit")).unwrap();
        assert_eq!(read(&state).excluded, ["Kit"]);
        let remove = Change::Remove {
            name: "thaddeus".into(),
        };
        let restore = Change::Restore { name: "kit".into() };
        for c in [&remove, &remove, &restore, &restore] {
            change(&state, c).unwrap();
        }
        assert_eq!(read(&state), Names::default());
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn a_paste_splits_at_commas_and_lines_and_each_name_counts_once() {
        let state = scratch("paste");
        let left = change(
            &state,
            &set_own(" alder, Birch,,\nCEDAR\r\n\n  birch \n Dogwood ,"),
        )
        .unwrap();
        assert!(left.is_empty());
        assert_eq!(read(&state).own, ["Alder", "Birch", "CEDAR", "Dogwood"]);
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn a_paste_keeps_its_names_and_says_which_entries_it_left_out() {
        let state = scratch("left-out");
        let text = format!("Alder, {}, main", BAD.join("\n"));
        let left = change(&state, &set_own(&text)).unwrap();
        assert_eq!(read(&state).own, ["Alder"]);
        assert_eq!(left.len(), BAD.len() + 1);
        assert!(left.contains(&"main".to_owned()));
        assert_eq!(answer(&state, left).left_out.len(), BAD.len() + 1);
        assert!(listed(&state).left_out.is_empty());
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn a_paste_with_no_name_in_it_keeps_the_list_but_an_empty_box_clears_it() {
        let state = scratch("nothing");
        change(&state, &set_own("Alder, Birch")).unwrap();
        assert!(change(&state, &set_own("José, 😀")).is_err());
        assert_eq!(read(&state).own, ["Alder", "Birch"]);
        change(&state, &set_own(" \n , ")).unwrap();
        assert!(read(&state).own.is_empty());
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn the_own_list_replaces_the_rest_and_exclusions_apply_on_top() {
        let state = scratch("own");
        change(&state, &add("Thaddeus")).unwrap();
        change(&state, &exclude("Kit")).unwrap();
        change(&state, &set_own("Alder, Kit, Birch")).unwrap();
        assert_eq!(read(&state).allowed().len(), NAMES.len());
        change(&state, &Change::UseOwn { on: true }).unwrap();
        assert_eq!(read(&state).allowed(), ["Alder", "Birch"]);
        change(&state, &exclude("alder")).unwrap();
        assert_eq!(read(&state).allowed(), ["Birch"]);
        // A name of their own, unlike an added one, can be excluded, and
        // adding it brings it back and puts it on the other list too.
        change(&state, &add("ALDER")).unwrap();
        assert_eq!(read(&state).allowed(), ["Alder", "Birch"]);
        assert_eq!(read(&state).added, ["Thaddeus", "ALDER"]);
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn switching_the_own_list_off_and_on_loses_nothing_across_a_restart() {
        let state = scratch("switch");
        change(&state, &add("Thaddeus")).unwrap();
        change(&state, &set_own("Alder, Birch")).unwrap();
        change(&state, &Change::UseOwn { on: true }).unwrap();
        // Each read is the file alone, as a restarted host finds it.
        assert_eq!(read(&state).allowed(), ["Alder", "Birch"]);
        change(&state, &Change::UseOwn { on: false }).unwrap();
        let off = read(&state);
        assert_eq!(off.own, ["Alder", "Birch"]);
        assert_eq!(off.allowed().len(), NAMES.len() + 1);
        assert!(off.allowed().contains(&"Thaddeus".to_owned()));
        change(&state, &Change::UseOwn { on: true }).unwrap();
        assert_eq!(read(&state).allowed(), ["Alder", "Birch"]);
        assert_eq!(read(&state).added, ["Thaddeus"]);
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn excluding_every_name_or_an_empty_own_list_leaves_none_allowed() {
        let state = scratch("none");
        for n in NAMES {
            change(&state, &exclude(n)).unwrap();
        }
        assert!(read(&state).allowed().is_empty());
        assert_eq!(listed(&state).allowed, 0);
        change(&state, &add("Thaddeus")).unwrap();
        assert_eq!(read(&state).allowed(), ["Thaddeus"]);
        fs::remove_dir_all(&state).unwrap();

        let state = scratch("none-own");
        change(&state, &Change::UseOwn { on: true }).unwrap();
        assert_eq!(listed(&state).allowed, 0);
        change(&state, &set_own("Alder")).unwrap();
        assert_eq!(listed(&state).allowed, 1);
        change(&state, &exclude("Alder")).unwrap();
        assert_eq!(listed(&state).allowed, 0);
        fs::remove_dir_all(&state).unwrap();
    }
}
