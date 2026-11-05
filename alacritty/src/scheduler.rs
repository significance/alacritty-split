//! Scheduler for emitting events at a specific time in the future.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::event::Event;
use crate::pane::PaneId;

/// ID uniquely identifying a timer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TimerId {
    topic: Topic,
    window_id: WindowId,
    pane_id: Option<PaneId>,
}

impl TimerId {
    pub fn new(topic: Topic, window_id: WindowId) -> Self {
        Self { topic, window_id, pane_id: None }
    }

    /// Leave frame timers window-scoped; all terminal timers carry a pane identity.
    pub fn with_pane(mut self, pane_id: Option<PaneId>) -> Self {
        self.pane_id = pane_id;
        self
    }
}

/// Available timer topics.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Topic {
    SelectionScrolling,
    DelayedSearch,
    BlinkCursor,
    BlinkTimeout,
    Frame,
}

/// Event scheduled to be emitted at a specific time.
pub struct Timer {
    pub deadline: Instant,
    pub event: Event,
    pub id: TimerId,

    interval: Option<Duration>,
}

/// Scheduler tracking all pending timers.
pub struct Scheduler {
    timers: VecDeque<Timer>,
    event_proxy: EventLoopProxy<Event>,
}

impl Scheduler {
    pub fn new(event_proxy: EventLoopProxy<Event>) -> Self {
        Self { timers: VecDeque::new(), event_proxy }
    }

    /// Process all pending timers.
    ///
    /// If there are still timers pending after all ready events have been processed, the closest
    /// pending deadline will be returned.
    pub fn update(&mut self) -> Option<Instant> {
        let now = Instant::now();

        while !self.timers.is_empty() && self.timers[0].deadline <= now {
            if let Some(timer) = self.timers.pop_front() {
                // Automatically repeat the event.
                if let Some(interval) = timer.interval {
                    self.schedule(timer.event.clone(), interval, true, timer.id);
                }

                let _ = self.event_proxy.send_event(timer.event);
            }
        }

        self.timers.front().map(|timer| timer.deadline)
    }

    /// Schedule a new event.
    pub fn schedule(&mut self, event: Event, interval: Duration, repeat: bool, timer_id: TimerId) {
        let deadline = Instant::now() + interval;

        // Get insert position in the schedule.
        let index = self
            .timers
            .iter()
            .position(|timer| timer.deadline > deadline)
            .unwrap_or(self.timers.len());

        // Set the automatic event repeat rate.
        let interval = if repeat { Some(interval) } else { None };

        self.timers.insert(index, Timer { interval, deadline, event, id: timer_id });
    }

    /// Cancel a scheduled event.
    pub fn unschedule(&mut self, id: TimerId) -> Option<Timer> {
        let index = self.timers.iter().position(|timer| timer.id == id)?;
        self.timers.remove(index)
    }

    /// Check if a timer is already scheduled.
    pub fn scheduled(&mut self, id: TimerId) -> bool {
        self.timers.iter().any(|timer| timer.id == id)
    }

    /// Remove all timers for a closed terminal without affecting other panes or frames.
    pub fn unschedule_pane(&mut self, window_id: WindowId, pane_id: PaneId) {
        remove_pane_timers(&mut self.timers, window_id, pane_id);
    }

    /// Remove all timers scheduled for a window.
    ///
    /// This must be called when a window is removed to ensure that timers on intervals do not
    /// stick around forever and cause a memory leak.
    pub fn unschedule_window(&mut self, window_id: WindowId) {
        self.timers.retain(|timer| timer.id.window_id != window_id);
    }
}

fn remove_pane_timers(timers: &mut VecDeque<Timer>, window_id: WindowId, pane_id: PaneId) {
    timers.retain(|timer| timer.id.window_id != window_id || timer.id.pane_id != Some(pane_id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_pane_preserves_other_panes_and_window_frames() {
        let window = WindowId::dummy();
        let other_window = WindowId::from(42);
        let ids = [
            TimerId::new(Topic::BlinkCursor, window).with_pane(Some(PaneId(0))),
            TimerId::new(Topic::BlinkTimeout, window).with_pane(Some(PaneId(0))),
            TimerId::new(Topic::DelayedSearch, window).with_pane(Some(PaneId(1))),
            TimerId::new(Topic::Frame, window),
            TimerId::new(Topic::BlinkCursor, other_window).with_pane(Some(PaneId(0))),
        ];
        let mut timers = ids
            .into_iter()
            .map(|id| Timer {
                deadline: Instant::now(),
                event: Event::new(crate::event::EventType::BlinkCursor, id.window_id)
                    .with_pane(id.pane_id),
                id,
                interval: Some(Duration::from_secs(1)),
            })
            .collect();
        remove_pane_timers(&mut timers, window, PaneId(0));
        assert_eq!(timers.iter().map(|timer| timer.id).collect::<Vec<_>>(), ids[2..]);
        remove_pane_timers(&mut timers, window, PaneId(0));
        assert_eq!(timers.len(), 3);
    }

    #[test]
    fn pane_timers_do_not_collide_with_each_other_or_window_timers() {
        let window = WindowId::dummy();
        for topic in [
            Topic::BlinkCursor,
            Topic::BlinkTimeout,
            Topic::DelayedSearch,
            Topic::SelectionScrolling,
        ] {
            let window_timer = TimerId::new(topic, window);
            let first = window_timer.with_pane(Some(PaneId(0)));
            let second = window_timer.with_pane(Some(PaneId(1)));
            assert_ne!(first, second);
            assert_ne!(first, window_timer);
            assert_eq!(first, window_timer.with_pane(Some(PaneId(0))));
        }
    }
}
