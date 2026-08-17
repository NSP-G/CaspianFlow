//! Skill IPC commands.
//!
//! These functions are designed to be annotated with `#[tauri::command]`
//! once the Tauri runtime is integrated. For now they are plain async
//! functions that can be called from Rust or wrapped by the frontend bridge.

use crate::skill::{Skill, SkillExample, SkillManager};
use crate::types::AppResult;

/// List all registered skills.
pub async fn list_skills(manager: &SkillManager) -> AppResult<Vec<Skill>> {
    Ok(manager.registry().list_all())
}

/// List only enabled skills.
pub async fn list_enabled_skills(manager: &SkillManager) -> AppResult<Vec<Skill>> {
    Ok(manager.registry().list_enabled())
}

/// Get a skill by name.
pub async fn get_skill(manager: &SkillManager, name: &str) -> AppResult<Option<Skill>> {
    Ok(manager.registry().get(name))
}

/// List skills by category.
pub async fn list_skills_by_category(
    manager: &SkillManager,
    category: &str,
) -> AppResult<Vec<Skill>> {
    Ok(manager.registry().list_by_category(category))
}

/// List skills by tag.
pub async fn list_skills_by_tag(manager: &SkillManager, tag: &str) -> AppResult<Vec<Skill>> {
    Ok(manager.registry().list_by_tag(tag))
}

/// List all categories.
pub async fn list_categories(manager: &SkillManager) -> AppResult<Vec<String>> {
    Ok(manager.registry().categories())
}

/// List all tags.
pub async fn list_tags(manager: &SkillManager) -> AppResult<Vec<String>> {
    Ok(manager.registry().tags())
}

/// Search skills by text query.
pub async fn search_skills(manager: &SkillManager, query: &str) -> AppResult<Vec<Skill>> {
    Ok(manager.registry().search(query))
}

/// Enable a skill by name.
pub async fn enable_skill(manager: &SkillManager, name: &str) -> AppResult<bool> {
    Ok(manager.registry().enable(name))
}

/// Disable a skill by name.
pub async fn disable_skill(manager: &SkillManager, name: &str) -> AppResult<bool> {
    Ok(manager.registry().disable(name))
}

/// Set the enabled state of a skill.
pub async fn set_skill_enabled(
    manager: &SkillManager,
    name: &str,
    enabled: bool,
) -> AppResult<bool> {
    Ok(manager.registry().set_enabled(name, enabled))
}

/// Reload skills from disk (re-scans the skills directory).
pub async fn reload_skills(manager: &SkillManager) -> AppResult<usize> {
    manager.reload().await?;
    Ok(manager.registry().count())
}

/// Load few-shot examples for a skill.
pub async fn load_skill_examples(
    manager: &SkillManager,
    name: &str,
) -> AppResult<Vec<SkillExample>> {
    Ok(manager.load_examples(name))
}

/// Create a new skill from the template.
pub async fn create_skill(manager: &SkillManager, name: &str) -> AppResult<String> {
    let dir = manager.create_skill(name)?;
    Ok(dir.to_string_lossy().to_string())
}

/// Get the total skill count.
pub async fn skill_count(manager: &SkillManager) -> AppResult<usize> {
    Ok(manager.registry().count())
}

/// Get the enabled skill count.
pub async fn enabled_skill_count(manager: &SkillManager) -> AppResult<usize> {
    Ok(manager.registry().count_enabled())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn make_skill_yaml(name: &str, category: &str, tags: Vec<&str>) -> String {
        let tags_yaml = tags
            .iter()
            .map(|t| format!("  - \"{t}\""))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"schema_version: "1.0"
name: "{name}"
display_name: "{name}"
version: "1.0.0"
description: "Test skill {name}"
category: "{category}"

trigger_phrases:
  - "test"

runtime:
  type: "python"
  entry: "script.py"
  timeout: 30
  memory_limit_mb: 256

input_schema:
  type: "object"

output_schema:
  type: "object"

permissions:
  fs: []
  network: false
  shell: false

tags:
{tags_yaml}

author: "Test"
license: "MIT"
"#,
        )
    }

    async fn setup_manager() -> (tempfile::TempDir, SkillManager) {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Create test skills
        let dir = skills_dir.join("read_file");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("skill.yaml"),
            make_skill_yaml("read_file", "file-system", vec!["file", "read"]),
        )
        .unwrap();

        let dir = skills_dir.join("send_email");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("skill.yaml"),
            make_skill_yaml("send_email", "network", vec!["email"]),
        )
        .unwrap();

        // Add examples to read_file
        let examples_dir = skills_dir.join("read_file").join("examples");
        std::fs::create_dir_all(&examples_dir).unwrap();
        std::fs::write(examples_dir.join("01_basic.md"), "# Basic example").unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        (tmp, manager)
    }

    #[tokio::test]
    async fn test_list_skills() {
        let (_tmp, manager) = setup_manager().await;
        let skills = list_skills(&manager).await.unwrap();
        // 2 user + built-in skills (read_file built-in skipped; user version kept)
        assert_eq!(
            skills.len(),
            crate::skill::builtin::BUILTIN_SKILL_NAMES.len() + 1
        );
    }

    #[tokio::test]
    async fn test_list_enabled_skills() {
        let (_tmp, manager) = setup_manager().await;
        let skills = list_enabled_skills(&manager).await.unwrap();
        // 2 user + built-in, all enabled
        assert_eq!(
            skills.len(),
            crate::skill::builtin::BUILTIN_SKILL_NAMES.len() + 1
        );

        disable_skill(&manager, "read_file").await.unwrap();
        let skills = list_enabled_skills(&manager).await.unwrap();
        // 1 disabled (read_file)
        assert_eq!(
            skills.len(),
            crate::skill::builtin::BUILTIN_SKILL_NAMES.len()
        );
    }

    #[tokio::test]
    async fn test_get_skill() {
        let (_tmp, manager) = setup_manager().await;
        let skill = get_skill(&manager, "read_file").await.unwrap();
        assert!(skill.is_some());
        assert_eq!(skill.unwrap().name, "read_file");

        let skill = get_skill(&manager, "nonexistent").await.unwrap();
        assert!(skill.is_none());
    }

    #[tokio::test]
    async fn test_list_by_category() {
        let (_tmp, manager) = setup_manager().await;
        let skills = list_skills_by_category(&manager, "file-system")
            .await
            .unwrap();
        // read_file (test) + write_file (built-in)
        assert_eq!(skills.len(), 2);
        assert!(skills.iter().any(|s| s.name == "read_file"));
    }

    #[tokio::test]
    async fn test_list_by_tag() {
        let (_tmp, manager) = setup_manager().await;
        let skills = list_skills_by_tag(&manager, "file").await.unwrap();
        // read_file (test) + write_file, file-reader, file-writer, file-search (built-in, tag "file")
        assert_eq!(skills.len(), 5);
        assert!(skills.iter().any(|s| s.name == "read_file"));
    }

    #[tokio::test]
    async fn test_list_categories() {
        let (_tmp, manager) = setup_manager().await;
        let cats = list_categories(&manager).await.unwrap();
        // file-system, network, system, text, file, data, self
        assert_eq!(cats.len(), 7);
        assert!(cats.contains(&"file-system".to_string()));
        assert!(cats.contains(&"network".to_string()));
    }

    #[tokio::test]
    async fn test_list_tags() {
        let (_tmp, manager) = setup_manager().await;
        let tags = list_tags(&manager).await.unwrap();
        assert!(tags.contains(&"file".to_string()));
        assert!(tags.contains(&"read".to_string()));
        assert!(tags.contains(&"email".to_string()));
    }

    #[tokio::test]
    async fn test_search_skills() {
        let (_tmp, manager) = setup_manager().await;
        let results = search_skills(&manager, "file").await.unwrap();
        // read_file + write_file + file-reader + file-writer + file-search + file-search desc
        assert_eq!(results.len(), 6);
        assert!(results.iter().any(|s| s.name == "read_file"));
    }

    #[tokio::test]
    async fn test_enable_disable() {
        let (_tmp, manager) = setup_manager().await;

        let result = disable_skill(&manager, "read_file").await.unwrap();
        assert!(result);

        let count = enabled_skill_count(&manager).await.unwrap();
        // all - 1 disabled = built-in count
        assert_eq!(count, crate::skill::builtin::BUILTIN_SKILL_NAMES.len());

        let result = enable_skill(&manager, "read_file").await.unwrap();
        assert!(result);

        let count = enabled_skill_count(&manager).await.unwrap();
        // All enabled
        assert_eq!(
            count,
            crate::skill::builtin::BUILTIN_SKILL_NAMES.len() + 1
        );
    }

    #[tokio::test]
    async fn test_set_skill_enabled() {
        let (_tmp, manager) = setup_manager().await;

        set_skill_enabled(&manager, "read_file", false)
            .await
            .unwrap();
        // total - 1 disabled = built-in count
        assert_eq!(
            enabled_skill_count(&manager).await.unwrap(),
            crate::skill::builtin::BUILTIN_SKILL_NAMES.len()
        );

        set_skill_enabled(&manager, "read_file", true)
            .await
            .unwrap();
        // All enabled
        assert_eq!(
            enabled_skill_count(&manager).await.unwrap(),
            crate::skill::builtin::BUILTIN_SKILL_NAMES.len() + 1
        );
    }

    #[tokio::test]
    async fn test_load_skill_examples() {
        let (_tmp, manager) = setup_manager().await;
        let examples = load_skill_examples(&manager, "read_file").await.unwrap();
        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].name, "01_basic");
    }

    #[tokio::test]
    async fn test_skill_count() {
        let (_tmp, manager) = setup_manager().await;
        let count = skill_count(&manager).await.unwrap();
        // 2 user + built-in (read_file built-in skipped)
        assert_eq!(count, crate::skill::builtin::BUILTIN_SKILL_NAMES.len() + 1);
    }

    #[tokio::test]
    async fn test_create_skill_command() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let manager = SkillManager::init(&skills_dir).await.unwrap();
        let dir = create_skill(&manager, "new_skill").await.unwrap();
        assert!(!dir.is_empty());

        // The skill.yaml should be valid
        let path = Path::new(&dir).join("skill.yaml");
        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_reload_skills() {
        let (_tmp, manager) = setup_manager().await;
        let count = reload_skills(&manager).await.unwrap();
        // 2 user + built-in (read_file built-in skipped)
        assert_eq!(count, crate::skill::builtin::BUILTIN_SKILL_NAMES.len() + 1);
    }
}
