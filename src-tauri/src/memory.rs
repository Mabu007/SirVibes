//! What the agent knows before the conversation starts.
//!
//! Two kinds, kept apart because they have different lifetimes:
//!
//! - **user** — how this person likes to work. Lives with the install and
//!   follows them between projects.
//! - **project** — what this piece of work is. Lives in the workspace, so it
//!   travels with the folder and is obvious to anyone who opens it.
//!
//! Deliberately plain: a JSON file of short notes, injected into every system
//! prompt. Recall that depends on the agent remembering to go and look is
//! recall that does not happen, so there is no `recall` tool — what is
//! remembered is simply there, every turn.
//!
//! Small on purpose. Enough to stop the user repeating themselves; not a
//! knowledge base. Semantic retrieval can be added later without changing the
//! shape of what is stored.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Notes per scope. Past this the oldest go, so the prompt cannot grow without
/// limit and an agent that writes a note every turn cannot bury the useful ones.
const MAX_NOTES: usize = 40;
/// One note is a sentence or two, not a document.
const MAX_NOTE_CHARS: usize = 400;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Note {
    /// A short handle, so the same fact updates rather than accumulating.
    pub key: String,
    pub value: String,
    pub written_ms: u64,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Store {
    notes: Vec<Note>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scope {
    User,
    Project,
}

impl Scope {
    pub fn parse(raw: &str) -> Scope {
        match raw.trim().to_lowercase().as_str() {
            "user" | "me" | "preference" | "preferences" => Scope::User,
            _ => Scope::Project,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
        }
    }
}

/// Where each kind of memory lives. A project remembers itself inside itself.
pub fn path_for(scope: Scope, data_dir: &Path, workspace: Option<&Path>) -> Option<PathBuf> {
    match scope {
        Scope::User => Some(data_dir.join("memory.json")),
        Scope::Project => workspace.map(|ws| ws.join(".sirvibe").join("memory.json")),
    }
}

pub fn read(path: &Path) -> Vec<Note> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Store>(&raw).ok())
        .map(|store| store.notes)
        .unwrap_or_default()
}

/// Write one note. The same key replaces rather than repeats, so a preference
/// that changes does not leave both versions behind.
pub fn write(path: &Path, key: &str, value: &str) -> Result<Note, String> {
    let key = key.trim().to_lowercase();
    if key.is_empty() {
        return Err("a note needs a short key, so it can be updated later".into());
    }
    let value = value.trim();
    if value.is_empty() {
        return Err("a note needs something to say".into());
    }

    let note = Note {
        key: key.clone(),
        value: value.chars().take(MAX_NOTE_CHARS).collect(),
        written_ms: now_ms(),
    };

    let mut notes = read(path);
    notes.retain(|n| n.key != key);
    notes.push(note.clone());
    if notes.len() > MAX_NOTES {
        let excess = notes.len() - MAX_NOTES;
        notes.drain(..excess);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create '{}': {}", parent.display(), e))?;
    }
    let body = serde_json::to_string_pretty(&Store { notes })
        .map_err(|e| format!("cannot write that note: {}", e))?;
    std::fs::write(path, body).map_err(|e| format!("cannot save '{}': {}", path.display(), e))?;
    Ok(note)
}

pub fn forget(path: &Path, key: &str) -> Result<bool, String> {
    let key = key.trim().to_lowercase();
    let mut notes = read(path);
    let before = notes.len();
    notes.retain(|n| n.key != key);
    if notes.len() == before {
        return Ok(false);
    }
    let body = serde_json::to_string_pretty(&Store { notes })
        .map_err(|e| format!("cannot update memory: {}", e))?;
    std::fs::write(path, body).map_err(|e| e.to_string())?;
    Ok(true)
}

/// The block that goes into the system prompt. Empty when there is nothing to
/// say, so a new install carries no dead weight.
pub fn recall_block(data_dir: &Path, workspace: Option<&Path>) -> String {
    let mut sections: Vec<String> = Vec::new();
    for scope in [Scope::User, Scope::Project] {
        let Some(path) = path_for(scope, data_dir, workspace) else {
            continue;
        };
        let notes = read(&path);
        if notes.is_empty() {
            continue;
        }
        let lines: Vec<String> = notes
            .iter()
            .map(|n| format!("- {}: {}", n.key, n.value))
            .collect();
        sections.push(format!(
            "{}:\n{}",
            match scope {
                Scope::User => "About this person and how they work",
                Scope::Project => "About this project",
            },
            lines.join("\n")
        ));
    }
    sections.join("\n\n")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sirvibe-memory-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("memory.json")
    }

    #[test]
    fn a_note_survives_and_comes_back_in_the_prompt() {
        let dir = std::env::temp_dir().join("sirvibe-memory-recall");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        write(&dir.join("memory.json"), "captions", "Big bold captions, four words at a time").unwrap();
        let block = recall_block(&dir, None);
        assert!(block.contains("About this person"), "{}", block);
        assert!(block.contains("four words at a time"), "{}", block);
        assert!(!block.contains("About this project"), "{}", block);
    }

    #[test]
    fn the_same_fact_updates_rather_than_piling_up() {
        let path = temp("update");
        write(&path, "style", "fast cuts").unwrap();
        write(&path, "STYLE", "slow and considered").unwrap();
        let notes = read(&path);
        assert_eq!(notes.len(), 1, "{:?}", notes);
        assert_eq!(notes[0].value, "slow and considered");
    }

    #[test]
    fn memory_cannot_grow_until_it_owns_the_prompt() {
        let path = temp("bounded");
        for i in 0..(MAX_NOTES + 15) {
            write(&path, &format!("note-{}", i), "something").unwrap();
        }
        let notes = read(&path);
        assert_eq!(notes.len(), MAX_NOTES);
        assert!(notes.iter().any(|n| n.key == format!("note-{}", MAX_NOTES + 14)));
        assert!(!notes.iter().any(|n| n.key == "note-0"));

        let long = "x".repeat(5_000);
        let note = write(&path, "essay", &long).unwrap();
        assert_eq!(note.value.chars().count(), MAX_NOTE_CHARS);
    }

    #[test]
    fn a_note_can_be_taken_back() {
        let path = temp("forget");
        write(&path, "wrong", "the user hates music").unwrap();
        assert!(forget(&path, "wrong").unwrap());
        assert!(!forget(&path, "wrong").unwrap(), "already gone");
        assert!(read(&path).is_empty());
    }

    #[test]
    fn an_empty_note_is_refused_rather_than_stored() {
        let path = temp("empty");
        assert!(write(&path, "", "something").is_err());
        assert!(write(&path, "key", "   ").is_err());
        assert!(read(&path).is_empty());
    }

    #[test]
    fn a_project_remembers_itself_inside_itself() {
        let data = std::env::temp_dir().join("sirvibe-memory-scopes-data");
        let ws = std::env::temp_dir().join("sirvibe-memory-scopes-ws");
        let user = path_for(Scope::User, &data, Some(&ws)).unwrap();
        let project = path_for(Scope::Project, &data, Some(&ws)).unwrap();
        assert!(user.starts_with(&data));
        assert!(project.starts_with(&ws), "project memory travels with the folder");
        assert!(project.to_string_lossy().contains(".sirvibe"));
        assert!(path_for(Scope::Project, &data, None).is_none());
    }

    #[test]
    fn the_words_a_user_would_use_reach_the_right_scope() {
        assert_eq!(Scope::parse("user"), Scope::User);
        assert_eq!(Scope::parse("preferences"), Scope::User);
        assert_eq!(Scope::parse("project"), Scope::Project);
        assert_eq!(Scope::parse("anything else"), Scope::Project);
    }
}
