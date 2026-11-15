//! A watch: the one thing that happens on its own, shaped like a Home
//! Assistant automation. `when` is a time, an interval or a push, and `then`
//! is a note said word for word or a turn of the model, in the main chat or a
//! named worker's thread. A reminder is a note at a time; a routine a turn at
//! a time.
//!
//! A watch may also `look`: on its timer it calls a read-only tool, and only
//! an item it has not seen that passes its tests fires it. The first look
//! primes it, counting what is there. What fires waits in `watch_items`
//! until `then` has run, so a hung turn or a restart loses nothing and
//! fires nothing twice. A push (see `hook`) brings its own item down the
//! same path.
//!
//! A due watch stays due until it is delivered: the bot marks it done only
//! after the door takes the message, and a turn's answer is kept until then,
//! so a failed send neither loses it nor asks the model again.

use crate::hook::Hook;
use crate::look::{Found, Look};
use chrono::{Days, Duration, Months, NaiveDateTime};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const WATCH: &str = "
create table if not exists watch (
    id integer primary key autoincrement,
    at text,
    every text,
    turn integer not null default 0,
    text text not null,
    worker text,
    share_with text not null default '[]',
    answer text,
    active integer not null default 1,
    look text,
    tests text not null default '[]',
    fields text not null default '[]',
    primed integer not null default 0,
    failures integer not null default 0,
    error text,
    told integer not null default 0,
    hook text
);
create table if not exists watch_seen (
    watch integer not null,
    key text not null,
    at text not null,
    primary key (watch, key)
);
create table if not exists watch_items (
    id integer primary key autoincrement,
    watch integer not null,
    at text not null,
    item text not null
);
";

/// Failed looks in a row before the person hears, once. One server hiccup
/// is no news.
const TELL_AFTER: u32 = 3;

/// The longest a failing look waits before it tries again.
const MOST_BACKOFF: u32 = 6 * 60;

/// Most items one firing hands `then`; the rest wait for the next.
const MOST_ITEMS: usize = 20;

/// Most a push may have waiting for `then`; past it a sender must try again.
const MOST_WAITING: usize = 20;

/// How long a look remembers an item, so its arguments must keep to a
/// shorter window.
const SEEN_DAYS: i64 = 60;

/// How long a push remembers a delivery: past any sender's retries.
const SEEN_PUSH_DAYS: i64 = 7;

const AT: &str = "%Y-%m-%d %H:%M:%S";

/// How long the next time lies after the last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Every {
    Minutes(u32),
    Daily,
    Weekly,
    Monthly,
}

impl Every {
    /// The shortest interval: a watch on a timer is a job for the model or
    /// a server, neither of which needs asking more often.
    pub const FLOOR: u32 = 5;

    fn after(self, t: NaiveDateTime) -> NaiveDateTime {
        match self {
            Every::Minutes(m) => t + Duration::minutes(m.into()),
            Every::Daily => t + Days::new(1),
            Every::Weekly => t + Days::new(7),
            Every::Monthly => t + Months::new(1),
        }
    }

    pub fn name(self) -> String {
        match self {
            Every::Minutes(m) if m % 60 == 0 => format!("{}h", m / 60),
            Every::Minutes(m) => format!("{m}m"),
            Every::Daily => "daily".into(),
            Every::Weekly => "weekly".into(),
            Every::Monthly => "monthly".into(),
        }
    }

    /// `daily`, `weekly`, `monthly`, or an interval such as `30m` or `3h`.
    pub fn parse(s: &str) -> Option<Every> {
        Some(match s.trim() {
            "daily" => Every::Daily,
            "weekly" => Every::Weekly,
            "monthly" => Every::Monthly,
            s => Every::Minutes(match (s.strip_suffix('m'), s.strip_suffix('h')) {
                (Some(m), _) => m.parse().ok()?,
                (_, Some(h)) => h.parse::<u32>().ok()?.checked_mul(60)?,
                _ => return None,
            }),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    /// The field equals the value, in any case.
    Is,
    /// The field holds the value, in any case.
    Has,
    /// The field is a number above the value.
    Over,
    /// The field is a number below the value.
    Under,
}

/// One test on an item; a watch fires only when all of them pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Test {
    /// A field of the item, as a dotted path into it.
    pub field: String,
    pub op: Op,
    pub value: String,
}

impl Test {
    fn passes(&self, item: &Value) -> bool {
        let Some(f) = field(item, &self.field) else {
            return false;
        };
        let text = shown(f).to_lowercase();
        let want = self.value.trim().to_lowercase();
        let num = || {
            Some((
                f.as_f64().or_else(|| f.as_str()?.parse().ok())?,
                want.trim_start_matches('$').parse::<f64>().ok()?,
            ))
        };
        match self.op {
            Op::Is => text == want,
            Op::Has => text.contains(&want),
            Op::Over => num().is_some_and(|(n, v)| n > v),
            Op::Under => num().is_some_and(|(n, v)| n < v),
        }
    }
}

/// The value at a dotted path: `head_commit.message`, `results.0.id`.
pub fn field<'a>(item: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(item, |v, part| match v {
        Value::Array(list) => list.get(part.parse::<usize>().ok()?),
        _ => v.get(part),
    })
}

/// A value as text: a string as itself.
pub fn shown(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// What `then` may see of an item: only the fields the watch names.
fn named(item: &Value, fields: &[String]) -> Value {
    let chosen = fields
        .iter()
        .filter_map(|f| Some((f.clone(), field(item, f)?.clone())));
    Value::Object(chosen.collect::<Map<_, _>>())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Watch {
    pub id: u64,
    /// The next time it is due, local; None for one a push wakes.
    pub at: Option<NaiveDateTime>,
    /// None fires once.
    pub every: Option<Every>,
    /// A turn: the model carries out `text` as a request and its answer is
    /// sent; otherwise `text` is sent itself.
    pub turn: bool,
    pub text: String,
    /// The worker whose thread takes the turn; None is the main chat, which
    /// also takes it while the worker is gone.
    pub worker: Option<String>,
    /// For a turn: who else its answer goes to, each by name.
    pub share_with: Vec<String>,
    /// A turn's answer, kept until it is delivered.
    pub answer: Option<String>,
    /// False holds it off without forgetting it.
    pub active: bool,
    /// What it reads on its timer; None always fires.
    pub look: Option<Look>,
    /// All must pass for an item to fire it.
    pub tests: Vec<Test>,
    /// The fields of an item that reach `then`.
    pub fields: Vec<String>,
    /// The first look has counted what was there.
    pub primed: bool,
    /// Failed looks in a row, and the last one's reason.
    pub failures: u32,
    pub error: Option<String>,
    /// The person has heard of these failures.
    pub told: bool,
    /// Set, a push wakes it and it has no timer.
    pub hook: Option<Hook>,
}

impl Watch {
    /// An ask on a look or a push that names no fields: its turn sees only
    /// the named fields of an item, so it would see none at all.
    pub fn sees_nothing(&self) -> bool {
        self.turn && (self.look.is_some() || self.hook.is_some()) && self.fields.is_empty()
    }

    fn passes(&self, item: &Value) -> bool {
        self.tests.iter().all(|t| t.passes(item))
    }

    /// A line for the tool that shows them.
    pub fn show(&self) -> String {
        let when = match (&self.hook, self.at) {
            (None, Some(at)) => at.format("%a %Y-%m-%d %H:%M").to_string(),
            _ => "on push".to_owned(),
        };
        let mut out = format!("{}: {when}", self.id);
        if let Some(every) = self.every {
            out.push_str(&format!(", {}", every.name()));
        }
        out.push_str(&match self.turn {
            true => format!(" turn: {}", self.text),
            false => format!(" note: {}", self.text),
        });
        if let Some(l) = &self.look {
            out.push_str(&format!(" (looks at {} {})", l.server, l.tool));
        }
        if !self.tests.is_empty() {
            let tests: Vec<String> = self
                .tests
                .iter()
                .map(|t| format!("{} {:?} {}", t.field, t.op, t.value))
                .collect();
            out.push_str(&format!(" (tests {})", tests.join(", ")));
        }
        if let Some(e) = &self.error {
            out.push_str(&format!(" (failing: {e})"));
        }
        if let Some(w) = &self.worker {
            out.push_str(&format!(" (in worker {w}'s thread)"));
        }
        if !self.share_with.is_empty() {
            out.push_str(&format!(" (shared with {})", self.share_with.join(", ")));
        }
        if !self.active {
            out.push_str(" (off)");
        }
        out
    }
}

fn time(t: Option<NaiveDateTime>) -> Option<String> {
    t.map(|t| t.format(AT).to_string())
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Watch> {
    let at: Option<String> = r.get(1)?;
    let every: Option<String> = r.get(2)?;
    Ok(Watch {
        id: r.get(0)?,
        at: at.and_then(|at| NaiveDateTime::parse_from_str(&at, AT).ok()),
        every: every.as_deref().and_then(Every::parse),
        turn: r.get(3)?,
        text: r.get(4)?,
        worker: r.get(5)?,
        share_with: crate::book::names(&r.get::<_, String>(6)?),
        answer: r.get(7)?,
        active: r.get(8)?,
        look: json(r.get::<_, Option<String>>(9)?),
        tests: json(r.get::<_, Option<String>>(10)?).unwrap_or_default(),
        fields: json(r.get::<_, Option<String>>(11)?).unwrap_or_default(),
        primed: r.get(12)?,
        failures: r.get(13)?,
        error: r.get(14)?,
        told: r.get(15)?,
        hook: json(r.get::<_, Option<String>>(16)?),
    })
}

fn json<T: for<'a> Deserialize<'a>>(s: Option<String>) -> Option<T> {
    serde_json::from_str(&s?).ok()
}

fn text<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap()
}

const COLUMNS: &str = "id, at, every, turn, text, worker, share_with, answer, active, look, \
                       tests, fields, primed, failures, error, told, hook";

pub fn add(db: &Connection, w: &Watch) -> rusqlite::Result<u64> {
    db.execute(
        "insert into watch (at, every, turn, text, worker, share_with, look, tests, fields, hook)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            time(w.at),
            w.every.map(Every::name),
            w.turn,
            w.text,
            w.worker,
            text(&w.share_with),
            w.look.as_ref().map(text),
            text(&w.tests),
            text(&w.fields),
            w.hook.as_ref().map(text)
        ],
    )?;
    Ok(db.last_insert_rowid() as u64)
}

/// Saves what the tool can change of a watch, whose time moves with it.
pub fn change(db: &Connection, w: &Watch) -> rusqlite::Result<()> {
    db.execute(
        "update watch set at = ?2, every = ?3, turn = ?4, text = ?5, worker = ?6,
         share_with = ?7, active = ?8, look = ?9, tests = ?10, fields = ?11, hook = ?12,
         answer = null where id = ?1",
        params![
            w.id,
            time(w.at),
            w.every.map(Every::name),
            w.turn,
            w.text,
            w.worker,
            text(&w.share_with),
            w.active,
            w.look.as_ref().map(text),
            text(&w.tests),
            text(&w.fields),
            w.hook.as_ref().map(text)
        ],
    )?;
    Ok(())
}

/// Every watch, soonest first, those with no time last.
pub fn all(db: &Connection) -> rusqlite::Result<Vec<Watch>> {
    db.prepare(&format!(
        "select {COLUMNS} from watch order by at is null, at, id"
    ))?
    .query_map([], row)?
    .collect()
}

pub fn get(db: &Connection, id: u64) -> rusqlite::Result<Option<Watch>> {
    db.query_row(
        &format!("select {COLUMNS} from watch where id = ?1"),
        [id],
        row,
    )
    .optional()
}

pub fn remove(db: &Connection, id: u64) -> rusqlite::Result<bool> {
    db.execute("delete from watch_seen where watch = ?1", [id])?;
    db.execute("delete from watch_items where watch = ?1", [id])?;
    Ok(db.execute("delete from watch where id = ?1", [id])? > 0)
}

/// The person's watches due by `now` with nothing to look at first; they
/// stay until `done`.
pub fn due(db: &Connection, now: NaiveDateTime) -> rusqlite::Result<Vec<Watch>> {
    db.prepare(&format!(
        "select {COLUMNS} from watch
         where at <= ?1 and active != 0 and look is null order by at, id"
    ))?
    .query_map([now.format(AT).to_string()], row)?
    .collect()
}

/// The watches with a look due by `now`.
pub fn looks(db: &Connection, now: NaiveDateTime) -> rusqlite::Result<Vec<Watch>> {
    db.prepare(&format!(
        "select {COLUMNS} from watch
         where at <= ?1 and active != 0 and look is not null order by at, id"
    ))?
    .query_map([now.format(AT).to_string()], row)?
    .collect()
}

/// What came of an item.
#[derive(Debug, PartialEq)]
pub enum Arrival {
    /// Already counted.
    Seen,
    /// It failed a test, and may pass a later look.
    Failed,
    /// It fires the watch.
    Fired,
    /// Too much waits already; it was not counted.
    Full,
}

/// An item reaches a watch, from a look or a push alike: new and passing,
/// it is counted and waits for `then`, both in one transaction.
pub fn arrived(
    db: &Connection,
    w: &Watch,
    key: &str,
    item: &Value,
    now: NaiveDateTime,
) -> rusqlite::Result<Arrival> {
    if seen(db, w, key)? {
        return Ok(Arrival::Seen);
    }
    if !w.passes(item) {
        return Ok(Arrival::Failed);
    }
    count(db, w, key, now)?;
    db.execute(
        "insert into watch_items (watch, at, item) values (?1, ?2, ?3)",
        params![
            w.id,
            now.format(AT).to_string(),
            text(&named(item, &w.fields))
        ],
    )?;
    Ok(Arrival::Fired)
}

/// An item a sender pushed. With its queue full a new one is turned away,
/// uncounted, so the sender's next try can still bring it.
pub fn pushed(
    db: &Connection,
    w: &Watch,
    key: &str,
    item: &Value,
    now: NaiveDateTime,
) -> rusqlite::Result<Arrival> {
    let waiting: usize = db.query_row(
        "select count(*) from watch_items where watch = ?1",
        [w.id],
        |r| r.get(0),
    )?;
    if waiting >= MOST_WAITING && w.passes(item) && !seen(db, w, key)? {
        return Ok(Arrival::Full);
    }
    arrived(db, w, key, item, now)
}

fn seen(db: &Connection, w: &Watch, key: &str) -> rusqlite::Result<bool> {
    let known = db.query_row(
        "select 1 from watch_seen where watch = ?1 and key = ?2",
        params![w.id, key],
        |_| Ok(()),
    );
    Ok(known.optional()?.is_some())
}

fn count(db: &Connection, w: &Watch, key: &str, now: NaiveDateTime) -> rusqlite::Result<()> {
    db.execute(
        "insert or ignore into watch_seen values (?1, ?2, ?3)",
        params![w.id, key, now.format(AT).to_string()],
    )
    .map(drop)
}

/// A look came back. The first only counts what is there; any clears the
/// watch's failures.
pub fn listed(
    db: &Connection,
    w: &Watch,
    found: &[Found],
    now: NaiveDateTime,
) -> rusqlite::Result<()> {
    for (key, item) in found {
        if w.primed {
            arrived(db, w, key, item, now)?;
        } else {
            count(db, w, key, now)?;
        }
    }
    db.execute(
        "update watch set primed = 1, failures = 0, error = null, told = 0 where id = ?1",
        [w.id],
    )
    .map(drop)
}

/// A changed look starts over, counting what it finds.
pub fn reprime(db: &Connection, w: &Watch) -> rusqlite::Result<()> {
    db.execute("delete from watch_seen where watch = ?1", [w.id])?;
    db.execute(
        "update watch set primed = 0, failures = 0, error = null, told = 0 where id = ?1",
        [w.id],
    )
    .map(drop)
}

/// A look failed: the next waits twice as long each time, up to six hours.
pub fn look_failed(
    db: &Connection,
    w: &Watch,
    error: &str,
    now: NaiveDateTime,
) -> rusqlite::Result<()> {
    let failures = w.failures + 1;
    let mut at = w.every.map_or(now, |e| e.after(now));
    if let Some(Every::Minutes(m)) = w.every {
        let doubled = m.saturating_mul(1u32 << failures.min(10));
        at = now + Duration::minutes(doubled.min(MOST_BACKOFF.max(m)).into());
    }
    db.execute(
        "update watch set at = ?2, failures = ?3, error = ?4 where id = ?1",
        params![w.id, time(Some(at)), failures, error],
    )
    .map(drop)
}

/// Looks that have failed long enough for the person to hear, and have not.
pub fn failing(db: &Connection) -> rusqlite::Result<Vec<Watch>> {
    db.prepare(&format!(
        "select {COLUMNS} from watch where failures >= ?1 and told = 0 order by id"
    ))?
    .query_map([TELL_AFTER], row)?
    .collect()
}

pub fn told_of_failure(db: &Connection, w: &Watch) -> rusqlite::Result<()> {
    db.execute("update watch set told = 1 where id = ?1", [w.id])
        .map(drop)
}

/// A watch that has fired, with the items waiting for `then`.
pub struct Fired {
    pub watch: Watch,
    pub ids: Vec<i64>,
    /// Each item as `then` may see it, one line of JSON.
    pub items: Vec<String>,
}

/// The watches of one kind of `then` with items waiting, the items oldest
/// first and at most `MOST_ITEMS` of them.
pub fn fired(db: &Connection, turn: bool) -> rusqlite::Result<Vec<Fired>> {
    let watches: Vec<Watch> = db
        .prepare(&format!(
            "select {COLUMNS} from watch
             where turn = ?1 and active != 0
             and id in (select watch from watch_items) order by id"
        ))?
        .query_map([turn], row)?
        .collect::<rusqlite::Result<_>>()?;
    let mut read =
        db.prepare("select id, item from watch_items where watch = ?1 order by id limit ?2")?;
    watches
        .into_iter()
        .map(|watch| {
            let rows: Vec<(i64, String)> = read
                .query_map(params![watch.id, MOST_ITEMS], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let (ids, items) = rows.into_iter().unzip();
            Ok(Fired { watch, ids, items })
        })
        .collect()
}

/// `then` has run for these items.
pub fn ran(db: &Connection, ids: &[i64]) -> rusqlite::Result<()> {
    let mut gone = db.prepare("delete from watch_items where id = ?1")?;
    ids.iter().try_for_each(|id| gone.execute([id]).map(drop))
}

/// Forgets items long seen; a look's arguments keep to a shorter window, so
/// an old key can't come back. A sender's retries end sooner.
pub fn prune(db: &Connection, now: NaiveDateTime) -> rusqlite::Result<()> {
    let cut = |days| (now - Duration::days(days)).format(AT).to_string();
    db.execute(
        "delete from watch_seen where at < ?1 or (at < ?2 and watch in
            (select id from watch where hook is not null))",
        params![cut(SEEN_DAYS), cut(SEEN_PUSH_DAYS)],
    )
    .map(drop)
}

/// Keeps a turn's answer until it is delivered.
pub fn keep_answer(db: &Connection, w: &Watch, answer: &str) -> rusqlite::Result<()> {
    db.execute(
        "update watch set answer = ?2 where id = ?1",
        params![w.id, answer],
    )?;
    Ok(())
}

/// A due watch was delivered: a one-off goes, a repeat moves to its next time
/// after `now`, so a missed stretch fires once. Only the occurrence that was
/// due is touched, should it have changed meanwhile.
pub fn done(db: &Connection, w: &Watch, now: NaiveDateTime) -> rusqlite::Result<()> {
    let Some(was) = w.at else {
        return Ok(());
    };
    match w.every {
        Some(every) => {
            let mut at = was;
            while at <= now {
                at = every.after(at);
            }
            db.execute(
                "update watch set at = ?3, answer = null where id = ?1 and at = ?2",
                params![w.id, time(Some(was)), time(Some(at))],
            )?
        }
        None => db.execute(
            "delete from watch where id = ?1 and at = ?2",
            params![w.id, time(Some(was))],
        )?,
    };
    Ok(())
}

/// Whether the book has a table.
fn has_table(db: &Connection, name: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "select count(*) from sqlite_master where type = 'table' and name = ?1",
        [name],
        |r| r.get(0),
    )
}

/// The copy of a book taken before it became one of watches, beside it.
pub const BEFORE: &str = "book-before-watch.db";

/// Copies a book about to be migrated to `to`, so that a build from before
/// the one watch can be given its reminders back. Once: a copy that exists
/// stays. Remove with the migration.
pub fn keep_before(db: &Connection, to: &std::path::Path) -> rusqlite::Result<()> {
    if has_table(db, "watch")? || !has_table(db, "reminders")? || to.exists() {
        return Ok(());
    }
    db.execute("vacuum into ?1", [to.to_string_lossy()])
        .map(drop)
}

/// A book made before there was one watch keeps its reminders, routines and
/// polled watches as rows of `watch`. The built-in brief and check-ins go
/// with quiet hours and the daily cap, leaving no routine in their place;
/// what quiet hours held is said at once.
/// A new book starts empty. Run on every open, once in effect: the table is
/// made and filled in one transaction, against the daemon and any tool
/// server opening the book with it.
pub fn migrate(db: &mut Connection, now: NaiveDateTime) -> rusqlite::Result<()> {
    if has_table(db, "watch")? {
        return Ok(());
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !has_table(&tx, "watch")? {
        // The old table's rows belong to the old watches' ids.
        tx.execute_batch("drop table if exists watch_seen;")?;
        tx.execute_batch(WATCH)?;
        moved_reminders(&tx)?;
        moved_held(&tx, now)?;
        moved_mail(&tx, now)?;
    }
    tx.commit()
}

fn moved_reminders(tx: &Transaction) -> rusqlite::Result<()> {
    if !has_table(tx, "reminders")? {
        return Ok(());
    }
    tx.execute_batch(
        "insert into watch (at, every, turn, text, share_with, answer)
             select at, repeat, ask, text, share_with, answer from reminders order by id;
         drop table reminders;",
    )
}

/// What quiet hours held becomes a note due now; the cadence's own rows go.
fn moved_held(tx: &Transaction, now: NaiveDateTime) -> rusqlite::Result<()> {
    if has_table(tx, "held")? {
        let held: Vec<String> = tx
            .prepare("select text from held order by id")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for text in held {
            let note = Watch {
                at: Some(now),
                text,
                ..Watch::default()
            };
            add(tx, &note)?;
        }
    }
    tx.execute_batch("drop table if exists cadence; drop table if exists held;")
}

/// Old mail watches become looks at Fastmail, primed afresh since their seen
/// keys named another source's items. Their calendar and money rows go.
fn moved_mail(tx: &Transaction, now: NaiveDateTime) -> rusqlite::Result<()> {
    if !has_table(tx, "watches")? {
        return Ok(());
    }
    let rows: Vec<(String, String, String)> = tx
        .prepare("select tests, text, share_with from watches where source = 'mail' order by id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (tests, text, share_with) in rows {
        // `from` is now a list of addresses, which only `has` can test.
        let tests = json::<Vec<Test>>(Some(tests))
            .unwrap_or_default()
            .into_iter()
            .map(|t| match t.field.as_str() {
                "from" => Test { op: Op::Has, ..t },
                _ => t,
            })
            .collect();
        let arguments = [
            ("query".to_owned(), Value::from("after:2d")),
            ("limit".to_owned(), Value::from(50)),
        ];
        add(
            tx,
            &Watch {
                at: Some(now),
                every: Some(Every::Minutes(Every::FLOOR)),
                turn: true,
                text,
                share_with: crate::book::names(&share_with),
                look: Some(Look {
                    server: "fastmail".into(),
                    tool: "search_email".into(),
                    arguments: arguments.into_iter().collect(),
                    items: Some("results".into()),
                    key: "id".into(),
                }),
                tests,
                fields: ["from", "subject", "preview", "receivedAt"]
                    .map(String::from)
                    .to_vec(),
                ..Watch::default()
            },
        )?;
    }
    tx.execute_batch("drop table watches;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn fresh(now: NaiveDateTime) -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        migrate(&mut db, now).unwrap();
        db
    }

    /// A book as the last release left it.
    fn old_book(setting: Option<&str>, day: &str) -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "create table cadence (key text primary key, value text not null);
             create table held (id integer primary key autoincrement, at text not null,
                text text not null);",
        )
        .unwrap();
        db.execute_batch(
            "create table reminders (
                id integer primary key autoincrement, at text not null, text text not null,
                repeat text, ask integer not null default 0,
                share_with text not null default '[]', answer text
             );
             insert into reminders (at, text, repeat, ask, share_with, answer) values
                ('2026-11-02 09:00:00', 'the bins', null, 0, '[]', null),
                ('2026-11-01 18:00:00', 'check the weather', 'weekly', 1, '[\"Kit\"]', 'rain');",
        )
        .unwrap();
        if let Some(s) = setting {
            db.execute("insert into cadence values ('setting', ?1)", [s])
                .unwrap();
        }
        db.execute("insert into cadence values ('day', ?1)", [day])
            .unwrap();
        db
    }

    fn test(field: &str, op: Op, value: &str) -> Test {
        Test {
            field: field.into(),
            op,
            value: value.into(),
        }
    }

    #[test]
    fn tests_read_dotted_paths_and_numbers_as_written() {
        let item = serde_json::json!({
            "from": [{"email": "Bank@Example.com"}],
            "repo": {"name": "ark"},
            "amount": 120.5,
            "price": "99.50",
        });
        assert!(test("from.0.email", Op::Is, "bank@example.com").passes(&item));
        assert!(test("from", Op::Has, "bank@").passes(&item));
        assert!(test("repo.name", Op::Is, "ARK").passes(&item));
        assert!(test("amount", Op::Over, "$100").passes(&item));
        assert!(test("price", Op::Under, "100").passes(&item));
        assert!(!test("amount", Op::Under, "100").passes(&item));
        assert!(!test("missing", Op::Has, "").passes(&item));
    }

    #[test]
    fn only_the_named_fields_are_kept_for_then() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate(&mut db, t("2026-10-26 08:00")).unwrap();
        let id = add(
            &db,
            &Watch {
                turn: true,
                text: "t".into(),
                fields: vec!["a.b".into(), "missing".into()],
                ..Watch::default()
            },
        )
        .unwrap();
        let w = get(&db, id).unwrap().unwrap();
        let item = serde_json::json!({"a": {"b": 1}, "secret": 2});
        assert_eq!(
            arrived(&db, &w, "k", &item, t("2026-10-26 08:00")).unwrap(),
            Arrival::Fired
        );
        assert_eq!(
            arrived(&db, &w, "k", &item, t("2026-10-26 08:00")).unwrap(),
            Arrival::Seen
        );
        let fired = fired(&db, true).unwrap();
        assert_eq!(fired[0].items, [r#"{"a.b":1}"#]);
        ran(&db, &fired[0].ids).unwrap();
        assert!(super::fired(&db, true).unwrap().is_empty());
    }

    #[test]
    fn seen_keys_go_after_sixty_days_and_with_their_watch() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate(&mut db, t("2026-10-26 08:00")).unwrap();
        let id = add(&db, &Watch::default()).unwrap();
        let w = get(&db, id).unwrap().unwrap();
        let item = serde_json::json!({});
        arrived(
            &db,
            &w,
            "old",
            &item,
            t("2026-10-26 08:00") - Duration::days(61),
        )
        .unwrap();
        arrived(&db, &w, "new", &item, t("2026-10-26 08:00")).unwrap();
        prune(&db, t("2026-10-26 08:00")).unwrap();
        assert!(!seen(&db, &w, "old").unwrap());
        assert!(seen(&db, &w, "new").unwrap());
        remove(&db, id).unwrap();
        assert!(!seen(&db, &w, "new").unwrap());
        assert!(fired(&db, false).unwrap().is_empty());
    }

    #[test]
    fn an_ask_on_a_look_or_a_push_must_name_its_fields() {
        let look = Look {
            server: "fastmail".into(),
            tool: "search_email".into(),
            arguments: Default::default(),
            items: Some("results".into()),
            key: "id".into(),
        };
        let push = crate::hook::Hook::made(crate::hook::Push {
            key: None,
            signature: None,
        })
        .ok();
        let ask = |look: Option<Look>, hook: Option<crate::hook::Hook>, fields: &[&str]| Watch {
            turn: true,
            look,
            hook,
            fields: fields.iter().map(|f| f.to_string()).collect(),
            ..Watch::default()
        };
        assert!(ask(Some(look.clone()), None, &[]).sees_nothing());
        assert!(ask(None, push.clone(), &[]).sees_nothing());
        assert!(!ask(Some(look.clone()), None, &["subject"]).sees_nothing());
        assert!(!ask(None, push.clone(), &["subject"]).sees_nothing());
        // A timed ask and a note see no item, so they name no fields.
        assert!(!ask(None, None, &[]).sees_nothing());
        let note = Watch {
            turn: false,
            ..ask(Some(look), push, &[])
        };
        assert!(!note.sees_nothing());
    }

    #[test]
    fn a_push_forgets_in_a_week_and_a_full_queue_turns_only_new_items_away() {
        let now = t("2026-10-26 08:00");
        let db = fresh(now);
        let id = add(
            &db,
            &Watch {
                hook: crate::hook::Hook::made(crate::hook::Push {
                    key: None,
                    signature: None,
                })
                .ok(),
                ..Watch::default()
            },
        )
        .unwrap();
        let w = get(&db, id).unwrap().unwrap();
        let item = serde_json::json!({});
        assert!(w.show().contains("on push"), "{}", w.show());
        assert!(!w.show().contains(&w.hook.as_ref().unwrap().token));
        pushed(&db, &w, "old", &item, now - Duration::days(8)).unwrap();
        for n in 1..MOST_WAITING {
            let arrival = pushed(&db, &w, &n.to_string(), &item, now).unwrap();
            assert_eq!(arrival, Arrival::Fired);
        }
        assert_eq!(pushed(&db, &w, "next", &item, now).unwrap(), Arrival::Full);
        assert_eq!(pushed(&db, &w, "1", &item, now).unwrap(), Arrival::Seen);
        assert!(!seen(&db, &w, "next").unwrap());
        prune(&db, now).unwrap();
        assert!(!seen(&db, &w, "old").unwrap());
        assert!(seen(&db, &w, "1").unwrap());
    }

    #[test]
    fn old_mail_watches_become_looks_and_the_others_go() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            r#"create table reminders (id integer primary key, at text not null,
                 text text not null, repeat text, ask integer not null default 0,
                 share_with text not null default '[]', answer text);
               create table watches (id integer primary key, source text not null,
                 tests text not null, text text not null, before integer not null default 0,
                 share_with text not null default '[]', primed integer not null default 0);
               create table watch_seen (watch integer not null, key text not null,
                 at text not null, primary key (watch, key));
               insert into watches (source, tests, text, primed, share_with) values
                 ('mail', '[{"field":"from","op":"is","value":"bank"}]', 'the bank', 1, '["Kit"]'),
                 ('money', '[]', 'spending', 1, '[]'),
                 ('calendar', '[]', 'events', 1, '[]');
               insert into watch_seen values (1, 'mail/1', '2026-10-01 08:00');"#,
        )
        .unwrap();
        migrate(&mut db, t("2026-10-26 08:00")).unwrap();
        let moved: Vec<Watch> = all(&db)
            .unwrap()
            .into_iter()
            .filter(|w| w.look.is_some())
            .collect();
        assert_eq!(moved.len(), 1);
        let w = &moved[0];
        assert_eq!((w.text.as_str(), w.primed), ("the bank", false));
        assert_eq!(w.share_with, ["Kit"]);
        assert_eq!(w.tests, [test("from", Op::Has, "bank")]);
        let look = w.look.as_ref().unwrap();
        assert_eq!(
            (look.server.as_str(), look.tool.as_str()),
            ("fastmail", "search_email")
        );
        assert!(!has_table(&db, "watches").unwrap());
        assert!(!seen(&db, w, "mail/1").unwrap());
    }

    #[test]
    fn intervals_read_as_they_are_written() {
        assert_eq!(Every::parse("daily"), Some(Every::Daily));
        assert_eq!(Every::parse("30m"), Some(Every::Minutes(30)));
        assert_eq!(Every::parse("3h"), Some(Every::Minutes(180)));
        assert_eq!(Every::Minutes(180).name(), "3h");
        assert_eq!(Every::Minutes(90).name(), "90m");
        assert_eq!(Every::parse("fortnightly"), None);
        assert_eq!(Every::parse(""), None);
        assert_eq!(Every::parse("é"), None);
    }

    #[test]
    fn a_reminder_fires_once_at_its_time() {
        let db = fresh(t("2026-10-08 07:00"));
        let w = Watch {
            at: Some(t("2026-10-08 09:00")),
            text: "the bins".into(),
            ..Watch::default()
        };
        add(&db, &w).unwrap();
        assert!(due(&db, t("2026-10-08 08:59")).unwrap().is_empty());
        let fired = due(&db, t("2026-10-08 09:00")).unwrap();
        assert_eq!(fired.len(), 1);
        // Undelivered, it is due again on the next tick.
        assert_eq!(due(&db, t("2026-10-08 09:01")).unwrap().len(), 1);
        done(&db, &fired[0], t("2026-10-08 09:01")).unwrap();
        assert!(due(&db, t("2026-10-08 09:02")).unwrap().is_empty());
        assert!(get(&db, fired[0].id).unwrap().is_none());
    }

    #[test]
    fn a_repeat_moves_past_a_missed_stretch_and_loses_its_answer() {
        let db = fresh(t("2026-10-01 07:00"));
        let id = add(
            &db,
            &Watch {
                at: Some(t("2026-10-01 18:00")),
                every: Some(Every::Weekly),
                turn: true,
                text: "weekly".into(),
                ..Watch::default()
            },
        )
        .unwrap();
        let now = t("2026-10-16 12:00");
        let w = due(&db, now).unwrap().remove(0);
        keep_answer(&db, &w, "all quiet").unwrap();
        assert_eq!(
            due(&db, now).unwrap()[0].answer.as_deref(),
            Some("all quiet")
        );
        done(&db, &w, now).unwrap();
        let w = get(&db, id).unwrap().unwrap();
        assert_eq!((w.at, w.answer), (Some(t("2026-10-22 18:00")), None));
    }

    #[test]
    fn a_new_book_starts_with_no_watches() {
        let db = fresh(t("2026-10-08 14:00"));
        assert!(all(&db).unwrap().is_empty());
        assert!(!has_table(&db, "cadence").unwrap());
    }

    /// A host's setting and day on 11-02, as its book held them.
    const OLD_SETTING: &str =
        r#"{"quiet_from":"23:00:00","quiet_until":"08:00:00","every":3,"most":2,"on":true}"#;
    const OLD_DAY: &str = r#"{"date":"2026-11-02","briefed":true,"looked":"2026-11-02T09:00:00",
        "sent":0,"first":null,"wrote":"2026-11-02T10:56:51.777045876"}"#;

    #[test]
    fn migration_keeps_reminders_and_makes_no_brief_or_check_ins() {
        let mut db = old_book(Some(OLD_SETTING), OLD_DAY);
        db.execute(
            "insert into held (at, text) values ('2026-11-02 02:00:00', 'The boiler alarm')",
            [],
        )
        .unwrap();
        let now = t("2026-11-02 11:00");
        migrate(&mut db, now).unwrap();
        migrate(&mut db, now).unwrap();
        for gone in ["reminders", "cadence", "held"] {
            assert!(!has_table(&db, gone).unwrap(), "{gone}");
        }
        let rows = all(&db).unwrap();
        let bins = rows.iter().find(|w| w.text == "the bins").unwrap();
        assert_eq!(
            (bins.at, bins.every, bins.turn),
            (Some(t("2026-11-02 09:00")), None, false)
        );
        let weather = rows.iter().find(|w| w.text == "check the weather").unwrap();
        assert_eq!(weather.every, Some(Every::Weekly));
        assert!(weather.turn);
        assert_eq!(weather.share_with, ["Kit"]);
        assert_eq!(weather.answer.as_deref(), Some("rain"));
        let held = rows.iter().find(|w| w.text == "The boiler alarm").unwrap();
        assert_eq!((held.at, held.turn, held.every), (Some(now), false, None));
        assert_eq!(rows.len(), 3, "the two reminders and the held note alone");
        // A book with no setting row ran on defaults that had the brief on;
        // it gets none either.
        let mut db = old_book(None, OLD_DAY);
        migrate(&mut db, now).unwrap();
        assert_eq!(all(&db).unwrap().len(), 2);
    }
}
