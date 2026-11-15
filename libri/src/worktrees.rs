//! The git worktrees workers make, and their cleanup, as a forge deletes a
//! branch on merge: once a worker's branch is on the repository's main
//! branch, Libri deletes it here and on origin and removes its worktree.
//! An archived worker's worktree goes too when its branch never left main;
//! one with commits not on main, or edits not committed, stays, and its
//! card says so. Nothing is discarded unseen.
//!
//! Each worker's worktrees are listed in `worktrees.json` in its folder,
//! written by `make_worktree`: what Libri didn't make, it never touches.

use crate::lifecycle::{self, Phase};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FILE: &str = "worktrees.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Worktree {
    /// The repository's main checkout.
    pub repo: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    /// The commit it started from: a branch still on it has no work of its
    /// own, which for a worker at work is not yet landed.
    pub base: String,
}

pub fn list(d: &Path) -> Vec<Worktree> {
    fs::read_to_string(d.join(FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(d: &Path, all: &[Worktree]) -> Result<(), String> {
    let file = d.join(FILE);
    if all.is_empty() {
        return match fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let text = serde_json::to_string_pretty(all).map_err(|e| e.to_string())?;
    let part = d.join(format!("{FILE}.part"));
    fs::write(&part, text).map_err(|e| e.to_string())?;
    fs::rename(&part, &file).map_err(|e| e.to_string())
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The ref a branch lands on: origin's default branch, else the local
/// `main` or `master` of a repository without one.
fn main_ref(repo: &Path) -> Result<String, String> {
    if let Ok(r) = git(
        repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        return Ok(r);
    }
    for r in ["origin/main", "origin/master", "main", "master"] {
        if git(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{r}^{{commit}}"),
            ],
        )
        .is_ok()
        {
            return Ok(r.to_owned());
        }
    }
    Err(format!("{} has no main branch.", repo.display()))
}

fn has_origin(repo: &Path) -> bool {
    git(repo, &["remote", "get-url", "origin"]).is_ok()
}

/// Makes a worktree of `repo` on a new branch `branch` from its main
/// branch, beside the repository in `.worktrees/`, and lists it as worker
/// folder `d`'s.
pub fn make(d: &Path, repo: &Path, branch: &str) -> Result<Worktree, String> {
    git(repo, &["check-ref-format", "--branch", branch])
        .map_err(|_| format!("{branch} is not a branch name git takes."))?;
    // The main checkout, whichever of its worktrees was named.
    let common = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map_err(|_| format!("{} is not a git repository.", repo.display()))?;
    let repo = Path::new(&common)
        .parent()
        .ok_or("No repository above its git folder.")?
        .to_path_buf();
    if has_origin(&repo) {
        git(&repo, &["fetch", "--quiet", "origin"])?;
    }
    let from = main_ref(&repo)?;
    let base = git(&repo, &["rev-parse", &format!("{from}^{{commit}}")])?;
    let name = repo
        .file_name()
        .map_or_else(|| "repo".into(), |n| n.to_string_lossy().into_owned());
    let path = repo
        .parent()
        .ok_or("The repository has no folder above it.")?
        .join(".worktrees")
        .join(format!("{name}-{}", branch.replace('/', "-")));
    if path.exists() {
        return Err(format!("{} is already there.", path.display()));
    }
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "--no-track",
            "-b",
            branch,
            &path.to_string_lossy(),
            &base,
        ],
    )?;
    let made = Worktree {
        repo,
        path,
        branch: branch.to_owned(),
        base,
    };
    lifecycle::change(d, |_| {
        let mut all = list(d);
        all.push(made.clone());
        save(d, &all)
    })?;
    Ok(made)
}

/// What becomes of a listed worktree.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fate {
    /// Its branch is on main: branch and worktree go.
    Remove,
    /// Its worker may still want it.
    Keep,
}

/// `archived`: its worker is put away. `dirty`: edits not committed.
/// `ahead`: commits not on main, here or on origin. `moved`: commits since
/// its base at all.
fn fate(archived: bool, dirty: bool, ahead: bool, moved: bool) -> Fate {
    if dirty || ahead || !(moved || archived) {
        Fate::Keep
    } else {
        Fate::Remove
    }
}

/// Whether `tip` has a commit main lacks, by content as well as by name, so
/// a branch landed by a rebase or a cherry-pick counts as landed.
fn ahead_of(repo: &Path, main: &str, tip: &str) -> Result<bool, String> {
    let cherry = git(repo, &["cherry", main, tip])?;
    Ok(cherry.lines().any(|l| l.starts_with('+')))
}

fn tip(repo: &Path, r: &str) -> Option<String> {
    git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{r}^{{commit}}"),
        ],
    )
    .ok()
}

/// Judges one worktree, with origin already fetched; a question git can't
/// answer keeps it.
fn judge(w: &Worktree, archived: bool) -> Fate {
    let Ok(main) = main_ref(&w.repo) else {
        return Fate::Keep;
    };
    let dirty =
        w.path.exists() && git(&w.path, &["status", "--porcelain"]).map_or(true, |s| !s.is_empty());
    let tips: Vec<String> = [w.branch.clone(), format!("origin/{}", w.branch)]
        .iter()
        .filter_map(|r| tip(&w.repo, r))
        .collect();
    let mut ahead = false;
    for t in &tips {
        match ahead_of(&w.repo, &main, t) {
            Ok(a) => ahead |= a,
            Err(_) => return Fate::Keep,
        }
    }
    let moved = tips.iter().any(|t| *t != w.base);
    fate(archived, dirty, ahead, moved)
}

/// Deletes a landed worktree's worktree, branch and origin branch.
fn remove(w: &Worktree) -> Result<(), String> {
    if w.path.exists() {
        // Without --force, so git refuses edits made since it was judged.
        git(&w.repo, &["worktree", "remove", &w.path.to_string_lossy()])?;
    } else {
        git(&w.repo, &["worktree", "prune"])?;
    }
    if tip(&w.repo, &format!("refs/heads/{}", w.branch)).is_some() {
        git(&w.repo, &["branch", "-D", &w.branch])?;
    }
    if tip(&w.repo, &format!("refs/remotes/origin/{}", w.branch)).is_some() {
        git(
            &w.repo,
            &["push", "--quiet", "origin", "--delete", &w.branch],
        )?;
    }
    Ok(())
}

/// Cleans up after every worker not at work. Fetches each repository once.
pub fn sweep(tasks: &Path) {
    let Ok(entries) = fs::read_dir(tasks) else {
        return;
    };
    let folders: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    let mut fetched: BTreeMap<PathBuf, bool> = BTreeMap::new();
    for d in folders {
        let all = list(&d);
        if all.is_empty() {
            continue;
        }
        for w in &all {
            fetched.entry(w.repo.clone()).or_insert_with(|| {
                !has_origin(&w.repo)
                    || git(&w.repo, &["fetch", "--quiet", "--prune", "origin"]).is_ok()
            });
        }
        sweep_one(&d, &all, |repo| fetched.get(repo).copied().unwrap_or(false));
    }
}

/// Cleans up after worker folder `d`, whose list is `all`; `fresh` says
/// whether a repository's origin was fetched, without which nothing goes.
fn sweep_one(d: &Path, all: &[Worktree], fresh: impl Fn(&Path) -> bool) {
    let rec = lifecycle::read(d);
    if rec.at_work() {
        return;
    }
    let archived = matches!(rec.phase, Phase::Archived { .. });
    let mut gone = Vec::new();
    for w in all {
        if fresh(&w.repo) && judge(w, archived) == Fate::Remove {
            match remove(w) {
                Ok(()) => gone.push(w.clone()),
                Err(e) => eprintln!("worktrees: {}: {e}", w.path.display()),
            }
        }
    }
    if gone.is_empty() {
        return;
    }
    let left = lifecycle::change(d, |_| {
        let left: Vec<Worktree> = list(d).into_iter().filter(|w| !gone.contains(w)).collect();
        save(d, &left)
    });
    if let Err(e) = left {
        eprintln!("worktrees: {}: {e}", d.display());
    }
}

/// Cleans up after worker folder `d` alone, as when it is archived.
pub fn sweep_worker(d: &Path) {
    let all = list(d);
    if all.is_empty() {
        return;
    }
    let mut fetched: BTreeMap<PathBuf, bool> = BTreeMap::new();
    for w in &all {
        fetched.entry(w.repo.clone()).or_insert_with(|| {
            !has_origin(&w.repo) || git(&w.repo, &["fetch", "--quiet", "--prune", "origin"]).is_ok()
        });
    }
    sweep_one(d, &all, |repo| fetched.get(repo).copied().unwrap_or(false));
}

/// The branches an archived worker left with work not on main, for its card.
pub fn unlanded(d: &Path, phase: Phase) -> Vec<String> {
    match phase {
        Phase::Archived { .. } => list(d).into_iter().map(|w| w.branch).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("libri-worktrees-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn run(dir: &Path, args: &[&str]) {
        git(dir, args).unwrap();
    }

    /// An origin, its clone, and a worker folder.
    fn setup(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let top = scratch(name);
        let origin = top.join("origin.git");
        let repo = top.join("repo");
        let worker = top.join("task").join("w-1");
        fs::create_dir_all(&worker).unwrap();
        fs::create_dir_all(&repo).unwrap();
        Command::new("git")
            .args(["init", "-q", "--bare", "-b", "main"])
            .arg(&origin)
            .status()
            .unwrap();
        run(&repo, &["init", "-q", "-b", "main"]);
        for (k, v) in [
            ("user.name", "T"),
            ("user.email", "t@t"),
            ("commit.gpgsign", "false"),
        ] {
            run(&repo, &["config", k, v]);
        }
        fs::write(repo.join("a"), "a").unwrap();
        run(&repo, &["add", "a"]);
        run(&repo, &["commit", "-qm", "a"]);
        run(
            &repo,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
        run(&repo, &["push", "-q", "-u", "origin", "main"]);
        run(&repo, &["remote", "set-head", "origin", "main"]);
        (repo, worker, top)
    }

    fn commit(dir: &Path, file: &str) {
        fs::write(dir.join(file), file).unwrap();
        run(dir, &["add", file]);
        run(
            dir,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                file,
            ],
        );
    }

    fn archive(worker: &Path) {
        lifecycle::apply(worker, lifecycle::Event::Archive { at: 1 }).unwrap();
    }

    #[test]
    fn the_rules() {
        // Edits or commits not on main keep it, archived or not.
        for archived in [false, true] {
            assert_eq!(fate(archived, true, false, true), Fate::Keep);
            assert_eq!(fate(archived, false, true, true), Fate::Keep);
            assert_eq!(fate(archived, false, false, true), Fate::Remove);
        }
        // A branch with no work yet goes only once its worker is archived.
        assert_eq!(fate(false, false, false, false), Fate::Keep);
        assert_eq!(fate(true, false, false, false), Fate::Remove);
    }

    #[test]
    fn a_landed_branch_goes_here_and_on_origin() {
        let (repo, worker, top) = setup("landed");
        let w = make(&worker, &repo, "feature").unwrap();
        assert_eq!(w.path, top.join(".worktrees/repo-feature"));
        commit(&w.path, "b");
        run(&w.path, &["push", "-q", "origin", "feature"]);
        sweep(&top.join("task"));
        assert_eq!(list(&worker).len(), 1, "not on main yet");
        // Landed by a rebase, so under another commit than the branch's.
        commit(&repo, "m");
        run(&repo, &["cherry-pick", "feature"]);
        run(&repo, &["push", "-q", "origin", "main"]);
        sweep(&top.join("task"));
        assert!(list(&worker).is_empty());
        assert!(!w.path.exists());
        assert!(tip(&repo, "refs/heads/feature").is_none());
        assert!(
            git(
                &repo,
                &["ls-remote", "--exit-code", "--heads", "origin", "feature"]
            )
            .is_err()
        );
    }

    #[test]
    fn a_fresh_branch_stays_until_its_worker_is_archived() {
        let (repo, worker, top) = setup("fresh");
        let w = make(&worker, &repo, "idea").unwrap();
        sweep(&top.join("task"));
        assert!(w.path.exists());
        assert!(unlanded(&worker, lifecycle::read(&worker).phase).is_empty());
        archive(&worker);
        sweep_worker(&worker);
        assert!(!w.path.exists());
        assert!(list(&worker).is_empty());
    }

    #[test]
    fn unlanded_work_stays_and_shows() {
        let (repo, worker, _top) = setup("unlanded");
        let w = make(&worker, &repo, "half").unwrap();
        commit(&w.path, "c");
        let edited = make(&worker, &repo, "edited").unwrap();
        fs::write(edited.path.join("a"), "changed").unwrap();
        archive(&worker);
        sweep_worker(&worker);
        assert!(w.path.exists() && edited.path.exists());
        assert_eq!(
            unlanded(&worker, lifecycle::read(&worker).phase),
            vec!["half".to_owned(), "edited".to_owned()]
        );
    }

    #[test]
    fn a_worker_at_work_is_left_alone() {
        let (repo, worker, top) = setup("busy");
        let w = make(&worker, &repo, "now").unwrap();
        commit(&w.path, "d");
        run(&repo, &["merge", "-q", "--ff-only", "now"]);
        run(&repo, &["push", "-q", "origin", "main"]);
        lifecycle::apply(&worker, lifecycle::Event::Go { at: 1 }).unwrap();
        sweep(&top.join("task"));
        assert!(w.path.exists());
    }

    #[test]
    fn a_bad_name_or_folder_is_refused() {
        let (repo, worker, top) = setup("refused");
        assert!(make(&worker, &repo, "no..dots").is_err());
        assert!(make(&worker, &top, "x").is_err());
        make(&worker, &repo, "twice").unwrap();
        assert!(make(&worker, &repo, "twice").is_err());
    }
}
