use crate::qmp::Qmp;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;
pub fn key(name: &str) -> Result<&'static str> {
    Ok(match name {
        "play" => "spc",
        "cue" => "c",
        "loop-in" => "q",
        "loop-out" => "w",
        "reloop" => "e",
        "reverse" => "r",
        "quantize" => "z",
        "vinyl" => "v",
        "range" => "p",
        "master-tempo" => "k",
        "browse-up" => "up",
        "browse-down" => "down",
        "load" => "ret",
        "back" => "esc",
        "tag" => "t",
        "time" => "d",
        "shortcut" => "s",
        "previous" => "comma",
        "next" => "dot",
        "search-back" => "bracket_left",
        "search-forward" => "bracket_right",
        "jog-touch" => "j",
        "jog-forward" => "h",
        "jog-reverse" => "n",
        "pitch-up" => "pgup",
        "pitch-down" => "pgdn",
        "pitch-center" => "home",
        "track-filter" => "f",
        "usb-stop" => "u",
        _ => bail!("Unknown deck button"),
    })
}
pub fn integer(value: &Value, lo: i64, hi: i64) -> Result<i64> {
    let n = value.as_i64().context("Expected an integer")?;
    ensure!(
        (lo..=hi).contains(&n),
        "Expected an integer between {lo} and {hi}"
    );
    Ok(n)
}
#[derive(Default)]
pub struct Controls {
    held: BTreeSet<String>,
    touch_down: bool,
}
impl Controls {
    pub fn button(&mut self, qmp: &mut Qmp, name: &str, down: bool) -> Result<()> {
        let qcode = key(name)?;
        if down == self.held.contains(name) {
            return Ok(());
        }
        qmp.execute("input-send-event",json!({"events":[{"type":"key","data":{"key":{"type":"qcode","data":qcode},"down":down}}]}))?;
        if down {
            self.held.insert(name.to_owned());
        } else {
            self.held.remove(name);
        }
        Ok(())
    }
    pub fn touch(&mut self, qmp: &mut Qmp, x: i64, y: i64, down: bool) -> Result<()> {
        ensure!(
            (0..800).contains(&x) && (0..480).contains(&y),
            "Touch outside LCD"
        );
        let x = (x as f64 * 32767.0 / 799.0).round_ties_even() as i64;
        let y = (y as f64 * 32767.0 / 479.0).round_ties_even() as i64;
        qmp.execute("input-send-event",json!({"events":[{"type":"abs","data":{"axis":"x","value":x}},{"type":"abs","data":{"axis":"y","value":y}},{"type":"btn","data":{"button":"left","down":down}}]}))?;
        self.touch_down = down;
        Ok(())
    }
    pub fn release(&mut self, qmp: &mut Qmp) -> Result<()> {
        for name in self.held.clone() {
            self.button(qmp, &name, false)?;
        }
        if self.touch_down {
            self.touch(qmp, 0, 0, false)?;
        }
        Ok(())
    }
    pub fn apply(&mut self, qmp: &mut Qmp, message: &Value) -> Result<()> {
        let boolean = |name: &str| message[name].as_bool().context("Expected a boolean");
        match message["action"].as_str().context("Missing action")? {
            "button" => self.button(
                qmp,
                message["name"].as_str().context("Missing button name")?,
                boolean("down")?,
            )?,
            "touch" => self.touch(
                qmp,
                integer(&message["x"], 0, 799)?,
                integer(&message["y"], 0, 479)?,
                boolean("down")?,
            )?,
            "pitch" => {
                qmp.set(
                    "xdj-pitch-slider",
                    json!(integer(&message["value"], 0, 65535)?),
                )?;
            }
            "jog" => {
                qmp.set(
                    "xdj-jog-step",
                    json!(integer(&message["ticks"], -4096, 4096)?),
                )?;
            }
            "vinyl-speed" => {
                let v = integer(&message["value"], 0, 255)?;
                for i in 0..2 {
                    qmp.set(&format!("xdj-panel-analog{i}"), json!(v))?;
                }
            }
            "mute" => {
                qmp.set("xdj-audio-muted", json!(boolean("muted")?))?;
            }
            "release" => self.release(qmp)?,
            _ => bail!("Unknown control action"),
        }
        Ok(())
    }
}
