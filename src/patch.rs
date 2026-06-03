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

use std::collections::HashMap;
use crate::defs::{START_MARKER_PREFIX, END_MARKER_PREFIX};

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
                (!trimmed.is_empty()).then(|| format!("{}\n", trimmed))
            }
            Block::Managed { name, content } => {
                let mut s = format!("{}{}\n", START_MARKER_PREFIX, name);
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    s.push_str(trimmed);
                    s.push('\n');
                }
                s.push_str(&format!("{}{}\n", END_MARKER_PREFIX, name));
                Some(s)
            }
        }
    }
}


/// A parser and serializer for block-based `.gitignore` management.
pub struct Patcher {
    blocks: Vec<Block>,
}

impl Patcher {
    /// Parses an existing `.gitignore` string into a sequence of [Block]s.
    ///
    /// It recognizes the start and end markers defined in [crate::defs].
    /// Legacy markers are intentionally ignored and treated as custom content.
    pub fn parse(content: &str) -> Self {
        let mut blocks = Vec::new();
        let mut unmanaged_buffer = String::new();
        let mut lines = content.lines();

        while let Some(line) = lines.next() {
            // Check for the new, cleaner marker
            if let Some(name) = line.strip_prefix(START_MARKER_PREFIX) {
                if !unmanaged_buffer.is_empty() {
                    blocks.push(Block::Custom(unmanaged_buffer.clone()));
                    unmanaged_buffer.clear();
                }

                let name = name.to_string();
                let end_marker = format!("{}{}", END_MARKER_PREFIX, name);

                // Functionally consume all lines until the end marker (which is also consumed and discarded)
                let managed_content = lines
                    .by_ref()
                    .take_while(|&m_line| m_line != end_marker.as_str())
                    .fold(String::new(), |mut acc, l| {
                        acc.push_str(l);
                        acc.push('\n');
                        acc
                    });

                blocks.push(Block::Managed { name, content: managed_content });
            } else {
                // Accumulate unmanaged content
                unmanaged_buffer.push_str(line);
                unmanaged_buffer.push('\n');
            }
        }

        if !unmanaged_buffer.is_empty() {
            blocks.push(Block::Custom(unmanaged_buffer));
        }

        Self { blocks }
    }

    /// Merges new template content into the parsed blocks.
    ///
    /// If a [Block::Managed] section with a matching name (case-insensitive) exists, 
    /// its content is updated. Otherwise, a new managed block is appended to the end.
    pub fn patch(&mut self, new_templates: HashMap<String, String>) {
        // Pre-normalize templates for O(1) lookup while preserving original casing
        let mut templates: HashMap<String, (String, String)> = new_templates
            .into_iter()
            .map(|(k, v)| (k.to_lowercase(), (k, v)))
            .collect();

        // 1. Update existing managed blocks
        for block in &mut self.blocks {
            if let Block::Managed { name, content } = block
                && let Some((original_name, new_content)) = templates.remove(&name.to_lowercase())
            {
                *name = original_name; // Sync casing to the latest requested format
                *content = new_content.trim().to_string();
            }
        }

        // 2. Append new blocks for remaining templates
        self.blocks.extend(templates.into_values().map(|(name, content)| {
            Block::Managed {
                name,
                content: content.trim().to_string(),
            }
        }));
    }

    /// Serializes the blocks back into a single string.
    ///
    /// This method ensures a consistent format with exactly one blank line between
    /// blocks and clean trimming of managed content.
    pub fn serialize(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|b| b.render())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_and_serialize_custom() {
        let content = "node_modules/\n.env\n";
        let patcher = Patcher::parse(content);
        assert_eq!(patcher.serialize(), content);
    }

    #[test]
    fn test_patch_new_template() {
        let content = "custom_entry\n";
        let mut patcher = Patcher::parse(content);
        
        let mut templates = HashMap::new();
        templates.insert("Rust".to_string(), "target/\n".to_string());
        
        patcher.patch(templates);
        let result = patcher.serialize();
        
        assert!(result.contains("custom_entry"));
        assert!(result.contains(START_MARKER_PREFIX));
        assert!(result.contains("Rust"));
        assert!(result.contains("target/"));
        assert!(result.contains(END_MARKER_PREFIX));
    }

    #[test]
    fn test_patch_existing_template() {
        let content = format!("{}Rust\nold_content\n{}Rust\n", START_MARKER_PREFIX, END_MARKER_PREFIX);
        let mut patcher = Patcher::parse(&content);
        
        let mut templates = HashMap::new();
        templates.insert("Rust".to_string(), "new_content\n".to_string());
        
        patcher.patch(templates);
        let result = patcher.serialize();
        
        assert!(!result.contains("old_content"));
        assert!(result.contains("new_content"));
    }
}
