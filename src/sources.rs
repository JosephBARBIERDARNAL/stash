//! Where skills live on disk, and how to find them.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::skill::{SKILL_FILE, Skill};

/// How deep to look inside a source folder. Skills sit at depth 1; deeper
/// ones (e.g. synced or system skills) are managed by their tool.
const MAX_DEPTH: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A plain `skills/` folder the user can add to and remove from.
    Folder,
    /// Skills shipped inside installed Claude Code plugins (read-only).
    ClaudePlugins,
}

#[derive(Debug, Clone)]
pub struct Source {
    pub label: &'static str,
    pub root: PathBuf,
    pub kind: Kind,
    pub skills: Vec<Skill>,
}

impl Source {
    pub fn writable(&self) -> bool {
        self.kind == Kind::Folder
    }
}

fn env_dir(var: &str) -> Option<PathBuf> {
    env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Every global skill location stash knows about, whether it exists or not.
pub fn known_sources(home: &Path) -> Vec<Source> {
    let claude = env_dir("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
    let codex = env_dir("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
    let config = env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));

    let folder = |label, root: PathBuf| Source {
        label,
        root,
        kind: Kind::Folder,
        skills: Vec::new(),
    };
    vec![
        folder("Claude Code", claude.join("skills")),
        folder("Agents", home.join(".agents").join("skills")),
        folder("Codex", codex.join("skills")),
        folder("Gemini CLI", home.join(".gemini").join("skills")),
        folder("Cursor", home.join(".cursor").join("skills")),
        folder("GitHub Copilot", home.join(".copilot").join("skills")),
        folder("OpenCode", config.join("opencode").join("skills")),
        Source {
            label: "Claude Code plugins",
            root: claude.join("plugins"),
            kind: Kind::ClaudePlugins,
            skills: Vec::new(),
        },
    ]
}

/// Scan all known sources and keep the ones that contain skills.
pub fn discover(home: &Path) -> Vec<Source> {
    known_sources(home)
        .into_iter()
        .filter_map(|mut source| {
            source.skills = match source.kind {
                Kind::Folder => scan_folder(&source.root),
                Kind::ClaudePlugins => scan_plugins(&source.root),
            };
            (!source.skills.is_empty()).then_some(source)
        })
        .collect()
}

/// Skills inside a `skills/` folder.
pub fn scan_folder(root: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    walk(root, 1, &mut skills);
    skills.sort_by_key(|s| s.name.to_lowercase());
    skills
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Skill>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let is_link = fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
        if path.join(SKILL_FILE).is_file() {
            out.push(Skill::load(&path, depth > 1));
        } else if is_link && !path.exists() && depth == 1 {
            // Dangling symlink: show it so the user can clean it up.
            out.push(Skill::load(&path, false));
        } else if path.is_dir() && depth < MAX_DEPTH && !is_link && !is_ignored(&path) {
            // Hidden folders (e.g. Codex `.system`) are scanned too.
            walk(&path, depth + 1, out);
        }
    }
}

fn is_ignored(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| matches!(n.to_str(), Some(".git" | "node_modules" | "target")))
}

/// Skills bundled in installed Claude Code plugins, read from
/// `plugins/installed_plugins.json`.
pub fn scan_plugins(plugins_dir: &Path) -> Vec<Skill> {
    let Ok(text) = fs::read_to_string(plugins_dir.join("installed_plugins.json")) else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for install in plugin_install_paths(&text) {
        let Ok(entries) = fs::read_dir(install.join("skills")) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join(SKILL_FILE).is_file())
            .collect();
        dirs.sort();
        skills.extend(dirs.iter().map(|d| Skill::load(d, true)));
    }
    skills.sort_by_key(|s| s.name.to_lowercase());
    skills
}

/// Install paths listed in `installed_plugins.json` (v1 or v2 layout).
fn plugin_install_paths(json: &str) -> Vec<PathBuf> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(plugins) = value.get("plugins").and_then(|p| p.as_object()) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = plugins
        .values()
        .flat_map(|entry| match entry {
            serde_json::Value::Array(installs) => installs.iter().collect(),
            other => vec![other],
        })
        .filter_map(|install| install.get("installPath")?.as_str())
        .map(PathBuf::from)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join(SKILL_FILE),
            format!("---\nname: {name}\ndescription: Test.\n---\nBody\n"),
        )
        .unwrap();
    }

    #[test]
    fn finds_top_level_and_nested_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_skill(&root.join("beta"), "beta");
        write_skill(&root.join("alpha"), "alpha");
        write_skill(&root.join(".system").join("gamma"), "gamma");
        fs::create_dir_all(root.join("empty")).unwrap();

        let skills = scan_folder(root);
        let names: Vec<_> = skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["alpha", "beta", "gamma"]);
        assert!(!skills[0].read_only);
        assert!(skills[2].read_only, "nested skills are managed");
    }

    #[test]
    fn missing_folder_is_empty() {
        assert!(scan_folder(Path::new("/definitely/not/here")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        write_skill(&real, "linked");
        let root = tmp.path().join("skills");
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&real, root.join("linked")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("gone"), root.join("gone")).unwrap();

        let skills = scan_folder(&root);
        assert_eq!(skills.len(), 2);
        let linked = skills.iter().find(|s| s.name == "linked").unwrap();
        assert_eq!(linked.link_target, Some(fs::canonicalize(&real).unwrap()));
        assert!(linked.issues.is_empty());
        let gone = skills.iter().find(|s| s.name == "gone").unwrap();
        assert!(gone.issues[0].message.contains("Broken link"));
    }

    #[test]
    fn reads_plugin_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp
            .path()
            .join("cache")
            .join("market")
            .join("tool")
            .join("1.0");
        write_skill(&install.join("skills").join("tool-skill"), "tool-skill");
        let json = serde_json::json!({
            "version": 2,
            "plugins": { "tool@market": [{ "installPath": install }] }
        });
        fs::write(tmp.path().join("installed_plugins.json"), json.to_string()).unwrap();

        let skills = scan_plugins(tmp.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "tool-skill");
        assert!(skills[0].read_only);
    }

    #[test]
    fn known_sources_cover_main_tools() {
        let labels: Vec<_> = known_sources(Path::new("/home/u"))
            .iter()
            .map(|s| s.label)
            .collect();
        for expected in ["Claude Code", "Agents", "Codex", "Claude Code plugins"] {
            assert!(labels.contains(&expected));
        }
    }
}
