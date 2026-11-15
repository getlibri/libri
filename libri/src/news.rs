//! What's new: the release notes since the version a device last ran, so
//! an app that has just updated says once, in one note, everything that
//! changed while it was away. The notes are the changelog, written at each
//! release (set_version.sh drafts the entry from the commits) and built in.

use serde::Serialize;

const CHANGELOG: &str = include_str!("../CHANGELOG.md");

#[derive(Serialize, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub date: String,
    pub changes: Vec<String>,
}

/// A version as numbers, to order by; None for one that is not x.y.z.
fn parse(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().trim_start_matches('v').split('.');
    let mut next = || parts.next()?.parse().ok();
    let v = (next()?, next()?, next()?);
    parts.next().is_none().then_some(v)
}

/// Every release in the changelog, newest first.
fn releases(text: &str) -> Vec<Release> {
    let mut out: Vec<Release> = Vec::new();
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            let (version, date) = head.split_once(' ').unwrap_or((head, ""));
            out.push(Release {
                version: version.to_owned(),
                date: date
                    .trim_matches(|c| c == '(' || c == ')' || c == ' ')
                    .to_owned(),
                changes: Vec::new(),
            });
        } else if let Some(item) = line.strip_prefix("- ")
            && let Some(r) = out.last_mut()
        {
            r.changes.push(item.to_owned());
        } else if let Some(r) = out.last_mut()
            && let Some(last) = r.changes.last_mut()
            && line.starts_with("  ")
        {
            // An item wrapped onto the next line.
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    out
}

/// The releases after `since` up to this build's, newest first. None
/// when `since` is not a version: a device with none kept is new and has
/// nothing to catch up on.
pub fn since(since: &str) -> Option<Vec<Release>> {
    within(CHANGELOG, since, env!("CARGO_PKG_VERSION"))
}

fn within(text: &str, since: &str, now: &str) -> Option<Vec<Release>> {
    let (from, to) = (parse(since)?, parse(now)?);
    Some(
        releases(text)
            .into_iter()
            .filter(|r| parse(&r.version).is_some_and(|v| v > from && v <= to))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "# Changelog\n\nWords.\n\n## 0.9.3 (2026-01-03)\n\n- Three.\n- Three again,\n  wrapped.\n\n## 0.9.2 (2026-01-02)\n\n- Two.\n\n## 0.9.1 (2026-01-01)\n\n- One.\n";

    #[test]
    fn gathers_every_release_since_the_one_last_run() {
        let got = within(TEXT, "0.9.1", "0.9.3").unwrap();
        assert_eq!(
            got.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(),
            ["0.9.3", "0.9.2"]
        );
        assert_eq!(got[0].changes, ["Three.", "Three again, wrapped."]);
        assert_eq!(got[0].date, "2026-01-03");
        assert!(within(TEXT, "0.9.3", "0.9.3").unwrap().is_empty());
        // A device ahead of the host (rolled back) hears nothing.
        assert!(within(TEXT, "0.9.9", "0.9.3").unwrap().is_empty());
        assert_eq!(within(TEXT, "", "0.9.3"), None);
        // Releases beyond this build's, written ahead, wait for it.
        assert_eq!(within(TEXT, "0.9.1", "0.9.2").unwrap().len(), 1);
    }

    /// Each release carries its note: the release commit sets the version
    /// and writes the entry together.
    #[test]
    fn this_version_has_its_entry() {
        let top = releases(CHANGELOG);
        assert_eq!(
            top.first().map(|r| r.version.as_str()),
            Some(env!("CARGO_PKG_VERSION")),
            "CHANGELOG.md's newest entry is not this version's"
        );
        assert!(!top[0].changes.is_empty());
    }
}
