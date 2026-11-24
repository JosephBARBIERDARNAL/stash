//! The stash window.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Modal, RichText,
    ScrollArea, Sense, Stroke, TextEdit, ThemePreference, Ui, vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use serde::{Deserialize, Serialize};

use crate::ops;
use crate::skill::{self, SKILL_FILE, Severity, Skill, Status};
use crate::sources::{self, Source};
use crate::theme::{self, Palette};

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

enum Dialog {
    Add(Box<AddDialog>),
    Remove {
        dir: PathBuf,
        name: String,
        link: Option<PathBuf>,
    },
}

enum Content {
    Text(String),
    Binary,
    TooLarge(u64),
    Error(String),
}

/// The files of the selected skill and the one being viewed.
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

pub struct StashApp {
    home: PathBuf,
    sources: Vec<Source>,
    /// Folders new skills can go to: (label, path).
    targets: Vec<(&'static str, PathBuf)>,
    settings: Settings,
    selected: Option<PathBuf>,
    query: String,
    viewer: Option<Viewer>,
    rendered: bool,
    md_cache: CommonMarkCache,
    dialog: Option<Dialog>,
    status: Option<(String, bool, Instant)>,
    focused: bool,
}

impl StashApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        if let Some(dark) = settings.dark {
            cc.egui_ctx.set_theme(if dark {
                ThemePreference::Dark
            } else {
                ThemePreference::Light
            });
        }
        let home = dirs::home_dir().unwrap_or_default();
        let targets = sources::known_sources(&home)
            .into_iter()
            .filter(Source::writable)
            .map(|s| (s.label, s.root))
            .collect();
        let mut app = Self {
            home,
            sources: Vec::new(),
            targets,
            settings,
            selected: None,
            query: String::new(),
            viewer: None,
            rendered: true,
            md_cache: CommonMarkCache::default(),
            dialog: None,
            status: None,
            focused: true,
        };
        app.refresh();
        let first = app.all_skills().next().map(|s| s.dir.clone());
        app.selected = first;
        app
    }

    fn refresh(&mut self) {
        self.sources = sources::discover(&self.home);
        if let Some(sel) = &self.selected
            && self.find(sel).is_none()
        {
            self.selected = None;
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

    fn all_skills(&self) -> impl Iterator<Item = &Skill> {
        self.sources.iter().flat_map(|s| s.skills.iter())
    }

    fn find(&self, dir: &Path) -> Option<(&Source, &Skill)> {
        self.sources
            .iter()
            .find_map(|src| src.skills.iter().find(|s| s.dir == dir).map(|s| (src, s)))
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

    fn select(&mut self, dir: PathBuf) {
        self.selected = Some(dir);
    }

    fn set_dark(&mut self, ctx: &egui::Context, dark: bool) {
        self.settings.dark = Some(dark);
        ctx.set_theme(if dark {
            ThemePreference::Dark
        } else {
            ThemePreference::Light
        });
    }

    // ---------------------------------------------------------------- top bar

    fn top_bar(&mut self, ui: &mut Ui, p: &Palette) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("stash").size(17.0).strong().color(p.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dark = ui.visuals().dark_mode;
                let icon = if dark { "☀" } else { "☾" };
                let tip = if dark { "Light mode" } else { "Dark mode" };
                if ui
                    .add(egui::Button::new(icon).frame(false))
                    .on_hover_text(tip)
                    .clicked()
                {
                    self.set_dark(ui.ctx(), !dark);
                }
                if ui
                    .add(egui::Button::new("⟳").frame(false))
                    .on_hover_text("Rescan skills (⌘R)")
                    .clicked()
                {
                    self.refresh();
                    self.notify("Rescanned", false);
                }
                if ui.button("+  Add").clicked() {
                    self.dialog = Some(Dialog::Add(Box::new(AddDialog::new(AddTab::Folder))));
                }
            });
        });
    }

    // ---------------------------------------------------------------- sidebar

    fn sidebar(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(4.0);
        ui.add(
            TextEdit::singleline(&mut self.query)
                .hint_text("Search")
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(8, 5)),
        );
        ui.add_space(4.0);

        let query = self.query.trim().to_lowercase();
        let mut clicked = None;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let mut any = false;
            for source in &self.sources {
                let skills: Vec<&Skill> = source
                    .skills
                    .iter()
                    .filter(|s| {
                        query.is_empty()
                            || s.name.to_lowercase().contains(&query)
                            || s.description.to_lowercase().contains(&query)
                    })
                    .collect();
                if skills.is_empty() {
                    continue;
                }
                any = true;
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!("{}  {}", source.label.to_uppercase(), skills.len()))
                        .size(10.5)
                        .color(p.weak),
                )
                .on_hover_text(tilde(&source.root, &self.home));
                ui.add_space(2.0);
                for skill in skills {
                    let selected = self.selected.as_deref() == Some(skill.dir.as_path());
                    if skill_row(ui, p, skill, selected).clicked() {
                        clicked = Some(skill.dir.clone());
                    }
                }
            }
            if !any {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    let msg = if self.sources.is_empty() {
                        "No skills installed yet"
                    } else {
                        "No matches"
                    };
                    ui.label(RichText::new(msg).color(p.weak));
                });
            }
        });
        if let Some(dir) = clicked {
            self.select(dir);
        }
    }

    // ---------------------------------------------------------------- details

    fn details(&mut self, ui: &mut Ui, p: &Palette) {
        let Some(selected) = self.selected.clone() else {
            empty_state(ui, p, "Select a skill");
            return;
        };
        let Some((source, skill)) = self.find(&selected) else {
            empty_state(ui, p, "Select a skill");
            return;
        };
        let skill = skill.clone();
        let source_label = source.label;

        if self.viewer.as_ref().is_none_or(|v| v.dir != skill.dir) {
            self.viewer = Some(Viewer::new(&skill.dir));
        }

        ScrollArea::vertical()
            .auto_shrink(false)
            .id_salt(&skill.dir)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&skill.name).size(24.0).strong().color(p.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !skill.read_only {
                            let remove = egui::Button::new(RichText::new("Remove").color(p.err));
                            if ui.add(remove).clicked() {
                                self.dialog = Some(Dialog::Remove {
                                    dir: skill.dir.clone(),
                                    name: skill.name.clone(),
                                    link: skill.link_target.clone(),
                                });
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

                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    chip(ui, p, source_label);
                    if skill.read_only {
                        chip(ui, p, "read-only");
                    }
                    if skill.link_target.is_some() {
                        chip(ui, p, "linked");
                    }
                    ui.label(
                        RichText::new(tilde(&skill.dir, &self.home))
                            .monospace()
                            .size(12.0)
                            .color(p.weak),
                    );
                    if let Some(target) = &skill.link_target {
                        ui.label(
                            RichText::new(format!("→ {}", tilde(target, &self.home)))
                                .monospace()
                                .size(12.0)
                                .color(p.weak),
                        );
                    }
                });

                ui.add_space(14.0);
                validation(ui, p, &skill);
                ui.add_space(18.0);
                self.files(ui, p);
            });
    }

    fn files(&mut self, ui: &mut Ui, p: &Palette) {
        let Some(viewer) = &mut self.viewer else {
            return;
        };
        let markdown = is_markdown(&viewer.open);
        ui.horizontal(|ui| {
            ui.label(RichText::new("FILES").size(10.5).color(p.weak));
            ui.label(
                RichText::new(viewer.files.len().to_string())
                    .size(10.5)
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
                let text =
                    RichText::new(label)
                        .size(12.5)
                        .color(if active { p.text } else { p.weak });
                if ui.selectable_label(active, text).clicked() && !active {
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
            .inner_margin(Margin::same(16))
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

    fn dialogs(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let keep = match &mut dialog {
            Dialog::Add(add) => self.add_dialog(ctx, p, add),
            Dialog::Remove { dir, name, link } => {
                let (dir, name, link) = (dir.clone(), name.clone(), link.clone());
                self.remove_dialog(ctx, p, &dir, &name, link.as_deref())
            }
        };
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    fn remove_dialog(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        dir: &Path,
        name: &str,
        link: Option<&Path>,
    ) -> bool {
        let mut keep = true;
        let modal = Modal::new(Id::unique("stash-remove")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(format!("Remove {name}?")).size(17.0).strong());
            ui.add_space(6.0);
            let detail = match link {
                Some(target) => format!(
                    "Only the link is removed. The skill stays in {}.",
                    tilde(target, &self.home)
                ),
                None => "The folder will be moved to the Trash.".to_string(),
            };
            ui.label(RichText::new(detail).color(p.weak));
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let confirm =
                    egui::Button::new(RichText::new("Remove").color(Color32::WHITE)).fill(p.err);
                if ui.add(confirm).clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                    match ops::remove(dir) {
                        Ok(()) => {
                            self.notify(format!("Removed {name}"), false);
                            self.viewer = None;
                            self.selected = None;
                            self.refresh();
                        }
                        Err(e) => self.notify(format!("{e:#}"), true),
                    }
                    keep = false;
                }
                if ui.button("Cancel").clicked() {
                    keep = false;
                }
            });
        });
        keep && !modal.should_close()
    }

    fn add_dialog(&mut self, ctx: &egui::Context, p: &Palette, d: &mut AddDialog) -> bool {
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
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
                Err(mpsc::TryRecvError::Disconnected) => d.cloning = None,
            }
        }

        let mut keep = true;
        let modal = Modal::new(Id::unique("stash-add")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Add a skill").size(17.0).strong());
            });
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
                                .desired_width(ui.available_width() - 70.0)
                                .margin(Margin::symmetric(8, 5)),
                        );
                        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if busy {
                            ui.spinner();
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
                            .hint_text("name  (e.g. pdf-tools)")
                            .desired_width(f32::INFINITY)
                            .margin(Margin::symmetric(8, 5)),
                    );
                    ui.add(
                        TextEdit::multiline(&mut d.description)
                            .hint_text("What it does and when to use it")
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .margin(Margin::symmetric(8, 5)),
                    );
                    if !d.name.is_empty()
                        && let Some(problem) = skill::name_problem(&d.name)
                    {
                        ui.label(RichText::new(problem).size(12.0).color(p.warn));
                    }
                }
            }

            if let Some(c) = &mut d.candidates {
                ui.add_space(10.0);
                for (s, checked) in &mut c.skills {
                    ui.horizontal(|ui| {
                        ui.checkbox(checked, "");
                        status_dot(ui, p.status(s.status()));
                        ui.label(&s.name);
                        if let Some(issue) = s.issues.iter().max_by_key(|i| i.severity) {
                            ui.label(RichText::new(&issue.message).size(12.0).color(p.weak))
                                .on_hover_text(&issue.message);
                        }
                    });
                }
            }

            if let Some(e) = &d.error {
                ui.add_space(8.0);
                ui.label(RichText::new(e).size(12.5).color(p.err));
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
                    .width(150.0)
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
            self.select(dir);
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
            self.select(first.clone());
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
            let total = self.all_skills().count();
            let invalid = self
                .all_skills()
                .filter(|s| s.status() == Status::Invalid)
                .count();
            let mut summary = format!("{total} skills");
            if invalid > 0 {
                summary.push_str(&format!(" · {invalid} invalid"));
            }
            ui.label(RichText::new(summary).size(12.0).color(p.weak));
            if let Some((msg, error, at)) = &self.status {
                let age = at.elapsed();
                if age < STATUS_TTL {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let color = if *error { p.err } else { p.weak };
                        ui.label(RichText::new(msg).size(12.0).color(color));
                    });
                    ui.ctx().request_repaint_after(STATUS_TTL - age);
                }
            }
        });
    }
}

impl eframe::App for StashApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let p = theme::palette(ctx.theme() == egui::Theme::Dark);

        // Rescan when the window regains focus, so outside changes show up.
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if focused && !self.focused {
            self.refresh();
        }
        self.focused = focused;
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::R)) {
            self.refresh();
            self.notify("Rescanned", false);
        }

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

        self.dialogs(&ctx, &p);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
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

fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

fn chip(ui: &mut Ui, p: &Palette, text: &str) {
    Frame::new()
        .fill(p.surface)
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(p.weak));
        });
}

/// One row in the sidebar: status dot, name, and a hint for links / read-only.
fn skill_row(ui: &mut Ui, p: &Palette, skill: &Skill, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 6.0, p.hover);
    } else if response.hovered() {
        painter.rect_filled(rect, 6.0, p.surface);
    }
    painter.circle_filled(
        rect.left_center() + vec2(12.0, 0.0),
        3.5,
        p.status(skill.status()),
    );

    let hint = match (skill.link_target.is_some(), skill.read_only) {
        (true, _) => "link",
        (_, true) => "🔒",
        _ => "",
    };
    let hint_width = if hint.is_empty() { 0.0 } else { 34.0 };
    if !hint.is_empty() {
        painter.text(
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            hint,
            FontId::proportional(11.0),
            p.weak,
        );
    }
    let text_rect = rect
        .with_min_x(rect.left() + 24.0)
        .with_max_x(rect.right() - hint_width - 4.0);
    ui.painter_at(text_rect).text(
        text_rect.left_center(),
        Align2::LEFT_CENTER,
        &skill.name,
        FontId::proportional(14.0),
        if selected {
            p.text
        } else {
            p.text.gamma_multiply(0.85)
        },
    );

    let tip = if skill.description.is_empty() {
        skill.name.clone()
    } else {
        skill.description.clone()
    };
    response.on_hover_text(tip)
}

fn validation(ui: &mut Ui, p: &Palette, skill: &Skill) {
    let status = skill.status();
    let (title, color) = match status {
        Status::Valid => ("Valid", p.ok),
        Status::Warnings => ("Valid, with warnings", p.warn),
        Status::Invalid => ("Invalid", p.err),
    };
    Frame::new()
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.label(RichText::new(title).color(color).strong());
            });
            let mut issues = skill.issues.clone();
            issues.sort_by_key(|i| std::cmp::Reverse(i.severity));
            for issue in issues {
                let c = match issue.severity {
                    Severity::Error => p.err,
                    Severity::Warning => p.warn,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new("–").color(c));
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
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.spacing_mut().button_padding = vec2(10.0, 3.0);
                for (label, option) in options {
                    let on = *value == option;
                    let text =
                        RichText::new(label)
                            .size(12.5)
                            .color(if on { p.text } else { p.weak });
                    let button = egui::Button::new(text)
                        .fill(if on { p.bg } else { Color32::TRANSPARENT })
                        .corner_radius(CornerRadius::same(5));
                    if ui.add(button).clicked() {
                        *value = option;
                    }
                }
            });
        });
}
