//! Few-shot examples loader — reads Markdown files from a skill's `examples/` directory.

use std::path::Path;

/// A single few-shot example loaded from the `examples/` directory.
#[derive(Debug, Clone)]
pub struct SkillExample {
    /// The file stem (e.g. `01_basic` from `01_basic.md`).
    pub name: String,
    /// The full Markdown content.
    pub content: String,
}

impl SkillExample {
    /// Extract a numeric index from the name if it starts with digits.
    ///
    /// `01_basic` → `Some(1)`, `advanced` → `None`.
    pub fn index(&self) -> Option<usize> {
        self.name
            .split('_')
            .next()
            .and_then(|s| s.parse::<usize>().ok())
    }
}

/// Load all few-shot examples from a skill's `examples/` directory.
///
/// Files are sorted by name for consistent ordering. Non-`.md` files are skipped.
/// If the directory doesn't exist, returns an empty vector.
pub fn load_examples(skill_dir: &Path) -> Vec<SkillExample> {
    let examples_dir = skill_dir.join("examples");
    if !examples_dir.exists() {
        tracing::debug!(
            dir = %skill_dir.display(),
            "no examples directory, skipping"
        );
        return Vec::new();
    }

    let mut examples: Vec<(String, SkillExample)> = Vec::new();

    let entries = match std::fs::read_dir(&examples_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                path = %examples_dir.display(),
                error = %e,
                "failed to read examples directory"
            );
            return Vec::new();
        }
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }

        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                examples.push((name.clone(), SkillExample { name, content }));
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to read example file"
                );
            }
        }
    }

    // Sort by name for deterministic ordering
    examples.sort_by(|a, b| a.0.cmp(&b.0));
    examples.into_iter().map(|(_, ex)| ex).collect()
}

/// Load examples and return them indexed by their numeric index.
///
/// Only examples with a numeric prefix (e.g. `01_basic`) are included.
pub fn load_examples_indexed(skill_dir: &Path) -> Vec<(usize, SkillExample)> {
    load_examples(skill_dir)
        .into_iter()
        .filter_map(|ex| {
            let idx = ex.index()?;
            Some((idx, ex))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_from_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let examples = load_examples(tmp.path());
        assert!(examples.is_empty());
    }

    #[test]
    fn test_load_from_nonexistent_dir() {
        let examples = load_examples(Path::new("/nonexistent/path/skill"));
        assert!(examples.is_empty());
    }

    #[test]
    fn test_load_examples() {
        let tmp = tempfile::tempdir().unwrap();
        let examples_dir = tmp.path().join("examples");
        std::fs::create_dir_all(&examples_dir).unwrap();

        std::fs::write(examples_dir.join("01_basic.md"), "# Basic\n\nHello").unwrap();
        std::fs::write(examples_dir.join("02_advanced.md"), "# Advanced\n\nWorld").unwrap();
        std::fs::write(examples_dir.join("readme.txt"), "not markdown").unwrap();

        let examples = load_examples(tmp.path());
        assert_eq!(examples.len(), 2);
        assert_eq!(examples[0].name, "01_basic");
        assert_eq!(examples[0].content, "# Basic\n\nHello");
        assert_eq!(examples[1].name, "02_advanced");
    }

    #[test]
    fn test_examples_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let examples_dir = tmp.path().join("examples");
        std::fs::create_dir_all(&examples_dir).unwrap();

        // Write in reverse order
        std::fs::write(examples_dir.join("03_edge.md"), "third").unwrap();
        std::fs::write(examples_dir.join("01_basic.md"), "first").unwrap();
        std::fs::write(examples_dir.join("02_adv.md"), "second").unwrap();

        let examples = load_examples(tmp.path());
        assert_eq!(examples[0].name, "01_basic");
        assert_eq!(examples[1].name, "02_adv");
        assert_eq!(examples[2].name, "03_edge");
    }

    #[test]
    fn test_example_index() {
        let ex = SkillExample {
            name: "01_basic".to_string(),
            content: String::new(),
        };
        assert_eq!(ex.index(), Some(1));

        let ex = SkillExample {
            name: "10_advanced".to_string(),
            content: String::new(),
        };
        assert_eq!(ex.index(), Some(10));

        let ex = SkillExample {
            name: "no_prefix".to_string(),
            content: String::new(),
        };
        assert_eq!(ex.index(), None);
    }

    #[test]
    fn test_load_indexed() {
        let tmp = tempfile::tempdir().unwrap();
        let examples_dir = tmp.path().join("examples");
        std::fs::create_dir_all(&examples_dir).unwrap();

        std::fs::write(examples_dir.join("01_basic.md"), "first").unwrap();
        std::fs::write(examples_dir.join("02_adv.md"), "second").unwrap();
        std::fs::write(examples_dir.join("notes.md"), "no index").unwrap();

        let indexed = load_examples_indexed(tmp.path());
        assert_eq!(indexed.len(), 2);
        assert_eq!(indexed[0].0, 1);
        assert_eq!(indexed[1].0, 2);
    }

    #[test]
    fn test_skips_non_md_files() {
        let tmp = tempfile::tempdir().unwrap();
        let examples_dir = tmp.path().join("examples");
        std::fs::create_dir_all(&examples_dir).unwrap();

        std::fs::write(examples_dir.join("01_basic.md"), "markdown").unwrap();
        std::fs::write(examples_dir.join("02_basic.txt"), "text").unwrap();
        std::fs::write(examples_dir.join("03_basic.json"), "{}").unwrap();

        let examples = load_examples(tmp.path());
        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].name, "01_basic");
    }
}
