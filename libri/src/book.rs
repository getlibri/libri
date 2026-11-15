//! Libri's book: the watches and lists its tools keep, one SQLite file in
//! its state directory. The tool server (one per model call) and the bot
//! both change it, each change one transaction.

use chrono::Local;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::path::Path;
use std::time::Duration;

const SCHEMA: &str = "
create table if not exists lists (name text primary key, items text not null);
";

pub struct Book {
    db: Connection,
}

impl Book {
    /// The book in `dir`, made if new.
    pub fn open(dir: &Path) -> Result<Book, String> {
        let mut db = Connection::open(dir.join("book.db")).map_err(|e| format!("book: {e}"))?;
        db.busy_timeout(Duration::from_secs(10))
            .map_err(|e| format!("book: {e}"))?;
        db.execute_batch(SCHEMA).map_err(|e| format!("book: {e}"))?;
        let before = dir.join(crate::watch::BEFORE);
        if let Err(e) = crate::watch::keep_before(&db, &before) {
            eprintln!("libri: {}: {e}", before.display());
        }
        crate::watch::migrate(&mut db, Local::now().naive_local())
            .map_err(|e| format!("book: {e}"))?;
        Ok(Book { db })
    }

    /// One transaction that takes the write lock at once, so a read and the
    /// change it decides on cannot interleave with another process's.
    pub fn change<T>(
        &mut self,
        f: impl FnOnce(&Transaction) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        let t = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("book: {e}"))?;
        let out = f(&t).map_err(|e| format!("book: {e}"))?;
        t.commit().map_err(|e| format!("book: {e}"))?;
        Ok(out)
    }

    /// Reads without taking the write lock.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        f(&self.db).map_err(|e| format!("book: {e}"))
    }
}

/// Names as a column keeps them, a JSON array.
pub fn names(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

pub fn list(db: &Connection, name: &str) -> rusqlite::Result<Option<Vec<String>>> {
    let items: Option<String> = db
        .query_row("select items from lists where name = ?1", [name], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(items.map(|s| serde_json::from_str(&s).unwrap_or_default()))
}

/// Every list, by name.
pub fn lists(db: &Connection) -> rusqlite::Result<Vec<(String, Vec<String>)>> {
    db.prepare("select name, items from lists order by name")?
        .query_map([], |r| {
            let items: String = r.get(1)?;
            Ok((r.get(0)?, serde_json::from_str(&items).unwrap_or_default()))
        })?
        .collect()
}

/// Saves a list; an empty one goes away.
pub fn set_list(db: &Connection, name: &str, items: &[String]) -> rusqlite::Result<()> {
    if items.is_empty() {
        db.execute("delete from lists where name = ?1", [name])?;
    } else {
        db.execute(
            "insert or replace into lists values (?1, ?2)",
            params![name, serde_json::to_string(items).unwrap()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        let d: PathBuf =
            std::env::temp_dir().join(format!("libri-book-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_book_of_reminders_opens_as_watches() {
        let d = dir("migrated");
        let db = Connection::open(d.join("book.db")).unwrap();
        db.execute_batch(
            "create table reminders (id integer primary key autoincrement, at text not null,
                text text not null, repeat text, ask integer not null default 0,
                share_with text not null default '[]', answer text);
             insert into reminders (at, text) values ('2030-01-01 09:00:00', 'the bins');",
        )
        .unwrap();
        drop(db);
        let b = Book::open(&d).unwrap();
        let all = b.read(crate::watch::all).unwrap();
        assert!(all.iter().any(|w| w.text == "the bins"));
        assert_eq!(all.len(), 1);
        drop(b);
        let b = Book::open(&d).unwrap();
        assert_eq!(b.read(crate::watch::all).unwrap().len(), 1);
        // The book as it was is kept beside it, once.
        let kept = Connection::open(d.join(crate::watch::BEFORE)).unwrap();
        let n: i64 = kept
            .query_row("select count(*) from reminders", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn a_list_is_kept_and_an_empty_one_goes() {
        let mut b = Book::open(&dir("lists")).unwrap();
        b.change(|db| set_list(db, "groceries", &["milk".into()]))
            .unwrap();
        assert_eq!(
            b.read(|db| list(db, "groceries")).unwrap(),
            Some(vec!["milk".to_string()])
        );
        b.change(|db| set_list(db, "groceries", &[])).unwrap();
        assert!(b.read(lists).unwrap().is_empty());
    }
}
