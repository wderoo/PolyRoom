//! Overall progress of the background work that follows an import (generating previews).

use std::collections::{HashSet, VecDeque};

use polyroom_catalog::ModelId;

/// What the status bar shows while background work runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub text: String,
    /// `None` when the amount of work isn't known yet (e.g. while scanning folders).
    pub fraction: Option<f32>,
}

/// Counts how many of the models queued by imports have their preview, and hands out the
/// not-yet-requested ones a few at a time so that cells the user is looking at aren't stuck
/// behind a long queue.
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    total: usize,
    outstanding: HashSet<ModelId>,
    backlog: VecDeque<ModelId>,
}

impl Tracker {
    /// Adds models to the work, ignoring ones already tracked.
    pub fn add(&mut self, ids: impl IntoIterator<Item = ModelId>) {
        for id in ids {
            if self.outstanding.insert(id) {
                self.total = self.total.saturating_add(1);
                self.backlog.push_back(id);
            }
        }
    }

    /// Counts `id` as work already queued elsewhere (e.g. a visible cell asked for it).
    pub fn track(&mut self, id: ModelId) {
        if self.outstanding.insert(id) {
            self.total = self.total.saturating_add(1);
        }
    }

    /// Marks `id` as done (its preview arrived or failed). Resets once everything is done.
    pub fn finish(&mut self, id: ModelId) {
        self.outstanding.remove(&id);
        if self.outstanding.is_empty() {
            self.total = 0;
            self.backlog.clear();
        }
    }

    /// The next model that still needs a preview requested.
    pub fn next(&mut self) -> Option<ModelId> {
        while let Some(id) = self.backlog.pop_front() {
            if self.outstanding.contains(&id) {
                return Some(id);
            }
        }
        None
    }

    pub fn is_active(&self) -> bool {
        self.total > 0
    }

    pub fn done(&self) -> usize {
        self.total.saturating_sub(self.outstanding.len())
    }

    pub fn progress(&self) -> Option<Progress> {
        self.is_active().then(|| Progress {
            text: format!("Generating previews {} / {}", self.done(), self.total),
            fraction: Some(self.done() as f32 / self.total.max(1) as f32),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_done_over_total_and_resets() {
        let mut t = Tracker::default();
        assert_eq!(t.progress(), None);
        t.add([1, 2, 3, 4]);
        t.add([4, 5]); // already tracked ids don't count twice
        assert_eq!((t.done(), t.total), (0, 5));
        t.finish(1);
        t.finish(1); // finishing twice is harmless
        t.finish(2);
        let p = t.progress().unwrap();
        assert_eq!(p.text, "Generating previews 2 / 5");
        assert_eq!(p.fraction, Some(0.4));
        for id in [3, 4, 5] {
            t.finish(id);
        }
        assert_eq!(t.progress(), None);
        assert_eq!(t.total, 0);
    }

    #[test]
    fn next_skips_models_that_finished_early() {
        let mut t = Tracker::default();
        t.add([1, 2, 3]);
        t.finish(1); // e.g. a visible cell got its thumbnail first
        assert_eq!(t.next(), Some(2));
        assert_eq!(t.next(), Some(3));
        assert_eq!(t.next(), None);
    }

    #[test]
    fn tracking_a_queued_model_counts_it_without_backlog() {
        let mut t = Tracker::default();
        t.track(7);
        t.track(7);
        assert_eq!((t.done(), t.total), (0, 1));
        assert_eq!(t.next(), None);
        t.finish(7);
        assert!(!t.is_active());
    }

    #[test]
    fn imports_add_to_the_same_total() {
        let mut t = Tracker::default();
        t.add([1, 2]);
        t.finish(1);
        t.add([3, 4]);
        assert_eq!((t.done(), t.total), (1, 4));
    }
}
