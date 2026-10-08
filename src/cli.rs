use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
#[derive(Debug, Default, Clone)]
pub struct Args {
    pub command: String,
    pub values: BTreeMap<String, Vec<String>>,
    pub flags: BTreeSet<String>,
    pub positional: Vec<String>,
}
impl Args {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut it = arguments.into_iter();
        let command = it.next().unwrap_or_else(|| "help".into());
        let mut result = Self {
            command,
            ..Self::default()
        };
        while let Some(arg) = it.next() {
            if let Some(name) = arg.strip_prefix("--") {
                if [
                    "open",
                    "mute-audio",
                    "unmute-audio",
                    "validate",
                    "skip-preparation",
                    "write-header",
                    "check-header",
                    "force",
                ]
                .contains(&name)
                {
                    result.flags.insert(name.to_owned());
                } else if [
                    "root",
                    "engine",
                    "qemu-data",
                    "python",
                    "firmware",
                    "state",
                    "state-root",
                    "usb-image",
                    "image",
                    "audio",
                    "diagnostics",
                    "dsp-lag-cap-ms",
                    "cache-donor-root",
                    "out",
                    "seconds",
                    "jobs",
                    "wav-image",
                    "mp3-image",
                    "warm-seconds",
                    "boot-seconds",
                    "compile-mode",
                    "audio-root",
                ]
                .contains(&name)
                {
                    result.values.entry(name.into()).or_default().push(
                        it.next()
                            .with_context(|| format!("Missing --{name} value"))?,
                    );
                } else {
                    bail!("Unknown option --{name}");
                }
            } else {
                result.positional.push(arg);
            }
        }
        if result.flags.contains("mute-audio") && result.flags.contains("unmute-audio") {
            bail!("Choose one audio monitoring mode");
        }
        Ok(result)
    }
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name)?.last().map(String::as_str)
    }
    pub fn path(&self, name: &str) -> Option<PathBuf> {
        self.value(name).map(PathBuf::from)
    }
    pub fn required(&self, name: &str) -> Result<PathBuf> {
        self.path(name)
            .with_context(|| format!("--{name} is required"))
    }
    pub fn number(&self, name: &str, default: u64) -> Result<u64> {
        match self.value(name) {
            Some(s) => s.parse().with_context(|| format!("Invalid --{name}")),
            None => Ok(default),
        }
    }
    pub fn root(&self) -> Result<PathBuf> {
        self.path("root")
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
            .canonicalize()
            .context("Repository root does not exist")
    }
    pub fn paths(&self, name: &str) -> Vec<PathBuf> {
        self.values
            .get(name)
            .map(|v| v.iter().map(PathBuf::from).collect())
            .unwrap_or_default()
    }
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }
}
pub fn options(args: &Args, root: &Path) -> Result<crate::runtime::Options> {
    let mut options = crate::runtime::Options::at(root);
    let selection = root.join("output/engine.json");
    if selection.exists() && (args.value("engine").is_none() || args.value("qemu-data").is_none()) {
        let selected = crate::util::read_json(&selection)?;
        for (name, target) in [
            ("engine", &mut options.engine),
            ("qemu_data", &mut options.data),
        ] {
            let flag = if name == "engine" {
                "engine"
            } else {
                "qemu-data"
            };
            if args.value(flag).is_none() {
                *target = PathBuf::from(
                    selected[name]
                        .as_str()
                        .context("Incomplete engine selection")?,
                )
                .canonicalize()
                .context("Selected engine build is missing; run build-engine")?;
            }
        }
    }
    if let Some(p) = args.path("engine") {
        options.engine = p
            .canonicalize()
            .context("Emulator engine does not exist; run build-engine")?;
    }
    if let Some(p) = args.path("qemu-data") {
        options.data = p.canonicalize()?;
    }
    if let Some(p) = args.path("python") {
        options.python = p;
    }
    options.audio = args.value("audio").unwrap_or("coreaudio").into();
    options.muted = !args.flag("unmute-audio") || args.path("diagnostics").is_some();
    options.lag_cap_ms = args.number("dsp-lag-cap-ms", 300)?;
    options.mode = args.value("compile-mode").unwrap_or("adaptive").into();
    Ok(options)
}
