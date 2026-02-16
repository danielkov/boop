use std::path::Path;

use crate::errors::DetectError;

type Detector = fn(&Path) -> Result<Option<String>, DetectError>;

/// Scan `base` for known package manifests and extract version if present.
/// First match wins, scanned in priority order per RFC 001.
pub fn detect_version(base: &Path) -> Result<Option<String>, DetectError> {
    let detectors: &[Detector] = &[
        detect_cargo_toml,
        detect_package_json,
        detect_pyproject_toml,
        detect_setup_cfg,
        detect_go_mod,
        detect_pom_xml,
        detect_gradle,
        detect_gemspec,
        detect_mix_exs,
        detect_pubspec_yaml,
        detect_composer_json,
        detect_csproj,
        // Package.swift skipped per RFC
        detect_cmakelists,
        detect_deno_json,
    ];

    for detector in detectors {
        if let Some(version) = detector(base)? {
            return Ok(Some(version));
        }
    }

    Ok(None)
}

fn read_if_exists(path: &Path) -> Result<Option<String>, DetectError> {
    if path.exists() {
        Ok(Some(std::fs::read_to_string(path)?))
    } else {
        Ok(None)
    }
}

fn detect_cargo_toml(base: &Path) -> Result<Option<String>, DetectError> {
    let path = base.join("Cargo.toml");
    let Some(content) = read_if_exists(&path)? else {
        return Ok(None);
    };
    let parsed: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    Ok(parsed
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(String::from))
}

fn detect_package_json(base: &Path) -> Result<Option<String>, DetectError> {
    let path = base.join("package.json");
    let Some(content) = read_if_exists(&path)? else {
        return Ok(None);
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    Ok(parsed
        .get("version")
        .and_then(|v| v.as_str())
        .map(String::from))
}

fn detect_pyproject_toml(base: &Path) -> Result<Option<String>, DetectError> {
    let path = base.join("pyproject.toml");
    let Some(content) = read_if_exists(&path)? else {
        return Ok(None);
    };
    let parsed: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    // Try project.version first, then tool.poetry.version
    if let Some(version) = parsed
        .get("project")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(Some(version.to_string()));
    }
    Ok(parsed
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(String::from))
}

fn detect_setup_cfg(base: &Path) -> Result<Option<String>, DetectError> {
    let path = base.join("setup.cfg");
    let Some(content) = read_if_exists(&path)? else {
        return Ok(None);
    };
    // Simple INI-like parsing: find [metadata] section, then version = ...
    let mut in_metadata = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_metadata = trimmed.eq_ignore_ascii_case("[metadata]");
            continue;
        }
        if in_metadata && let Some(rest) = trimmed.strip_prefix("version") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                let version = value.trim();
                if !version.is_empty() {
                    return Ok(Some(version.to_string()));
                }
            }
        }
    }
    Ok(None)
}

fn detect_go_mod(base: &Path) -> Result<Option<String>, DetectError> {
    let go_mod = base.join("go.mod");
    if !go_mod.exists() {
        return Ok(None);
    }
    // go.mod exists — version comes from git tags; read them from the filesystem
    let git_dir = base.join(".git");
    if !git_dir.is_dir() {
        return Ok(None);
    }

    let mut tags = Vec::new();

    // Loose tags in .git/refs/tags/
    let tags_dir = git_dir.join("refs").join("tags");
    if tags_dir.is_dir() {
        for entry in std::fs::read_dir(&tags_dir)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                tags.push(name.to_string());
            }
        }
    }

    // Packed refs
    let packed_refs = git_dir.join("packed-refs");
    if packed_refs.exists() {
        let content = std::fs::read_to_string(&packed_refs)?;
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.starts_with('^') {
                continue;
            }
            // Format: <sha> refs/tags/<name>
            if let Some(refpath) = line.split_whitespace().nth(1)
                && let Some(name) = refpath.strip_prefix("refs/tags/")
            {
                tags.push(name.to_string());
            }
        }
    }

    // Parse as semver (strip optional 'v' prefix) and return the latest
    let mut versions: Vec<(semver::Version, String)> = tags
        .iter()
        .filter_map(|tag| {
            let stripped = tag.strip_prefix('v').unwrap_or(tag);
            semver::Version::parse(stripped)
                .ok()
                .map(|v| (v, stripped.to_string()))
        })
        .collect();

    versions.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(versions.last().map(|(_, ver)| ver.clone()))
}

fn detect_pom_xml(base: &Path) -> Result<Option<String>, DetectError> {
    let path = base.join("pom.xml");
    let Some(content) = read_if_exists(&path)? else {
        return Ok(None);
    };
    // Simple regex: find <version> inside <project> but not inside nested elements like <parent>
    // Strategy: find the first <version>...</version> that's not inside a <parent> block
    let re = regex::Regex::new(r"<version>([^<]+)</version>").unwrap();
    // Remove <parent>...</parent> blocks first to avoid matching parent version
    let parent_re = regex::Regex::new(r"(?s)<parent>.*?</parent>").unwrap();
    let cleaned = parent_re.replace_all(&content, "");
    if let Some(caps) = re.captures(&cleaned) {
        return Ok(Some(caps[1].trim().to_string()));
    }
    Ok(None)
}

fn detect_gradle(base: &Path) -> Result<Option<String>, DetectError> {
    let re = regex::Regex::new(r#"version\s*=\s*["'](.+?)["']|version\s+["'](.+?)["']"#).unwrap();
    for name in &["build.gradle", "build.gradle.kts"] {
        let path = base.join(name);
        let Some(content) = read_if_exists(&path)? else {
            continue;
        };
        if let Some(caps) = re.captures(&content) {
            let version = caps.get(1).or_else(|| caps.get(2)).unwrap();
            return Ok(Some(version.as_str().to_string()));
        }
    }
    Ok(None)
}

fn detect_gemspec(base: &Path) -> Result<Option<String>, DetectError> {
    let pattern = base.join("*.gemspec");
    let pattern_str = pattern.to_string_lossy();
    let paths: Vec<_> = glob::glob(&pattern_str)
        .map_err(std::io::Error::other)?
        .filter_map(|r| r.ok())
        .collect();
    let re = regex::Regex::new(r#"\.version\s*=\s*["'](.+?)["']"#).unwrap();
    for path in paths {
        let content = std::fs::read_to_string(&path)?;
        if let Some(caps) = re.captures(&content) {
            return Ok(Some(caps[1].to_string()));
        }
    }
    Ok(None)
}

fn detect_mix_exs(base: &Path) -> Result<Option<String>, DetectError> {
    let path = base.join("mix.exs");
    let Some(content) = read_if_exists(&path)? else {
        return Ok(None);
    };
    // Try @version "..." first, then version: "..."
    let at_re = regex::Regex::new(r#"@version\s+"(.+?)""#).unwrap();
    if let Some(caps) = at_re.captures(&content) {
        return Ok(Some(caps[1].to_string()));
    }
    let kw_re = regex::Regex::new(r#"version:\s+"(.+?)""#).unwrap();
    if let Some(caps) = kw_re.captures(&content) {
        return Ok(Some(caps[1].to_string()));
    }
    Ok(None)
}

fn detect_pubspec_yaml(base: &Path) -> Result<Option<String>, DetectError> {
    let path = base.join("pubspec.yaml");
    let Some(content) = read_if_exists(&path)? else {
        return Ok(None);
    };
    // Simple line-based parsing for "version: <value>"
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("version:") {
            let version = rest.trim();
            if !version.is_empty() {
                return Ok(Some(version.to_string()));
            }
        }
    }
    Ok(None)
}

fn detect_composer_json(base: &Path) -> Result<Option<String>, DetectError> {
    let path = base.join("composer.json");
    let Some(content) = read_if_exists(&path)? else {
        return Ok(None);
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    Ok(parsed
        .get("version")
        .and_then(|v| v.as_str())
        .map(String::from))
}

fn detect_csproj(base: &Path) -> Result<Option<String>, DetectError> {
    let pattern = base.join("*.csproj");
    let pattern_str = pattern.to_string_lossy();
    let paths: Vec<_> = glob::glob(&pattern_str)
        .map_err(std::io::Error::other)?
        .filter_map(|r| r.ok())
        .collect();
    let version_re = regex::Regex::new(r"<Version>(.+?)</Version>").unwrap();
    let pkg_re = regex::Regex::new(r"<PackageVersion>(.+?)</PackageVersion>").unwrap();
    for path in paths {
        let content = std::fs::read_to_string(&path)?;
        if let Some(caps) = version_re.captures(&content) {
            return Ok(Some(caps[1].to_string()));
        }
        if let Some(caps) = pkg_re.captures(&content) {
            return Ok(Some(caps[1].to_string()));
        }
    }
    Ok(None)
}

fn detect_cmakelists(base: &Path) -> Result<Option<String>, DetectError> {
    let path = base.join("CMakeLists.txt");
    let Some(content) = read_if_exists(&path)? else {
        return Ok(None);
    };
    let re = regex::Regex::new(r"(?i)project\s*\([^)]*VERSION\s+(\S+)").unwrap();
    if let Some(caps) = re.captures(&content) {
        return Ok(Some(caps[1].to_string()));
    }
    Ok(None)
}

fn detect_deno_json(base: &Path) -> Result<Option<String>, DetectError> {
    for name in &["deno.json", "deno.jsonc"] {
        let path = base.join(name);
        let Some(content) = read_if_exists(&path)? else {
            continue;
        };
        // For jsonc, strip single-line comments before parsing
        let cleaned: String = if *name == "deno.jsonc" {
            content
                .lines()
                .map(|line| {
                    // Strip // comments that aren't inside strings
                    // Simple heuristic: remove everything after // that's not preceded by :
                    // (to preserve URLs). A full parser isn't needed here.
                    if let Some(idx) = line.find("//") {
                        // Check if this is likely a URL (preceded by ':')
                        let before = &line[..idx];
                        if before.ends_with(':') || before.ends_with('"') {
                            line.to_string()
                        } else {
                            before.to_string()
                        }
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            content
        };
        let parsed: serde_json::Value = match serde_json::from_str(&cleaned) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(version) = parsed.get("version").and_then(|v| v.as_str()) {
            return Ok(Some(version.to_string()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_detect_cargo_toml() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"1.2.3\"\n",
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("1.2.3".to_string()));
    }

    #[test]
    fn test_detect_package_json() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name": "test", "version": "2.0.0"}"#,
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("2.0.0".to_string()));
    }

    #[test]
    fn test_detect_pyproject_toml_project() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"test\"\nversion = \"3.1.0\"\n",
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("3.1.0".to_string()));
    }

    #[test]
    fn test_detect_pyproject_toml_poetry() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.poetry]\nname = \"test\"\nversion = \"4.0.0\"\n",
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("4.0.0".to_string()));
    }

    #[test]
    fn test_detect_none() {
        let dir = setup_dir();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_cargo_toml_takes_priority_over_package_json() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("package.json"), r#"{"version": "2.0.0"}"#).unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("1.0.0".to_string()));
    }

    #[test]
    fn test_detect_own_cargo_toml() {
        // Test with the project's own Cargo.toml
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let result = detect_version(project_root).unwrap();
        assert_eq!(result, Some("0.1.0".to_string()));
    }

    #[test]
    fn test_detect_go_mod_with_loose_tags() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("go.mod"),
            "module example.com/foo\n\ngo 1.21\n",
        )
        .unwrap();

        // Create a fake .git with loose tags
        let git_dir = dir.path().join(".git");
        let tags_dir = git_dir.join("refs").join("tags");
        fs::create_dir_all(&tags_dir).unwrap();
        fs::write(tags_dir.join("v0.1.0"), "aaaaaa\n").unwrap();
        fs::write(tags_dir.join("v1.3.0"), "bbbbbb\n").unwrap();
        fs::write(tags_dir.join("v1.2.0"), "cccccc\n").unwrap();
        // Non-semver tag should be ignored
        fs::write(tags_dir.join("nightly"), "dddddd\n").unwrap();

        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("1.3.0".to_string()));
    }

    #[test]
    fn test_detect_go_mod_with_packed_refs() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("go.mod"),
            "module example.com/foo\n\ngo 1.21\n",
        )
        .unwrap();

        let git_dir = dir.path().join(".git");
        fs::create_dir_all(git_dir.join("refs").join("tags")).unwrap();
        fs::write(
            git_dir.join("packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted\n\
             aaa111 refs/tags/v2.0.0\n\
             ^bbb222\n\
             ccc333 refs/heads/main\n\
             ddd444 refs/tags/v1.9.0\n",
        )
        .unwrap();

        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("2.0.0".to_string()));
    }

    #[test]
    fn test_detect_go_mod_no_git_dir() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("go.mod"),
            "module example.com/foo\n\ngo 1.21\n",
        )
        .unwrap();
        // No .git directory — should return None
        let result = detect_go_mod(dir.path()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_detect_setup_cfg() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("setup.cfg"),
            "[metadata]\nname = test\nversion = 1.5.0\n\n[options]\npackages = find:\n",
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("1.5.0".to_string()));
    }

    #[test]
    fn test_detect_pom_xml() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("pom.xml"),
            r#"<project>
  <parent>
    <version>2.0.0</version>
  </parent>
  <groupId>com.example</groupId>
  <artifactId>test</artifactId>
  <version>3.0.0</version>
</project>"#,
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("3.0.0".to_string()));
    }

    #[test]
    fn test_detect_gradle() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("build.gradle"),
            "plugins { id 'java' }\nversion = '1.0.0'\n",
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("1.0.0".to_string()));
    }

    #[test]
    fn test_detect_pubspec_yaml() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("pubspec.yaml"),
            "name: test\nversion: 1.0.0+1\n",
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("1.0.0+1".to_string()));
    }

    #[test]
    fn test_detect_composer_json() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("composer.json"),
            r#"{"name": "test/pkg", "version": "2.1.0"}"#,
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("2.1.0".to_string()));
    }

    #[test]
    fn test_detect_cmakelists() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.10)\nproject(MyProject VERSION 1.2.3 LANGUAGES CXX)\n",
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("1.2.3".to_string()));
    }

    #[test]
    fn test_detect_deno_json() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("deno.json"),
            r#"{"version": "0.5.0", "tasks": {}}"#,
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("0.5.0".to_string()));
    }

    #[test]
    fn test_detect_mix_exs() {
        let dir = setup_dir();
        fs::write(
            dir.path().join("mix.exs"),
            r#"defmodule MyApp.MixProject do
  use Mix.Project
  @version "1.0.0"
  def project do
    [app: :my_app, version: @version]
  end
end"#,
        )
        .unwrap();
        let result = detect_version(dir.path()).unwrap();
        assert_eq!(result, Some("1.0.0".to_string()));
    }
}
