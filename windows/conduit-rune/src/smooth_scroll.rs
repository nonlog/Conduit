//! Smooth only notched-wheel steps. Trackpads keep their native precise deltas.
//! Rune's bounded glide owns frames and becomes completely idle at its destination.

use rune::{El, ScrollPos, WheelEvent};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Position {
    current: ScrollPos,
    target: Option<f32>,
    last_input: Option<Instant>,
    direction: f32,
}

impl Position {
    fn target_for(&mut self, event: WheelEvent, now: Instant) -> Option<f32> {
        if event.precise
            || event.mods.ctrl
            || event.mods.cmd
            || event.mods.shift
            || !event.dy.is_finite()
            || event.dy == 0.0
            || event.dx.abs() > event.dy.abs()
        {
            self.target = None;
            self.last_input = None;
            return None;
        }
        let max = self.current.max[1].max(0.0);
        if max == 0.0 {
            return None;
        }
        let direction = -event.dy.signum();
        let continuing = self
            .last_input
            .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_millis(350))
            && self.direction == direction;
        // A reversal starts from the presented offset, not the unconsumed old target.
        let from = if continuing {
            self.target.unwrap_or(self.current.offset[1])
        } else {
            self.current.offset[1]
        };
        let next = (from - event.dy).clamp(0.0, max);
        self.last_input = Some(now);
        self.direction = direction;
        self.target = Some(next);
        Some(next)
    }
}

#[derive(Clone, Default)]
pub struct SmoothScroll(Arc<Mutex<Position>>);

impl SmoothScroll {
    pub fn reset(&self) {
        if let Ok(mut state) = self.0.lock() {
            *state = Position::default();
        }
    }

    pub fn viewport<S: 'static>(&self, element: El, key: String, enabled: bool) -> El {
        let observe = self.0.clone();
        let input = self.0.clone();
        element
            .on_scroll_offset(move |pos: ScrollPos| {
                if let Ok(mut state) = observe.lock() {
                    state.current = pos;
                    if let Some(target) = state.target {
                        let target = target.clamp(0.0, pos.max[1].max(0.0));
                        state.target = ((target - pos.offset[1]).abs() > 0.5).then_some(target);
                    }
                }
            })
            .on_wheel(move |_: &mut S, event, tx| {
                if !enabled {
                    return false;
                }
                let Ok(mut state) = input.lock() else {
                    return false;
                };
                let Some(target) = state.target_for(event, Instant::now()) else {
                    return false;
                };
                // The runtime respects reduced motion and clamps after layout/content changes.
                tx.scroll_to(key.clone(), 0.0, target);
                tx.live_only();
                true
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Cx, Env, Runtime, ScrollSpec, col, el};

    struct State {
        scroll: SmoothScroll,
        enabled: bool,
    }
    fn view(s: &State, _: &mut Cx<State>) -> El {
        s.scroll.viewport::<State>(
            col()
                .key("viewport")
                .size(300.0, 200.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(el().key("content").w_full().h(1200.0)),
            "viewport".into(),
            s.enabled,
        )
    }
    fn rig(enabled: bool) -> Runtime<State> {
        let mut rt = Runtime::new(
            State {
                scroll: SmoothScroll::default(),
                enabled,
            },
            view,
            (300.0, 200.0),
            Env::default(),
        );
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);
        rt
    }
    fn offset(rt: &Runtime<State>) -> f32 {
        rt.state.scroll.0.lock().unwrap().current.offset[1]
    }

    #[test]
    fn notched_wheel_moves_over_multiple_frames_then_settles() {
        let mut rt = rig(true);
        rt.wheel_event(WheelEvent::new(0.0, -120.0).at(100.0, 100.0));
        rt.frame(1.0 / 60.0);
        let first = offset(&rt);
        assert!(first > 0.0 && first < 120.0, "first offset {first}");
        rt.frame(1.0 / 60.0);
        assert!(offset(&rt) > first && offset(&rt) < 120.0);
        for _ in 0..90 {
            rt.frame(1.0 / 60.0);
        }
        assert!((offset(&rt) - 120.0).abs() < 0.2);
        let end = offset(&rt);
        for _ in 0..5 {
            rt.frame(1.0 / 60.0);
            assert_eq!(offset(&rt), end);
        }
    }

    #[test]
    fn precise_input_and_disabled_smoothing_are_not_double_smoothed() {
        for (enabled, precise) in [(true, true), (false, false)] {
            let mut rt = rig(enabled);
            rt.wheel_event(
                WheelEvent::new(0.0, -75.0)
                    .at(100.0, 100.0)
                    .precise(precise),
            );
            rt.frame(1.0 / 60.0);
            assert!((offset(&rt) - 75.0).abs() < 0.1);
        }
    }

    #[test]
    fn reversal_uses_presented_position_and_targets_are_bounded() {
        let now = Instant::now();
        let mut position = ScrollPos::default();
        position.offset = [0.0, 200.0];
        position.max = [0.0, 1000.0];
        let mut p = Position {
            current: position,
            ..Default::default()
        };
        assert_eq!(p.target_for(WheelEvent::new(0.0, -120.0), now), Some(320.0));
        assert_eq!(p.target_for(WheelEvent::new(0.0, -120.0), now), Some(440.0));
        assert_eq!(p.target_for(WheelEvent::new(0.0, 120.0), now), Some(80.0));
        assert_eq!(p.target_for(WheelEvent::new(0.0, 5000.0), now), Some(0.0));
        assert_eq!(
            p.target_for(WheelEvent::new(0.0, -5000.0), now),
            Some(1000.0)
        );
    }
}
