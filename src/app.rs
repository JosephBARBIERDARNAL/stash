//! The stash window.
//!
//! Accessibility: every widget is reachable with Tab / Shift-Tab and exposed to
//! screen readers (AccessKit); custom widgets set their own accessible names.
//! Status is shown by shape as well as color. ⌘/Ctrl +/− zooms and is remembered.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Modal, Modifiers,
    Painter, Pos2, Rect, Response, RichText, Role, ScrollArea, Sense, Stroke, StrokeKind, TextEdit,
    ThemePreference, Ui, WidgetInfo, vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use serde::{Deserialize, Serialize};

use crate::catalog::{self, Entry, Install};
use crate::ops;
use crate::skill::{self, SKILL_FILE, Severity, Skill, Status};
use crate::sources::{self, Source};
use crate::theme::{self, Palette, SMALL};

const MAX_VIEW_BYTES: u64 = 512 * 1024;
const MAX_FILES: usize = 300;
const STATUS_TTL: Duration = Duration::from_secs(6);

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Settings {
    /// `None` follows the system theme.
    dark: Option<bool>,
    /// Folder new skills are installed into.
    target: Option<PathBuf>,
    /// UI zoom (⌘/Ctrl +/−).
    zoom: Option<f32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AddTab {
    Folder,
    Git,
    New,
}

/// Skills found in a picked folder or a cloned repository, ready to install.
struct Candidates {
    /// Keeps a cloned repository alive until the dialog closes.
    _tmp: Option<tempfile::TempDir>,
    skills: Vec<(Skill, bool)>,
}

impl Candidates {
    fn from_dirs(dirs: Vec<PathBuf>, tmp: Option<tempfile::TempDir>) -> Self {
        let skills = dirs
            .into_iter()
            .map(|d| {
                let s = Skill::load(&d, false);
                let ok = s.status() != Status::Invalid;
                (s, ok)
            })
            .collect();
        Self { _tmp: tmp, skills }
    }
}

struct AddDialog {
    tab: AddTab,
    url: String,
    name: String,
    description: String,
    candidates: Option<Candidates>,
    cloning: Option<Receiver<Result<Candidates, String>>>,
    error: Option<String>,
}

impl AddDialog {
    fn new(tab: AddTab) -> Self {
        Self {
            tab,
            url: String::new(),
            name: String::new(),
            description: String::new(),
            candidates: None,
            cloning: None,
            error: None,
        }
    }
}

struct RemoveDialog {
    dir: PathBuf,
    name: String,
    provider: &'static str,
    link: Option<PathBuf>,
}

enum Dialog {
    Add(Box<AddDialog>),
    Remove(RemoveDialog),
}

enum Content {
    Text(String),
    Binary,
    TooLarge(u64),
    Error(String),
}

/// The files of the viewed install and the one being shown.
struct Viewer {
    dir: PathBuf,
    files: Vec<PathBuf>,
    open: PathBuf,
    content: Content,
}

impl Viewer {
    fn new(dir: &Path) -> Self {
        let files = ops::list_files(dir, MAX_FILES);
        let open = files.first().cloned().unwrap_or_else(|| SKILL_FILE.into());
        let content = read_content(&dir.join(&open));
        Self {
            dir: dir.to_path_buf(),
            files,
            open,
            content,
        }
    }

    fn open(&mut self, file: PathBuf) {
        self.content = read_content(&self.dir.join(&file));
        self.open = file;
    }
}

fn read_content(path: &Path) -> Content {
    match fs::metadata(path) {
        Ok(m) if m.len() > MAX_VIEW_BYTES => Content::TooLarge(m.len()),
        Ok(_) => match fs::read(path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) if !text.contains('\0') => Content::Text(text),
                _ => Content::Binary,
            },
            Err(e) => Content::Error(e.to_string()),
        },
        Err(e) => Content::Error(e.to_string()),
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
}

/// Replace the home folder with `~` for display.
fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

fn status_words(status: Status) -> &'static str {
    match status {
        Status::Valid => "valid",
        Status::Warnings => "valid with warnings",
        Status::Invalid => "invalid",
    }
}

pub struct StashApp {
    ctx: egui::Context,
    home: PathBuf,
    entries: Vec<Entry>,
    /// Folders new skills can go to: (label, path).
    targets: Vec<(&'static str, PathBuf)>,
    settings: Settings,
    /// Name of the selected skill.
    selected: Option<String>,
    /// Install (folder) of the selected skill being viewed.
    active: Option<PathBuf>,
    query: String,
    viewer: Option<Viewer>,
    rendered: bool,
    md_cache: CommonMarkCache,
    dialog: Option<Dialog>,
    status: Option<(String, bool, Instant)>,
    focused: bool,
    focus_search: bool,
    scroll_to_selected: bool,
}

impl StashApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        theme::install(&ctx);
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        if let Some(dark) = settings.dark {
            ctx.set_theme(if dark {
                ThemePreference::Dark
            } else {
                ThemePreference::Light
            });
        }
        if let Some(zoom) = settings.zoom {
            ctx.set_zoom_factor(zoom.clamp(0.5, 3.0));
        }
        let home = dirs::home_dir().unwrap_or_default();
        let targets = sources::known_sources(&home)
            .into_iter()
            .filter(Source::writable)
            .map(|s| (s.label, s.root))
            .collect();
        let mut app = Self {
            ctx,
            home,
            entries: Vec::new(),
            targets,
            settings,
            selected: None,
            active: None,
            query: String::new(),
            viewer: None,
            rendered: true,
            md_cache: CommonMarkCache::default(),
            dialog: None,
            status: None,
            focused: true,
            focus_search: false,
            scroll_to_selected: false,
        };
        app.refresh();
        if let Some(first) = app.entries.first() {
            let name = first.name.clone();
            app.select(name, None);
        }
        app
    }

    fn refresh(&mut self) {
        self.entries = catalog::group(sources::discover(&self.home));
        match self.selected.clone() {
            Some(name) if self.entry(&name).is_some() => self.select(name, self.active.clone()),
            _ => {
                self.selected = None;
                self.active = None;
            }
        }
        // Reload the viewed file, keeping the position.
        if let Some(v) = &self.viewer {
            let (dir, open) = (v.dir.clone(), v.open.clone());
            let mut fresh = Viewer::new(&dir);
            if fresh.files.contains(&open) {
                fresh.open(open);
            }
            self.viewer = Some(fresh);
        }
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// The selected skill and the install being viewed.
    fn current(&self) -> Option<(&Entry, &Install)> {
        let entry = self.entry(self.selected.as_deref()?)?;
        let install = entry
            .installs
            .iter()
            .find(|i| Some(&i.skill.dir) == self.active.as_ref())
            .or_else(|| entry.installs.first())?;
        Some((entry, install))
    }

    /// Select a skill by name, viewing `dir` if it is one of its installs.
    fn select(&mut self, name: String, dir: Option<PathBuf>) {
        let Some(entry) = self.entry(&name) else {
            return;
        };
        let active = dir
            .filter(|d| entry.installs.iter().any(|i| &i.skill.dir == d))
            .or_else(|| entry.installs.first().map(|i| i.skill.dir.clone()));
        self.selected = Some(name);
        self.active = active;
    }

    /// Select whichever skill has an install at `dir`.
    fn select_dir(&mut self, dir: PathBuf) {
        let name = self
            .entries
            .iter()
            .find(|e| e.installs.iter().any(|i| i.skill.dir == dir))
            .map(|e| e.name.clone());
        if let Some(name) = name {
            self.select(name, Some(dir));
            self.scroll_to_selected = true;
        }
    }

    fn visible(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        (0..self.entries.len())
            .filter(|&i| self.entries[i].matches(&query))
            .collect()
    }

    /// Move the selection up or down the visible list.
    fn step(&mut self, delta: isize) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        let pos = visible
            .iter()
            .position(|&i| Some(&self.entries[i].name) == self.selected.as_ref());
        let next = match pos {
            Some(p) => (p as isize + delta).clamp(0, visible.len() as isize - 1) as usize,
            None => 0,
        };
        let name = self.entries[visible[next]].name.clone();
        self.select(name, None);
        self.scroll_to_selected = true;
    }

    fn target(&self) -> PathBuf {
        self.settings
            .target
            .clone()
            .filter(|t| self.targets.iter().any(|(_, p)| p == t))
            .or_else(|| {
                // Default: the first tool folder that already exists.
                self.targets
                    .iter()
                    .find(|(_, p)| p.is_dir())
                    .map(|(_, p)| p.clone())
            })
            .unwrap_or_else(|| self.targets[0].1.clone())
    }

    fn notify(&mut self, msg: impl Into<String>, error: bool) {
        self.status = Some((msg.into(), error, Instant::now()));
    }

    fn set_dark(&mut self, dark: bool) {
        self.settings.dark = Some(dark);
        self.ctx.set_theme(if dark {
            ThemePreference::Dark
        } else {
            ThemePreference::Light
        });
    }

    fn open_add(&mut self) {
        self.dialog = Some(Dialog::Add(Box::new(AddDialog::new(AddTab::Folder))));
    }

    fn shortcuts(&mut self) {
        let ctx = self.ctx.clone();
        let cmd = |key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, key));
        if cmd(Key::R) {
            self.refresh();
            self.notify("Rescanned", false);
        }
        if self.dialog.is_some() {
            return;
        }
        if cmd(Key::F) {
            self.focus_search = true;
        }
        if cmd(Key::N) {
            self.open_add();
        }
        // Arrow keys move through the list, also while typing in search.
        let search_focused = ctx.memory(|m| m.focused()) == Some(search_id());
        if search_focused || !ctx.egui_wants_keyboard_input() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                self.step(1);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                self.step(-1);
            }
        }
    }

    // ---------------------------------------------------------------- top bar

    fn top_bar(&mut self, ui: &mut Ui, p: &Palette) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("stash")
                    .size(19.0)
                    .family(theme::bold())
                    .color(p.text),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dark = ui.visuals().dark_mode;
                let (icon, label) = if dark {
                    ("☀", "Switch to light mode")
                } else {
                    ("☾", "Switch to dark mode")
                };
                if icon_button(ui, icon, label).clicked() {
                    self.set_dark(!dark);
                }
                if icon_button(ui, "⟳", "Rescan skills (⌘R)").clicked() {
                    self.refresh();
                    self.notify("Rescanned", false);
                }
                if ui
                    .button("+  Add")
                    .on_hover_text("Add a skill (⌘N)")
                    .clicked()
                {
                    self.open_add();
                }
            });
        });
    }

    // ---------------------------------------------------------------- sidebar

    fn sidebar(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(4.0);
        let search = ui.add(
            TextEdit::singleline(&mut self.query)
                .id(search_id())
                .hint_text("Search")
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(8, 6)),
        );
        if std::mem::take(&mut self.focus_search) {
            search.request_focus();
        }
        if search.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
            self.query.clear();
        }
        ui.add_space(8.0);

        let visible = self.visible();
        let scroll = std::mem::take(&mut self.scroll_to_selected);
        let mut clicked = None;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for &i in &visible {
                let entry = &self.entries[i];
                let selected = self.selected.as_ref() == Some(&entry.name);
                let row = skill_row(ui, p, entry, selected);
                if selected && scroll {
                    row.scroll_to_me(Some(Align::Center));
                }
                if row.clicked() {
                    clicked = Some(entry.name.clone());
                }
            }
            if visible.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    let msg = if self.entries.is_empty() {
                        "No skills installed yet"
                    } else {
                        "No matches"
                    };
                    ui.label(RichText::new(msg).color(p.weak));
                });
            }
        });
        if let Some(name) = clicked {
            self.select(name, None);
        }
    }

    // ---------------------------------------------------------------- details

    fn details(&mut self, ui: &mut Ui, p: &Palette) {
        let Some((entry, install)) = self.current() else {
            empty_state(ui, p, "Select a skill");
            return;
        };
        let entry = entry.clone();
        let install = install.clone();
        let skill = &install.skill;

        if self.viewer.as_ref().is_none_or(|v| v.dir != skill.dir) {
            self.viewer = Some(Viewer::new(&skill.dir));
        }

        ScrollArea::vertical()
            .auto_shrink(false)
            .id_salt(&skill.dir)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&entry.name)
                            .size(26.0)
                            .family(theme::bold())
                            .color(p.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !skill.read_only {
                            let remove = egui::Button::new(RichText::new("Remove").color(p.err));
                            let tip = format!("Remove from {}", install.provider);
                            if ui.add(remove).on_hover_text(tip).clicked() {
                                self.dialog = Some(Dialog::Remove(RemoveDialog {
                                    dir: skill.dir.clone(),
                                    name: entry.name.clone(),
                                    provider: install.provider,
                                    link: skill.link_target.clone(),
                                }));
                            }
                        }
                        if ui.button("Open folder").clicked()
                            && let Err(e) = open::that_detached(&skill.dir)
                        {
                            self.notify(format!("Cannot open folder: {e}"), true);
                        }
                    });
                });

                if !skill.description.is_empty() {
                    ui.add_space(2.0);
                    ui.label(RichText::new(&skill.description).color(p.text));
                }

                ui.add_space(14.0);
                self.providers(ui, p, &entry);
                ui.add_space(4.0);
                self.location(ui, p, skill);

                ui.add_space(16.0);
                validation(ui, p, skill);
                ui.add_space(20.0);
                self.files(ui, p);
            });
    }

    /// One pill per tool that has this skill; click to view that copy.
    fn providers(&mut self, ui: &mut Ui, p: &Palette, entry: &Entry) {
        let mut pick = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new("Available in").size(SMALL).color(p.weak));
            for install in &entry.installs {
                let active = self.active.as_ref() == Some(&install.skill.dir);
                let mut tip = tilde(&install.skill.dir, &self.home);
                if install.skill.read_only {
                    tip.push_str("\nManaged by the app (read-only)");
                }
                let response = pill(ui, p, install.provider, active).on_hover_text(tip);
                if response.clicked() {
                    pick = Some(install.skill.dir.clone());
                }
            }
        });
        if let Some(dir) = pick {
            self.active = Some(dir);
        }
    }

    fn location(&self, ui: &mut Ui, p: &Palette, skill: &Skill) {
        ui.horizontal_wrapped(|ui| {
            let mono = |text: String| RichText::new(text).monospace().size(SMALL).color(p.weak);
            ui.label(mono(tilde(&skill.dir, &self.home)));
            if let Some(target) = &skill.link_target {
                ui.label(mono(format!("→ {}", tilde(target, &self.home))));
            }
            if skill.read_only {
                ui.label(RichText::new("· read-only").size(SMALL).color(p.weak));
            }
        });
    }

    fn files(&mut self, ui: &mut Ui, p: &Palette) {
        let Some(viewer) = &mut self.viewer else {
            return;
        };
        let markdown = is_markdown(&viewer.open);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Files  {}", viewer.files.len()))
                    .size(SMALL)
                    .color(p.weak),
            );
            if markdown {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    segmented(
                        ui,
                        p,
                        &mut self.rendered,
                        [("Raw", false), ("Preview", true)],
                    );
                });
            }
        });
        ui.add_space(4.0);

        let mut open = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for file in &viewer.files {
                let label = file.to_string_lossy().replace('\\', "/");
                let active = *file == viewer.open;
                if tab(ui, p, &label, active).clicked() && !active {
                    open = Some(file.clone());
                }
            }
        });
        if let Some(file) = open {
            viewer.open(file);
        }

        ui.add_space(10.0);
        Frame::new()
            .fill(p.surface)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                match &viewer.content {
                    Content::Text(text) if markdown && self.rendered => {
                        let body = if viewer.open == Path::new(SKILL_FILE) {
                            skill::body_of(text)
                        } else {
                            text
                        };
                        CommonMarkViewer::new().show(ui, &mut self.md_cache, body);
                    }
                    Content::Text(text) => {
                        let mut text = text.as_str();
                        ui.add(
                            TextEdit::multiline(&mut text)
                                .code_editor()
                                .frame(Frame::NONE)
                                .desired_width(f32::INFINITY),
                        );
                    }
                    Content::Binary => {
                        ui.label(RichText::new("Binary file").color(p.weak));
                    }
                    Content::TooLarge(len) => {
                        ui.label(
                            RichText::new(format!("File too large to preview ({} KB)", len / 1024))
                                .color(p.weak),
                        );
                    }
                    Content::Error(e) => {
                        ui.label(RichText::new(e).color(p.err));
                    }
                }
            });
        ui.add_space(12.0);
    }

    // ---------------------------------------------------------------- dialogs

    fn dialogs(&mut self, p: &Palette) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let keep = match &mut dialog {
            Dialog::Add(add) => self.add_dialog(p, add),
            Dialog::Remove(remove) => self.remove_dialog(p, remove),
        };
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    fn remove_dialog(&mut self, p: &Palette, d: &RemoveDialog) -> bool {
        let mut keep = true;
        let ctx = self.ctx.clone();
        let modal = Modal::new(Id::unique("stash-remove")).show(&ctx, |ui| {
            ui.set_width(400.0);
            ui.label(
                RichText::new(format!("Remove {} from {}?", d.name, d.provider))
                    .size(18.0)
                    .family(theme::bold()),
            );
            ui.add_space(6.0);
            let detail = match &d.link {
                Some(target) => format!(
                    "Only the link is removed. The skill stays in {}.",
                    tilde(target, &self.home)
                ),
                None => "The folder will be moved to the Trash.".to_string(),
            };
            ui.label(RichText::new(detail).color(p.weak));
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let confirm =
                    egui::Button::new(RichText::new("Remove").color(Color32::WHITE)).fill(p.err);
                if ui.add(confirm).clicked() {
                    match ops::remove(&d.dir) {
                        Ok(()) => {
                            self.notify(format!("Removed {} from {}", d.name, d.provider), false);
                            self.viewer = None;
                            self.active = None;
                            self.refresh();
                        }
                        Err(e) => self.notify(format!("{e:#}"), true),
                    }
                    keep = false;
                }
                let cancel = ui.button("Cancel");
                // Safe default: keyboard focus starts on Cancel.
                if !cancel.has_focus() && ui.memory(|m| m.focused().is_none()) {
                    cancel.request_focus();
                }
                if cancel.clicked() {
                    keep = false;
                }
            });
        });
        keep && !modal.should_close()
    }

    fn add_dialog(&mut self, p: &Palette, d: &mut AddDialog) -> bool {
        let ctx = self.ctx.clone();
        // Collect a finished clone.
        if let Some(rx) = &d.cloning {
            match rx.try_recv() {
                Ok(Ok(found)) => {
                    d.cloning = None;
                    if found.skills.is_empty() {
                        d.error = Some("No SKILL.md found in this repository".into());
                    } else {
                        d.candidates = Some(found);
                    }
                }
                Ok(Err(e)) => {
                    d.cloning = None;
                    d.error = Some(e);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => d.cloning = None,
            }
        }

        let mut keep = true;
        let modal = Modal::new(Id::unique("stash-add")).show(&ctx, |ui| {
            ui.set_width(480.0);
            ui.label(
                RichText::new("Add a skill")
                    .size(18.0)
                    .family(theme::bold()),
            );
            ui.add_space(10.0);

            let before = d.tab;
            segmented(
                ui,
                p,
                &mut d.tab,
                [
                    ("Folder", AddTab::Folder),
                    ("Git", AddTab::Git),
                    ("New", AddTab::New),
                ],
            );
            if d.tab != before {
                d.candidates = None;
                d.error = None;
            }
            ui.add_space(14.0);

            match d.tab {
                AddTab::Folder => {
                    ui.label(
                        RichText::new("Copy a folder that contains a SKILL.md.").color(p.weak),
                    );
                    ui.add_space(6.0);
                    if ui.button("Choose folder…").clicked()
                        && let Some(folder) = rfd::FileDialog::new().pick_folder()
                    {
                        let found = ops::find_skills(&folder, 3);
                        d.error = found
                            .is_empty()
                            .then(|| format!("No SKILL.md found in {}", folder.display()));
                        d.candidates =
                            (!found.is_empty()).then(|| Candidates::from_dirs(found, None));
                    }
                }
                AddTab::Git => {
                    ui.label(
                        RichText::new("Clone a repository and pick its skills.").color(p.weak),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let busy = d.cloning.is_some();
                        let edit = ui.add_enabled(
                            !busy,
                            TextEdit::singleline(&mut d.url)
                                .hint_text("https://github.com/owner/repo")
                                .desired_width(ui.available_width() - 80.0)
                                .margin(Margin::symmetric(8, 6)),
                        );
                        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if busy {
                            ui.spinner();
                            ui.label(RichText::new("Cloning…").size(SMALL).color(p.weak));
                        } else if ui.button("Fetch").clicked() || enter {
                            d.error = None;
                            d.candidates = None;
                            d.cloning = Some(spawn_clone(d.url.clone(), ctx.clone()));
                        }
                    });
                }
                AddTab::New => {
                    ui.label(RichText::new("Start from a blank SKILL.md.").color(p.weak));
                    ui.add_space(6.0);
                    ui.add(
                        TextEdit::singleline(&mut d.name)
                            .hint_text("Name, e.g. pdf-tools")
                            .desired_width(f32::INFINITY)
                            .margin(Margin::symmetric(8, 6)),
                    );
                    ui.add(
                        TextEdit::multiline(&mut d.description)
                            .hint_text("What it does and when to use it")
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .margin(Margin::symmetric(8, 6)),
                    );
                    if !d.name.is_empty()
                        && let Some(problem) = skill::name_problem(&d.name)
                    {
                        ui.label(RichText::new(problem).size(SMALL).color(p.warn));
                    }
                }
            }

            if let Some(c) = &mut d.candidates {
                ui.add_space(10.0);
                for (s, checked) in &mut c.skills {
                    ui.horizontal(|ui| {
                        ui.checkbox(checked, &s.name);
                        status_mark_widget(ui, p, s.status());
                        if let Some(issue) = s.issues.iter().max_by_key(|i| i.severity) {
                            ui.label(RichText::new(&issue.message).size(SMALL).color(p.weak));
                        }
                    });
                }
            }

            if let Some(e) = &d.error {
                ui.add_space(8.0);
                ui.label(RichText::new(e).color(p.err));
            }

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Install to").color(p.weak));
                let target = self.target();
                let current = self
                    .targets
                    .iter()
                    .find(|(_, path)| *path == target)
                    .map_or("", |(label, _)| *label);
                egui::ComboBox::from_id_salt("target")
                    .selected_text(current)
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for (label, path) in &self.targets {
                            let text = format!("{label}   {}", tilde(path, &self.home));
                            if ui.selectable_label(*path == target, text).clicked() {
                                self.settings.target = Some(path.clone());
                            }
                        }
                    })
                    .response
                    .on_hover_text(tilde(&target, &self.home));

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = match d.tab {
                        AddTab::New => skill::name_problem(&d.name).is_none(),
                        _ => d
                            .candidates
                            .as_ref()
                            .is_some_and(|c| c.skills.iter().any(|(_, on)| *on)),
                    };
                    let label = if d.tab == AddTab::New {
                        "Create"
                    } else {
                        "Install"
                    };
                    if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                        match self.apply_add(d) {
                            Ok(msg) => {
                                self.notify(msg, false);
                                keep = false;
                            }
                            Err(e) => d.error = Some(e),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
        });
        keep && !modal.should_close()
    }

    /// Install or create, then select the result.
    fn apply_add(&mut self, d: &AddDialog) -> Result<String, String> {
        let target = self.target();
        if d.tab == AddTab::New {
            let dir =
                ops::create(&target, &d.name, &d.description).map_err(|e| format!("{e:#}"))?;
            self.refresh();
            self.select_dir(dir);
            return Ok(format!("Created {}", d.name));
        }
        let Some(c) = &d.candidates else {
            return Err("Nothing to install".into());
        };
        let mut installed = Vec::new();
        let mut errors = Vec::new();
        for (s, _) in c.skills.iter().filter(|(_, on)| *on) {
            match ops::install_folder(&s.dir, &target) {
                Ok(dir) => installed.push(dir),
                Err(e) => errors.push(format!("{}: {e:#}", s.name)),
            }
        }
        self.refresh();
        if let Some(first) = installed.first() {
            self.select_dir(first.clone());
        }
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        Ok(match installed.len() {
            1 => format!("Installed {}", skill::folder_name(&installed[0])),
            n => format!("Installed {n} skills"),
        })
    }

    // ---------------------------------------------------------------- status

    fn status_bar(&mut self, ui: &mut Ui, p: &Palette) {
        ui.horizontal(|ui| {
            let total = self.entries.len();
            let invalid = self
                .entries
                .iter()
                .filter(|e| e.status() == Status::Invalid)
                .count();
            let mut summary = format!("{total} skills");
            if invalid > 0 {
                summary.push_str(&format!(" · {invalid} invalid"));
            }
            ui.label(RichText::new(summary).size(SMALL).color(p.weak));
            if let Some((msg, error, at)) = &self.status {
                let age = at.elapsed();
                if age < STATUS_TTL {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let color = if *error { p.err } else { p.weak };
                        ui.label(RichText::new(msg).size(SMALL).color(color));
                    });
                    ui.ctx().request_repaint_after(STATUS_TTL - age);
                }
            }
        });
    }
}

impl eframe::App for StashApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = self.ctx.clone();
        let p = theme::palette(ctx.theme() == egui::Theme::Dark);

        // Rescan when the window regains focus, so outside changes show up.
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if focused && !self.focused {
            self.refresh();
        }
        self.focused = focused;
        self.shortcuts();

        let bar = Frame::new()
            .fill(p.bg)
            .inner_margin(Margin::symmetric(16, 10));
        egui::Panel::top("top")
            .frame(bar)
            .show(ui, |ui| self.top_bar(ui, &p));
        egui::Panel::bottom("status")
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(16, 6)),
            )
            .show(ui, |ui| self.status_bar(ui, &p));
        egui::Panel::left("skills")
            .resizable(true)
            .default_size(270.0)
            .min_size(200.0)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(12, 4)),
            )
            .show(ui, |ui| self.sidebar(ui, &p));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin {
                left: 28,
                right: 28,
                top: 8,
                bottom: 0,
            }))
            .show(ui, |ui| self.details(ui, &p));

        self.dialogs(&p);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.zoom = Some(self.ctx.zoom_factor());
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}

fn search_id() -> Id {
    Id::unique("stash-search")
}

fn spawn_clone(url: String, ctx: egui::Context) -> Receiver<Result<Candidates, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let tmp = tempfile::Builder::new()
                .prefix("stash-")
                .tempdir()
                .map_err(|e| e.to_string())?;
            let repo = tmp.path().join("repo");
            ops::git_clone(&url, &repo).map_err(|e| format!("{e:#}"))?;
            let found = ops::find_skills(&repo, 4);
            Ok(Candidates::from_dirs(found, Some(tmp)))
        })();
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

// -------------------------------------------------------------------- widgets

fn empty_state(ui: &mut Ui, p: &Palette, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(RichText::new(text).color(p.weak));
    });
}

/// Status drawn as a shape, so it does not rely on color alone:
/// circle = valid, triangle = warnings, square = invalid.
fn paint_status(painter: &Painter, center: Pos2, status: Status, color: Color32) {
    match status {
        Status::Valid => {
            painter.circle_filled(center, 4.0, color);
        }
        Status::Warnings => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + vec2(0.0, -4.8),
                    center + vec2(4.6, 3.6),
                    center + vec2(-4.6, 3.6),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Status::Invalid => {
            painter.rect_filled(Rect::from_center_size(center, vec2(7.5, 7.5)), 1.5, color);
        }
    }
}

fn status_mark_widget(ui: &mut Ui, p: &Palette, status: Status) {
    let (rect, response) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    paint_status(ui.painter(), rect.center(), status, p.status(status));
    response.widget_info(|| WidgetInfo::labeled(Role::Image, true, status_words(status)));
}

/// A frameless button showing a symbol, with a spoken label.
fn icon_button(ui: &mut Ui, icon: &str, label: &str) -> Response {
    let response = ui
        .add(egui::Button::new(RichText::new(icon).size(17.0)).frame(false))
        .on_hover_text(label);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    response
}

/// A rounded, toggleable label (provider pills).
fn pill(ui: &mut Ui, p: &Palette, text: &str, active: bool) -> Response {
    let rich = RichText::new(text)
        .size(13.5)
        .color(if active { p.text } else { p.weak });
    let response = ui.add(
        egui::Button::new(rich)
            .fill(if active {
                p.hover
            } else {
                Color32::TRANSPARENT
            })
            .stroke(Stroke::new(1.0, if active { p.hover } else { p.border }))
            .corner_radius(CornerRadius::same(255)),
    );
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, active, text));
    response
}

/// A flat tab (file list).
fn tab(ui: &mut Ui, p: &Palette, text: &str, active: bool) -> Response {
    let rich = RichText::new(text)
        .size(13.5)
        .color(if active { p.text } else { p.weak });
    let response = ui.add(egui::Button::new(rich).fill(if active {
        p.hover
    } else {
        Color32::TRANSPARENT
    }));
    response.widget_info(|| WidgetInfo::selected(Role::Tab, true, active, text));
    response
}

/// One row in the sidebar: status mark, name, lock when read-only.
fn skill_row(ui: &mut Ui, p: &Palette, entry: &Entry, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let painter = ui.painter_at(rect.expand(2.0));
    if selected {
        painter.rect_filled(rect, 6.0, p.hover);
    } else if response.hovered() {
        painter.rect_filled(rect, 6.0, p.surface);
    }
    if response.has_focus() {
        painter.rect_stroke(rect, 6.0, Stroke::new(2.0, p.accent), StrokeKind::Inside);
    }
    let status = entry.status();
    paint_status(
        &painter,
        rect.left_center() + vec2(13.0, 0.0),
        status,
        p.status(status),
    );

    let read_only = entry.read_only();
    let hint_width = if read_only {
        painter.text(
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            "🔒",
            FontId::proportional(SMALL),
            p.weak,
        );
        24.0
    } else {
        0.0
    };
    let text_rect = rect
        .with_min_x(rect.left() + 26.0)
        .with_max_x(rect.right() - hint_width - 4.0);
    ui.painter_at(text_rect).text(
        text_rect.left_center(),
        Align2::LEFT_CENTER,
        &entry.name,
        FontId::proportional(15.0),
        p.text,
    );

    let providers = entry.providers().join(", ");
    let spoken = format!(
        "{}, {}, in {providers}{}",
        entry.name,
        status_words(status),
        if read_only { ", read-only" } else { "" }
    );
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &spoken));
    let tip = match entry.description() {
        "" => providers,
        d => format!("{d}\n\n{providers}"),
    };
    response.on_hover_text(tip)
}

fn validation(ui: &mut Ui, p: &Palette, skill: &Skill) {
    let status = skill.status();
    let title = match status {
        Status::Valid => "Valid",
        Status::Warnings => "Valid, with warnings",
        Status::Invalid => "Invalid",
    };
    let color = p.status(status);
    Frame::new()
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                status_mark_widget(ui, p, status);
                ui.label(RichText::new(title).color(color).family(theme::bold()));
            });
            let mut issues = skill.issues.clone();
            issues.sort_by_key(|i| std::cmp::Reverse(i.severity));
            for issue in issues {
                let (word, c) = match issue.severity {
                    Severity::Error => ("Error", p.err),
                    Severity::Warning => ("Warning", p.warn),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(20.0);
                    ui.label(RichText::new(word).color(c));
                    ui.label(RichText::new(&issue.message).color(p.text));
                });
            }
        });
}

/// A row of mutually exclusive, text-only buttons.
fn segmented<T: PartialEq + Copy, const N: usize>(
    ui: &mut Ui,
    p: &Palette,
    value: &mut T,
    options: [(&str, T); N],
) {
    Frame::new()
        .fill(p.surface)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.spacing_mut().button_padding = vec2(12.0, 3.0);
                for (label, option) in options {
                    let on = *value == option;
                    let text =
                        RichText::new(label)
                            .size(13.5)
                            .color(if on { p.text } else { p.weak });
                    let button = egui::Button::new(text)
                        .fill(if on { p.bg } else { Color32::TRANSPARENT })
                        .corner_radius(CornerRadius::same(6));
                    let response = ui.add(button);
                    response.widget_info(|| WidgetInfo::selected(Role::Tab, true, on, label));
                    if response.clicked() {
                        *value = option;
                    }
                }
            });
        });
}
