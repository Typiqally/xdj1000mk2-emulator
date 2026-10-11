use crate::{
    bridge::Bridge,
    input::{Input, lcd_point},
    layout::{self, *},
    model::Player,
    transport::{Display, Frame, Latest},
};
use anyhow::{Context, Result, bail, ensure};
use eframe::egui::{self, Align2, Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Vec2};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Options {
    pub root: PathBuf,
    pub backend: PathBuf,
    pub firmware: Option<PathBuf>,
    pub view: String,
    pub ui_scale: f32,
    pub diagnostics: Option<PathBuf>,
    pub script: Vec<Value>,
    pub exit_after: Option<f64>,
    pub backend_args: Vec<String>,
    pub usb_folder: Option<PathBuf>,
}
impl Default for Options {
    fn default() -> Self {
        let root = std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.ancestors()
                    .find(|p| p.join("native/hardware_profile.json").is_file())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        Self {
            backend: root.join(format!(
                "target/release/xdj-backend{}",
                std::env::consts::EXE_SUFFIX
            )),
            root,
            firmware: None,
            view: "firmware".into(),
            ui_scale: 1.,
            diagnostics: None,
            script: Vec::new(),
            exit_after: None,
            backend_args: Vec::new(),
            usb_folder: None,
        }
    }
}
impl Options {
    pub fn paths_for_executable(
        executable: &std::path::Path,
        fallback: &std::path::Path,
    ) -> Result<(PathBuf, PathBuf)> {
        let config = executable
            .parent()
            .filter(|p| p.file_name().is_some_and(|n| n == "MacOS"))
            .and_then(|p| p.parent())
            .map(|p| p.join("Resources/config.json"));
        if let Some(config) = config.filter(|p| p.is_file()) {
            let config: Value = serde_json::from_slice(&fs::read(config)?)?;
            let root = PathBuf::from(
                config["root"]
                    .as_str()
                    .context("App configuration needs a repository root")?,
            );
            let backend = PathBuf::from(
                config["backend"]
                    .as_str()
                    .context("App configuration needs a backend")?,
            );
            ensure!(
                root.is_absolute() && backend.is_absolute(),
                "App configuration paths must be absolute"
            );
            return Ok((root, backend));
        }
        let root = executable
            .ancestors()
            .find(|p| p.join("native/hardware_profile.json").is_file())
            .unwrap_or(fallback)
            .to_path_buf();
        let backend = root.join(format!(
            "target/release/xdj-backend{}",
            std::env::consts::EXE_SUFFIX
        ));
        Ok((root, backend))
    }
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut options = Self::default();
        let mut args = arguments.into_iter();
        if let Ok(executable) = std::env::current_exe() {
            (options.root, options.backend) =
                Self::paths_for_executable(&executable, &options.root)?;
        }
        let default_backend = options.backend.clone();
        let mut backend = None;
        let mut root_overridden = false;
        while let Some(arg) = args.next() {
            if arg == "--" {
                options.backend_args.extend(args);
                break;
            }
            if arg == "--help" {
                bail!(
                    "Usage: xdj-ui [--root REPO] [--backend FILE] [--firmware UPD] [--view firmware|player] [--ui-scale SCALE] [--diagnostics DIR] [--test-script JSON] [--exit-after SECONDS] [-- BACKEND OPTIONS]\nAudio starts muted. Windows/Linux require a compatible local backend."
                );
            }
            let value = args
                .next()
                .with_context(|| format!("Missing value for {arg}"))?;
            match arg.as_str() {
                "--root" => {
                    options.root = PathBuf::from(value);
                    root_overridden = true;
                }
                "--backend" => backend = Some(PathBuf::from(value)),
                "--firmware" => options.firmware = Some(PathBuf::from(value)),
                "--view" => options.view = value,
                "--ui-scale" => options.ui_scale = value.parse()?,
                "--diagnostics" => options.diagnostics = Some(PathBuf::from(value)),
                "--test-script" => options.script = serde_json::from_slice(&fs::read(value)?)?,
                "--exit-after" => options.exit_after = Some(value.parse()?),
                _ => bail!("Unknown UI option {arg}; pass backend options after --"),
            }
        }
        ensure!(
            ["firmware", "player"].contains(&options.view.as_str()),
            "Choose firmware or player view"
        );
        ensure!(
            options.ui_scale.is_finite() && (0.5..=2.).contains(&options.ui_scale),
            "UI scale must be between 0.5 and 2"
        );
        ensure!(
            options.exit_after.is_none_or(|s| s.is_finite() && s > 0.),
            "Invalid exit duration"
        );
        for action in &options.script {
            ensure!(
                action["after"]
                    .as_f64()
                    .is_some_and(|t| t.is_finite() && t >= 0.),
                "Script actions need a nonnegative after time"
            );
        }
        options.script.sort_by(|a, b| {
            a["after"]
                .as_f64()
                .unwrap()
                .total_cmp(&b["after"].as_f64().unwrap())
        });
        options.root = options.root.canonicalize()?;
        options.backend = backend.unwrap_or_else(|| {
            if !root_overridden {
                return default_backend;
            }
            options.root.join(format!(
                "target/release/xdj-backend{}",
                std::env::consts::EXE_SUFFIX
            ))
        });
        let defaults = options.root.join("output/desktop-defaults.json");
        if defaults.is_file() {
            let defaults: Value = serde_json::from_slice(&fs::read(defaults)?)
                .context("Read local desktop defaults")?;
            options.usb_folder = defaults["usb_folder"].as_str().map(PathBuf::from);
            if !options.backend_args.iter().any(|arg| arg == "--usb-image")
                && let Some(image) = defaults["usb_image"].as_str()
            {
                let image = PathBuf::from(image)
                    .canonicalize()
                    .context("Default USB image is missing")?;
                ensure!(image.is_file(), "Default USB image must be a file");
                options
                    .backend_args
                    .extend(["--usb-image".into(), image.display().to_string()]);
            }
        }
        Ok(options)
    }
}
#[derive(Default)]
struct Metrics {
    frames: u64,
    received: u64,
    paints: u64,
    upload_us: Vec<u64>,
    delivery_us: Vec<u64>,
    ui_us: Vec<u64>,
}
fn percentile(values: &[u64], fraction: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut values = values.to_vec();
    values.sort_unstable();
    values[((values.len() - 1) as f64 * fraction).round() as usize]
}

pub struct DeckApp {
    pub player: Player,
    pub options: Options,
    bridge: Option<Bridge>,
    display: Option<Display>,
    connected: Option<String>,
    latest: Arc<Latest<Frame>>,
    display_error: Arc<Mutex<Option<String>>>,
    texture: Option<egui::TextureHandle>,
    input: Input,
    pointer_held: BTreeSet<String>,
    key_held: BTreeSet<String>,
    rects: BTreeMap<String, Rect>,
    commands: Vec<Value>,
    offline: bool,
    started: Instant,
    script_index: usize,
    muted: bool,
    pitch: i32,
    vinyl_speed: i32,
    show_keys: bool,
    show_logs: bool,
    show_inspector: bool,
    input_log: std::collections::VecDeque<String>,
    last_point: Option<(i32, i32)>,
    framebuffer: Option<([usize; 2], Vec<u8>)>,
    log_filter: String,
    metrics: Metrics,
    renderer: String,
    events: Option<File>,
    command_log: Option<File>,
    last_touch: Option<(i32, i32)>,
    jog_down: bool,
    recent: crate::menus::Recent,
    #[cfg(target_os = "macos")]
    native_menu: Option<crate::menus::native::NativeMenu>,
}
impl DeckApp {
    pub fn offline(options: Options) -> Self {
        let recent_root = options
            .backend_args
            .windows(2)
            .find(|a| a[0] == "--state-root")
            .map(|a| PathBuf::from(&a[1]))
            .unwrap_or_else(|| options.root.join("output/firmwares"));
        let mut player = Player::default();
        player.event(&json!({"event":"catalog","firmwares":xdj_catalog::archived(&options.root.join("firmware"))}));
        Self {
            recent: crate::menus::Recent::load(recent_root.join("recent-firmware.json")),
            #[cfg(target_os = "macos")]
            native_menu: None,
            player,
            options,
            bridge: None,
            display: None,
            connected: None,
            latest: Arc::new(Latest::default()),
            display_error: Arc::new(Mutex::new(None)),
            texture: None,
            input: Input::default(),
            pointer_held: BTreeSet::new(),
            key_held: BTreeSet::new(),
            rects: BTreeMap::new(),
            commands: Vec::new(),
            offline: true,
            started: Instant::now(),
            script_index: 0,
            muted: true,
            pitch: 32768,
            vinyl_speed: 0,
            show_keys: false,
            show_logs: false,
            show_inspector: true,
            input_log: Default::default(),
            last_point: None,
            framebuffer: None,
            log_filter: String::new(),
            metrics: Metrics::default(),
            renderer: "headless".into(),
            events: None,
            command_log: None,
            last_touch: None,
            jog_down: false,
        }
    }
    pub fn new(cc: &eframe::CreationContext<'_>, options: Options) -> Result<Self> {
        let mut app = Self::offline(options);
        app.offline = false;
        #[cfg(target_os = "macos")]
        {
            let mut menu = crate::menus::native::NativeMenu::new(&cc.egui_ctx);
            menu.sync(app.menu_entries())?;
            app.native_menu = Some(menu);
        }
        crate::skin::install_fonts(&cc.egui_ctx);
        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = PAPER;
        style.visuals.override_text_color = Some(INK);
        style.spacing.interact_size = Vec2::new(44., 32.);
        for size in style.text_styles.values_mut() {
            size.size = 14.;
        }
        cc.egui_ctx.set_global_style(style);
        if let Some(state) = &cc.wgpu_render_state {
            let info = state.adapter.get_info();
            app.renderer = format!("{:?} · {}", info.backend, info.name);
        }
        if let Some(dir) = &app.options.diagnostics {
            fs::create_dir_all(dir)?;
            app.events = Some(File::create(dir.join("app-events.jsonl"))?);
            app.command_log = Some(File::create(dir.join("input.jsonl"))?);
        }
        if app.options.firmware.is_some() {
            app.start(&cc.egui_ctx);
        }
        Ok(app)
    }
    pub fn control_rect(&self, name: &str) -> Option<Rect> {
        self.rects.get(name).copied()
    }
    pub fn take_commands(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.commands)
    }
    fn emit(&mut self, command: Option<Value>) {
        if let Some(command) = command {
            if ["button", "touch", "jog", "pitch", "vinyl-speed"]
                .contains(&command["action"].as_str().unwrap_or(""))
            {
                let action = command["action"].as_str().unwrap_or("");
                let text = match action {
                    "button" => format!(
                        "{} {}",
                        command["name"].as_str().unwrap_or("?"),
                        if command["down"] == true {
                            "↓"
                        } else {
                            "↑"
                        }
                    ),
                    "touch" => format!(
                        "touch {}, {}{}",
                        command["x"],
                        command["y"],
                        if command["down"] == true { "" } else { " ↑" }
                    ),
                    "jog" => format!("jog {}", command["ticks"]),
                    "pitch" => format!("tempo fader {}", command["value"]),
                    _ => format!("vinyl speed {}", command["value"]),
                };
                if action != "button"
                    && self.input_log.front().is_some_and(|line| {
                        line.contains(text.split_whitespace().next().unwrap_or(""))
                    })
                {
                    self.input_log.pop_front();
                }
                self.input_log.push_front(format!(
                    "{:7.2}  {text}",
                    self.started.elapsed().as_secs_f64()
                ));
                self.input_log.truncate(9);
            }
            let command = crate::timing::stamp(
                command,
                !self.offline && self.options.diagnostics.is_some(),
                "host_sent_unix_us",
            );
            if let Some(log) = &mut self.command_log {
                let _ = writeln!(
                    log,
                    "{}",
                    json!({"elapsed":self.started.elapsed().as_secs_f64(),"command":command})
                );
            }
            if self.offline {
                self.commands.push(command);
            } else if let Some(bridge) = &self.bridge
                && let Err(e) = bridge.send(command)
            {
                self.player.status = e.to_string();
                self.player.log(e.to_string());
                self.player.ready = false;
            }
        }
    }
    fn release(&mut self) {
        let command = self.input.release();
        self.emit(command);
        self.pointer_held.clear();
        self.key_held.clear();
        self.last_touch = None;
        self.jog_down = false;
    }
    fn start(&mut self, ctx: &egui::Context) {
        let Some(firmware) = &self.options.firmware else {
            return;
        };
        self.player.ready = false;
        self.player.status = "Starting firmware…".into();
        let mut args = vec![
            "shell".into(),
            "--root".into(),
            self.options.root.display().to_string(),
            "--firmware".into(),
            firmware.display().to_string(),
            "--mute-audio".into(),
        ];
        args.extend(self.options.backend_args.clone());
        if let Some(dir) = &self.options.diagnostics {
            args.extend(["--diagnostics".into(), dir.display().to_string()]);
        }
        let ctx = ctx.clone();
        match Bridge::spawn(
            &self.options.backend,
            &args,
            &self.options.root,
            Arc::new(move || ctx.request_repaint()),
        ) {
            Ok(bridge) => self.bridge = Some(bridge),
            Err(e) => {
                self.player.status = format!("{e:#}");
                self.player.log(self.player.status.clone());
            }
        }
    }
    fn command(&mut self, value: Value, ctx: &egui::Context) {
        // Scheduled native inputs belong to the backend's control lifetime.
        // Keeping them in the physical UI input set would cancel every held
        // button/touch as soon as this window loses keyboard focus.
        if value["after"].as_f64().is_some()
            && matches!(value["action"].as_str(), Some("button" | "touch"))
        {
            if self.player.ready {
                self.emit(Some(value));
            }
            return;
        }
        match value["action"].as_str().unwrap_or("") {
            #[cfg(target_os = "macos")]
            "native-menu" if self.options.diagnostics.is_some() => {
                let path = value["path"]
                    .as_array()
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|p| p.as_str().map(str::to_owned))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if let Some(menu) = &self.native_menu
                    && let Err(error) = menu.invoke(&path)
                {
                    self.player.log(format!("Menu diagnostic failed: {error}"));
                }
            }
            "snapshot" => {
                if self.options.diagnostics.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                        value["name"].as_str().unwrap_or("deck").to_owned(),
                    )));
                }
            }
            "quit" => {
                self.release();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            "show-logs" => self.show_logs = true,
            "show-keys" => self.show_keys = value["visible"] != false,
            "hide-logs" => self.show_logs = false,
            "view" => {
                self.release();
                if let Some(view) = value["view"]
                    .as_str()
                    .filter(|v| ["player", "firmware"].contains(v))
                {
                    self.options.view = view.into();
                    let face = if view == "player" {
                        Vec2::new(1065., 1336.)
                    } else {
                        Vec2::new(1280., 724.)
                    };
                    let monitor = ctx
                        .input(|i| i.viewport().monitor_size)
                        .unwrap_or(Vec2::new(1440., 1100.));
                    let scale = ((monitor.x - 60.) / face.x)
                        .min((monitor.y - 140.) / face.y)
                        .min(self.options.ui_scale);
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                        face * scale + Vec2::new(0., 74.),
                    ));
                }
            }
            "firmware" => {
                self.release();
                self.player.ready = false;
                if let Some(path) = value["path"].as_str() {
                    self.options.firmware = Some(path.into());
                    if self.offline {
                        self.emit(Some(value));
                    } else if self.bridge.is_none() {
                        self.start(ctx);
                    } else {
                        self.emit(Some(value));
                    }
                }
            }
            "button" => {
                if self.player.ready
                    && let (Some(name), Some(down)) =
                        (value["name"].as_str(), value["down"].as_bool())
                {
                    let command = self.input.button(name, down);
                    self.emit(command);
                }
            }
            "touch" => {
                if self.player.ready {
                    let command = self.input.touch(
                        value["x"].as_i64().unwrap_or(0) as i32,
                        value["y"].as_i64().unwrap_or(0) as i32,
                        value["down"] == true,
                    );
                    self.emit(command);
                }
            }
            "pitch" => {
                if self.player.ready
                    && let Some(value) = value["value"].as_i64().filter(|v| (0..=65535).contains(v))
                {
                    self.pitch = value as i32;
                    self.emit(Some(json!({"action":"pitch","value":self.pitch})));
                }
            }
            "vinyl-speed" => {
                if self.player.ready
                    && let Some(value) = value["value"].as_i64().filter(|v| (0..=255).contains(v))
                {
                    self.vinyl_speed = value as i32;
                    self.emit(Some(
                        json!({"action":"vinyl-speed","value":self.vinyl_speed}),
                    ));
                }
            }
            "restart" | "validate" | "music" | "export" | "image" => {
                self.release();
                if self.bridge.is_none() {
                    self.start(ctx);
                } else {
                    self.player.ready = false;
                    self.emit(Some(value));
                }
            }
            "stop" => {
                self.release();
                self.emit(Some(value));
                self.player.ready = false;
            }
            "mute" => {
                self.muted = value["muted"] != false;
                self.emit(Some(json!({"action":"mute","muted":self.muted})));
            }
            _ => {
                if self.player.ready {
                    self.emit(Some(value));
                }
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let events: Vec<Value> = self
            .bridge
            .as_ref()
            .map(|b| b.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            let diagnostic_ack =
                event["event"] == "input-ack" && self.options.diagnostics.is_some();
            let event = crate::timing::stamp(event, diagnostic_ack, "host_received_unix_us");
            if let Some(log) = &mut self.events {
                let _ = writeln!(log, "{event}");
            }
            if event["event"] == "selected"
                && let Some(path) = event["requested_source"]
                    .as_str()
                    .or_else(|| event["source"].as_str())
                && let Err(error) = self.recent.remember(std::path::Path::new(path))
            {
                self.player
                    .log(format!("Cannot save recent firmware: {error}"));
            }
            self.player.event(&event);
            if !self.player.ready {
                self.release();
            }
        }
        let fault = self
            .bridge
            .as_ref()
            .and_then(|b| b.error.lock().unwrap().take());
        if let Some(fault) = fault {
            self.release();
            self.player.status = fault;
            self.player.ready = false;
        }
        if self
            .bridge
            .as_mut()
            .is_some_and(|b| b.exited().unwrap_or(true))
        {
            self.release();
            self.player.event(
                &json!({"event":"disconnected","message":"Player stopped. Restart to try again."}),
            );
            self.bridge = None;
        }
        if self.connected != self.player.endpoint {
            self.display = None;
            self.connected = None;
            self.texture = None;
            self.latest.take();
            if let Some(endpoint) = &self.player.endpoint {
                let ctx = ctx.clone();
                match Display::start(
                    endpoint,
                    self.latest.clone(),
                    self.display_error.clone(),
                    Arc::new(move || ctx.request_repaint()),
                ) {
                    Ok(display) => {
                        self.display = Some(display);
                        self.connected = Some(endpoint.clone());
                    }
                    Err(e) => {
                        self.player.status = format!("Cannot connect display: {e}");
                        self.connected = Some(endpoint.clone());
                    }
                }
            }
        }
        let fault = self.display_error.lock().unwrap().take();
        if let Some(fault) = fault {
            self.player.status = fault;
        }
        if let Some(frame) = self.latest.take() {
            self.metrics.received = frame.sequence;
            self.metrics.frames += 1;
            let age = frame.received.elapsed().as_micros() as u64;
            let start = Instant::now();
            // RFB normalizes alpha to 255, so these bytes are already premultiplied.
            let image = egui::ColorImage::from_rgba_premultiplied(frame.patch.size, &frame.rgba);
            if let Some(texture) = &mut self.texture {
                if texture.size() == frame.size {
                    texture.set_partial(frame.patch.origin, image, egui::TextureOptions::LINEAR);
                } else {
                    texture.set(image, egui::TextureOptions::LINEAR);
                }
            } else {
                self.texture =
                    Some(ctx.load_texture("firmware LCD", image, egui::TextureOptions::LINEAR));
            }
            if frame.patch.origin == [0, 0] && frame.patch.size == frame.size {
                self.framebuffer = Some((frame.size, frame.rgba));
            } else if let Some((size, pixels)) = &mut self.framebuffer {
                debug_assert_eq!(*size, frame.size);
                let [x, y] = frame.patch.origin;
                let [width, height] = frame.patch.size;
                for row in 0..height {
                    let dest = ((y + row) * size[0] + x) * 4;
                    pixels[dest..dest + width * 4]
                        .copy_from_slice(&frame.rgba[row * width * 4..(row + 1) * width * 4]);
                }
            }
            if self.metrics.upload_us.len() < 60000 {
                self.metrics
                    .upload_us
                    .push(start.elapsed().as_micros() as u64);
                self.metrics.delivery_us.push(age);
            }
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        while self.script_index < self.options.script.len()
            && self.options.script[self.script_index]["after"]
                .as_f64()
                .unwrap_or(f64::MAX)
                <= elapsed
        {
            let command = self.options.script[self.script_index].clone();
            self.script_index += 1;
            self.command(command, ctx);
        }
        if let Some(next) = self
            .options
            .script
            .get(self.script_index)
            .and_then(|v| v["after"].as_f64())
        {
            ctx.request_repaint_after(Duration::from_secs_f64((next - elapsed).max(0.001)));
        }
        if let Some(seconds) = self.options.exit_after {
            if elapsed >= seconds {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(seconds - elapsed));
            }
        }
        if self.bridge.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot {
                user_data, image, ..
            } = event
            {
                let name = user_data
                    .data
                    .as_ref()
                    .and_then(|v| v.downcast_ref::<String>())
                    .map(String::as_str)
                    .unwrap_or("deck");
                if let Some(dir) = &self.options.diagnostics
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                {
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    if let Err(e) = image::save_buffer(
                        dir.join(format!("{name}.png")),
                        &rgba,
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    ) {
                        self.player.status = e.to_string();
                    }
                    let _ = fs::write(dir.join(format!("{name}.json")), self.report().to_string());
                }
            }
        }
    }
    pub fn report(&self) -> Value {
        let native_menu = {
            #[cfg(target_os = "macos")]
            {
                self.native_menu
                    .as_ref()
                    .map(|menu| menu.installed())
                    .unwrap_or(Value::Null)
            }
            #[cfg(not(target_os = "macos"))]
            {
                Value::Null
            }
        };
        json!({"default_usb_folder":self.options.usb_folder,"default_usb_image":self.options.backend_args.windows(2).find(|args| args[0] == "--usb-image").map(|args| &args[1]),"native_menu":native_menu,"menus":self.menu_entries().iter().map(|entry| &entry.label).collect::<Vec<_>>(),"archived_firmware_count":self.player.catalog.len(),"recent_firmware":self.recent.paths,"renderer":self.renderer,"frames":self.metrics.frames,"received_frames":self.metrics.received,"paints":self.metrics.paints,"elapsed_seconds":self.started.elapsed().as_secs_f64(),"ready":self.player.ready,"has_frame":self.texture.is_some(),"muted":self.muted,"view":self.options.view,"state":self.player.state,"firmware":self.player.firmware,"status":self.player.status,"lamps":self.player.lit,"backend_pid":self.bridge.as_ref().map(Bridge::id),"upload_p50_us":percentile(&self.metrics.upload_us,0.5),"upload_p95_us":percentile(&self.metrics.upload_us,0.95),"delivery_p95_us":percentile(&self.metrics.delivery_us,0.95),"ui_p95_us":percentile(&self.metrics.ui_us,0.95)})
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        let native_shortcuts = {
            #[cfg(target_os = "macos")]
            {
                self.native_menu.is_some()
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        };
        if !native_shortcuts && ctx.input(|i| i.focused) && !ctx.text_edit_focused() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::F)) {
                self.open_firmware(ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::R)) {
                self.command(json!({"action":"restart"}), ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::L)) {
                self.show_logs = !self.show_logs;
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::O)) {
                self.activate_menu(json!({"action":"load-music"}), ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Slash)) {
                self.show_keys = !self.show_keys;
            }
            for (key, view) in [(Key::Num1, "firmware"), (Key::Num2, "player")] {
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key)) {
                    self.command(json!({"action":"view","view":view}), ctx);
                }
            }
        }
        if !ctx.input(|i| i.focused) || !self.player.ready {
            self.release();
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            self.release();
            return;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            {
                if repeat || modifiers.command || modifiers.ctrl || modifiers.alt {
                    continue;
                }
                let name = CONTROLS
                    .iter()
                    .find(|c| c.key == key)
                    .map(|c| c.name)
                    .or(match key {
                        Key::ArrowUp => Some("browse-up"),
                        Key::ArrowDown => Some("browse-down"),
                        Key::J => Some("jog-touch"),
                        Key::H => Some("jog-forward"),
                        Key::N => Some("jog-reverse"),
                        Key::PageUp => Some("pitch-up"),
                        Key::PageDown => Some("pitch-down"),
                        Key::Home => Some("pitch-center"),
                        _ => None,
                    });
                if let Some(name) = name {
                    if pressed {
                        self.key_held.insert(name.into());
                    } else {
                        self.key_held.remove(name);
                    }
                    let command = self
                        .input
                        .button(name, pressed || self.pointer_held.contains(name));
                    self.emit(command);
                }
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        #[cfg(target_os = "macos")]
        {
            let actions = self
                .native_menu
                .as_ref()
                .map(|m| m.pending())
                .unwrap_or_default();
            for action in actions {
                self.activate_menu(action, &ctx);
            }
            let entries = self.menu_entries();
            if let Some(menu) = &mut self.native_menu
                && let Err(error) = menu.sync(entries)
            {
                self.player.log(format!("Cannot update menus: {error}"));
            }
        }
        self.keyboard(&ctx);
        #[cfg(not(target_os = "macos"))]
        egui::Panel::top("desktop menus").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                if let Some(action) = crate::menus::draw(ui, &self.menu_entries()) {
                    self.activate_menu(action, &ctx);
                }
            });
        });
        egui::Panel::top("host bar")
            .exact_size(48.)
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(23, 25, 28)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                let p = ui.painter().clone();
                let centre = r.center().y;
                p.line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(1., crate::skin::DIVIDER),
                );
                p.circle_filled(
                    Pos2::new(r.left() + 15., centre),
                    3.,
                    if self.player.firmware.is_null() {
                        Color32::from_rgb(106, 106, 112)
                    } else if self.player.firmware["original"] == true {
                        GREEN
                    } else {
                        AMBER
                    },
                );
                p.text(
                    Pos2::new(r.left() + 28., centre),
                    Align2::LEFT_CENTER,
                    "XDJ-1000MK2",
                    crate::skin::weighted_font(&p, 14., true),
                    INK,
                );
                let mut edge = r.right() - 12.;
                for (label, width, on) in [
                    ("Mute", 70., self.muted),
                    ("Keys", 68., self.show_keys),
                    ("Load Music…", 116., false),
                    ("USB Folder…", 114., false),
                ] {
                    edge -= width;
                    let button =
                        Rect::from_min_size(Pos2::new(edge, centre - 15.), Vec2::new(width, 30.));
                    if crate::skin::host_button(ui, button, label, on, 12.).clicked() {
                        match label {
                            "Mute" => {
                                self.muted = !self.muted;
                                self.command(json!({"action":"mute","muted":self.muted}), &ctx);
                            }
                            "Keys" => self.show_keys = !self.show_keys,
                            "Load Music…" => {
                                self.release();
                                if let Some(paths) = rfd::FileDialog::new()
                                    .add_filter(
                                        "Audio",
                                        &["mp3", "wav", "flac", "aif", "aiff", "m4a"],
                                    )
                                    .pick_files()
                                {
                                    self.command(json!({"action":"music","paths":paths}), &ctx);
                                }
                            }
                            _ => {
                                self.release();
                                if let Some(folder) = rfd::FileDialog::new()
                                    .set_directory(
                                        self.options
                                            .usb_folder
                                            .as_ref()
                                            .unwrap_or(&self.options.root),
                                    )
                                    .pick_folder()
                                {
                                    self.command(json!({"action":"export","folder":folder}), &ctx);
                                }
                            }
                        }
                    }
                    edge -= 8.;
                }
                edge -= 6.;
                let switch =
                    Rect::from_min_size(Pos2::new(edge - 134., centre - 14.), Vec2::new(134., 28.));
                for (i, (view, label, width)) in
                    [("firmware", "Firmware", 76.), ("player", "Player", 58.)]
                        .iter()
                        .enumerate()
                {
                    let cell = Rect::from_min_size(
                        switch.min + Vec2::new(if i == 0 { 0. } else { 76. }, 0.),
                        Vec2::new(*width, 28.),
                    );
                    if self.options.view == *view {
                        p.rect_filled(cell, 0., Color32::from_rgb(57, 62, 69));
                    }
                    crate::skin::text(
                        &p,
                        cell.center(),
                        label,
                        12.,
                        if self.options.view == *view {
                            INK
                        } else {
                            MUTED
                        },
                    );
                    if ui
                        .interact(cell, ui.id().with(view), Sense::click())
                        .clicked()
                    {
                        self.command(json!({"action":"view","view":view}), &ctx);
                    }
                }
                p.rect_stroke(
                    switch,
                    4.,
                    Stroke::new(1., crate::skin::DIVIDER),
                    egui::StrokeKind::Inside,
                );
                let identity = if self.player.firmware.is_null() {
                    "No firmware loaded".to_owned()
                } else {
                    format!(
                        "Firmware {}",
                        self.player.firmware["version"].as_str().unwrap_or("—")
                    )
                };
                if switch.left() > 260. {
                    p.with_clip_rect(Rect::from_min_max(
                        Pos2::new(r.left() + 140., r.top()),
                        Pos2::new(switch.left() - 12., r.bottom()),
                    ))
                    .text(
                        Pos2::new(r.left() + 140., centre),
                        Align2::LEFT_CENTER,
                        identity,
                        FontId::proportional(12.),
                        MUTED,
                    );
                }
                let model = ui.interact(
                    Rect::from_min_size(r.min, Vec2::new(140., 48.)),
                    ui.id().with("firmware-menu"),
                    Sense::click(),
                );
                model.context_menu(|ui| {
                    if let Some(action) = crate::menus::draw(ui, &self.menu_entries()) {
                        self.activate_menu(action, &ctx);
                    }
                });
            });
        egui::Panel::bottom("footer")
            .exact_size(26.)
            .frame(egui::Frame::NONE.fill(PAPER))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_top(), r.right_top()],
                    Stroke::new(1., crate::skin::DIVIDER),
                );
                ui.painter().text(
                    r.left_center() + Vec2::new(12., 0.),
                    Align2::LEFT_CENTER,
                    if self.options.firmware.is_none() {
                        "No firmware loaded · Open Firmware… or choose Recent Firmware"
                    } else {
                        &self.player.status
                    },
                    FontId::proportional(12.),
                    MUTED,
                );
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(PAPER))
            .show(ui, |ui| {
                self.deck(ui);
            });
        if self.show_logs {
            egui::Window::new("Live logs")
                .default_size([700., 400.])
                .open(&mut self.show_logs)
                .show(&ctx, |ui| {
                    ui.text_edit_singleline(&mut self.log_filter);
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for line in &self.player.logs {
                                if self.log_filter.is_empty() || line.contains(&self.log_filter) {
                                    ui.label(egui::RichText::new(line).monospace().size(12.));
                                }
                            }
                        });
                });
        }
    }
    pub fn menu_entries(&self) -> Vec<crate::menus::Entry> {
        use crate::menus::Entry as E;
        let mut archives: Vec<_> = self
            .player
            .catalog
            .iter()
            .filter_map(|entry| {
                let path = entry["path"].as_str()?;
                Some(
                    E::item(
                        format!("Firmware {}", entry["version"].as_str()?),
                        json!({"action":"firmware","path":path}),
                    )
                    .enabled(std::path::Path::new(path).is_file()),
                )
            })
            .collect();
        if archives.is_empty() {
            archives.push(E::empty("No archived firmware available"));
        }
        let has_firmware = self.options.firmware.is_some();
        let mut files = vec![
            E::item("Open Firmware…", json!({"action":"open-firmware"})).shortcut("CmdOrCtrl+F"),
            E::group("Recent Firmware", self.recent.entries()),
            E::group("Archived Firmware", archives),
            E::separator(),
            E::item("Load Music…", json!({"action":"load-music"}))
                .shortcut("CmdOrCtrl+O")
                .enabled(has_firmware),
            E::item("Load rekordbox USB Folder…", json!({"action":"usb-folder"}))
                .enabled(has_firmware),
        ];
        if self
            .options
            .root
            .join("media/reference/PIONEER/rekordbox/export.pdb")
            .is_file()
        {
            files.push(E::item("Load Reference USB Metadata", json!({"action":"export","folder":self.options.root.join("media/reference/PIONEER")})).enabled(has_firmware));
            files.push(
                E::item(
                    "Add Audio to Reference USB…",
                    json!({"action":"reference-audio"}),
                )
                .enabled(has_firmware),
            );
        }
        files.push(E::item("Open USB Image…", json!({"action":"usb-image"})).enabled(has_firmware));
        vec![
            E::group("File", files),
            E::group(
                "Player",
                vec![
                    E::item("Restart Player", json!({"action":"restart"}))
                        .shortcut("CmdOrCtrl+R")
                        .enabled(has_firmware),
                    E::item("Stop Player", json!({"action":"stop"})).enabled(self.player.ready),
                    E::group(
                        "Virtual Link Cable",
                        vec![
                            E::item("Unplug Cable", json!({"action":"link","connected":false}))
                                .enabled(
                                    self.player.ready
                                        && self.options.backend_args.iter().any(|arg| {
                                            arg == "--link-listen" || arg == "--link-connect"
                                        }),
                                ),
                            E::item("Plug Cable", json!({"action":"link","connected":true}))
                                .enabled(
                                    self.player.ready
                                        && self.options.backend_args.iter().any(|arg| {
                                            arg == "--link-listen" || arg == "--link-connect"
                                        }),
                                ),
                        ],
                    ),
                    E::separator(),
                    E::item("Show Live Logs", json!({"action":"show-logs"}))
                        .shortcut("CmdOrCtrl+L"),
                    E::item("Mute Audio", json!({"action":"mute","muted":!self.muted}))
                        .checked(self.muted),
                    E::item("Validate Playback…", json!({"action":"validate"}))
                        .enabled(has_firmware),
                ],
            ),
            E::group(
                "View",
                vec![
                    E::item("Firmware View", json!({"action":"view","view":"firmware"}))
                        .shortcut("CmdOrCtrl+Digit1")
                        .checked(self.options.view == "firmware"),
                    E::item("Player View", json!({"action":"view","view":"player"}))
                        .shortcut("CmdOrCtrl+Digit2")
                        .checked(self.options.view == "player"),
                    E::separator(),
                    E::item(
                        "Show Keys",
                        json!({"action":"show-keys","visible":!self.show_keys}),
                    )
                    .shortcut("CmdOrCtrl+Slash")
                    .checked(self.show_keys),
                ],
            ),
        ]
    }
    pub fn activate_menu(&mut self, action: Value, ctx: &egui::Context) {
        self.release();
        match action["action"].as_str().unwrap_or("") {
            "open-firmware" => self.open_firmware(ctx),
            "load-music" => {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Audio", &["mp3", "wav", "flac", "aif", "aiff", "m4a"])
                    .pick_files()
                {
                    self.command(json!({"action":"music","paths":paths}), ctx);
                }
            }
            "usb-folder" | "reference-audio" => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_directory(
                        self.options
                            .usb_folder
                            .as_ref()
                            .unwrap_or(&self.options.root),
                    )
                    .pick_folder()
                {
                    let command = if action["action"] == "reference-audio" {
                        json!({"action":"export","folder":self.options.root.join("media/reference/PIONEER"),"audio_root":folder})
                    } else {
                        json!({"action":"export","folder":folder})
                    };
                    self.command(command, ctx);
                }
            }
            "usb-image" => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("USB image", &["img", "raw"])
                    .pick_file()
                {
                    self.command(json!({"action":"image","path":path}), ctx);
                }
            }
            "clear-recent" => {
                if let Err(error) = self.recent.clear() {
                    self.player
                        .log(format!("Cannot clear recent firmware: {error}"));
                }
            }
            _ => self.command(action, ctx),
        }
    }
    fn open_firmware(&mut self, ctx: &egui::Context) {
        self.release();
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open XDJ-1000MK2 Firmware")
            .add_filter("Firmware", &["upd", "UPD"])
            .pick_file()
        {
            self.command(json!({"action":"firmware","path":path}), ctx);
        }
    }
    fn inspector(&mut self, ui: &mut egui::Ui, r: Rect, s: f32, lcd: Rect) {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let width = 242.;
        let point = |x: f32, y: f32| r.min + Vec2::new(x, y) * s;
        let line = |y: f32| {
            p.line_segment(
                [point(16., y), point(258., y)],
                Stroke::new(s, crate::skin::DIVIDER),
            );
        };
        p.line_segment(
            [r.left_top(), r.left_bottom()],
            Stroke::new(s, crate::skin::DIVIDER),
        );
        let label = |text: &str, x: f32, y: f32, right: bool, mono: bool, color: Color32| {
            let font = if mono {
                FontId::monospace(12. * s)
            } else {
                FontId::proportional(12. * s)
            };
            // Clip long firmware names and hashes inside their own value column.
            let clip = Rect::from_min_size(
                point(x, y),
                Vec2::new(if right { x - 16. } else { width }, 18.) * s,
            );
            let clip = if right {
                Rect::from_min_max(point(125., y), point(x, y + 18.))
            } else {
                clip
            };
            p.with_clip_rect(clip).text(
                point(x, y + 8.),
                if right {
                    Align2::RIGHT_CENTER
                } else {
                    Align2::LEFT_CENTER
                },
                text,
                font,
                color,
            );
        };
        let heading = |title: &str, y: f32| {
            p.text(
                point(16., y + 8.),
                Align2::LEFT_CENTER,
                title,
                crate::skin::weighted_font(&p, 12. * s, true),
                INK,
            );
        };
        let row = |name: &str, value: &str, y: f32| {
            label(name, 16., y, false, false, MUTED);
            label(value, 258., y, true, true, INK);
        };
        heading("Firmware", 0.);
        line(131.);
        let identity = if self.player.firmware.is_null() {
            "—".to_owned()
        } else {
            format!(
                "{} {}",
                if self.player.firmware["original"] == true {
                    "Original"
                } else {
                    "Unverified"
                },
                self.player.firmware["version"]
                    .as_str()
                    .unwrap_or("Unknown")
            )
        };
        let hash = self.player.firmware["sha256"]
            .as_str()
            .map(|v| format!("{}…", v.chars().take(8).collect::<String>()))
            .unwrap_or_else(|| "—".into());
        let state = std::path::Path::new(&self.player.state)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("—");
        row("Image", &identity, 24.);
        row("SHA-256", &hash, 42.);
        row("State", state, 60.);
        let third = (width - 12.) / 3.;
        for (i, name) in ["Open…", "Restart", "Log"].iter().enumerate() {
            let button = Rect::from_min_size(
                point(16. + i as f32 * (third + 6.), 90.),
                Vec2::new(third, 30.) * s,
            );
            if crate::skin::host_button(ui, button, name, false, 12. * s).clicked() {
                match *name {
                    "Open…" => self.open_firmware(&ctx),
                    "Restart" => {
                        if self.bridge.is_none() {
                            self.start(&ctx);
                        } else {
                            self.command(json!({"action":"restart"}), &ctx);
                        }
                    }
                    _ => self.show_logs = true,
                }
            }
        }
        heading("Screen", 144.);
        line(275.);
        row(
            "Scale",
            &format!("{}:1 · 800 × 480", lcd.width() / 800.),
            168.,
        );
        let pointer = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| lcd.contains(*p))
            .and_then(|pos| {
                lcd_point(
                    [lcd.min.x, lcd.min.y, lcd.width(), lcd.height()],
                    [pos.x, pos.y],
                )
            })
            .map(|(x, y)| format!("{x}, {y}"))
            .unwrap_or_else(|| "—".into());
        row("Pointer", &pointer, 186.);
        row(
            "Last touch",
            &self
                .last_point
                .map(|(x, y)| format!("{x}, {y}"))
                .unwrap_or_else(|| "—".into()),
            204.,
        );
        if crate::skin::host_button(
            ui,
            Rect::from_min_size(point(16., 234.), Vec2::new(width, 30.) * s),
            "Save PNG",
            false,
            12. * s,
        )
        .clicked()
            && let Some((size, rgba)) = &self.framebuffer
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("xdj-screen.png")
                .add_filter("PNG", &["png"])
                .save_file()
            && let Err(e) = image::save_buffer(
                path,
                rgba,
                size[0] as u32,
                size[1] as u32,
                image::ColorType::Rgba8,
            )
        {
            self.player.status = format!("Cannot save screen: {e}");
        }
        heading("Panel lamps", 288.);
        label(
            if self.player.lamps_known {
                "Reported by firmware"
            } else {
                "Awaiting panel data"
            },
            16.,
            312.,
            false,
            false,
            MUTED,
        );
        line(433.);
        let lamps = [
            ("play", "Play", GREEN),
            ("cue", "Cue", AMBER),
            ("loop-in", "In", AMBER),
            ("loop-out", "Out", AMBER),
            ("reloop", "Reloop", AMBER),
            ("reverse", "Reverse", RED),
            ("vinyl", "Vinyl", BLUE),
            ("master-tempo", "M. tempo", RED),
            ("quantize", "Quantize", RED),
            ("usb", "USB", INK),
        ];
        for (i, (name, title, color)) in lamps.iter().enumerate() {
            let x = 16. + (i % 2) as f32 * 127.;
            let y = 332. + (i / 2) as f32 * 18.;
            let lit = self.player.lit.contains(*name);
            p.circle(
                point(x + 3., y + 8.),
                3. * s,
                if lit { *color } else { Color32::TRANSPARENT },
                Stroke::new(s, color.gamma_multiply(0.62)),
            );
            label(title, x + 12., y, false, false, INK);
            let state = if !self.player.lamps_known {
                "—"
            } else if self.player.flashing.contains(*name) {
                "flash"
            } else if lit {
                "on"
            } else {
                "off"
            };
            p.text(
                point(x + 115., y + 8.),
                Align2::RIGHT_CENTER,
                state,
                FontId::monospace(12. * s),
                MUTED,
            );
        }
        heading("Panel input", 446.);
        row("Tempo", &format!("{} / 65535", self.pitch), 470.);
        line(493.);
        line(569.);
        if self.input_log.is_empty() {
            label(
                "Press a control to inspect its input.",
                16.,
                506.,
                false,
                false,
                MUTED,
            );
        } else {
            for (i, entry) in self.input_log.iter().take(3).enumerate() {
                label(
                    entry,
                    16.,
                    506. + i as f32 * 18.,
                    false,
                    true,
                    if i == 0 { INK } else { MUTED },
                );
            }
        }
        heading("Audio", 582.);
        for (i, (name, key)) in [
            ("Output", "output"),
            ("DSP", "dsp"),
            ("Dropped frames", "dropped_frames"),
            ("Queue recoveries", "queue_recoveries"),
            ("Serializer underruns", "serializer_underruns"),
        ]
        .iter()
        .enumerate()
        {
            let value = match (*key, &self.player.audio[*key]) {
                ("output", _) if !self.player.audio.is_null() => {
                    if self.muted { "muted" } else { "on" }.into()
                }
                ("dsp", _) if self.player.audio["running"].is_boolean() => {
                    if self.player.audio["running"] == true {
                        "running"
                    } else {
                        "stopped"
                    }
                    .into()
                }
                (_, Value::Null) => "—".to_owned(),
                (_, Value::String(v)) => v.clone(),
                (_, v) => v.to_string(),
            };
            row(name, &value, 606. + i as f32 * 18.);
        }
    }
    fn deck(&mut self, ui: &mut egui::Ui) {
        let player = self.options.view == "player";
        let size = if player {
            Vec2::new(1065., 1336.)
        } else {
            Vec2::new(1280., 724.)
        };
        let available = ui.available_size();
        let scale = (available.x / size.x)
            .min(available.y / size.y)
            .min(self.options.ui_scale)
            .max(0.1);
        let origin = ui.min_rect().min + Vec2::new((available.x - size.x * scale).max(0.) / 2., 0.);
        let painter = ui.painter().clone();
        painter.rect_filled(Rect::from_min_size(origin, size * scale), 0, PAPER);
        self.rects.clear();
        let lcd = layout::box_rect(
            if player {
                [268.315, 75.79, 532.37, 319.42]
            } else {
                [24., 27., 800., 480.]
            },
            origin,
            scale,
        );
        self.rects.insert("lcd".into(), lcd);
        painter.rect_filled(lcd, 0, Color32::BLACK);
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                lcd,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                Color32::WHITE,
            );
        } else {
            painter.text(
                lcd.center(),
                Align2::CENTER_CENTER,
                if self.options.firmware.is_none() {
                    if cfg!(target_os = "macos") {
                        "Open firmware to start · ⌘F"
                    } else {
                        "Open firmware to start · Ctrl+F"
                    }
                } else {
                    &self.player.status
                },
                FontId::proportional(12. * scale),
                MUTED,
            );
        }
        painter.rect_stroke(
            lcd.expand(scale),
            0.,
            Stroke::new(scale, MUTED),
            egui::StrokeKind::Inside,
        );
        let response = ui.interact(lcd, ui.id().with("lcd"), Sense::drag());
        if self.player.ready {
            if response.is_pointer_button_down_on() {
                if let Some(pos) = response.interact_pointer_pos()
                    && let Some((x, y)) = lcd_point(
                        [lcd.min.x, lcd.min.y, lcd.width(), lcd.height()],
                        [pos.x, pos.y],
                    )
                {
                    self.last_touch = Some((x, y));
                    self.last_point = Some((x, y));
                    let command = self.input.touch(x, y, true);
                    self.emit(command);
                }
            } else if self.last_touch.take().is_some() {
                let command = self.input.end_touch();
                self.emit(command);
            }
        }
        for control in CONTROLS {
            let rect = layout::rect(
                if player {
                    control.player
                } else {
                    control.firmware
                },
                origin,
                scale,
            );
            self.rects.insert(control.name.into(), rect);
            let lit = self.player.lit.contains(if control.name == "load" {
                "selector"
            } else {
                control.name
            });
            let response = ui
                .interact(rect, ui.id().with(control.name), Sense::click_and_drag())
                .on_hover_text(format!("{} · {:?}", control.label, control.key));
            crate::skin::button(
                &painter,
                rect,
                control,
                self.pointer_held.contains(control.name) || self.key_held.contains(control.name),
                lit,
                player,
                scale,
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    self.player.ready,
                    control.label,
                )
            });
            let down = self.player.ready && response.is_pointer_button_down_on();
            if down != self.pointer_held.contains(control.name) {
                if down {
                    self.pointer_held.insert(control.name.into());
                } else {
                    self.pointer_held.remove(control.name);
                }
                let command = self
                    .input
                    .button(control.name, down || self.key_held.contains(control.name));
                self.emit(command);
            }
            if self.player.ready && response.clicked() && !ui.input(|i| i.pointer.any_released()) {
                let command = self.input.button(control.name, true);
                self.emit(command);
                let command = self.input.button(control.name, false);
                self.emit(command);
            }
            if control.name == "load" && response.hovered() && self.player.ready {
                let delta = ui.input(|i| i.smooth_scroll_delta().y);
                if delta.abs() > 0.1 {
                    let name = if delta > 0. {
                        "browse-up"
                    } else {
                        "browse-down"
                    };
                    self.emit(Some(json!({"action":"button","name":name,"down":true})));
                    self.emit(Some(json!({"action":"button","name":name,"down":false})));
                }
            }
        }
        let jog = layout::rect(
            if player {
                [530., 858., 719.31, 719.31]
            } else {
                [560., 614., 204., 204.]
            },
            origin,
            scale,
        );
        self.rects.insert("jog".into(), jog);
        crate::skin::jog(
            &painter,
            jog,
            &self.player.jog,
            self.jog_down,
            player,
            scale,
        );
        let response = ui
            .interact(jog, ui.id().with("jog"), Sense::drag())
            .on_hover_text("Drag the top to scratch; drag the rim to bend pitch. Scroll to turn.");
        if self.player.ready {
            let down = response.is_pointer_button_down_on();
            if let Some(point) = response.interact_pointer_pos() {
                let p = point - jog.center();
                let command = if down && !self.jog_down {
                    self.input.jog_begin([p.x, p.y], jog.width() / 2.)
                } else if down {
                    self.input.jog_move([p.x, p.y])
                } else {
                    self.input.jog_end()
                };
                self.emit(command);
            }
            if !down && self.jog_down {
                let command = self.input.jog_end();
                self.emit(command);
            }
            self.jog_down = down;
            if response.hovered() {
                let ticks = (ui.input(|i| i.smooth_scroll_delta().y) * 2.).round() as i32;
                if ticks != 0 {
                    self.emit(Some(
                        json!({"action":"jog","ticks":ticks.clamp(-4096,4096)}),
                    ));
                }
            }
        }
        let fader = layout::box_rect(
            if player {
                [949., 812., 72., 404.]
            } else {
                [916., 526., 60., 166.]
            },
            origin,
            scale,
        );
        self.rects.insert("fader".into(), fader);
        let track = crate::skin::fader(&painter, fader, self.pitch, scale);
        let response = ui.interact(fader, ui.id().with("tempo"), Sense::click_and_drag());
        response.widget_info(|| {
            egui::WidgetInfo::slider(self.player.ready, self.pitch as f64, "Tempo")
        });
        if self.player.ready {
            let mut value = self.pitch;
            if response.double_clicked() {
                value = 32768;
            } else if response.is_pointer_button_down_on()
                && let Some(pos) = response.interact_pointer_pos()
            {
                value = (((pos.y - track.top()) / track.height()).clamp(0., 1.) * 65535.).round()
                    as i32;
                if (value - 32768).abs() < 983 {
                    value = 32768;
                }
            }
            if value != self.pitch {
                self.pitch = value;
                self.emit(Some(json!({"action":"pitch","value":value})));
            }
        }
        let speed = layout::rect(
            if player {
                [990., 491., 40., 40.]
            } else {
                [742., 570., 42., 42.]
            },
            origin,
            scale,
        );
        self.rects.insert("vinyl-speed".into(), speed);
        let response = ui
            .interact(speed, ui.id().with("vinyl-speed"), Sense::drag())
            .on_hover_text("Vinyl speed adjust · drag");
        crate::skin::knob(&painter, speed, self.vinyl_speed, response.dragged(), scale);
        if self.player.ready && response.dragged() {
            let value = (self.vinyl_speed as f32 + response.drag_delta().x / scale)
                .round()
                .clamp(0., 255.) as i32;
            if value != self.vinyl_speed {
                self.vinyl_speed = value;
                self.emit(Some(json!({"action":"vinyl-speed","value":value})));
            }
        }
        crate::skin::usb(
            &painter,
            layout::rect(
                if player {
                    [167., 128., 78., 88.]
                } else {
                    [880., 396., 56., 62.]
                },
                origin,
                scale,
            ),
            self.player.usb,
            scale,
        );
        for &(x, y, label, font, dim, boxed) in if player {
            PLAYER_LEGENDS
        } else {
            FIRMWARE_LEGENDS
        } {
            let at = origin + Vec2::new(x, y) * scale;
            if boxed {
                let galley = painter.layout_no_wrap(
                    label.to_owned(),
                    FontId::proportional(font * scale),
                    INK,
                );
                painter.rect_stroke(
                    Rect::from_center_size(at, galley.size() + Vec2::new(8., 2.) * scale),
                    2. * scale,
                    Stroke::new(scale, LINE),
                    egui::StrokeKind::Inside,
                );
            }
            if dim {
                crate::skin::text(&painter, at, label, font * scale, MUTED);
            } else {
                crate::skin::medium_text(&painter, at, label, font * scale, INK);
            }
        }
        let (x, y, count) = if player {
            (939., 844., 21)
        } else {
            (904., 539., 11)
        };
        let _ = y;
        for tick in 0..count {
            let y = fader.top() + fader.height() * (0.08 + 0.84 * tick as f32 / (count - 1) as f32);
            let a = Pos2::new(
                origin.x + (x + if tick % 5 == 0 { 0. } else { 4. }) * scale,
                y,
            );
            painter.line_segment(
                [
                    a,
                    a + Vec2::new(if tick % 5 == 0 { 9. } else { 5. } * scale, 0.),
                ],
                Stroke::new(1.1 * scale, LINE),
            );
        }
        if player {
            for y in [812., 900.] {
                painter.rect_stroke(
                    layout::box_rect([22., y - 30., 118., 60.], origin, scale),
                    30. * scale,
                    Stroke::new(scale, LINE),
                    egui::StrokeKind::Inside,
                );
            }
            for (x, length) in [(114., 24.), (202., 50.)] {
                let a = origin + Vec2::new(x, 491.) * scale;
                painter.line_segment(
                    [a, a + Vec2::new(length * scale, 0.)],
                    Stroke::new(scale, LINE),
                );
            }
            painter.circle_filled(
                origin + Vec2::new(922., 62.) * scale,
                3. * scale,
                RED.gamma_multiply(0.35),
            );
            for (start, end) in [(141., 128.), (39., 52.)] {
                let radius = jog.width() / 2. + 62. * scale;
                let point = |a: f32| {
                    jog.center() + Vec2::new(a.to_radians().cos(), a.to_radians().sin()) * radius
                };
                let points = (0..=20)
                    .map(|i| point(start + (end - start) * i as f32 / 20.))
                    .collect();
                painter.add(egui::Shape::line(points, Stroke::new(scale, LINE)));
                let tip = point(end);
                let back = point(end - (end - start) * 0.2);
                let normal = (tip - jog.center()).normalized() * 7. * scale;
                painter.add(egui::Shape::line(
                    vec![back + normal, tip, back - normal],
                    Stroke::new(scale, LINE),
                ));
            }
        }
        if !player && self.show_inspector {
            self.inspector(
                ui,
                layout::box_rect([1014., 8., 258., 708.], origin, scale),
                scale,
                lcd,
            );
        }
        if self.show_keys {
            for control in CONTROLS {
                let r = self.rects[control.name];
                let label = match control.name {
                    "back" => "Esc".to_owned(),
                    "previous" => ",".into(),
                    "next" => ".".into(),
                    "search-back" => "[".into(),
                    "search-forward" => "]".into(),
                    "load" => "↑ ↓ turn · ⏎ press".into(),
                    _ => format!("{:?}", control.key),
                };
                let centered =
                    ["back", "tag", "track-filter", "shortcut", "load"].contains(&control.name);
                let at = if centered {
                    r.center()
                } else {
                    r.center_bottom() + Vec2::new(0., 10.)
                };
                crate::skin::key_tag(&painter, at, &label);
            }
            crate::skin::key_tag(&painter, speed.center_bottom() + Vec2::new(0., 10.), "drag");
            crate::skin::key_tag(
                &painter,
                jog.center_bottom() - Vec2::new(0., 26.),
                "J touch · H / N turn",
            );
            crate::skin::key_tag(
                &painter,
                fader.center_bottom() - Vec2::new(0., 28.),
                "PgUp\nPgDn\nHome",
            );
        }
        ui.allocate_space(size * scale);
    }
}
impl eframe::App for DeckApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let start = Instant::now();
        self.poll(&ctx);
        self.show(ui);
        self.metrics.paints += 1;
        if self.metrics.ui_us.len() < 60000 {
            self.metrics.ui_us.push(start.elapsed().as_micros() as u64);
        }
    }
    fn on_exit(&mut self) {
        self.release();
        if let Some(dir) = &self.options.diagnostics {
            let _ = fs::write(dir.join("metrics.json"), self.report().to_string());
        }
        self.display = None;
        self.bridge = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portrait_button_labels_do_not_split_words() {
        let ctx = egui::Context::default();
        let mut app = DeckApp::offline(Options {
            view: "player".into(),
            ..Options::default()
        });
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 798.))),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        let mut labels = Vec::new();
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                let label = text.galley.text();
                if [
                    "Quantize", "Shortcut", "Range", "Back", "Tag", "Load", "VINYL", "Out", "CUE",
                    "QZ", "SC", "U", "D", "MT", "P", "↶", "+", "F",
                ]
                .contains(&label)
                {
                    labels.push((label.to_owned(), text.galley.rows.len()));
                }
            }
        }
        output.textures_delta.clear();
        assert!(labels.iter().any(|(label, _)| label == "CUE"));
        assert!(
            labels.iter().all(|(_, rows)| *rows == 1),
            "Button words must remain legible: {labels:?}"
        );
    }
    #[test]
    fn player_view_resizes_to_the_hardware_aspect_and_keys_overlay_toggles() {
        let ctx = egui::Context::default();
        let mut app = DeckApp::offline(Options::default());
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 798.))),
                ..Default::default()
            },
            |ui| {
                app.command(json!({"action":"view","view":"player"}), ui.ctx());
                app.command(json!({"action":"show-keys","visible":true}), ui.ctx());
            },
        );
        let resize = output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .find_map(|command| match command {
                egui::ViewportCommand::InnerSize(size) => Some(*size),
                _ => None,
            });
        output.textures_delta.clear();
        let size = resize.expect("Switching to player view should fit the hardware faceplate");
        assert!((size.x / (size.y - 74.) - 1065. / 1336.).abs() < 0.001);
        assert!(app.show_keys);
    }
    #[test]
    fn shortcut_overlay_uses_reference_key_names_and_jog_help() {
        let ctx = egui::Context::default();
        let mut app = DeckApp::offline(Options::default());
        app.show_keys = true;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 798.))),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        output.textures_delta.clear();
        for text in [
            "[",
            "]",
            "Esc",
            "↑ ↓ turn · ⏎ press",
            "J touch · H / N turn",
            "PgUp\nPgDn\nHome",
        ] {
            assert!(
                labels.iter().any(|l| l == text),
                "Missing reference shortcut {text}"
            );
        }
        assert!(
            !labels
                .iter()
                .any(|l| l == "OpenBracket" || l == "CloseBracket")
        );
    }
    #[test]
    fn dirty_frames_upload_only_damage_and_keep_the_full_export_image() {
        let ctx = egui::Context::default();
        let mut app = DeckApp::offline(Options::default());
        let mut fb = crate::rfb::Framebuffer::new(3, 2).unwrap();
        app.latest.publish(&mut fb, 1);
        let mut first = ctx.run_ui(egui::RawInput::default(), |ui| app.poll(ui.ctx()));
        let texture = app.texture.as_ref().unwrap().id();
        first.textures_delta.clear();
        fb.raw(2, 1, 1, 1, &[0, 0, 255, 0]).unwrap();
        app.latest.publish(&mut fb, 2);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.poll(ui.ctx()));
        let delta = output.textures_delta.set[&texture].last().unwrap();
        assert_eq!(delta.pos, Some([2, 1]));
        assert_eq!(delta.image.size(), [1, 1]);
        let (size, pixels) = app.framebuffer.as_ref().unwrap();
        assert_eq!(*size, [3, 2]);
        assert_eq!(&pixels[20..24], &[255, 0, 0, 255]);
        assert_eq!(&pixels[..4], &[0, 0, 0, 255]);
        output.textures_delta.clear();
    }
    #[test]
    fn consecutive_frames_update_the_existing_gpu_texture() {
        let ctx = egui::Context::default();
        let mut app = DeckApp::offline(Options::default());
        for (sequence, size, partial) in [(1, [2, 1], false), (2, [2, 1], true), (3, [3, 1], false)]
        {
            app.latest.offer(Frame {
                size,
                patch: crate::rfb::Damage {
                    origin: [0, 0],
                    size,
                },
                rgba: vec![255; size[0] * size[1] * 4],
                received: Instant::now(),
                sequence,
            });
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.poll(ui.ctx()));
            let texture = app.texture.as_ref().unwrap().id();
            let delta = output.textures_delta.set[&texture].last().unwrap().clone();
            output.textures_delta.clear();
            assert_eq!(
                delta.pos.is_some(),
                partial,
                "Reuse allocation when frame dimensions match"
            );
            assert_eq!(
                delta.options.magnification,
                egui::TextureFilter::Linear,
                "Enlarged LCD text must interpolate instead of duplicating jagged pixels"
            );
            assert_eq!(app.framebuffer.as_ref().unwrap().0, size);
            assert_eq!(
                app.framebuffer.as_ref().unwrap().1,
                vec![255; size[0] * size[1] * 4]
            );
            output.textures_delta.clear();
        }
    }
    #[test]
    fn scripted_touch_hold_survives_repaints_until_explicit_release() {
        let ctx = egui::Context::default();
        let mut app = DeckApp::offline(Options::default());
        app.player.event(&json!({"event":"running"}));
        app.command(json!({"action":"touch","x":30,"y":40,"down":true}), &ctx);
        app.take_commands();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 798.))),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        out.textures_delta.clear();
        assert!(
            app.take_commands().is_empty(),
            "A repaint must not release a diagnostic held touch"
        );
        app.command(json!({"action":"touch","x":30,"y":40,"down":false}), &ctx);
        assert_eq!(app.take_commands()[0]["down"], false);
    }
    #[test]
    fn scripted_controls_survive_focus_loss_until_explicit_release() {
        for command in [
            json!({"after":0,"action":"button","name":"load","down":true}),
            json!({"after":0,"action":"touch","x":30,"y":40,"down":true}),
        ] {
            let ctx = egui::Context::default();
            let mut app = DeckApp::offline(Options::default());
            app.player.event(&json!({"event":"running"}));
            app.command(command.clone(), &ctx);
            assert_eq!(app.take_commands().len(), 1);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused: false,
                    ..Default::default()
                },
                |ui| app.keyboard(ui.ctx()),
            );
            output.textures_delta.clear();
            assert!(
                app.take_commands().is_empty(),
                "Focus loss cancelled a scripted control"
            );
            let mut up = command;
            up["down"] = json!(false);
            app.command(up, &ctx);
            assert_eq!(app.take_commands()[0]["down"], false);
        }
    }
    #[test]
    fn diagnostic_pitch_updates_the_visible_fader() {
        let ctx = egui::Context::default();
        let mut app = DeckApp::offline(Options::default());
        app.player.event(&json!({"event":"running"}));
        app.command(json!({"action":"pitch","value":60000}), &ctx);
        assert_eq!(app.pitch, 60000);
    }
}
