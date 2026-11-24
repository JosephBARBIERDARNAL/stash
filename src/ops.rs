//! Adding and removing skills on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::skill::{self, SKILL_FILE};

/// Folders never copied when installing.
const SKIPPED: &[&str] = &[".git", "node_modules", "target", ".DS_Store"];

/// Name a skill folder from `src` should get once installed.
pub fn install_name(src: &Path) -> String {
    let folder = skill::folder_name(src);
    fs::read_to_string(src.join(SKILL_FILE))
        .ok()
        .and_then(|content| skill::check(&folder, &content).0.name)
        .filter(|name| skill::name_problem(name).is_none())
        .unwrap_or(folder)
}

/// Copy the skill folder `src` into `target_root`. Returns the new folder.
pub fn install_folder(src: &Path, target_root: &Path) -> Result<PathBuf> {
    if !src.join(SKILL_FILE).is_file() {
        bail!("{} has no {SKILL_FILE}", src.display());
    }
    let dest = target_root.join(install_name(src));
    if fs::symlink_metadata(&dest).is_ok() {
        bail!(
            "A skill named \"{}\" already exists there",
            skill::folder_name(&dest)
        );
    }
    fs::create_dir_all(target_root)
        .with_context(|| format!("Cannot create {}", target_root.display()))?;
    copy_dir(src, &dest).inspect_err(|_| {
        let _ = fs::remove_dir_all(&dest);
    })?;
    Ok(dest)
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("Cannot create {}", dest.display()))?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if SKIPPED.iter().any(|s| name == *s) {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir(&from, &to)?;
        } else if file_type.is_file() || (file_type.is_symlink() && from.is_file()) {
            fs::copy(&from, &to).with_context(|| format!("Cannot copy {}", from.display()))?;
        }
        // Symlinked folders are skipped to avoid copying outside the skill.
    }
    Ok(())
}

/// Create a new skill from the template. Returns its folder.
pub fn create(target_root: &Path, name: &str, description: &str) -> Result<PathBuf> {
    if let Some(problem) = skill::name_problem(name) {
        bail!(problem);
    }
    let dest = target_root.join(name);
    if fs::symlink_metadata(&dest).is_ok() {
        bail!("A skill named \"{name}\" already exists there");
    }
    fs::create_dir_all(&dest).with_context(|| format!("Cannot create {}", dest.display()))?;
    fs::write(dest.join(SKILL_FILE), skill::template(name, description))?;
    Ok(dest)
}

/// Remove a skill: links are unlinked (their target is kept), folders go to the Trash.
pub fn remove(dir: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(dir).with_context(|| format!("{} not found", dir.display()))?;
    if meta.file_type().is_symlink() {
        remove_link(dir)
    } else {
        trash::delete(dir).with_context(|| format!("Cannot move {} to the Trash", dir.display()))
    }
}

fn remove_link(link: &Path) -> Result<()> {
    // On Windows a directory symlink/junction must be removed as a directory.
    fs::remove_file(link)
        .or_else(|_| fs::remove_dir(link))
        .with_context(|| format!("Cannot remove link {}", link.display()))
}

/// Folders containing a `SKILL.md` under `root` (the root itself included).
pub fn find_skills(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    find_into(root, max_depth, &mut found);
    found.sort();
    found
}

fn find_into(dir: &Path, depth_left: usize, out: &mut Vec<PathBuf>) {
    if dir.join(SKILL_FILE).is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth_left == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if entry.file_type().is_ok_and(|t| t.is_dir()) && !SKIPPED.iter().any(|s| name == *s) {
            find_into(&entry.path(), depth_left - 1, out);
        }
    }
}

/// Shallow-clone a git repository into `dest`.
pub fn git_clone(url: &str, dest: &Path) -> Result<()> {
    let url = url.trim();
    if url.is_empty() {
        bail!("Enter a repository URL");
    }
    if url.starts_with('-') {
        bail!("Invalid repository URL");
    }
    let output = Command::new("git")
        .args(["clone", "--depth", "1", "--quiet", "--"])
        .arg(url)
        .arg(dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .context("Could not run git. Is it installed?")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("git clone failed: {}", err.trim());
    }
    Ok(())
}

/// List the files of a skill, relative to its folder, `SKILL.md` first.
pub fn list_files(dir: &Path, limit: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files(dir, dir, limit, &mut files);
    files.sort_by_key(|p| {
        (
            p != Path::new(SKILL_FILE),
            p.components().count(),
            p.clone(),
        )
    });
    files
}

fn collect_files(root: &Path, dir: &Path, limit: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if out.len() >= limit {
            return;
        }
        let name = entry.file_name();
        if SKIPPED.iter().any(|s| name == *s) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            if !entry.file_type().is_ok_and(|t| t.is_symlink()) {
                collect_files(root, &path, limit, out);
            }
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
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
    fn install_copies_files_and_uses_frontmatter_name() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("download-folder");
        write_skill(&src, "nice-skill");
        fs::create_dir_all(src.join("scripts")).unwrap();
        fs::write(src.join("scripts/run.sh"), "echo hi").unwrap();
        fs::create_dir_all(src.join(".git")).unwrap();

        let target = tmp.path().join("skills");
        let dest = install_folder(&src, &target).unwrap();
        assert_eq!(dest, target.join("nice-skill"));
        assert!(dest.join("scripts/run.sh").is_file());
        assert!(!dest.join(".git").exists());

        let again = install_folder(&src, &target).unwrap_err();
        assert!(again.to_string().contains("already exists"));
    }

    #[test]
    fn install_requires_skill_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(install_folder(tmp.path(), &tmp.path().join("t")).is_err());
    }

    #[test]
    fn create_writes_valid_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = create(tmp.path(), "fresh", "Fresh skill.").unwrap();
        let s = crate::skill::Skill::load(&dir, false);
        assert!(s.issues.is_empty(), "{:?}", s.issues);
        assert!(create(tmp.path(), "fresh", "").is_err());
        assert!(create(tmp.path(), "Bad Name", "").is_err());
    }

    #[test]
    fn find_skills_in_repo_layouts() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(&tmp.path().join("skills/one"), "one");
        write_skill(&tmp.path().join("skills/two"), "two");
        write_skill(&tmp.path().join("node_modules/x"), "x");
        let found = find_skills(tmp.path(), 3);
        assert_eq!(found.len(), 2);

        write_skill(tmp.path(), "root");
        assert_eq!(find_skills(tmp.path(), 3), vec![tmp.path().to_path_buf()]);
    }

    #[cfg(unix)]
    #[test]
    fn remove_symlink_keeps_target() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        write_skill(&real, "real");
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        remove(&link).unwrap();
        assert!(fs::symlink_metadata(&link).is_err());
        assert!(real.join(SKILL_FILE).is_file());
    }

    #[test]
    fn list_files_puts_skill_file_first() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(tmp.path(), "a");
        fs::create_dir_all(tmp.path().join("ref")).unwrap();
        fs::write(tmp.path().join("ref/guide.md"), "x").unwrap();
        fs::write(tmp.path().join("LICENSE"), "x").unwrap();
        let files = list_files(tmp.path(), 100);
        assert_eq!(files[0], PathBuf::from(SKILL_FILE));
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn git_clone_rejects_bad_urls() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(git_clone("", tmp.path()).is_err());
        assert!(git_clone("--upload-pack=x", tmp.path()).is_err());
    }
}
