use super::exchange::L1;
use matching_engine::{Qty, Side};

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Buy(Qty),
    Sell(Qty),
}

impl Action {
    pub fn side(&self) -> Side {
        match self {
            Action::Buy(_) => Side::Buy,
            Action::Sell(_) => Side::Sell,
        }
    }

    pub fn qty(&self) -> Qty {
        match self {
            Action::Buy(q) | Action::Sell(q) => *q,
        }
    }
}

/// Event-capture strategy with a three-state lifecycle:
///   FLAT → (entry signal) → HOLDING → (exit signal) → COOLDOWN → FLAT
///
/// Enters when L1 imbalance exceeds threshold (flow event detected).
/// Holds at least `min_hold` ticks, then exits when imbalance fades.
/// Observes a post-exit cooldown to avoid re-entering the same event.
pub struct Strategy {
    entry_threshold: f64,
    exit_threshold: f64,
    min_hold: u64,
    max_hold: u64,
    post_exit_cooldown: u64,
    entry_tick: u64,
    exit_tick: u64,
    has_exited: bool,
}

impl Strategy {
    pub fn new() -> Self {
        Self {
            entry_threshold: 0.35,
            exit_threshold: 0.05,
            min_hold: 25,
            max_hold: 160,
            post_exit_cooldown: 120,
            entry_tick: 0,
            exit_tick: 0,
            has_exited: false,
        }
    }

    pub fn decide(&mut self, l1: &L1, position: i64, current_tick: u64) -> Option<Action> {
        if !l1.valid() {
            return None;
        }

        let imbalance = l1.imbalance();

        // ── FLAT: look for entry ────────────────────────────────────
        if position == 0 {
            if self.has_exited && current_tick < self.exit_tick + self.post_exit_cooldown {
                return None;
            }
            if imbalance > self.entry_threshold {
                self.entry_tick = current_tick;
                self.has_exited = false;
                return Some(Action::Buy(1));
            }
            if imbalance < -self.entry_threshold {
                self.entry_tick = current_tick;
                self.has_exited = false;
                return Some(Action::Sell(1));
            }
            return None;
        }

        // ── HOLDING: look for exit ──────────────────────────────────
        let held = current_tick.saturating_sub(self.entry_tick);

        if held >= self.max_hold {
            self.exit_tick = current_tick;
            self.has_exited = true;
            return Some(flatten(position));
        }

        if held >= self.min_hold {
            let should_exit = (position > 0 && imbalance < self.exit_threshold)
                || (position < 0 && imbalance > -self.exit_threshold);
            if should_exit {
                self.exit_tick = current_tick;
                self.has_exited = true;
                return Some(flatten(position));
            }
        }

        None
    }
}

fn flatten(position: i64) -> Action {
    if position > 0 {
        Action::Sell(position as Qty)
    } else {
        Action::Buy((-position) as Qty)
    }
}
