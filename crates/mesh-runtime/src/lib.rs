//! Pure deterministic diagnostic runtime, with bounded event replay.
use mesh_types::{Error, Event, Snapshot, MAX_COUNTER, MAX_EVENTS};
use std::collections::VecDeque;

pub struct Runtime {
    id: u64,
    sequence: u64,
    last_request: u64,
    events: VecDeque<Event>,
}
impl Runtime {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            sequence: 0,
            last_request: 0,
            events: VecDeque::new(),
        }
    }
    pub fn subscribe(&self, cursor: u64) -> Snapshot {
        let oldest = self
            .events
            .front()
            .map_or(self.sequence + 1, |e| e.sequence);
        let reset = cursor > self.sequence || cursor.saturating_add(1) < oldest;
        Snapshot {
            runtime_id: self.id,
            cursor: self.sequence,
            probe_count: self.sequence,
            cursor_reset: reset,
            events: if reset {
                vec![]
            } else {
                self.events
                    .iter()
                    .filter(|e| e.sequence > cursor)
                    .cloned()
                    .collect()
            },
        }
    }
    pub fn verify_bridge(&mut self, request_id: u64) -> Result<Snapshot, Error> {
        if request_id == 0 || request_id > MAX_COUNTER {
            return Err(Error::InvalidArgument);
        }
        let old_cursor = self.sequence;
        if request_id <= self.last_request {
            if self.events.iter().any(|e| e.request_id == request_id) {
                return Ok(self.subscribe(old_cursor));
            }
            return Err(Error::StaleRequest);
        }
        if self.sequence == MAX_COUNTER {
            return Err(Error::ResourcePressure);
        }
        self.sequence += 1;
        self.last_request = request_id;
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            sequence: self.sequence,
            request_id,
        });
        Ok(self.subscribe(old_cursor))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rec_replay_atomic_snapshot_and_idempotency() {
        let mut r = Runtime::new(42);
        let a = r.verify_bridge(100).unwrap();
        assert_eq!(a.cursor, 1);
        assert_eq!(a.events[0].sequence, a.cursor);
        assert_eq!(r.verify_bridge(100).unwrap().cursor, 1);
        assert_eq!(r.subscribe(0), a);
        assert!(r.subscribe(1).events.is_empty());
        assert!(r.subscribe(2).cursor_reset);
    }
    #[test]
    fn rec_gap_resets_without_guessing_and_memory_is_bounded() {
        let mut r = Runtime::new(1);
        for id in 1..=100_000 {
            r.verify_bridge(id).unwrap();
        }
        assert!(r.subscribe(0).cursor_reset);
        assert!(r.subscribe(0).events.is_empty());
        assert_eq!(r.subscribe(99_936).events.len(), 64);
        assert_eq!(r.events.len(), 64);
        assert_eq!(r.verify_bridge(1), Err(Error::StaleRequest));
    }
    #[test]
    fn deterministic_history() {
        let mut a = Runtime::new(1);
        let mut b = Runtime::new(1);
        for id in 1..100 {
            assert_eq!(a.verify_bridge(id), b.verify_bridge(id));
        }
    }
}

pub mod durable;

pub mod link;
pub mod reception;
