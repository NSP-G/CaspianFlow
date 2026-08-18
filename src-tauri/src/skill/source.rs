//! External skill sources for the external-source protocol (P36 外部源协议).
//!
//! Lets CaspianFlow pull skills from places other than the local
//! `~/.caspian/skills` directory:
//!
//! - **Local** — a directory of skill folders (already scanned today).
//! - **Git** — clone a repo into the source cache, then scan it.
//! - **Http** — download a bundle into the source cache, then scan it.
//! - **Mcp** — connect to an MCP server and turn each of its tools into a
//!   virtual skill bound via [`crate::skill::schema::McpRef`] (see `mcp.rs` /
//!   B-2 检查点).
//!
//! Git/Http require network + the `git`/`curl` CLIs and are therefore
//! **best-effort**: they are implemented and unit-tested for parsing, but the
//! actual clone/download runs only where those tools + network are available
//! (recorded as a Seeker-local gate, same pattern as A1/P33). Local and Mcp are
//! fully exercised headlessly.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::config::CaspianPaths;
use crate::skill::mcp::{tools_to_skills, McpClient};
use crate::skill::scanner::SkillScanner;
use crate::skill::schema::Skill;

/// Declarative description of where skills come from. Serializable so a workspace
/// can list its external sources in a manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExternalSource {
    /// A local directory containing one or more skill folders.
    Local { path: PathBuf },
    /// A git repository (cloned into the source cache, then scanned).
    Git { url: String, rev: Option<String> },
    /// An HTTP(S) archive of skills (downloaded into the source cache).
    Http { url: String },
    /// An MCP server whose tools become skills.
    Mcp { server_command: Vec<String> },
}

impl ExternalSource {
    /// Short, filesystem-safe slug derived from the source identity.
    pub fn slug(&self) -> String {
        match self {
            ExternalSource::Local { path } => slugify(&path.to_string_lossy()),
            ExternalSource::Git { url, .. } => slugify(url),
            ExternalSource::Http { url } => slugify(url),
            ExternalSource::Mcp { server_command } => {
                slugify(&server_command.join("_"))
            }
        }
    }
}

/// Materialize a source into a local directory of skills (if it has one).
///
/// - `Local` → the path itself.
/// - `Git`/`Http` → clone/download into `cache` (best-effort; needs network).
/// - `Mcp` → `None` (tools are fetched live, not as files).
pub fn resolve_source(source: &ExternalSource, cache: &Path) -> Result<Option<PathBuf>, String> {
    match source {
        ExternalSource::Local { path } => Ok(Some(path.clone())),
        ExternalSource::Mcp { .. } => Ok(None),
        ExternalSource::Git { url, rev } => {
            let dest = cache.join(format!("git-{}", source.slug()));
            if dest.exists() {
                // Best-effort fast-forward; ignore failure (offline etc.).
                let _ = Command::new("git")
                    .args(["-C", &dest.to_string_lossy(), "pull", "--ff-only"])
                    .status();
                return Ok(Some(dest));
            }
            std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
            let mut cmd = Command::new("git");
            cmd.args(["clone", url, &dest.to_string_lossy()]);
            if let Some(r) = rev {
                cmd.args(["--branch", r]);
            }
            let status = cmd
                .status()
                .map_err(|e| format!("git unavailable: {e}"))?;
            if !status.success() {
                return Err(format!("git clone failed for {url}"));
            }
            Ok(Some(dest))
        }
        ExternalSource::Http { url } => {
            let dest = cache.join(format!("http-{}", source.slug()));
            if dest.exists() {
                return Ok(Some(dest));
            }
            std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
            let status = Command::new("curl")
                .args(["-fsSL", url, "-o", &dest.to_string_lossy()])
                .status()
                .map_err(|e| format!("curl unavailable: {e}"))?;
            if !status.success() {
                return Err(format!("download failed for {url}"));
            }
            Ok(Some(dest))
        }
    }
}

/// Load skills from a source. Local/Git/Http return scanned skill structs; Mcp
/// returns virtual skills bound to the server's tools.
pub async fn load_skills(
    source: &ExternalSource,
    paths: &CaspianPaths,
) -> Result<Vec<Skill>, String> {
    match source {
        ExternalSource::Mcp { server_command } => {
            let client = McpClient::start(server_command, None)
                .await
                .map_err(|e| e.to_string())?;
            let tools = client.list_tools().await.map_err(|e| e.to_string())?;
            Ok(tools_to_skills(server_command, &tools))
        }
        other => {
            let dir = resolve_source(other, &paths.shared.join("sources"))?
                .ok_or_else(|| "source has no local directory".to_string())?;
            let scanner = SkillScanner::new(&dir);
            let report = scanner.scan().await;
            Ok(report.skills)
        }
    }
}

/// Build a filesystem-safe slug from arbitrary text.
fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.push(c);
            prev_underscore = false;
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SAMPLE_YAML: &str = r#"schema_version: "1.0"
name: "ext_skill"
display_name: "ext_skill"
version: "1.0.0"
description: "test shell skill"
category: "test"
trigger_phrases:
  - "test"
runtime:
  type: "shell"
  entry: "run.sh"
  timeout: 30
  memory_limit_mb: 256
input_schema:
  type: "object"
output_schema:
  type: "object"
permissions:
  fs: []
  network: false
  shell: true
tags:
  - "test"
author: "test"
license: "MIT"
"#;

    fn write_sample_skill(dir: &Path, name: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        // Inject the directory name as the skill name so each sample is distinct.
        let yaml = SAMPLE_YAML.replace("ext_skill", name);
        let mut f = std::fs::File::create(skill_dir.join("skill.yaml")).unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
    }

    #[test]
    fn test_external_source_parse_all_variants() {
        let json = r#"{"type":"local","path":"/tmp/x"}"#;
        assert!(serde_json::from_str::<ExternalSource>(json).is_ok());
        let json = r#"{"type":"git","url":"https://github.com/a/b","rev":"main"}"#;
        assert!(serde_json::from_str::<ExternalSource>(json).is_ok());
        let json = r#"{"type":"http","url":"https://example.com/skills.tar.gz"}"#;
        assert!(serde_json::from_str::<ExternalSource>(json).is_ok());
        let json = r#"{"type":"mcp","server_command":["npx","-y","@x/server"]}"#;
        assert!(serde_json::from_str::<ExternalSource>(json).is_ok());
    }

    #[tokio::test]
    async fn test_load_skills_from_local_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("skills");
        write_sample_skill(&src, "alpha");
        write_sample_skill(&src, "beta");

        let paths = CaspianPaths::resolve(Some(dir.path()));
        let source = ExternalSource::Local { path: src };
        let skills = load_skills(&source, &paths).await.unwrap();
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("https://github.com/a/b"), "https_github_com_a_b");
        assert_eq!(slugify("/tmp/with space"), "tmp_with_space");
    }
}
