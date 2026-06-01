use std::{collections::BTreeMap, path::PathBuf};

use crate::error::ConfigError;
use crate::result::Result;

pub const DEFAULT_REPO_URL: &str = "https://github.com/github/gitignore.git";

#[derive(Debug, Clone)]
pub struct TemplateSource {
    pub name: String,
    pub url: String,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct Config {
    pub sources: Vec<TemplateSource>,
    pub db_path: PathBuf,
    pub fst_path: PathBuf,
    pub project_path: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| ConfigError::Env("Could not resolve data directory".to_string()))?;

        let base_path = data_dir.join("git-ignore");
        let db_path = base_path.join("templates.redb");
        let fst_path = base_path.join("templates.fst");

        let source_list_env = std::env::var("GIT_IGNORE_SOURCE_LIST").unwrap_or_default();

        // use BTreeMap since we also want to keep order
        let mut sources = BTreeMap::new();

        let default_ns = "default".to_string();

        // set this as default then it can be replaced later on below
        sources.insert(
            default_ns.clone(),
            TemplateSource {
                name: default_ns,
                url: DEFAULT_REPO_URL.to_string(),
                path: base_path.join("sources/default"),
            },
        );

        if !source_list_env.trim().is_empty() {
            for part in source_list_env.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }

                if let Some((name, url)) = part.split_once('=') {
                    let name = name.trim().to_string();
                    let url = url.trim().to_string();

                    if !name.is_empty() && !url.is_empty() {
                        let path = base_path.join(format!("sources/{}", name));
                        sources.insert(name.clone(), TemplateSource { name, url, path });
                    }
                } else {
                    eprintln!(
                        "Warning: invalid source format '{}', expected name=url",
                        part
                    );
                }
            }
        };

        let sources = sources.values().cloned().collect();

        Ok(Self {
            sources,
            db_path,
            fst_path,
            project_path: std::env::current_dir()?,
        })
    }
}
