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
//! # --- BEGIN Rust ---
//! target/
//! # --- END Rust ---
//! ```
//! The tool achieves:
//! - **Ownership**: It knows exactly which lines it is responsible for.
//! - **In-place Updates**: It can replace the content of a specific template without moving it.
//! - **Idempotency**: Running the same command multiple times results in the same file state.
//! - **Deduplication**: It prevents the same template from being added multiple times.

use std::collections::HashMap;

/// Represents a discrete section of a `.gitignore` file.
#[derive(Debug, PartialEq, Eq)]
pub enum Block {
    /// Unmanaged content (e.g., user-defined rules or comments).
    Custom(String),
    /// A template section managed by the tool, delimited by BEGIN/END markers.
    Managed { name: String, content: String },
}

/// A parser and serializer for block-based `.gitignore` management.
pub struct Patcher {
    blocks: Vec<Block>,
}

impl Patcher {
    /// Parses an existing `.gitignore` string into a sequence of [Block]s.
    ///
    /// It recognizes both the current BEGIN/END marker format and legacy single-line
    /// headers (`# === Name ===`), allowing for automatic migration to the new format.
    pub fn parse(content: &str) -> Self {
        let mut blocks = Vec::new();
        let mut current_custom = String::new();
        
        let lines: Vec<&str> = content.lines().collect();
        let mut i = 0;
        
        while i < lines.len() {
            let line = lines[i];
            
            // Check for new style BEGIN marker: # --- BEGIN Name ---
            if line.starts_with("# --- BEGIN ") && line.ends_with(" ---") {
                if !current_custom.is_empty() {
                    blocks.push(Block::Custom(current_custom.clone()));
                    current_custom.clear();
                }
                
                let name = line[12..line.len()-4].to_string();
                let mut managed_content = String::new();
                i += 1;
                
                let mut found_end = false;
                while i < lines.len() {
                    let m_line = lines[i];
                    if m_line == format!("# --- END {} ---", name) {
                        found_end = true;
                        break;
                    }
                    managed_content.push_str(m_line);
                    managed_content.push('\n');
                    i += 1;
                }
                
                blocks.push(Block::Managed { name, content: managed_content });
                if found_end { i += 1; }
                continue;
            }
            
            // Check for legacy style marker: # === Name ===
            if line.starts_with("# === ") && line.ends_with(" ===") {
                 if !current_custom.is_empty() {
                    blocks.push(Block::Custom(current_custom.clone()));
                    current_custom.clear();
                }
                
                let name = line[6..line.len()-4].to_string();
                let mut managed_content = String::new();
                i += 1;
                
                // Legacy blocks end at the next marker or EOF
                while i < lines.len() {
                    let m_line = lines[i];
                    if m_line.starts_with("# --- BEGIN ") || (m_line.starts_with("# === ") && m_line.ends_with(" ===")) {
                        break;
                    }
                    managed_content.push_str(m_line);
                    managed_content.push('\n');
                    i += 1;
                }
                
                blocks.push(Block::Managed { name, content: managed_content });
                continue;
            }
            
            current_custom.push_str(line);
            current_custom.push('\n');
            i += 1;
        }
        
        if !current_custom.is_empty() {
            blocks.push(Block::Custom(current_custom));
        }
        
        Self { blocks }
    }

    /// Merges new template content into the parsed blocks.
    ///
    /// If a [Block::Managed] section with a matching name (case-insensitive) exists, 
    /// its content is updated. Otherwise, a new managed block is appended to the end.
    pub fn patch(&mut self, new_templates: HashMap<String, String>) {
        let mut template_map = new_templates;
        
        // 1. Update existing managed blocks
        for block in &mut self.blocks {
            if let Block::Managed { name, content } = block {
                let key = template_map.keys().find(|k| k.to_lowercase() == name.to_lowercase()).cloned();
                if let Some(k) = key
                    && let Some(new_content) = template_map.remove(&k) {
                        *content = new_content.trim().to_string();
                    }
            }
        }
        
        // 2. Append new blocks for remaining templates
        for (name, content) in template_map {
            self.blocks.push(Block::Managed { name, content: content.trim().to_string() });
        }
    }

    /// Serializes the blocks back into a single string.
    ///
    /// This method ensures a consistent format with exactly one blank line between
    /// blocks and clean trimming of managed content.
    pub fn serialize(&self) -> String {
        // Pre-allocate capacity to reduce reallocations
        let total_len: usize = self.blocks.iter().map(|b| match b {
            Block::Custom(c) => c.len(),
            Block::Managed { name, content } => name.len() * 2 + content.len() + 30, // Estimate for markers and extra spacing
        }).sum();
        
        let mut out = String::with_capacity(total_len);
        for (i, block) in self.blocks.iter().enumerate() {
            match block {
                Block::Custom(c) => {
                    let trimmed = c.trim_end();
                    if !trimmed.is_empty() {
                        out.push_str(trimmed);
                        out.push('\n');
                    }
                },
                Block::Managed { name, content } => {
                    out.push_str("# --- BEGIN ");
                    out.push_str(name);
                    out.push_str(" ---\n");
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        out.push_str(trimmed);
                        out.push('\n');
                    }
                    out.push_str("# --- END ");
                    out.push_str(name);
                    out.push_str(" ---\n");
                }
            }
            
            // Add exactly one blank line between blocks
            if i < self.blocks.len() - 1 {
                out.push('\n');
            }
        }
        out
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
        assert!(result.contains("# --- BEGIN Rust ---"));
        assert!(result.contains("target/"));
        assert!(result.contains("# --- END Rust ---"));
    }

    #[test]
    fn test_patch_existing_template() {
        let content = "# --- BEGIN Rust ---\nold_content\n# --- END Rust ---\n";
        let mut patcher = Patcher::parse(content);
        
        let mut templates = HashMap::new();
        templates.insert("Rust".to_string(), "new_content\n".to_string());
        
        patcher.patch(templates);
        let result = patcher.serialize();
        
        assert!(!result.contains("old_content"));
        assert!(result.contains("new_content"));
    }

    #[test]
    fn test_legacy_migration() {
        let content = "# === Rust ===\nlegacy_content\n";
        let mut patcher = Patcher::parse(content);
        
        let mut templates = HashMap::new();
        templates.insert("Rust".to_string(), "new_content\n".to_string());
        
        patcher.patch(templates);
        let result = patcher.serialize();
        
        assert!(!result.contains("# === Rust ==="));
        assert!(result.contains("# --- BEGIN Rust ---"));
        assert!(result.contains("new_content"));
    }
}
