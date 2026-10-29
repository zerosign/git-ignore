//! Intelligent .gitignore patching logic.
//!
//! This module implements a "Smart Patch" mechanism that allows `git-ignore` to
//! manage sections of a `.gitignore` file without interfering with user-defined rules.
//!
//! ### The Rationale for Markers
//!
//! To provide idempotent and precise updates, the tool must distinguish between:
//! 1. **Custom Content**: Rules added manually by the user.
//! 2. **Managed Content**: Templates fetched and maintained by this tool.
//!
//! By wrapping managed templates in explicit markers:
//! ```gitignore
//! # git-ignore-start: Rust
//! target/
//! # git-ignore-end: Rust
//! ```
//! The tool achieves:
//! - **Ownership**: It knows exactly which lines it is responsible for.
//! - **In-place Updates**: It can replace the content of a specific template without moving it.
//! - **Idempotency**: Running the same command multiple times results in the same file state.
//! - **Deduplication**: It prevents the same template from being added multiple times.

use core::fmt;
use std::collections::HashMap;

pub const START_MARKER_PREFIX: &str = "# git-ignore-start: ";
pub const END_MARKER_PREFIX: &str = "# git-ignore-end: ";

/// Represents a discrete section of a `.gitignore` file.
#[derive(Debug, PartialEq, Eq)]
pub enum Block {
    /// Unmanaged content (e.g., user-defined rules or comments).
    Custom(String),
    /// A template section managed by the tool, delimited by start/end markers.
    Managed { name: String, content: String },
}

impl Block {
    fn render(&self) -> Option<String> {
        match self {
            Block::Custom(c) => {
                let trimmed = c.trim_end();
                (!trimmed.is_empty()).then(|| format!("{trimmed}\n"))
            }
            Block::Managed { name, content } => {
                let mut s = format!("{START_MARKER_PREFIX}{name}\n");
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    s.push_str(trimmed);
                    s.push('\n');
                }
                s.push_str(END_MARKER_PREFIX);
                s.push_str(name);
                s.push('\n');
                Some(s)
            }
        }
    }
}

pub struct Blocks(Vec<Block>);

impl fmt::Display for Blocks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: do this better in a way I think we can abstract the block then abstract the vec ?
        for item in self.0.iter().filter_map(Block::render) {
            writeln!(f, "{}", item)?;
        }

        Ok(())
    }
}

/// A parser and serializer for block-based `.gitignore` management.
pub struct Patcher {
    blocks: Vec<Block>,
}

impl Patcher {
    /// Parses an existing `.gitignore` string into a sequence of [Block]s.
    ///
    /// It recognizes the start and end markers defined in [`patch::START_MARKER_PREFIX`, `patch::END_MARKER_PREFIX`].
    /// Legacy markers are intentionally ignored and treated as custom content.
    #[must_use]
    pub fn parse(content: &str) -> Self {
        let mut blocks = Vec::new();
        let mut buffer = String::new();
        let mut lines = content.lines();

        while let Some(line) = lines.next() {
            // Check for the new, cleaner marker
            if let Some(name) = line.strip_prefix(START_MARKER_PREFIX) {
                if !buffer.is_empty() {
                    blocks.push(Block::Custom(buffer.clone()));
                    buffer.clear();
                }

                let name = name.to_string();
                let end_marker = format!("{END_MARKER_PREFIX}{name}");

                // Functionally consume all lines until the end marker (which is also consumed and discarded)
                let managed_content = lines
                    .by_ref()
                    .take_while(|&m_line| m_line != end_marker.as_str())
                    .fold(String::new(), |mut acc, l| {
                        acc.push_str(l);
                        acc.push('\n');
                        acc
                    });

                blocks.push(Block::Managed {
                    name,
                    content: managed_content,
                });
            } else {
                // Accumulate content
                buffer.push_str(line);
                buffer.push('\n');
            }
        }

        if !buffer.is_empty() {
            blocks.push(Block::Custom(buffer));
        }

        Self { blocks }
    }

    /// Merges new template content into the parsed blocks. (Only can be called once, this move
    /// things).
    ///
    /// If a [`Block::Managed`] section with a matching name (case-insensitive) exists,
    /// its content is updated. Otherwise, a new managed block is appended to the end.
    pub fn patch(mut self, new_templates: HashMap<String, String>) -> Option<Blocks> {
        // Pre-normalize templates for O(1) lookup while preserving original casing
        let mut templates: HashMap<String, (String, String)> = new_templates
            .into_iter()
            .map(|(k, v)| (k.to_lowercase(), (k, v)))
            .collect();

        let mut updated = false;

        // 1. Update existing managed blocks if new_content != content (but we still remove if there
        //    is entry).
        for block in &mut self.blocks {
            if let Block::Managed { name, content } = block
                && let Some((original_name, new_content)) = templates.remove(&name.to_lowercase())
                && !new_content.eq(content)
            {
                // mark that there is an update
                updated = true;

                *name = original_name; // Sync casing to the latest requested format
                *content = new_content.trim().to_string();
            }
        }

        if !updated && templates.is_empty() {
            return None;
        }

        if updated && templates.is_empty() {
            return Some(Blocks(self.blocks));
        }

        // 2. Append new blocks for remaining templates
        self.blocks.extend(
            templates
                .into_values()
                .map(|(name, content)| Block::Managed {
                    name,
                    content: content.trim().to_string(),
                }),
        );

        Some(Blocks(self.blocks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_and_serialize_custom() {
        let content = "node_modules/\n.env\n";
        let patcher = Patcher::parse(content);

        let patched = patcher.patch(HashMap::new());

        assert!(patched.is_none());

        let patched = patched.unwrap();
        let result = format!("{}", patched);

        assert_eq!(result, content);
    }

    #[test]
    fn test_patch_new_template() {
        let content = "custom_entry\n";
        let patcher = Patcher::parse(content);

        let mut templates = HashMap::new();

        templates.insert("Rust".to_string(), "target/\n".to_string());
        let patched = patcher.patch(templates);

        assert!(patched.is_some());

        let patched = patched.unwrap();
        let result = format!("{}", patched);

        assert!(result.contains("custom_entry"));
        assert!(result.contains(START_MARKER_PREFIX));
        assert!(result.contains("Rust"));
        assert!(result.contains("target/"));
        assert!(result.contains(END_MARKER_PREFIX));
    }

    #[test]
    fn test_patch_existing_template() {
        let content = format!("{START_MARKER_PREFIX}Rust\nold_content\n{END_MARKER_PREFIX}Rust\n");

        let patcher = Patcher::parse(&content);
        let mut templates = HashMap::new();

        templates.insert("Rust".to_string(), "new_content\n".to_string());
        let patched = patcher.patch(templates);

        assert!(patched.is_some());

        let patched = patched.unwrap();
        let result = format!("{}", patched);

        assert!(!result.contains("old_content"));
        assert!(result.contains("new_content"));
    }
}
