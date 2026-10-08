use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct Input {
    held: BTreeSet<String>,
    touch: Option<(i32, i32)>,
    angle: Option<f32>,
    remainder: f32,
}
impl Input {
    pub fn button(&mut self, name: &str, down: bool) -> Option<Value> {
        if down == self.held.contains(name) {
            return None;
        }
        if down {
            self.held.insert(name.into());
        } else {
            self.held.remove(name);
        }
        Some(json!({"action":"button","name":name,"down":down}))
    }
    pub fn touch(&mut self, x: i32, y: i32, down: bool) -> Option<Value> {
        if !(0..800).contains(&x) || !(0..480).contains(&y) {
            return None;
        }
        if down && self.touch == Some((x, y)) {
            return None;
        }
        if !down && self.touch.is_none() {
            return None;
        }
        self.touch = down.then_some((x, y));
        Some(json!({"action":"touch","x":x,"y":y,"down":down}))
    }
    pub fn end_touch(&mut self) -> Option<Value> {
        let (x, y) = self.touch?;
        self.touch(x, y, false)
    }
    pub fn release(&mut self) -> Option<Value> {
        self.angle = None;
        if self.held.is_empty() && self.touch.is_none() {
            return None;
        }
        self.held.clear();
        self.touch = None;
        Some(json!({"action":"release"}))
    }
    pub fn jog_begin(&mut self, point: [f32; 2], radius: f32) -> Option<Value> {
        self.angle = Some(point[1].atan2(point[0]));
        self.remainder = 0.0;
        if point[0].hypot(point[1]) < radius * 0.73 {
            self.button("jog-touch", true)
        } else {
            None
        }
    }
    pub fn jog_move(&mut self, point: [f32; 2]) -> Option<Value> {
        let angle = point[1].atan2(point[0]);
        let previous = self.angle.replace(angle)?;
        let delta = (angle - previous + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        self.remainder += delta.to_degrees();
        let ticks = self.remainder as i32;
        self.remainder -= ticks as f32;
        (ticks != 0).then(|| json!({"action":"jog","ticks":ticks.clamp(-4096,4096)}))
    }
    pub fn jog_end(&mut self) -> Option<Value> {
        self.angle = None;
        self.button("jog-touch", false)
    }
}
pub fn lcd_point(rect: [f32; 4], point: [f32; 2]) -> Option<(i32, i32)> {
    let [x, y, w, h] = rect;
    let [px, py] = point;
    if ![x, y, w, h, px, py].iter().all(|v| v.is_finite())
        || w <= 0.0
        || h <= 0.0
        || px < x
        || py < y
        || px > x + w
        || py > y + h
    {
        return None;
    }
    Some((((px - x) / w * 800.0) as i32).clamp(0, 799))
        .zip(Some((((py - y) / h * 480.0) as i32).clamp(0, 479)))
}
