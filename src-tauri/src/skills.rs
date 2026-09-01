//! Skills are Markdown files on disk — editorial intelligence, not code. The
//! bundled skills are loaded by exactly this loader, from exactly this format,
//! so a user-authored skill is indistinguishable from a first-party one.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub when_to_use: String,
    pub path: String,
    pub source: String,
}

#[derive(Serialize, Clone)]
pub struct SkillDir {
    pub path: String,
    pub source: String,
    pub exists: bool,
}

/// Every directory searched for skills, in precedence order. Later entries with
/// the same skill name win, so a workspace skill can override a bundled one.
pub fn skill_dirs(
    bundled: Option<PathBuf>,
    user_dir: &Path,
    extra: &[String],
    workspace: Option<&Path>,
) -> Vec<SkillDir> {
    let mut dirs = Vec::new();
    if let Some(b) = bundled {
        dirs.push(SkillDir {
            exists: b.is_dir(),
            path: b.to_string_lossy().to_string(),
            source: "bundled".into(),
        });
    }
    dirs.push(SkillDir {
        exists: user_dir.is_dir(),
        path: user_dir.to_string_lossy().to_string(),
        source: "user".into(),
    });
    for e in extra {
        let p = PathBuf::from(crate::workspace::expand_home(e));
        dirs.push(SkillDir {
            exists: p.is_dir(),
            path: p.to_string_lossy().to_string(),
            source: "custom".into(),
        });
    }
    if let Some(ws) = workspace {
        let p = ws.join("skills");
        dirs.push(SkillDir {
            exists: p.is_dir(),
            path: p.to_string_lossy().to_string(),
            source: "workspace".into(),
        });
    }
    dirs
}

pub fn discover(dirs: &[SkillDir]) -> Vec<Skill> {
    let mut skills: Vec<Skill> = Vec::new();
    for dir in dirs {
        if !dir.exists {
            continue;
        }
        for entry in walkdir::WalkDir::new(&dir.path)
            .max_depth(3)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            if let Some(skill) = parse(path, &dir.source) {
                skills.retain(|s| s.name != skill.name);
                skills.push(skill);
            }
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

pub fn read(dirs: &[SkillDir], name: &str) -> Result<String, String> {
    let skills = discover(dirs);
    let skill = skills
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            format!(
                "no skill named '{}'. Available: {}",
                name,
                skills
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    std::fs::read_to_string(&skill.path).map_err(|e| format!("cannot read skill: {}", e))
}

/// Editing is confined to directories the app already searches, so a path from
/// the interface cannot be used to read or write somewhere else.
fn is_within_known_dir(dirs: &[SkillDir], path: &Path) -> bool {
    let target = crate::workspace::canonical_ish(path);
    dirs.iter().any(|d| {
        let root = crate::workspace::canonical_ish(Path::new(&d.path));
        target.starts_with(&root)
    })
}

pub fn read_file(dirs: &[SkillDir], path: &str) -> Result<String, String> {
    let p = PathBuf::from(path);
    if !is_within_known_dir(dirs, &p) {
        return Err("That file is not in a skills folder.".into());
    }
    std::fs::read_to_string(&p).map_err(|e| format!("cannot read the skill: {}", e))
}

/// Write a skill into the user's own skills folder. Editing a bundled skill
/// saves a copy here under the same name, which then overrides the original —
/// the shipped file is never modified.
pub fn write_user_skill(user_dir: &Path, name: &str, content: &str) -> Result<String, String> {
    let file_name = safe_file_name(name)?;
    std::fs::create_dir_all(user_dir).map_err(|e| format!("cannot create skills folder: {}", e))?;
    let path = user_dir.join(format!("{}.md", file_name));
    std::fs::write(&path, content).map_err(|e| format!("cannot save the skill: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}

pub fn delete_file(dirs: &[SkillDir], path: &str) -> Result<(), String> {
    let p = PathBuf::from(path);
    if !is_within_known_dir(dirs, &p) {
        return Err("That file is not in a skills folder.".into());
    }
    let is_bundled = dirs
        .iter()
        .filter(|d| d.source == "bundled")
        .any(|d| crate::workspace::canonical_ish(&p)
            .starts_with(crate::workspace::canonical_ish(Path::new(&d.path))));
    if is_bundled {
        return Err("Built-in skills cannot be deleted. Edit it instead — your version overrides the original.".into());
    }
    std::fs::remove_file(&p).map_err(|e| format!("cannot delete the skill: {}", e))
}

/// What an import actually did, so the interface can say so rather than
/// leaving the user to guess whether anything happened.
#[derive(Serialize, Clone, Debug)]
pub struct Imported {
    /// The name the skill will appear under — from its frontmatter, which is
    /// often not the file name the user picked.
    pub name: String,
    pub path: String,
    /// True when a skill of the same name was already there and has been
    /// overwritten. Silently replacing is what makes an import look like it
    /// did nothing.
    pub replaced: bool,
}

#[derive(Serialize, Clone)]
pub struct ImportFailure {
    pub source: String,
    pub reason: String,
}

#[derive(Serialize, Clone, Default)]
pub struct ImportReport {
    pub imported: Vec<Imported>,
    pub failed: Vec<ImportFailure>,
}

/// Import everything the user picked, reporting each result separately: one bad
/// file should not throw away the good ones.
pub fn import_all(user_dir: &Path, sources: &[String]) -> ImportReport {
    let mut report = ImportReport::default();
    for source in sources {
        match import_file(user_dir, source) {
            Ok(imported) => report.imported.push(imported),
            Err(reason) => report.failed.push(ImportFailure {
                source: source.clone(),
                reason,
            }),
        }
    }
    report
}

/// Copy a Markdown file — or a folder holding one — into the skills folder.
pub fn import_file(user_dir: &Path, source: &str) -> Result<Imported, String> {
    let src = PathBuf::from(crate::workspace::expand_home(source));
    if src.is_dir() {
        return import_folder(user_dir, &src);
    }
    let extension = src
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    if extension != "md" && extension != "markdown" {
        return Err("A skill must be a Markdown (.md) file, or a folder containing one.".into());
    }
    let content =
        std::fs::read_to_string(&src).map_err(|e| format!("cannot read that file: {}", e))?;
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "imported-skill".into());
    let file_name = safe_file_name(&stem)?;
    let target = user_dir.join(format!("{}.md", file_name));
    let replaced = target.exists();
    let path = write_user_skill(user_dir, &file_name, &content)?;
    Ok(Imported {
        name: parse(Path::new(&path), "user")
            .map(|s| s.name)
            .unwrap_or(file_name),
        path,
        replaced,
    })
}

/// A folder-shaped skill: SKILL.md plus whatever it references. The whole
/// folder is copied, so relative links inside it keep working.
fn import_folder(user_dir: &Path, src: &Path) -> Result<Imported, String> {
    let primary = primary_markdown(src)
        .ok_or("That folder has no Markdown file in it, so there is no skill to import.")?;
    let stem = src
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "imported-skill".into());
    let folder_name = safe_file_name(&stem)?;
    let target = user_dir.join(&folder_name);
    let replaced = target.exists();
    std::fs::create_dir_all(user_dir).map_err(|e| format!("cannot create skills folder: {}", e))?;
    copy_tree(src, &target, 0)?;
    let copied = target.join(primary.file_name().unwrap_or_default());
    Ok(Imported {
        name: parse(&copied, "user")
            .map(|s| s.name)
            .unwrap_or(folder_name),
        path: target.to_string_lossy().to_string(),
        replaced,
    })
}

fn primary_markdown(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
                    .unwrap_or(false)
        })
        .collect();
    candidates.sort();
    candidates
        .iter()
        .find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("skill") || s.eq_ignore_ascii_case("index"))
                .unwrap_or(false)
        })
        .cloned()
        .or_else(|| candidates.first().cloned())
}

fn copy_tree(from: &Path, to: &Path, depth: usize) -> Result<(), String> {
    if depth > 3 {
        return Ok(());
    }
    std::fs::create_dir_all(to).map_err(|e| format!("cannot create '{}': {}", to.display(), e))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("cannot read that folder: {}", e))?;
    for entry in entries.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target, depth + 1)?;
        } else {
            std::fs::copy(&source, &target)
                .map_err(|e| format!("cannot copy '{}': {}", source.display(), e))?;
        }
    }
    Ok(())
}

fn safe_file_name(name: &str) -> Result<String, String> {
    let cleaned: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    let mut out = String::new();
    let mut dash = false;
    for c in cleaned.chars() {
        if c == '-' {
            if !dash {
                out.push(c);
            }
            dash = true;
        } else {
            out.push(c);
            dash = false;
        }
    }
    let out: String = out.chars().take(60).collect();
    if out.is_empty() {
        Err("Give the skill a name.".into())
    } else {
        Ok(out)
    }
}

fn parse(path: &Path, source: &str) -> Option<Skill> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (front, body) = split_frontmatter(&raw);

    let stem = path.file_stem()?.to_string_lossy().to_string();
    let fallback_name = if matches!(stem.to_lowercase().as_str(), "skill" | "index" | "readme") {
        path.parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(stem)
    } else {
        stem
    };

    let name = front_value(&front, "name").unwrap_or(fallback_name);
    let description = front_value(&front, "description")
        .or_else(|| first_paragraph(body))
        .unwrap_or_default();
    let when_to_use = front_value(&front, "when_to_use")
        .or_else(|| front_value(&front, "when-to-use"))
        .unwrap_or_default();

    Some(Skill {
        name,
        description,
        when_to_use,
        path: path.to_string_lossy().to_string(),
        source: source.to_string(),
    })
}

fn split_frontmatter(raw: &str) -> (String, &str) {
    let trimmed = raw.trim_start_matches('\u{feff}');
    if let Some(rest) = trimmed.strip_prefix("---") {
        let rest = rest.trim_start_matches(['\r', '\n']);
        if let Some(end) = rest.find("\n---") {
            let front = &rest[..end];
            let body = rest[end + 4..].trim_start_matches(['\r', '\n']);
            return (front.to_string(), body);
        }
    }
    (String::new(), trimmed)
}

fn front_value(front: &str, key: &str) -> Option<String> {
    for line in front.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(v) = rest.strip_prefix(':') {
                let v = v.trim().trim_matches('"').trim_matches('\'').trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn first_paragraph(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("eplug-skills-{}", name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn as_dirs(paths: Vec<(PathBuf, &str)>) -> Vec<SkillDir> {
        paths
            .into_iter()
            .map(|(p, source)| SkillDir {
                exists: p.is_dir(),
                path: p.to_string_lossy().to_string(),
                source: source.to_string(),
            })
            .collect()
    }

    #[test]
    fn frontmatter_is_parsed() {
        let d = dir("front");
        std::fs::write(
            d.join("shorts.md"),
            "---\nname: shorts\ndescription: Cut vertical clips.\nwhen_to_use: User wants Shorts.\n---\n\n# Shorts\n\nBody.\n",
        )
        .unwrap();
        let found = discover(&as_dirs(vec![(d, "user")]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "shorts");
        assert_eq!(found[0].description, "Cut vertical clips.");
        assert_eq!(found[0].when_to_use, "User wants Shorts.");
    }

    #[test]
    fn a_plain_markdown_file_is_a_valid_skill() {
        let d = dir("plain");
        std::fs::write(d.join("my-house-style.md"), "# House Style\n\nAlways grade warm.\n")
            .unwrap();
        let found = discover(&as_dirs(vec![(d, "user")]));
        assert_eq!(found[0].name, "my-house-style");
        assert_eq!(found[0].description, "Always grade warm.");
    }

    #[test]
    fn skills_can_be_organised_in_folders() {
        let d = dir("folders");
        std::fs::create_dir_all(d.join("captions")).unwrap();
        std::fs::write(d.join("captions/SKILL.md"), "Caption rules.\n").unwrap();
        let found = discover(&as_dirs(vec![(d, "user")]));
        assert_eq!(found[0].name, "captions");
    }

    #[test]
    fn a_user_skill_overrides_a_bundled_one_of_the_same_name() {
        let bundled = dir("bundled");
        let user = dir("override");
        std::fs::write(bundled.join("captions.md"), "---\nname: captions\ndescription: stock\n---\n")
            .unwrap();
        std::fs::write(user.join("captions.md"), "---\nname: captions\ndescription: mine\n---\n")
            .unwrap();
        let found = discover(&as_dirs(vec![(bundled, "bundled"), (user, "user")]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].description, "mine");
        assert_eq!(found[0].source, "user");
    }

    #[test]
    fn reading_an_unknown_skill_lists_what_is_available() {
        let d = dir("unknown");
        std::fs::write(d.join("shorts.md"), "x").unwrap();
        let dirs = as_dirs(vec![(d, "user")]);
        assert!(read(&dirs, "shorts").is_ok());
        let err = read(&dirs, "nope").unwrap_err();
        assert!(err.contains("Available: shorts"));
    }

    #[test]
    fn the_bundled_skills_load_through_the_same_path_as_user_skills() {
        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/skills");
        let found = discover(&as_dirs(vec![(bundled, "bundled")]));
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"shorts"), "got {:?}", names);
        assert!(names.contains(&"captions"));
        assert!(names.contains(&"hyperframes"), "captions are built with it: {:?}", names);
        assert!(names.contains(&"references"), "\"make it look like this\": {:?}", names);
        assert!(names.contains(&"video-analysis"));
        assert!(names.contains(&"podcast-editing"));
        assert!(found.iter().all(|s| !s.description.is_empty()));
        assert!(found.iter().all(|s| !s.when_to_use.is_empty()));
    }
}

#[cfg(test)]
mod editing_tests {
    use super::*;

    fn setup(name: &str) -> (PathBuf, PathBuf, Vec<SkillDir>) {
        let root = std::env::temp_dir().join(format!("sirvibe-edit-{}", name));
        let _ = std::fs::remove_dir_all(&root);
        let bundled = root.join("bundled");
        let user = root.join("user");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        let dirs = vec![
            SkillDir { path: bundled.to_string_lossy().into(), source: "bundled".into(), exists: true },
            SkillDir { path: user.to_string_lossy().into(), source: "user".into(), exists: true },
        ];
        (bundled, user, dirs)
    }

    #[test]
    fn a_written_skill_is_discovered_immediately() {
        let (_b, user, dirs) = setup("write");
        let path = write_user_skill(&user, "My House Style", "---\nname: my-house-style\ndescription: Ours.\n---\nBody.")
            .unwrap();
        assert!(path.ends_with("my-house-style.md"));
        let found = discover(&dirs);
        assert!(found.iter().any(|s| s.name == "my-house-style"));
    }

    #[test]
    fn editing_a_bundled_skill_writes_a_user_copy_that_overrides_it() {
        let (bundled, user, dirs) = setup("override");
        std::fs::write(bundled.join("captions.md"), "---\nname: captions\ndescription: stock\n---\n").unwrap();
        assert_eq!(discover(&dirs)[0].description, "stock");

        write_user_skill(&user, "captions", "---\nname: captions\ndescription: mine\n---\n").unwrap();
        let found = discover(&dirs);
        assert_eq!(found.len(), 1, "the override replaces, not duplicates");
        assert_eq!(found[0].description, "mine");
        assert_eq!(found[0].source, "user");
        // The shipped file is untouched.
        assert!(std::fs::read_to_string(bundled.join("captions.md")).unwrap().contains("stock"));
    }

    #[test]
    fn files_outside_a_skills_folder_cannot_be_read_or_deleted() {
        let (_b, _u, dirs) = setup("confined");
        let outside = std::env::temp_dir().join("sirvibe-not-a-skill.md");
        std::fs::write(&outside, "secret").unwrap();
        let path = outside.to_string_lossy().to_string();
        assert!(read_file(&dirs, &path).unwrap_err().contains("not in a skills folder"));
        assert!(delete_file(&dirs, &path).unwrap_err().contains("not in a skills folder"));
        assert!(outside.exists(), "the file must be left alone");
    }

    #[test]
    fn bundled_skills_cannot_be_deleted() {
        let (bundled, _u, dirs) = setup("nodelete");
        let target = bundled.join("captions.md");
        std::fs::write(&target, "x").unwrap();
        let err = delete_file(&dirs, &target.to_string_lossy()).unwrap_err();
        assert!(err.contains("Built-in skills cannot be deleted"));
        assert!(target.exists());
    }

    #[test]
    fn a_user_skill_can_be_deleted() {
        let (_b, user, dirs) = setup("delete");
        let path = write_user_skill(&user, "temp", "x").unwrap();
        delete_file(&dirs, &path).unwrap();
        assert!(!PathBuf::from(&path).exists());
    }

    #[test]
    fn importing_requires_markdown_and_lands_in_the_user_folder() {
        let (_b, user, _d) = setup("import");
        let wrong = std::env::temp_dir().join("sirvibe-import.txt");
        std::fs::write(&wrong, "x").unwrap();
        assert!(import_file(&user, &wrong.to_string_lossy()).unwrap_err().contains("Markdown"));

        let right = std::env::temp_dir().join("sirvibe-import-good.md");
        std::fs::write(&right, "# Imported\n\nRules.").unwrap();
        let saved = import_file(&user, &right.to_string_lossy()).unwrap();
        assert!(saved.path.ends_with("sirvibe-import-good.md"));
        assert!(!saved.replaced);
        assert!(std::fs::read_to_string(&saved.path).unwrap().contains("Rules."));
    }

    #[test]
    fn an_import_reports_the_name_it_will_appear_under() {
        let (_b, user, dirs) = setup("import-name");
        let file = std::env::temp_dir().join("sirvibe-name-differs.md");
        std::fs::write(
            &file,
            "---\nname: house-style\ndescription: Ours.\n---\n\nBody.",
        )
        .unwrap();

        let saved = import_file(&user, &file.to_string_lossy()).unwrap();
        // The file was called one thing; the skill is listed as another. Saying
        // so is the difference between "it worked" and "nothing happened".
        assert_eq!(saved.name, "house-style");
        assert!(discover(&dirs).iter().any(|s| s.name == "house-style"));
    }

    #[test]
    fn re_importing_the_same_skill_says_it_replaced_one() {
        let (_b, user, dirs) = setup("import-twice");
        let file = std::env::temp_dir().join("sirvibe-twice.md");
        std::fs::write(&file, "---\nname: twice\ndescription: v1\n---\n").unwrap();
        assert!(!import_file(&user, &file.to_string_lossy()).unwrap().replaced);

        std::fs::write(&file, "---\nname: twice\ndescription: v2\n---\n").unwrap();
        let again = import_file(&user, &file.to_string_lossy()).unwrap();
        assert!(again.replaced, "a silent overwrite reads as a failed import");
        let found = discover(&dirs);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].description, "v2");
    }

    #[test]
    fn a_folder_shaped_skill_can_be_imported_whole() {
        let (_b, user, dirs) = setup("import-folder");
        let src = std::env::temp_dir().join("sirvibe-folder-skill");
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(src.join("examples")).unwrap();
        std::fs::write(
            src.join("SKILL.md"),
            "---\nname: folder-skill\ndescription: From a folder.\n---\n",
        )
        .unwrap();
        std::fs::write(src.join("examples/one.txt"), "reference").unwrap();

        let saved = import_file(&user, &src.to_string_lossy()).unwrap();
        assert_eq!(saved.name, "folder-skill");
        assert!(PathBuf::from(&saved.path).join("examples/one.txt").is_file());
        assert!(discover(&dirs).iter().any(|s| s.name == "folder-skill"));
    }

    #[test]
    fn a_bad_file_does_not_discard_the_good_ones() {
        let (_b, user, _d) = setup("import-many");
        let good = std::env::temp_dir().join("sirvibe-many-good.md");
        let bad = std::env::temp_dir().join("sirvibe-many-bad.txt");
        std::fs::write(&good, "# Good\n\nRules.").unwrap();
        std::fs::write(&bad, "x").unwrap();

        let report = import_all(
            &user,
            &[good.to_string_lossy().to_string(), bad.to_string_lossy().to_string()],
        );
        assert_eq!(report.imported.len(), 1);
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].reason.contains("Markdown"));
    }

    #[test]
    fn skill_names_are_turned_into_safe_file_names() {
        assert_eq!(safe_file_name("My House Style!").unwrap(), "my-house-style");
        assert_eq!(safe_file_name("  a//b  ").unwrap(), "a-b");
        assert!(safe_file_name("   ").is_err());
        // No traversal can survive.
        assert_eq!(safe_file_name("../../etc/passwd").unwrap(), "etc-passwd");
    }
}
