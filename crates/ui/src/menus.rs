//! One command tree for the macOS menu bar and the in-window desktop menus.
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub shortcut: Option<&'static str>,
    pub action: Value,
    pub children: Vec<Entry>,
}
impl Entry {
    pub fn item(label: impl Into<String>, action: Value) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            checked: None,
            shortcut: None,
            action,
            children: Vec::new(),
        }
    }
    pub fn group(label: &str, children: Vec<Self>) -> Self {
        let mut entry = Self::item(label, Value::Null);
        entry.children = children;
        entry
    }
    pub fn empty(label: &str) -> Self {
        let mut entry = Self::item(label, Value::Null);
        entry.enabled = false;
        entry
    }
    pub fn shortcut(mut self, key: &'static str) -> Self {
        self.shortcut = Some(key);
        self
    }
    pub fn checked(mut self, value: bool) -> Self {
        self.checked = Some(value);
        self
    }
    pub fn enabled(mut self, value: bool) -> Self {
        self.enabled = value;
        self
    }
    pub fn separator() -> Self {
        Self::item("", Value::Null)
    }
}

pub struct Recent {
    file: PathBuf,
    pub paths: Vec<PathBuf>,
}
impl Recent {
    pub fn load(file: PathBuf) -> Self {
        let paths = fs::read(&file)
            .ok()
            .filter(|bytes| bytes.len() <= 1_048_576)
            .and_then(|bytes| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
            .unwrap_or_default();
        let mut recent = Self {
            file,
            paths: Vec::new(),
        };
        for path in paths.into_iter().filter(|p| p.is_absolute()) {
            if !recent.paths.contains(&path) {
                recent.paths.push(path);
            }
            if recent.paths.len() == 10 {
                break;
            }
        }
        recent
    }
    pub fn remember(&mut self, path: &Path) -> Result<()> {
        // Remember the selected source, never its imported cache copy.
        let path = path.canonicalize()?;
        self.paths.retain(|p| p != &path);
        self.paths.insert(0, path);
        self.paths.truncate(10);
        self.save()
    }
    pub fn clear(&mut self) -> Result<()> {
        self.paths.clear();
        self.save()
    }
    fn save(&self) -> Result<()> {
        let parent = self.file.parent().expect("Recent file has a parent");
        fs::create_dir_all(parent)?;
        let temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(temp.as_file(), &self.paths)?;
        temp.persist(&self.file)?;
        Ok(())
    }
    pub fn entries(&self) -> Vec<Entry> {
        if self.paths.is_empty() {
            return vec![Entry::empty("No recent firmware")];
        }
        let mut entries: Vec<_> = self
            .paths
            .iter()
            .map(|path| {
                let label = format!(
                    "{} — {}{}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    path.parent().unwrap_or(Path::new("")).display(),
                    if path.is_file() { "" } else { " (missing)" }
                );
                Entry::item(label, json!({"action":"firmware","path":path})).enabled(path.is_file())
            })
            .collect();
        entries.push(Entry::separator());
        entries.push(Entry::item(
            "Clear Recent Firmware",
            json!({"action":"clear-recent"}),
        ));
        entries
    }
}

pub fn draw(ui: &mut eframe::egui::Ui, entries: &[Entry]) -> Option<Value> {
    let mut selected = None;
    for entry in entries {
        if !entry.children.is_empty() {
            ui.menu_button(&entry.label, |ui| {
                if let Some(action) = draw(ui, &entry.children) {
                    selected = Some(action);
                }
            });
        } else if entry.label.is_empty() {
            ui.separator();
        } else {
            let button = if let Some(checked) = entry.checked {
                eframe::egui::Button::selectable(checked, &entry.label)
            } else {
                eframe::egui::Button::new(&entry.label)
            };
            if ui.add_enabled(entry.enabled, button).clicked() {
                selected = Some(entry.action.clone());
                ui.close();
            }
        }
    }
    selected
}

#[cfg(target_os = "macos")]
pub mod native {
    use super::*;
    use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use std::{
        collections::HashMap,
        sync::mpsc::{self, Receiver},
    };
    pub struct NativeMenu {
        menu: Menu,
        entries: Vec<Entry>,
        actions: HashMap<String, Value>,
        events: Receiver<String>,
    }
    impl NativeMenu {
        pub fn new(ctx: &eframe::egui::Context) -> Self {
            let menu = Menu::new();
            let (sender, events) = mpsc::channel();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let _ = sender.send(event.id.0);
                ctx.request_repaint();
            }));
            Self {
                menu,
                entries: Vec::new(),
                actions: HashMap::new(),
                events,
            }
        }
        pub fn sync(&mut self, entries: Vec<Entry>) -> Result<()> {
            if entries == self.entries {
                return Ok(());
            }
            while self.menu.remove_at(0).is_some() {}
            self.actions.clear();
            let app = Submenu::new("XDJ-1000MK2", true);
            app.append_items(&[
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
            ])?;
            let quit = MenuItem::with_id(
                "quit",
                "Quit XDJ-1000MK2 Emulator",
                true,
                Some("CmdOrCtrl+Q".parse()?),
            );
            app.append(&quit)?;
            self.actions.insert("quit".into(), json!({"action":"quit"}));
            self.menu.append(&app)?;
            for entry in &entries {
                let submenu = Submenu::new(&entry.label, entry.enabled);
                self.append(&submenu, &entry.children)?;
                self.menu.append(&submenu)?;
            }
            self.menu.init_for_nsapp();
            self.entries = entries;
            Ok(())
        }
        fn append(&mut self, menu: &Submenu, entries: &[Entry]) -> Result<()> {
            for entry in entries {
                if !entry.children.is_empty() {
                    let submenu = Submenu::new(&entry.label, entry.enabled);
                    self.append(&submenu, &entry.children)?;
                    menu.append(&submenu)?;
                } else if entry.label.is_empty() {
                    menu.append(&PredefinedMenuItem::separator())?;
                } else {
                    let id = format!("xdj-{}", self.actions.len());
                    let accelerator = entry.shortcut.map(str::parse).transpose()?;
                    if let Some(checked) = entry.checked {
                        menu.append(&CheckMenuItem::with_id(
                            &id,
                            &entry.label,
                            entry.enabled,
                            checked,
                            accelerator,
                        ))?;
                    } else {
                        menu.append(&MenuItem::with_id(
                            &id,
                            &entry.label,
                            entry.enabled,
                            accelerator,
                        ))?;
                    }
                    self.actions.insert(id, entry.action.clone());
                }
            }
            Ok(())
        }
        pub fn installed(&self) -> Value {
            use objc2::MainThreadMarker;
            use objc2_app_kit::{NSApplication, NSMenu};
            fn entries(menu: &NSMenu) -> Value {
                json!(
                    menu.itemArray()
                        .iter()
                        .map(|item| {
                            json!({"label":item.title().to_string(),"enabled":item.isEnabled(),
                        "shortcut":item.keyEquivalent().to_string(),
                        "children":item.submenu().map(|menu| entries(&menu)).unwrap_or(json!([]))})
                        })
                        .collect::<Vec<_>>()
                )
            }
            let application = NSApplication::sharedApplication(
                MainThreadMarker::new().expect("Native menus use the main thread"),
            );
            application
                .mainMenu()
                .map(|menu| entries(&menu))
                .unwrap_or(Value::Null)
        }
        /// Exercise the installed AppKit action target during owned diagnostics.
        pub fn invoke(&self, path: &[String]) -> Result<()> {
            use objc2::MainThreadMarker;
            use objc2_app_kit::NSApplication;
            let application = NSApplication::sharedApplication(
                MainThreadMarker::new().expect("Native menus use the main thread"),
            );
            let mut menu = application
                .mainMenu()
                .ok_or_else(|| anyhow::anyhow!("No native menu installed"))?;
            for (depth, title) in path.iter().enumerate() {
                let items = menu.itemArray();
                let (index, item) = items
                    .iter()
                    .enumerate()
                    .find(|(_, item)| item.title().to_string() == *title)
                    .ok_or_else(|| anyhow::anyhow!("Missing menu item: {title}"))?;
                anyhow::ensure!(item.isEnabled(), "Menu item is disabled: {title}");
                if depth == path.len() - 1 {
                    menu.performActionForItemAtIndex(index as isize);
                    return Ok(());
                }
                menu = item
                    .submenu()
                    .ok_or_else(|| anyhow::anyhow!("Not a submenu: {title}"))?;
            }
            anyhow::bail!("Empty menu path")
        }
        pub fn pending(&self) -> Vec<Value> {
            self.events
                .try_iter()
                .filter_map(|id| self.actions.get(&id).cloned())
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recents_persist_deduplicate_bound_and_clear_without_touching_firmware() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("history/recent.json");
        let mut recent = Recent::load(file.clone());
        assert_eq!(recent.entries()[0].label, "No recent firmware");
        for i in 0..12 {
            let path = dir.path().join(format!("{i}.UPD"));
            fs::write(&path, b"firmware").unwrap();
            recent.remember(&path).unwrap();
        }
        let first = recent.paths[0].clone();
        recent.remember(&first).unwrap();
        let mut restored = Recent::load(file.clone());
        assert_eq!(restored.paths.len(), 10);
        assert_eq!(restored.paths[0], first);
        assert_eq!(fs::read(&first).unwrap(), b"firmware");
        fs::remove_file(first).unwrap();
        assert!(!restored.entries()[0].enabled);
        assert!(restored.entries()[0].label.ends_with(" (missing)"));
        restored.clear().unwrap();
        assert!(Recent::load(file).paths.is_empty());
    }
    #[test]
    fn malformed_and_relative_history_is_not_offered() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recent.json");
        fs::write(&file, b"corrupt").unwrap();
        assert!(Recent::load(file.clone()).paths.is_empty());
        let absolute = dir.path().canonicalize().unwrap().join("absolute.upd");
        fs::write(
            &file,
            json!(["relative.upd", absolute, absolute]).to_string(),
        )
        .unwrap();
        assert_eq!(Recent::load(file).paths, [absolute]);
    }
}
