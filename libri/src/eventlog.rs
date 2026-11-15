//! The page stream's newest events, each numbered, so a page or app that
//! drops off (a phone asleep, a network blip, a page too slow to keep up)
//! gets back only what it missed, as T3 Code resumes its thread streams
//! (apps/server/src/ws.ts, ThreadStream.ts), instead of reading the whole
//! chat again and drawing it anew.
//!
//! Unlike T3's, the log is kept in memory, not in SQLite: T3's events are
//! its record and its projections are built from them, while Libri's record
//! is hippo and the turn record, and a restart (a deploy, a crash) changes
//! what a page shows by more than its events say (a cut-off turn sends no
//! end). So each start of Libri numbers afresh under a new boot mark, and a
//! client that names another boot's event reads the chat whole.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

/// The most events a client may be behind and still be caught up by
/// replay; past it, or past [`KEPT_BYTES`], it reads the chat whole. As
/// many as the live channel holds, so a client too slow for the channel is
/// always within reach of the log. T3 keeps 128, but each of its events
/// replaces a whole message; most of Libri's are a few streamed letters.
pub const KEPT: usize = 1024;
/// The most bytes of events kept for replay, T3's 1 MiB.
pub const KEPT_BYTES: usize = 1 << 20;

/// One event as it goes out: its number and its JSON text.
pub type Logged = (u64, Arc<str>);

pub struct EventLog {
    /// Names this run of Libri, so an event's id from before a restart is
    /// never taken for one of this run's.
    boot: String,
    kept: Mutex<Kept>,
    live: broadcast::Sender<Logged>,
}

#[derive(Default)]
struct Kept {
    /// The number of the newest event, 0 before the first.
    seq: u64,
    events: VecDeque<Logged>,
    bytes: usize,
}

/// What a client that names its last event gets.
#[derive(Debug, PartialEq)]
pub enum Missed {
    /// The events after it, oldest first; empty when it missed nothing.
    Replay(Vec<Logged>),
    /// It is too far behind, or named another run's event: it reads the
    /// chat whole. Carries the newest number, from which the live events go on.
    Snapshot(u64),
}

impl EventLog {
    pub fn new() -> Self {
        let boot = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        Self::with_boot(format!("{boot:x}"))
    }

    fn with_boot(boot: String) -> Self {
        Self {
            boot,
            kept: Mutex::default(),
            live: broadcast::channel(KEPT).0,
        }
    }

    /// Numbers an event, keeps it, and sends it to every client following.
    /// The number is given and the event sent under one lock, so clients
    /// get events in the order of their numbers.
    pub fn publish(&self, text: String) {
        let mut k = self.kept.lock().unwrap();
        k.seq += 1;
        let e: Logged = (k.seq, text.into());
        k.bytes += e.1.len();
        k.events.push_back(e.clone());
        while k.events.len() > KEPT || (k.bytes > KEPT_BYTES && k.events.len() > 1) {
            let (_, old) = k.events.pop_front().unwrap();
            k.bytes -= old.len();
        }
        // No client following is no error: the next reads the chat.
        let _ = self.live.send(e);
    }

    /// Follows the events from now on. Subscribe before calling
    /// [`Self::since`], and drop what both give by number.
    pub fn subscribe(&self) -> broadcast::Receiver<Logged> {
        self.live.subscribe()
    }

    /// What a client whose last event was number `after` lacks.
    pub fn since(&self, after: u64) -> Missed {
        let k = self.kept.lock().unwrap();
        if after > k.seq {
            return Missed::Snapshot(k.seq);
        }
        // The oldest event kept must come right after the client's last,
        // or before it: otherwise some it lacks are gone.
        let oldest = k.events.front().map_or(k.seq + 1, |e| e.0);
        if after + 1 < oldest {
            return Missed::Snapshot(k.seq);
        }
        Missed::Replay(k.events.iter().filter(|e| e.0 > after).cloned().collect())
    }

    /// The newest event's number.
    pub fn newest(&self) -> u64 {
        self.kept.lock().unwrap().seq
    }

    /// An event's id on the wire: the SSE `id` a client hands back.
    pub fn id(&self, seq: u64) -> String {
        format!("{}.{seq}", self.boot)
    }

    /// The number in an id this run gave; None for another run's or a
    /// malformed one.
    pub fn parse(&self, id: &str) -> Option<u64> {
        let (boot, seq) = id.rsplit_once('.')?;
        (boot == self.boot).then_some(())?;
        seq.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> EventLog {
        EventLog::with_boot("b1".into())
    }

    fn seqs(r: Missed) -> Vec<u64> {
        match r {
            Missed::Replay(es) => es.iter().map(|e| e.0).collect(),
            Missed::Snapshot(n) => panic!("snapshot at {n}"),
        }
    }

    #[test]
    fn a_client_gets_only_what_came_after_its_last_event() {
        let l = log();
        for i in 0..5 {
            l.publish(format!("{{\"n\":{i}}}"));
        }
        assert_eq!(seqs(l.since(2)), vec![3, 4, 5]);
        assert_eq!(seqs(l.since(5)), Vec::<u64>::new());
        assert_eq!(seqs(l.since(0)), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_client_too_far_behind_reads_the_chat_whole() {
        let l = log();
        for _ in 0..KEPT + 10 {
            l.publish("{}".into());
        }
        assert_eq!(l.since(5), Missed::Snapshot((KEPT + 10) as u64));
        // The oldest kept is 11: a client at 10 lacks nothing gone.
        assert_eq!(seqs(l.since(10)).len(), KEPT);
    }

    #[test]
    fn the_log_keeps_no_more_than_its_bytes() {
        let l = log();
        let big = "x".repeat(KEPT_BYTES / 4 + 1);
        for _ in 0..8 {
            l.publish(big.clone());
        }
        // Four would be over the cap: three stay.
        assert_eq!(seqs(l.since(5)), vec![6, 7, 8]);
        assert!(matches!(l.since(4), Missed::Snapshot(8)));
    }

    #[test]
    fn a_number_ahead_of_the_log_is_from_a_run_that_went_further() {
        let l = log();
        l.publish("{}".into());
        assert_eq!(l.since(9), Missed::Snapshot(1));
    }

    #[test]
    fn ids_name_their_run() {
        let l = log();
        assert_eq!(l.parse(&l.id(42)), Some(42));
        assert_eq!(l.parse("b0.42"), None);
        assert_eq!(l.parse("42"), None);
        assert_eq!(l.parse("b1.x"), None);
    }

    #[tokio::test]
    async fn live_events_come_in_number_order_after_a_subscription() {
        let l = log();
        l.publish("{}".into());
        let mut rx = l.subscribe();
        l.publish("{}".into());
        l.publish("{}".into());
        assert_eq!(rx.recv().await.unwrap().0, 2);
        assert_eq!(rx.recv().await.unwrap().0, 3);
    }
}
