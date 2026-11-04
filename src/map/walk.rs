//! Walking a route: one step at a time, the next sent only once the game reports the room
//! the route expected (user ruling 2026-11-01). Any other room, a refused move, no report
//! within the time allowed, a disconnect: the walk stops and says why. Pure: the caller
//! sends what it returns and says what it says.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::model::RoomId;
use super::path::Step;

/// How long a step may wait for the game's report of the room it leads to.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Walk {
    pub to: RoomId,
    pub steps: Vec<Step>,
    /// The step in flight (sent, waiting for its room).
    pub next: usize,
    /// The room that step leaves.
    pub from: RoomId,
    pub deadline: Instant,
}

/// What a walk asks of its caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalkFx {
    /// Send this command to the game.
    Send(String),
    /// Tell the user.
    Say(String),
}

/// A walk as a hot reload keeps it (its deadline as the time left).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct KeptWalk {
    pub to: RoomId,
    pub steps: Vec<(String, RoomId)>,
    pub next: usize,
    pub from: RoomId,
    pub remaining_ms: u64,
}

#[derive(Default, Debug, Clone)]
pub struct Walker {
    pub walk: Option<Walk>,
    /// Why the last walk ended, for the map's card and bar.
    pub last_stop: Option<String>,
}

/// A room's name and id as the walk's messages show it: "Palatine Steps (#4134)".
pub type Namer<'a> = &'a dyn Fn(&str) -> String;

impl Walker {
    pub fn walking(&self) -> bool {
        self.walk.is_some()
    }

    /// The walk going on, for a hot reload's state file.
    pub fn keep(&self, now: Instant) -> Option<KeptWalk> {
        let w = self.walk.as_ref()?;
        Some(KeptWalk {
            to: w.to.clone(),
            steps: w.steps.iter().map(|s| (s.cmd.clone(), s.to.clone())).collect(),
            next: w.next,
            from: w.from.clone(),
            remaining_ms: w.deadline.saturating_duration_since(now).as_millis() as u64,
        })
    }

    /// A walk kept across a hot reload: it goes on where it was (the step in flight's
    /// room is still to come, on the same connection).
    pub fn restore(&mut self, k: KeptWalk, now: Instant) {
        if k.next >= k.steps.len() {
            return;
        }
        self.walk = Some(Walk {
            to: k.to,
            steps: k.steps.into_iter().map(|(cmd, to)| Step { cmd, to }).collect(),
            next: k.next,
            from: k.from,
            deadline: now + Duration::from_millis(k.remaining_ms),
        });
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.walk.as_ref().map(|w| w.deadline)
    }

    /// Start walking `steps` from `here` to `to` (any walk already going is replaced).
    pub fn start(&mut self, here: &str, to: RoomId, steps: Vec<Step>, name: Namer, now: Instant) -> Vec<WalkFx> {
        self.walk = None;
        if steps.is_empty() {
            return vec![WalkFx::Say(format!("You are already in {}.", name(&to)))];
        }
        let route: Vec<&str> = steps.iter().map(|s| s.cmd.as_str()).collect();
        let n = steps.len();
        let say = format!("Walking to {}: {} ({} step{}). Esc or /map stop stops.", name(&to), route.join(", "), n, if n == 1 { "" } else { "s" });
        let first = steps[0].cmd.clone();
        self.last_stop = None;
        self.walk = Some(Walk { to, steps, next: 0, from: here.to_string(), deadline: now + STEP_TIMEOUT });
        vec![WalkFx::Say(say), WalkFx::Send(first)]
    }

    /// The game reported the room the user is now in.
    pub fn on_room(&mut self, id: &str, name: Namer, now: Instant) -> Vec<WalkFx> {
        let Some(walk) = self.walk.as_mut() else { return Vec::new() };
        let step = walk.steps[walk.next].clone();
        if id == step.to {
            walk.next += 1;
            if walk.next == walk.steps.len() {
                let to = walk.to.clone();
                self.walk = None;
                return vec![WalkFx::Say(format!("Arrived at {}.", name(&to)))];
            }
            walk.from = id.to_string();
            walk.deadline = now + STEP_TIMEOUT;
            return vec![WalkFx::Send(walk.steps[walk.next].cmd.clone())];
        }
        if id == walk.from {
            // The room the step leaves, reported again (a `look`): still waiting.
            return Vec::new();
        }
        self.stop_with(format!("Walk stopped: expected {} after '{}'; you are in {}.", name(&step.to), step.cmd, name(id)))
    }

    /// The game refused the step in flight.
    pub fn on_wrong_dir(&mut self, dir: &str) -> Vec<WalkFx> {
        let Some(walk) = &self.walk else { return Vec::new() };
        let cmd = walk.steps[walk.next].cmd.clone();
        let what = if dir.is_empty() { cmd } else { dir.to_string() };
        self.stop_with(format!("Walk stopped: the game refused '{what}'."))
    }

    /// Time passes: a step with no report in time ends the walk (never sent again - it may
    /// have worked, and a second send would move twice).
    pub fn tick(&mut self, now: Instant) -> Vec<WalkFx> {
        let Some(walk) = &self.walk else { return Vec::new() };
        if now < walk.deadline {
            return Vec::new();
        }
        let cmd = walk.steps[walk.next].cmd.clone();
        self.stop_with(format!("Walk stopped: no room report {} s after '{cmd}'.", STEP_TIMEOUT.as_secs()))
    }

    /// End the walk now, saying `why` (nothing when there was no walk).
    pub fn stop(&mut self, why: &str) -> Vec<WalkFx> {
        if self.walk.is_none() {
            return Vec::new();
        }
        self.stop_with(why.to_string())
    }

    fn stop_with(&mut self, say: String) -> Vec<WalkFx> {
        self.walk = None;
        self.last_stop = Some(say.clone());
        vec![WalkFx::Say(say)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(list: &[(&str, &str)]) -> Vec<Step> {
        list.iter().map(|(c, t)| Step { cmd: c.to_string(), to: t.to_string() }).collect()
    }

    fn name(id: &str) -> String {
        format!("Room {id} (#{id})")
    }

    #[test]
    fn a_step_goes_out_only_when_the_last_one_arrived() {
        let now = Instant::now();
        let mut w = Walker::default();
        let fx = w.start("F", "M".into(), steps(&[("e", "G"), ("s", "K"), ("e", "M")]), &name, now);
        assert_eq!(fx, [WalkFx::Say("Walking to Room M (#M): e, s, e (3 steps). Esc or /map stop stops.".into()), WalkFx::Send("e".into())]);
        assert_eq!(w.on_room("F", &name, now), [], "the room it left, again: still waiting");
        assert_eq!(w.on_room("G", &name, now), [WalkFx::Send("s".into())]);
        assert_eq!(w.on_room("K", &name, now), [WalkFx::Send("e".into())]);
        assert_eq!(w.on_room("M", &name, now), [WalkFx::Say("Arrived at Room M (#M).".into())]);
        assert!(!w.walking());
    }

    #[test]
    fn anything_unexpected_stops_it_and_says_where() {
        let now = Instant::now();
        let mut w = Walker::default();
        w.start("F", "M".into(), steps(&[("e", "G"), ("s", "K")]), &name, now);
        assert_eq!(w.on_room("X", &name, now), [WalkFx::Say("Walk stopped: expected Room G (#G) after 'e'; you are in Room X (#X).".into())]);
        assert!(!w.walking());
        assert!(w.last_stop.as_deref().unwrap().contains("you are in Room X"));

        w.start("F", "M".into(), steps(&[("e", "G")]), &name, now);
        assert_eq!(w.on_wrong_dir(""), [WalkFx::Say("Walk stopped: the game refused 'e'.".into())]);

        w.start("F", "M".into(), steps(&[("e", "G")]), &name, now);
        assert_eq!(w.tick(now + Duration::from_secs(5)), []);
        assert_eq!(w.tick(now + STEP_TIMEOUT), [WalkFx::Say("Walk stopped: no room report 10 s after 'e'.".into())]);

        w.start("F", "M".into(), steps(&[("e", "G")]), &name, now);
        assert_eq!(w.stop("Walk stopped."), [WalkFx::Say("Walk stopped.".into())]);
        assert_eq!(w.stop("Walk stopped."), [], "no walk: nothing to say");
    }

    #[test]
    fn a_walk_kept_across_a_reload_goes_on() {
        let now = Instant::now();
        let mut w = Walker::default();
        w.start("F", "M".into(), steps(&[("e", "G"), ("s", "K")]), &name, now);
        w.on_room("G", &name, now + Duration::from_secs(2));
        let kept = w.keep(now + Duration::from_secs(3)).unwrap();
        assert_eq!((kept.next, kept.from.as_str(), kept.remaining_ms), (1, "G", 9000));
        let json = serde_json::to_string(&kept).unwrap();
        let mut after = Walker::default();
        after.restore(serde_json::from_str(&json).unwrap(), now);
        assert_eq!(after.on_room("K", &name, now), [WalkFx::Say("Arrived at Room M (#M).".into())]);
    }

    #[test]
    fn already_there() {
        let mut w = Walker::default();
        assert_eq!(w.start("M", "M".into(), Vec::new(), &name, Instant::now()), [WalkFx::Say("You are already in Room M (#M).".into())]);
        assert!(!w.walking());
    }
}
