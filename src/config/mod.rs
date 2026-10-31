use std::{collections::BTreeMap, path::PathBuf};

mod error;

pub use error::ConfigError;

use crate::result::Result;

pub const DEFAULT_REPO_URL: &str = "https://github.com/github/gitignore.git";

#[derive(Debug, Clone)]
pub struct TemplateSource {
    pub name: String,
    pub url: String,
    pub path: PathBuf,
    pub branch: Option<String>,
}

impl TemplateSource {
    #[must_use]
    pub fn version_key(&self) -> String {
        format!("last_commit_hash:{}", self.name)
    }

    #[must_use]
    pub fn key_of(source: &str, name: &str) -> String {
        format!("{}/{}", source, name.to_lowercase())
    }

    #[must_use]
    pub fn key_of_loc(source: &str, path: &[u8]) -> Option<String> {
        if !path.ends_with(b".gitignore") {
            return None;
        }

        let filename = path.rsplit(|&b| b == b'/').next().unwrap_or(path);
        let stem = filename.strip_suffix(b".gitignore").unwrap_or(filename);
        
        if stem.is_empty() {
            return None;
        }

        let name = String::from_utf8_lossy(stem).to_lowercase();

        Some(TemplateSource::key_of(source, &name))
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub sources: Vec<TemplateSource>,
    pub db_path: PathBuf,
    pub cache_path: PathBuf,
    pub project_path: PathBuf,
}

impl Config {
    /// Resolves Config from environment variables and system directories.
    ///
    /// # Errors
    /// Returns `ConfigError` if system data directory cannot be resolved or if parsing custom sources fails.
    pub fn from_env() -> Result<Self, ConfigError> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| ConfigError::Env("Could not resolve data directory".to_string()))?;

        // data base dir for this app
        let base_path = data_dir.join("git-ignore");

        // namespaced for sources (contains repository)
        let source_path = base_path.join("sources");

        let default = (
            String::from("default"),
            TemplateSource {
                name: String::from("default"),
                url: String::from(DEFAULT_REPO_URL),
                path: source_path.join("default"),
                branch: None,
            },
        );

        let raw = std::env::var("GIT_IGNORE_SOURCE_LIST").unwrap_or_default();

        let envs = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|part| part.split_once('='))
            .filter_map(|(name, url)| {
                let name = name.trim();
                let url = url.trim();

                if !name.is_empty() && !url.is_empty() {
                    let path = source_path.join(name);

                    let (clean_url, branch) = url
                        .split_once('#')
                        .map(|(u, b)| (u.trim(), Some(format!("refs/heads/{}", b.trim()))))
                        .unwrap_or((url, None));

                    Some((
                        String::from(name),
                        TemplateSource {
                            name: String::from(name),
                            url: String::from(clean_url),
                            path,
                            branch,
                        },
                    ))
                } else {
                    eprintln!("Warning: invalid source format, expected name=url");
                    None
                }
            });

        // 3. Chain default and env sources, collect to BTreeMap (for deduplication/sorting), then to Vec
        let sources: Vec<TemplateSource> = std::iter::once(default)
            .chain(envs)
            .collect::<BTreeMap<String, TemplateSource>>()
            .into_values()
            .collect();

        let db_path = base_path.join("templates.redb");
        let cache_path = base_path.join("templates.rkyv");

        Ok(Self {
            sources,
            db_path,
            cache_path,
            project_path: std::env::current_dir()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn test_template_source_version_key() {
        let source = TemplateSource {
            name: "default".to_string(),
            url: DEFAULT_REPO_URL.to_string(),
            path: PathBuf::from("/tmp"),
            branch: None,
        };
        assert_eq!(source.version_key(), "last_commit_hash:default");
    }

    #[rstest]
    #[case("default", "Rust", "default/rust")]
    #[case("custom", "Python", "custom/python")]
    #[case("org", "Go-Lang", "org/go-lang")]
    fn test_template_source_key_of(
        #[case] source: &str,
        #[case] name: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(TemplateSource::key_of(source, name), expected);
    }

    #[rstest]
    #[case("Rust.gitignore", Some("default/rust"))]
    #[case("Global/macOS.gitignore", Some("default/macos"))]
    #[case("Community/Golang.gitignore", Some("default/golang"))]
    #[case("src/main.rs", None)]
    #[case("README.md", None)]
    #[case(".gitignore", None)]
    fn test_template_source_key_of_loc(#[case] path: &str, #[case] expected: Option<&str>) {
        assert_eq!(
            TemplateSource::key_of_loc("default", path.as_bytes()),
            expected.map(String::from)
        );
    }

    #[test]
    fn test_config_from_env_default() {
        let config = Config::from_env();
        assert!(config.is_ok());

        let cfg = config.unwrap();
        assert!(!cfg.sources.is_empty());
        assert_eq!(cfg.sources[0].name, "default");
        assert_eq!(cfg.sources[0].url, DEFAULT_REPO_URL);
    }
}
