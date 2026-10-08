use serde_json::Value;
use std::collections::{BTreeSet, VecDeque};

pub struct Player {
    pub ready: bool,
    pub status: String,
    pub endpoint: Option<String>,
    pub firmware: Value,
    pub source: String,
    pub state: String,
    pub lit: BTreeSet<String>,
    pub lamps_known: bool,
    pub flashing: BTreeSet<String>,
    pub usb: [u8; 3],
    pub jog: Value,
    pub audio: Value,
    pub catalog: Vec<Value>,
    pub logs: VecDeque<String>,
    running: bool,
}
impl Default for Player {
    fn default() -> Self {
        Self {
            ready: false,
            status: "Open firmware to start the player".into(),
            endpoint: None,
            firmware: Value::Null,
            source: String::new(),
            state: String::new(),
            lit: BTreeSet::new(),
            lamps_known: false,
            flashing: BTreeSet::new(),
            usb: [0; 3],
            jog: Value::Null,
            audio: Value::Null,
            catalog: Vec::new(),
            logs: VecDeque::new(),
            running: false,
        }
    }
}
impl Player {
    pub fn log(&mut self, line: String) {
        if self.logs.len() == 256 {
            self.logs.pop_front();
        }
        self.logs.push_back(line.chars().take(4096).collect());
    }
    pub fn event(&mut self, event: &Value) {
        if let Some(message) = event["message"].as_str() {
            self.status = message.into();
        }
        if let Some(info) = event.get("firmware") {
            self.firmware = info.clone();
        }
        if let Some(source) = event["source"].as_str() {
            self.source = source.into();
        }
        if let Some(state) = event["state"].as_str() {
            self.state = state.into();
        }
        match event["event"].as_str().unwrap_or("") {
            "running" => {
                self.running = true;
                self.ready = true;
                self.endpoint = event["vnc"].as_str().map(str::to_owned);
            }
            "starting" | "loading" | "preparing" => self.ready = false,
            "selected" if event["unchanged"] != true => {
                self.ready = false;
                self.endpoint = None;
                self.running = false;
            }
            "error" => {
                self.ready = self.running && event["recoverable"] == true;
                if event["recoverable"] != true {
                    self.running = false;
                    self.endpoint = None;
                }
            }
            "stopped" | "disconnected" => {
                self.running = false;
                self.ready = false;
                self.endpoint = None;
            }
            "catalog" => self.catalog = event["firmwares"].as_array().cloned().unwrap_or_default(),
            "lamps" => {
                self.lamps_known = true;
                self.flashing = event["flashing"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                self.lit = event["lit"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                for (i, channel) in self.usb.iter_mut().enumerate() {
                    *channel = event["usb"][i].as_u64().unwrap_or(0).min(255) as u8;
                }
                self.jog = event["jog"].clone();
            }
            "audio" => self.audio = event.clone(),
            _ => {}
        }
        if event["event"] != "audio" && event["event"] != "lamps" && event["event"] != "performance"
        {
            self.log(event.to_string());
        }
    }
}
