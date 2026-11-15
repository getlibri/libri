//! Acts that wait for the person's word. The model can only draft: a draft
//! lands here as a file, the tick loop shows it in the chat, and the site
//! carries it out on the person's 👍 to that message, or drops it on 👎. No tool the model holds can act, so no text it reads
//! can make it.

use crate::mail;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Act {
    Mail {
        account: Option<String>,
        to: String,
        subject: String,
        body: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pending {
    pub id: u64,
    pub act: Act,
    /// The room event that shows it, once posted.
    pub event: Option<String>,
    /// A scratchpad drafted it: neither the draft nor what comes of it
    /// reaches the memory.
    #[serde(default)]
    pub scratchpad: bool,
}

impl Act {
    /// What the room sees, verbatim.
    pub fn show(&self) -> String {
        match self {
            Act::Mail {
                account,
                to,
                subject,
                body,
            } => {
                let from = account
                    .as_ref()
                    .map(|a| format!(" from {a}"))
                    .unwrap_or_default();
                format!(
                    "Draft to {to}{from}\nSubject: {subject}\n\n{body}\n\n👍 sends it, 👎 drops it."
                )
            }
        }
    }

    /// Carries the act out; the model never reaches this. Mail goes out
    /// from the account in the accounts file at LIBRI_MAIL, over the SMTP
    /// server at LIBRI_SMTP.
    pub fn carry_out(&self) -> Result<String, String> {
        match self {
            Act::Mail {
                account,
                to,
                subject,
                body,
            } => {
                let host = std::env::var("LIBRI_SMTP").map_err(|_| "no mail here")?;
                let file = std::env::var("LIBRI_MAIL").map_err(|_| "no mail here")?;
                let text = fs::read_to_string(file).map_err(|e| format!("mail accounts: {e}"))?;
                let accounts = mail::accounts(&text)?;
                let from = mail::pick(&accounts, account.as_deref())?;
                mail::send(&host, from, to, subject, body)
            }
        }
    }
}

fn dir(state: &Path) -> PathBuf {
    state.join("pending")
}

fn path(state: &Path, id: u64) -> PathBuf {
    dir(state).join(format!("{id}.json"))
}

/// Files a new act and returns its id.
pub fn add(state: &Path, act: Act, scratchpad: bool) -> Result<u64, String> {
    let mut id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis() as u64;
    // Two acts in one millisecond, from the turn and the tools alike, each
    // claim an id of their own: the first free one from now on.
    let d = dir(state);
    fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path(state, id))
        {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => id += 1,
            Err(e) => return Err(format!("{}: {e}", d.display())),
        }
    }
    let p = Pending {
        id,
        act,
        event: None,
        scratchpad,
    };
    save(state, &p)?;
    Ok(id)
}

/// Writes the act whole or not at all.
pub fn save(state: &Path, p: &Pending) -> Result<(), String> {
    let d = dir(state);
    fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    let tmp = d.join(format!("{}.tmp", p.id));
    let text = serde_json::to_vec(p).map_err(|e| e.to_string())?;
    fs::write(&tmp, text)
        .and_then(|()| fs::rename(&tmp, path(state, p.id)))
        .map_err(|e| format!("{}: {e}", tmp.display()))
}

pub fn remove(state: &Path, id: u64) -> Result<(), String> {
    let p = path(state, id);
    fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Every act waiting, oldest first.
pub fn all(state: &Path) -> Result<Vec<Pending>, String> {
    let d = dir(state);
    let Ok(entries) = fs::read_dir(&d) else {
        return Ok(Vec::new());
    };
    let mut v = Vec::new();
    for e in entries {
        let p = e.map_err(|e| format!("{}: {e}", d.display()))?.path();
        if p.extension().is_some_and(|x| x == "json") {
            let text = fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            // Claimed by add, not yet written.
            if text.is_empty() {
                continue;
            }
            v.push(serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", p.display()))?);
        }
    }
    v.sort_by_key(|p: &Pending| p.id);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail() -> Act {
        Act::Mail {
            account: None,
            to: "a@b.c".into(),
            subject: "Hi".into(),
            body: "Hello.".into(),
        }
    }

    #[test]
    fn a_draft_says_whether_a_scratchpad_made_it_and_old_ones_read_as_not() {
        let dir = tempdir();
        add(&dir, mail(), true).unwrap();
        assert!(all(&dir).unwrap().remove(0).scratchpad);
        let mut old = serde_json::to_value(all(&dir).unwrap().remove(0)).unwrap();
        old.as_object_mut().unwrap().remove("scratchpad");
        let read: Pending = serde_json::from_value(old).unwrap();
        assert!(!read.scratchpad);
    }

    #[test]
    fn a_draft_waits_until_posted_then_stays_until_removed() {
        let dir = tempdir();
        let id = add(&dir, mail(), false).unwrap();
        let mut p = all(&dir).unwrap().remove(0);
        assert_eq!((p.id, p.event.as_deref()), (id, None));
        p.event = Some("$e".into());
        save(&dir, &p).unwrap();
        assert_eq!(all(&dir).unwrap()[0].event.as_deref(), Some("$e"));
        remove(&dir, id).unwrap();
        assert!(all(&dir).unwrap().is_empty());
    }

    #[test]
    fn acts_filed_at_once_keep_their_own_ids() {
        let dir = tempdir();
        let ids: Vec<u64> = (0..5).map(|_| add(&dir, mail(), false).unwrap()).collect();
        let mut seen = ids.clone();
        seen.dedup();
        assert_eq!(seen.len(), 5);
        assert_eq!(all(&dir).unwrap().len(), 5);
    }

    #[test]
    fn the_room_sees_the_whole_draft() {
        let s = mail().show();
        assert!(s.starts_with("Draft to a@b.c\nSubject: Hi\n\nHello.\n\n👍"));
    }

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "libri-pending-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }
}
