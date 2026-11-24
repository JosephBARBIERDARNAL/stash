//! One entry per skill name, listing every tool that has it installed.

use crate::skill::{Skill, Status};
use crate::sources::Source;

/// A skill as installed for one tool.
#[derive(Debug, Clone)]
pub struct Install {
    pub provider: &'static str,
    pub skill: Skill,
}

/// All installs of a skill sharing the same name.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub installs: Vec<Install>,
}

impl Entry {
    /// The worst status across installs.
    pub fn status(&self) -> Status {
        self.installs
            .iter()
            .map(|i| i.skill.status())
            .max()
            .unwrap_or(Status::Valid)
    }

    pub fn description(&self) -> &str {
        self.installs
            .iter()
            .map(|i| i.skill.description.as_str())
            .find(|d| !d.is_empty())
            .unwrap_or("")
    }

    /// True when every install is managed by its tool.
    pub fn read_only(&self) -> bool {
        self.installs.iter().all(|i| i.skill.read_only)
    }

    pub fn providers(&self) -> Vec<&'static str> {
        let mut providers: Vec<_> = self.installs.iter().map(|i| i.provider).collect();
        providers.dedup();
        providers
    }

    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.name.to_lowercase().contains(query)
            || self.description().to_lowercase().contains(query)
            || self
                .installs
                .iter()
                .any(|i| i.provider.to_lowercase().contains(query))
    }
}

/// Merge skills from all sources into a flat list sorted by name.
/// Installs keep the order of `sources`.
pub fn group(sources: Vec<Source>) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for source in sources {
        for skill in source.skills {
            let install = Install {
                provider: source.label,
                skill,
            };
            match entries.iter_mut().find(|e| e.name == install.skill.name) {
                Some(entry) => entry.installs.push(install),
                None => entries.push(Entry {
                    name: install.skill.name.clone(),
                    installs: vec![install],
                }),
            }
        }
    }
    entries.sort_by_key(|e| e.name.to_lowercase());
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::Issue;
    use crate::sources::Kind;
    use std::path::PathBuf;

    fn skill(name: &str, invalid: bool) -> Skill {
        Skill {
            name: name.into(),
            description: format!("{name} description"),
            dir: PathBuf::from(name),
            link_target: None,
            read_only: false,
            issues: if invalid {
                vec![Issue {
                    severity: crate::skill::Severity::Error,
                    message: "bad".into(),
                }]
            } else {
                Vec::new()
            },
        }
    }

    fn source(label: &'static str, skills: Vec<Skill>) -> Source {
        Source {
            label,
            root: PathBuf::from(label),
            kind: Kind::Folder,
            skills,
        }
    }

    #[test]
    fn groups_by_name_and_sorts() {
        let entries = group(vec![
            source(
                "Claude Code",
                vec![skill("zeta", false), skill("Alpha", false)],
            ),
            source("Codex", vec![skill("zeta", true)]),
        ]);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta"]);
        assert_eq!(entries[1].providers(), ["Claude Code", "Codex"]);
        assert_eq!(entries[1].status(), Status::Invalid);
        assert_eq!(entries[0].status(), Status::Valid);
    }

    #[test]
    fn search_matches_name_description_and_provider() {
        let entries = group(vec![source("Codex", vec![skill("pdf", false)])]);
        assert!(entries[0].matches("pd"));
        assert!(entries[0].matches("description"));
        assert!(entries[0].matches("codex"));
        assert!(!entries[0].matches("word"));
    }
}
