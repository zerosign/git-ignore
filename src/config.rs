use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{error::ConfigError, result::Result};

pub const DEFAULT_REPO_URL: &str = "https://github.com/github/gitignore.git";

#[derive(Debug, Clone)]
pub struct TemplateSource {
    pub name: String,
    pub url: String,
    pub path: PathBuf,
}

impl TemplateSource {
    pub fn version_key(&self) -> String {
        format!("last_commit_hash:{}", self.name)
    }

    pub fn key_of(source: &str, name: &str) -> String {
        format!("{}/{}", source, name.to_lowercase())
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub sources: Vec<TemplateSource>,
    pub db_path: PathBuf,
    pub fst_path: PathBuf,
    pub project_path: PathBuf,
}

fn format_file_hash(path: &Path) -> Option<String> {
    // TODO
    let bytes = fs::read(path).ok()?;
    let hash = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(64);

    for b in hash {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }

    Some(hex)
}

impl Config {
    pub fn checksum(&self) -> (Option<String>, Option<String>) {
        (
            format_file_hash(&self.db_path),
            format_file_hash(&self.fst_path),
        )
    }
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

                    Some((
                        String::from(name),
                        TemplateSource {
                            name: String::from(name),
                            url: String::from(url),
                            path: path,
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
        let fst_path = base_path.join("templates.fst");

        Ok(Self {
            sources,
            db_path,
            fst_path,
            project_path: std::env::current_dir()?,
        })
    }
}
