//! Reading and validating a single skill (a folder containing `SKILL.md`).
//!
//! Validation follows the Agent Skills format: YAML frontmatter with a
//! `name` (lowercase, digits, hyphens; matches the folder) and a `description`.

use std::fs;
use std::path::{Path, PathBuf};

use yaml_rust2::{Yaml, YamlLoader};

pub const SKILL_FILE: &str = "SKILL.md";
pub const MAX_NAME_LEN: usize = 64;
pub const MAX_DESCRIPTION_LEN: usize = 1024;
pub const MAX_COMPATIBILITY_LEN: usize = 500;
pub const RECOMMENDED_MAX_LINES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
}

impl Issue {
    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

/// Ordered from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Valid,
    Warnings,
    Invalid,
}

/// The parts of `SKILL.md` stash cares about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Markdown after the frontmatter.
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Skill {
    /// Display name: frontmatter `name`, or the folder name as a fallback.
    pub name: String,
    pub description: String,
    /// Path as found in the source folder (may be a symlink).
    pub dir: PathBuf,
    /// Where `dir` points to, when it is a symlink.
    pub link_target: Option<PathBuf>,
    /// Managed by a tool (plugins, synced skills): shown but never removed.
    pub read_only: bool,
    pub issues: Vec<Issue>,
}

impl Skill {
    /// Load and validate the skill stored in `dir`.
    pub fn load(dir: &Path, read_only: bool) -> Self {
        let folder = folder_name(dir);
        let link_target = fs::symlink_metadata(dir)
            .ok()
            .filter(|m| m.file_type().is_symlink())
            // Resolved to an absolute path; a broken link keeps its raw target.
            .and_then(|_| fs::canonicalize(dir).or_else(|_| fs::read_link(dir)).ok());

        let (manifest, issues) = match fs::read_to_string(dir.join(SKILL_FILE)) {
            Ok(content) => check(&folder, &content),
            Err(_) if !dir.exists() => (
                Manifest::default(),
                vec![Issue::error(
                    "Broken link: the target folder does not exist",
                )],
            ),
            Err(_) => (
                Manifest::default(),
                vec![Issue::error(format!(
                    "{SKILL_FILE} is missing or unreadable"
                ))],
            ),
        };

        Self {
            name: manifest.name.filter(|n| !n.is_empty()).unwrap_or(folder),
            description: manifest.description.unwrap_or_default(),
            dir: dir.to_path_buf(),
            link_target,
            read_only,
            issues,
        }
    }

    pub fn status(&self) -> Status {
        status_of(&self.issues)
    }

    pub fn skill_file(&self) -> PathBuf {
        self.dir.join(SKILL_FILE)
    }
}

pub fn status_of(issues: &[Issue]) -> Status {
    match issues.iter().map(|i| i.severity).max() {
        None => Status::Valid,
        Some(Severity::Warning) => Status::Warnings,
        Some(Severity::Error) => Status::Invalid,
    }
}

pub fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Split `SKILL.md` into its YAML frontmatter and markdown body.
pub fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let rest = content
        .strip_prefix("---\r\n")
        .or_else(|| content.strip_prefix("---\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

/// Markdown body of a `SKILL.md`, without frontmatter.
pub fn body_of(content: &str) -> &str {
    split_frontmatter(content).map_or(content, |(_, body)| body)
}

/// Check a skill name against the format rules. Returns a problem, if any.
pub fn name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Name is empty".into());
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Some(format!("Name is longer than {MAX_NAME_LEN} characters"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Some("Name may only contain lowercase letters, digits and hyphens".into());
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Some("Name cannot start or end with a hyphen".into());
    }
    if name.contains("--") {
        return Some("Name cannot contain consecutive hyphens".into());
    }
    None
}

/// Parse and validate the content of a `SKILL.md` stored in folder `folder`.
pub fn check(folder: &str, content: &str) -> (Manifest, Vec<Issue>) {
    let mut issues = Vec::new();
    let mut manifest = Manifest::default();

    let Some((yaml, body)) = split_frontmatter(content) else {
        let msg = if content.trim_start().starts_with("---") {
            "Frontmatter is not closed with ---"
        } else {
            "Missing YAML frontmatter (the file must start with ---)"
        };
        manifest.body = content.to_string();
        return (manifest, vec![Issue::error(msg)]);
    };
    manifest.body = body.to_string();

    let doc = match YamlLoader::load_from_str(yaml) {
        Ok(docs) => docs.into_iter().next().unwrap_or(Yaml::Null),
        Err(e) => {
            issues.push(Issue::error(format!("Frontmatter is not valid YAML: {e}")));
            return (manifest, issues);
        }
    };
    if !matches!(doc, Yaml::Hash(_)) {
        issues.push(Issue::error(
            "Frontmatter must be a set of key: value pairs",
        ));
        return (manifest, issues);
    }

    match &doc["name"] {
        Yaml::BadValue | Yaml::Null => issues.push(Issue::error("Missing required field: name")),
        Yaml::String(name) => {
            if let Some(problem) = name_problem(name) {
                issues.push(Issue::error(problem));
            } else if name != folder {
                issues.push(Issue::error(format!(
                    "Name \"{name}\" does not match its folder \"{folder}\""
                )));
            }
            manifest.name = Some(name.clone());
        }
        _ => issues.push(Issue::error("Field name must be text")),
    }

    match &doc["description"] {
        Yaml::BadValue | Yaml::Null => {
            issues.push(Issue::error("Missing required field: description"));
        }
        Yaml::String(desc) => {
            let len = desc.chars().count();
            if desc.trim().is_empty() {
                issues.push(Issue::error("Description is empty"));
            } else if len > MAX_DESCRIPTION_LEN {
                issues.push(Issue::error(format!(
                    "Description is {len} characters (max {MAX_DESCRIPTION_LEN})"
                )));
            }
            manifest.description = Some(desc.trim().to_string());
        }
        _ => issues.push(Issue::error("Field description must be text")),
    }

    if let Yaml::String(c) = &doc["compatibility"]
        && c.chars().count() > MAX_COMPATIBILITY_LEN
    {
        issues.push(Issue::error(format!(
            "Compatibility is longer than {MAX_COMPATIBILITY_LEN} characters"
        )));
    }

    if body.trim().is_empty() {
        issues.push(Issue::warning(
            "Instructions are empty (nothing after the frontmatter)",
        ));
    }
    let lines = content.lines().count();
    if lines > RECOMMENDED_MAX_LINES {
        issues.push(Issue::warning(format!(
            "{SKILL_FILE} is {lines} lines; under {RECOMMENDED_MAX_LINES} is recommended (move details to separate files)"
        )));
    }

    (manifest, issues)
}

/// The `SKILL.md` written for a new skill.
pub fn template(name: &str, description: &str) -> String {
    let description = description.trim().replace('\n', " ");
    let description = if description.is_empty() {
        "Describe what this skill does and when to use it.".to_string()
    } else {
        description
    };
    let title = name
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "---\nname: {name}\ndescription: {}\n---\n\n# {title}\n\n## Instructions\n\n1. First step.\n2. Second step.\n",
        yaml_string(&description)
    )
}

/// Quote a string for YAML when needed.
fn yaml_string(s: &str) -> String {
    let needs_quotes = s.contains([
        ':', '#', '"', '\'', '[', ']', '{', '}', ',', '&', '*', '!', '|', '>', '%', '@', '`',
    ]) || s.starts_with([' ', '-', '?'])
        || s.ends_with(' ');
    if needs_quotes {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(folder: &str, content: &str) -> Vec<String> {
        check(folder, content)
            .1
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.message)
            .collect()
    }

    #[test]
    fn valid_skill_has_no_issues() {
        let content =
            "---\nname: pdf-tools\ndescription: Work with PDFs.\n---\n\n# PDF\nDo things.\n";
        let (manifest, issues) = check("pdf-tools", content);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(manifest.name.as_deref(), Some("pdf-tools"));
        assert_eq!(manifest.description.as_deref(), Some("Work with PDFs."));
        assert_eq!(manifest.body.trim(), "# PDF\nDo things.");
    }

    #[test]
    fn crlf_and_bom_are_accepted() {
        let content = "\u{feff}---\r\nname: a\r\ndescription: b\r\n---\r\nbody\r\n";
        assert!(check("a", content).1.is_empty());
    }

    #[test]
    fn missing_frontmatter() {
        assert_eq!(errors("a", "# hello").len(), 1);
        assert!(errors("a", "---\nname: a\n")[0].contains("not closed"));
    }

    #[test]
    fn missing_fields() {
        let errs = errors("a", "---\nlicense: MIT\n---\nbody");
        assert!(errs.iter().any(|e| e.contains("name")));
        assert!(errs.iter().any(|e| e.contains("description")));
    }

    #[test]
    fn invalid_yaml() {
        assert!(errors("a", "---\nname: [a\n---\nbody")[0].contains("YAML"));
    }

    #[test]
    fn name_rules() {
        assert!(name_problem("good-name-2").is_none());
        assert!(name_problem("").is_some());
        assert!(name_problem("Bad").is_some());
        assert!(name_problem("bad_name").is_some());
        assert!(name_problem("-bad").is_some());
        assert!(name_problem("bad-").is_some());
        assert!(name_problem("bad--name").is_some());
        assert!(name_problem(&"a".repeat(65)).is_some());
    }

    #[test]
    fn name_must_match_folder() {
        let errs = errors("other", "---\nname: a\ndescription: b\n---\nbody");
        assert!(errs[0].contains("does not match"));
    }

    #[test]
    fn long_description_is_an_error() {
        let content = format!("---\nname: a\ndescription: {}\n---\nbody", "x".repeat(1025));
        assert!(errors("a", &content)[0].contains("max"));
    }

    #[test]
    fn empty_body_is_a_warning() {
        let (_, issues) = check("a", "---\nname: a\ndescription: b\n---\n");
        assert_eq!(status_of(&issues), Status::Warnings);
    }

    #[test]
    fn template_is_valid() {
        let content = template("my-skill", "Does: things, \"quoted\"");
        let (manifest, issues) = check("my-skill", &content);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(
            manifest.description.as_deref(),
            Some("Does: things, \"quoted\"")
        );
        assert!(content.contains("# My Skill"));
    }

    #[test]
    fn body_strips_frontmatter() {
        assert_eq!(body_of("---\nname: a\n---\nhi"), "hi");
        assert_eq!(body_of("hi"), "hi");
    }
}
