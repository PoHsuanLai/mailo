//! The Space editor, as data: a draft beside the Space it started from.
//!
//! Every change the sheet offers is a method here, so the rules (three dots at most, one at
//! least, where an arrow key moves a dot) are tested without a window. The window paints the
//! draft live and keeps [`Draft::saved`] for Esc.

use super::{PRESETS, Space};
use crate::palette::Dot;

/// The most dots a Space holds. The gradient reads as a gradient up to three.
pub const MOST_DOTS: usize = 3;

/// Degrees one arrow press turns a dot's hue.
const HUE_STEP: f32 = 1.0;
/// How much of the frame's chroma one arrow press adds or takes.
const CHROMA_STEP: f32 = 0.01;
/// How far along the wheel a new dot starts from the last one.
const NEW_DOT_TURN: f32 = 48.0;

/// Which way an arrow key moves a dot on the hue × chroma field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nudge {
    /// Hue down.
    Left,
    /// Hue up.
    Right,
    /// More chroma: the field's top is the most colour.
    Up,
    /// Less chroma.
    Down,
}

impl Nudge {
    /// The nudge a DOM key name asks for, or `None` for any other key.
    pub fn of_key(key: &str) -> Option<Nudge> {
        match key {
            "ArrowLeft" => Some(Nudge::Left),
            "ArrowRight" => Some(Nudge::Right),
            "ArrowUp" => Some(Nudge::Up),
            "ArrowDown" => Some(Nudge::Down),
            _ => None,
        }
    }
}

/// How far one press goes: one step, or ten with Shift held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stride {
    /// 1° of hue or 0.01 of chroma.
    One,
    /// Ten of those.
    Ten,
}

impl Stride {
    fn times(self) -> f32 {
        match self {
            Stride::One => 1.0,
            Stride::Ten => 10.0,
        }
    }
}

/// Why a change to the stops was not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Already [`MOST_DOTS`].
    Full,
    /// The last dot: a Space always has one.
    Last,
    /// No dot at that index.
    Missing,
}

/// A Space being edited, and the Space it was when the sheet opened.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// Which Space, by index into `Spaces::spaces`.
    pub index: usize,
    /// The Space as it was stored. Esc puts this back exactly.
    pub saved: Space,
    /// The Space as it looks now, painted live.
    pub space: Space,
    /// The dot the stops list and the arrow keys act on.
    pub active: usize,
}

impl Draft {
    /// Start editing `space`, the one at `index`.
    pub fn open(index: usize, space: Space) -> Self {
        Self {
            index,
            saved: space.clone(),
            space,
            active: 0,
        }
    }

    /// Move dot `dot` one stride in `way`. Hue wraps around the wheel; chroma stops at 0 and 1.
    pub fn nudge(&mut self, dot: usize, way: Nudge, stride: Stride) {
        let Some(target) = self.space.dots.get_mut(dot) else {
            return;
        };
        let times = stride.times();
        match way {
            Nudge::Left => target.hue = (target.hue - HUE_STEP * times).rem_euclid(360.0),
            Nudge::Right => target.hue = (target.hue + HUE_STEP * times).rem_euclid(360.0),
            Nudge::Up => target.chroma = (target.chroma + CHROMA_STEP * times).clamp(0.0, 1.0),
            Nudge::Down => target.chroma = (target.chroma - CHROMA_STEP * times).clamp(0.0, 1.0),
        }
        self.active = dot;
    }

    /// Put dot `dot` where the pointer is: `x` across the field is hue, `y` down it is less
    /// chroma. Both are fractions of the field, clamped to it.
    pub fn place(&mut self, dot: usize, x: f32, y: f32) {
        let Some(target) = self.space.dots.get_mut(dot) else {
            return;
        };
        // 359, not 360: the loader reads 360 as 0, and a Space saved at the field's right edge
        // would come back at its left.
        target.hue = (x.clamp(0.0, 1.0) * 360.0).min(359.0);
        target.chroma = 1.0 - y.clamp(0.0, 1.0);
        self.active = dot;
    }

    /// Add a dot after the last, turned along the wheel. Refused at [`MOST_DOTS`].
    pub fn add(&mut self) -> Result<(), Refused> {
        if self.space.dots.len() >= MOST_DOTS {
            return Err(Refused::Full);
        }
        let last = self.space.dots.last().copied().unwrap_or_default();
        self.space.dots.push(Dot {
            hue: (last.hue + NEW_DOT_TURN).rem_euclid(360.0),
            chroma: last.chroma,
        });
        self.active = self.space.dots.len() - 1;
        Ok(())
    }

    /// Remove dot `dot`. Refused for the last one left.
    pub fn remove(&mut self, dot: usize) -> Result<(), Refused> {
        if dot >= self.space.dots.len() {
            return Err(Refused::Missing);
        }
        if self.space.dots.len() <= 1 {
            return Err(Refused::Last);
        }
        self.space.dots.remove(dot);
        self.active = 0;
        Ok(())
    }

    /// Replace the dots with preset `index`. Anything past the list is ignored.
    pub fn preset(&mut self, index: usize) {
        if let Some(dots) = PRESETS.get(index) {
            self.space.dots = dots.to_vec();
            self.active = 0;
        }
    }

    /// The Space as the sheet found it. What Esc restores.
    pub fn reverted(&self) -> Space {
        self.saved.clone()
    }
}

#[cfg(test)]
mod tests;
