use std::io::Write;

use redb::Database;

pub mod action;
pub mod args;
pub mod config;
pub mod defs;
pub mod error;
pub mod patch;
pub mod result;
pub mod state;
pub mod sync;
pub mod visitor;

use action::CliAction;
pub use args::GitIgnoreArgs;
pub use config::{Config, TemplateSource};
use error::{CliError, CompactError, SyncError};
use result::Result;
use sync::SyncManager;

use crate::action::{generate_template, list_templates, show_info};

build_info::build_info!(fn show_version);

pub fn run_cli<W: Write>(args: GitIgnoreArgs, config: &Config, writer: W) -> Result<()> {
    let action = CliAction::from_args(&args, config)?;
    let sync_manager = SyncManager::new(config);

    match action {
        CliAction::Compact => {
            let mut db = Database::open(&config.db_path).map_err(CompactError::from)?;

            db.compact().map_err(CompactError::from)?;
            println!("Database compacted successfully.");

            Ok(())
        }
        CliAction::ShowVersion => {
            println!("{}", show_version());
            Ok(())
        }
        CliAction::UpdateOnly => {
            sync_manager.ensure_repos(args.update)?;
            sync_manager.sync()?;
            Ok(())
        }
        CliAction::ShowInfo => {
            let db = redb::Builder::new()
                .open_read_only(&config.db_path)
                .map_err(SyncError::from)?;

            show_info(config, &db, writer)?;

            Ok(())
        }
        CliAction::ListTemplates => {
            list_templates(&config.fst_path, writer)?;
            Ok(())
        }
        CliAction::Generate {
            templates_to_fetch,
            patch,
        } => {
            if !config.db_path.exists() {
                return Err(CliError::Discovery(
                    "Database not found. Run with -u first.".to_string(),
                ));
            }

            let db = redb::Builder::new()
                .open_read_only(&config.db_path)
                .map_err(SyncError::from)?;

            generate_template(
                &db,
                &templates_to_fetch,
                &config.sources,
                &config.project_path,
                patch,
                writer,
            )?;

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, process::Command};

    use redb::ReadableDatabase;
    use tempfile::{TempDir, tempdir};

    use crate::{
        defs::{METADATA_TABLE, TEMPLATES_TABLE},
        error::CliError,
        result::Result,
    };

    macro_rules! run_git {
        ($path:expr, $( [$($arg:expr),+ $(,)?] ),* $(,)?) => {{
            $(
                let output = std::process::Command::new("git")
                    .arg("-C")
                    .arg($path)
                    $(.arg($arg))+
                    .output()
                    .map_err(CliError::Io)?;
                if !output.status.success() {
                    return Err(CliError::Git(format!("git command failed: {:?}\nstdout: {}\nstderr: {}",
                            [$($arg),+],
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        )));
                }
            )*
        }};
    }

    fn run_in_repo<F>(func: F) -> Result<()>
    where
        F: FnOnce(TempDir, TempDir, &PathBuf) -> Result<()>,
    {
        let sandbox = tempdir()?;
        let data_dir = tempdir()?;
        let repo_path = data_dir.path().join("gitignore");
        fs::create_dir_all(&repo_path)?;

        run_git!(
            &repo_path,
            ["init"],
            ["config", "user.email", "test@example.com"],
            ["config", "user.name", "test"],
        );

        fs::write(repo_path.join("Rust.gitignore"), "target/")?;
        run_git!(&repo_path, ["add", "."], ["commit", "-m", "init"]);

        func(sandbox, data_dir, &repo_path)
    }

    use super::*;

    fn args_default(template: &str) -> GitIgnoreArgs {
        GitIgnoreArgs {
            update: false,
            patch: true,
            list: false,
            info: false,
            version: false,
            compact: false,
            templates: if template.is_empty() {
                vec![]
            } else {
                vec![template.to_string()]
            },
        }
    }

    #[test]
    fn test_patch_new_file() -> Result<()> {
        run_in_repo(|sandbox, data_dir, repo_path| {
            let config = Config {
                sources: vec![TemplateSource {
                    name: "default".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo_path.clone(),
                }],
                db_path: data_dir.path().join("templates.redb"),
                fst_path: data_dir.path().join("templates.fst"),
                project_path: sandbox.path().to_path_buf(),
            };

            SyncManager::new(&config).sync()?;
            run_cli(args_default("Rust"), &config, Vec::new())?;

            let gitignore_content = fs::read_to_string(sandbox.path().join(".gitignore"))?;
            assert!(gitignore_content.contains("# --- BEGIN Rust ---"));
            assert!(gitignore_content.contains("target/"));
            Ok(())
        })
    }

    #[test]
    fn test_patch_existing_file() -> Result<()> {
        run_in_repo(|sandbox, data_dir, repo_path| {
            let gitignore_path = sandbox.path().join(".gitignore");
            fs::write(&gitignore_path, "existing_entry")?;

            let config = Config {
                sources: vec![TemplateSource {
                    name: "default".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo_path.clone(),
                }],
                db_path: data_dir.path().join("templates.redb"),
                fst_path: data_dir.path().join("templates.fst"),
                project_path: sandbox.path().to_path_buf(),
            };

            SyncManager::new(&config).sync()?;
            run_cli(args_default("Rust"), &config, Vec::new())?;

            let gitignore_content = fs::read_to_string(&gitignore_path)?;
            assert!(gitignore_content.starts_with("existing_entry\n"));
            assert!(gitignore_content.contains("# --- BEGIN Rust ---"));
            Ok(())
        })
    }

    #[test]
    fn test_list_templates() -> Result<()> {
        run_in_repo(|_sandbox, data_dir, repo_path| {
            let config = Config {
                sources: vec![TemplateSource {
                    name: "default".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo_path.clone(),
                }],
                db_path: data_dir.path().join("templates.redb"),
                fst_path: data_dir.path().join("templates.fst"),
                project_path: data_dir.path().to_path_buf(),
            };

            SyncManager::new(&config).sync()?;
            list_templates(&config.fst_path, Vec::new())?;
            Ok(())
        })
    }

    #[test]
    fn test_clone_if_not_exists() -> Result<()> {
        run_in_repo(|sandbox, data_dir, repo_path| {
            let _ = fs::remove_dir_all(repo_path);
            let config = Config {
                sources: vec![TemplateSource {
                    name: "default".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo_path.clone(),
                }],
                db_path: data_dir.path().join("templates.redb"),
                fst_path: data_dir.path().join("templates.fst"),
                project_path: sandbox.path().to_path_buf(),
            };
            assert!(run_cli(args_default("Rust"), &config, Vec::new()).is_err());
            Ok(())
        })
    }

    #[test]
    fn test_update_reclone() -> Result<()> {
        run_in_repo(|sandbox, data_dir, repo_path| {
            fs::create_dir_all(repo_path.clone())?;
            fs::write(repo_path.clone().join("Old.gitignore"), "old")?;

            let config = Config {
                sources: vec![TemplateSource {
                    name: "default".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo_path.clone(),
                }],
                db_path: data_dir.path().join("templates.redb"),
                fst_path: data_dir.path().join("templates.fst"),
                project_path: sandbox.path().to_path_buf(),
            };

            let mut args = args_default("Rust");
            args.update = true;
            assert!(run_cli(args, &config, Vec::new()).is_err());
            Ok(())
        })
    }

    #[test]
    fn test_multiple_and_missing_templates() -> Result<()> {
        run_in_repo(|sandbox, data_dir, repo_path| {
            let config = Config {
                sources: vec![TemplateSource {
                    name: "default".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo_path.clone(),
                }],
                db_path: data_dir.path().join("templates.redb"),
                fst_path: data_dir.path().join("templates.fst"),
                project_path: sandbox.path().to_path_buf(),
            };

            let mut args = args_default("");
            args.patch = true;
            args.templates = vec!["Rust".to_string(), "NonExistent".to_string()];
            SyncManager::new(&config).sync()?;
            run_cli(args, &config, Vec::new())?;

            let gitignore_path = sandbox.path().join(".gitignore");
            let content = fs::read_to_string(&gitignore_path)?;
            assert!(content.contains("# --- BEGIN Rust ---"));
            assert!(!content.contains("# === NonExistent ==="));
            Ok(())
        })
    }

    #[test]
    fn test_show_info() -> Result<()> {
        run_in_repo(|_sandbox, data_dir, repo_path| {
            let config = Config {
                sources: vec![TemplateSource {
                    name: "default".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo_path.clone(),
                }],
                db_path: data_dir.path().join("templates.redb"),
                fst_path: data_dir.path().join("templates.fst"),
                project_path: data_dir.path().to_path_buf(),
            };

            SyncManager::new(&config).sync()?;
            let db = redb::Builder::new().open_read_only(&config.db_path)?;
            show_info(&config, &db, Vec::new())?;
            Ok(())
        })
    }

    #[test]
    fn test_incremental_sync() -> Result<()> {
        run_in_repo(|sandbox, data_dir, repo_path| {
            let output_c1 = Command::new("git")
                .arg("-C")
                .arg(repo_path)
                .arg("rev-parse")
                .arg("HEAD")
                .output()?;
            let commit1_hash = String::from_utf8_lossy(&output_c1.stdout)
                .trim()
                .to_string();

            fs::write(repo_path.join("Node.gitignore"), "node_modules/")?;
            fs::write(repo_path.join("Rust.gitignore"), "target/\nCargo.lock")?;
            run_git!(repo_path, ["add", "."], ["commit", "-m", "second"]);
            fs::remove_file(repo_path.join("Rust.gitignore"))?;
            fs::write(repo_path.join("Python.gitignore"), "__pycache__/")?;
            run_git!(repo_path, ["add", "."], ["commit", "-m", "third"]);

            let config = Config {
                sources: vec![TemplateSource {
                    name: "default".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo_path.clone(),
                }],
                db_path: data_dir.path().join("templates.redb"),
                fst_path: data_dir.path().join("templates.fst"),
                project_path: sandbox.path().to_path_buf(),
            };

            SyncManager::new(&config).sync()?;

            {
                let db = Database::create(&config.db_path)?;
                let write_txn = db.begin_write()?;
                {
                    let mut meta_table = write_txn.open_table(METADATA_TABLE)?;
                    meta_table.insert("last_commit_hash:default", commit1_hash.as_bytes())?;
                }
                write_txn.commit()?;
            }

            SyncManager::new(&config).sync()?;
            let db = Database::create(&config.db_path)?;
            let read_txn = db.begin_read()?;
            let table = read_txn.open_table(TEMPLATES_TABLE)?;

            assert!(table.get("default/rust")?.is_none());
            let node_val = table
                .get("default/node")?
                .ok_or_else(|| CliError::Git("Node exist".to_string()))?;
            assert_eq!(node_val.value(), "node_modules/");
            let py_val = table
                .get("default/python")?
                .ok_or_else(|| CliError::Git("Python exist".to_string()))?;
            assert_eq!(py_val.value(), "__pycache__/");
            Ok(())
        })
    }

    #[test]
    fn test_multi_source() -> Result<()> {
        let sandbox = tempdir()?;
        let data_dir = tempdir()?;

        let repo1_path = data_dir.path().join("repo1");
        let repo2_path = data_dir.path().join("repo2");

        fs::create_dir_all(&repo1_path)?;
        fs::create_dir_all(&repo2_path)?;

        run_git!(
            &repo1_path,
            ["init"],
            ["config", "user.email", "t@e.com"],
            ["config", "user.name", "t"]
        );

        fs::write(repo1_path.join("Common.gitignore"), "common1")?;

        run_git!(&repo1_path, ["add", "."], ["commit", "-m", "init"]);
        run_git!(
            &repo2_path,
            ["init"],
            ["config", "user.email", "t@e.com"],
            ["config", "user.name", "t"]
        );

        fs::write(repo2_path.join("Common.gitignore"), "common2")?;
        fs::write(repo2_path.join("Unique.gitignore"), "unique2")?;

        run_git!(&repo2_path, ["add", "."], ["commit", "-m", "init"]);

        let config = Config {
            sources: vec![
                TemplateSource {
                    name: "s1".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo1_path,
                },
                TemplateSource {
                    name: "s2".to_string(),
                    url: "file:///dev/null".to_string(),
                    path: repo2_path,
                },
            ],
            db_path: data_dir.path().join("templates.redb"),
            fst_path: data_dir.path().join("templates.fst"),
            project_path: sandbox.path().to_path_buf(),
            };

            SyncManager::new(&config).sync()?;

            let mut args = args_default("Common");
        args.patch = false;

        let mut output = Vec::new();
        run_cli(args, &config, &mut output)?;

        let content = String::from_utf8_lossy(&output);

        assert!(content.contains("common1"));
        assert!(content.contains("common2"));

        args = args_default("Unique");
        args.patch = false;

        output.clear();

        run_cli(args, &config, &mut output)?;

        assert!(String::from_utf8_lossy(&output).contains("unique2"));

        args = args_default("s2/Common");
        args.patch = false;
        output.clear();

        run_cli(args, &config, &mut output)?;

        let content = String::from_utf8_lossy(&output);

        assert!(content.contains("common2"));
        assert!(!content.contains("common1"));

        Ok(())
    }
}
