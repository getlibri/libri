//! The time the daemon goes by. The daemon runs on the system's; a test or
//! the simulation sets one of its own and moves it on by hand, so what is
//! meant to happen at a time is checked at that time instead of waited for,
//! and the same steps give the same result on every run.
//!
//! Only the local wall time is here: it is what watches and turns are
//! scheduled by. Timeouts on child processes measure real waits on real
//! processes and keep `Instant`; the simulation does not run them.

use chrono::{Local, NaiveDateTime};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct Clock(Option<Arc<Mutex<NaiveDateTime>>>);

impl Clock {
    /// The system's clock.
    pub fn system() -> Clock {
        Clock(None)
    }

    /// A clock that reads `at` until it is moved on. Clones share it. Only
    /// tests and the simulation move time by hand.
    #[cfg(test)]
    pub fn fixed(at: NaiveDateTime) -> Clock {
        Clock(Some(Arc::new(Mutex::new(at))))
    }

    /// The local time now, without a zone, as the book keeps it.
    pub fn local(&self) -> NaiveDateTime {
        match &self.0 {
            None => Local::now().naive_local(),
            Some(at) => *at.lock().unwrap(),
        }
    }

    /// Moves a fixed clock on by `by`. The system's clock moves on its own,
    /// so on it this is a mistake, and a test that makes it fails.
    #[cfg(test)]
    pub fn advance(&self, by: chrono::TimeDelta) {
        let at = self
            .0
            .as_ref()
            .expect("only a fixed clock is moved by hand");
        *at.lock().unwrap() += by;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn at(h: u32, m: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 11, 3)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap()
    }

    #[test]
    fn a_fixed_clock_moves_only_when_moved_and_clones_share_it() {
        let clock = Clock::fixed(at(8, 59));
        let shared = clock.clone();
        assert_eq!(clock.local(), at(8, 59));
        shared.advance(TimeDelta::minutes(1));
        assert_eq!(clock.local(), at(9, 0));
    }

    #[test]
    fn the_system_clock_reads_the_time_now() {
        let before = Local::now().naive_local();
        let read = Clock::system().local();
        assert!(read >= before && read <= Local::now().naive_local());
    }

    #[test]
    #[should_panic(expected = "only a fixed clock")]
    fn the_system_clock_is_not_moved_by_hand() {
        Clock::system().advance(TimeDelta::seconds(1));
    }
}
