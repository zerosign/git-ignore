use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

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
    pub fn parse_ignore_lists<P>(spath: P, raw: &str) -> Vec<(String, TemplateSource)>
    where
        P: AsRef<Path>,
    {
        let spath = spath.as_ref();

        raw.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|part| part.split_once('='))
            .filter_map(|(name, url)| {
                let name = name.trim();
                let url = url.trim();

                if !name.is_empty() && !url.is_empty() {
                    let path = spath.join(name);

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
            })
            .collect()
    }

    pub fn from_parent_dir<P, I>(parent: P, other: I) -> Result<Self, ConfigError>
    where
        P: AsRef<Path>,
        I: IntoIterator<Item = (String, TemplateSource)>,
    {
        // namespaced for sources (contains repository)
        let parent = PathBuf::from(parent.as_ref());
        let source_path = parent.join("sources");

        let default = (
            String::from("default"),
            TemplateSource {
                name: String::from("default"),
                url: String::from(DEFAULT_REPO_URL),
                path: source_path.join("default"),
                branch: None,
            },
        );

        // 3. Chain default and env sources, collect to BTreeMap (for deduplication/sorting), then to Vec
        let sources: Vec<TemplateSource> = std::iter::once(default)
            .chain(other)
            .collect::<BTreeMap<String, TemplateSource>>()
            .into_values()
            .collect();

        let db_path = parent.join("templates.redb");
        let cache_path = parent.join("templates.rkyv");

        Ok(Self {
            sources,
            db_path,
            cache_path,
            project_path: std::env::current_dir()?,
        })
    }

    /// Resolves Config from environment variables and system directories.
    ///
    /// # Errors
    /// Returns `ConfigError` if system data directory cannot be resolved or if parsing custom sources fails.
    pub fn from_env() -> Result<Self, ConfigError> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| ConfigError::Env("Could not resolve data directory".to_string()))?;

        // data base dir for this app
        let base_path = data_dir.join("git-ignore");
        let raw = std::env::var("GIT_IGNORE_SOURCE_LIST").unwrap_or_default();

        let other = Self::parse_ignore_lists(base_path.join("sources"), &raw);

        Self::from_parent_dir(base_path, other)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

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

    #[rstest]
    #[case("", 0, None, None, None)]
    #[case("  , , ", 0, None, None, None)]
    #[case("invalid", 0, None, None, None)]
    #[case("=nourl", 0, None, None, None)]
    #[case("noname=", 0, None, None, None)]
    #[case(
        "custom=https://github.com/org/repo",
        1,
        Some("custom"),
        Some("https://github.com/org/repo"),
        None
    )]
    #[case(
        "  custom = https://github.com/org/repo  ",
        1,
        Some("custom"),
        Some("https://github.com/org/repo"),
        None
    )]
    #[case(
        "custom=https://github.com/org/repo#main",
        1,
        Some("custom"),
        Some("https://github.com/org/repo"),
        Some("refs/heads/main")
    )]
    #[case(
        "team=https://git.team/repo#dev, bad, valid=https://valid.org",
        2,
        Some("team"),
        Some("https://git.team/repo"),
        Some("refs/heads/dev")
    )]
    fn test_parse_ignore_lists(
        #[case] input: &str,
        #[case] expected_count: usize,
        #[case] first_name: Option<&str>,
        #[case] first_url: Option<&str>,
        #[case] first_branch: Option<&str>,
    ) {
        let spath = Path::new("/tmp/sources");
        let sources = Config::parse_ignore_lists(spath, input);

        assert_eq!(sources.len(), expected_count);

        if let Some(expected_name) = first_name {
            let (name, source) = &sources[0];
            assert_eq!(name, expected_name);
            assert_eq!(source.name, expected_name);
            assert_eq!(source.url, first_url.unwrap());
            assert_eq!(source.branch.as_deref(), first_branch);
            assert_eq!(source.path, spath.join(expected_name));
        }
    }

    #[test]
    fn test_config_from_parent_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let custom = Config::parse_ignore_lists(
            temp.path().join("sources"),
            "extra=https://github.com/extra/repo",
        );
        let config = Config::from_parent_dir(temp.path(), custom).expect("config");

        assert_eq!(config.db_path, temp.path().join("templates.redb"));
        assert_eq!(config.cache_path, temp.path().join("templates.rkyv"));
        assert_eq!(config.sources.len(), 2);
    }
}
