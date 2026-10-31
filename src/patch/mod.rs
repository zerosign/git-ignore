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

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Block::Custom(c) => {
                let trimmed = c.trim();
                if !trimmed.is_empty() {
                    writeln!(f, "{trimmed}")?;
                }
            }
            Block::Managed { name, content } => {
                writeln!(f, "{START_MARKER_PREFIX}{name}")?;
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    writeln!(f, "{trimmed}")?;
                }
                writeln!(f, "{END_MARKER_PREFIX}{name}")?;
            }
        }
        Ok(())
    }
}

pub struct Blocks(Vec<Block>);

impl fmt::Display for Blocks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .filter(|b| match b {
                Block::Custom(c) => !c.trim().is_empty(),
                Block::Managed { .. } => true,
            })
            .try_fold("", |sep, block| {
                f.write_str(sep)?;
                write!(f, "{block}")?;
                Ok("\n")
            })
            .map(|_| ())
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
                let trimmed = buffer.trim();
                if !trimmed.is_empty() {
                    blocks.push(Block::Custom(trimmed.to_string()));
                }
                buffer.clear();

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

        let trimmed = buffer.trim();
        if !trimmed.is_empty() {
            blocks.push(Block::Custom(trimmed.to_string()));
        }

        Self { blocks }
    }

    /// Merges new template content into the parsed blocks. (Only can be called once, this move
    /// things).
    ///
    /// If a [`Block::Managed`] section with a matching name (case-insensitive) exists,
    /// its content is updated. Otherwise, a new managed block is appended to the end.
    #[must_use]
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
                && !new_content.trim().eq(content.trim())
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

    fn tmpl(name: &str, content: &str) -> HashMap<String, String> {
        HashMap::from([(name.to_string(), content.to_string())])
    }

    #[test]
    fn test_parse_plain_blocks() {
        let content = "node_modules/\n.env\n*.log\n";
        let patcher = Patcher::parse(content);

        assert_eq!(patcher.blocks.len(), 1);
        match &patcher.blocks[0] {
            Block::Custom(c) => assert_eq!(c, "node_modules/\n.env\n*.log"),
            Block::Managed { .. } => panic!("Expected custom block"),
        }

        let blocks = Blocks(patcher.blocks);
        let result = format!("{blocks}");
        assert_eq!(result, "node_modules/\n.env\n*.log\n");
    }

    #[test]
    fn test_patch_new_managed_block() {
        let content = "custom_entry\n";
        let patched = Patcher::parse(content).patch(tmpl("Rust", "target/\n"));

        assert!(patched.is_some());
        let result = format!("{}", patched.unwrap());

        assert!(result.contains("custom_entry\n\n# git-ignore-start: Rust\ntarget/\n# git-ignore-end: Rust\n"));
    }

    #[test]
    fn test_patch_idempotency() {
        let content = "custom_entry\n\n# git-ignore-start: Rust\ntarget/\n# git-ignore-end: Rust\n";

        // Applying identical content must return None (indicating up to date, zero writes)
        let patched = Patcher::parse(content).patch(tmpl("Rust", "target/\n"));
        assert!(patched.is_none());
    }

    #[test]
    fn test_no_newline_creep() {
        let initial = "first_rule\n\n# git-ignore-start: Rust\ntarget/\n# git-ignore-end: Rust\n\nsecond_rule\n";

        let pass1 = format!(
            "{}",
            Patcher::parse(initial)
                .patch(tmpl("Rust", "target/\nCargo.lock\n"))
                .expect("should update")
        );

        // Second pass with the identical content must be idempotent
        assert!(
            Patcher::parse(&pass1)
                .patch(tmpl("Rust", "target/\nCargo.lock\n"))
                .is_none()
        );

        // Third pass updating content again
        let pass3 = format!(
            "{}",
            Patcher::parse(&pass1)
                .patch(tmpl("Rust", "target/\nCargo.lock\n*.bk\n"))
                .expect("should update")
        );

        // Count empty lines to ensure spacer lines do not multiply
        let empty_lines_pass1 = pass1.lines().filter(|l| l.trim().is_empty()).count();
        let empty_lines_pass3 = pass3.lines().filter(|l| l.trim().is_empty()).count();
        assert_eq!(empty_lines_pass1, empty_lines_pass3);
    }

    #[test]
    fn test_update_existing_block() {
        let content = format!("{START_MARKER_PREFIX}Rust\nold_content\n{END_MARKER_PREFIX}Rust\n");
        let patched = Patcher::parse(&content).patch(tmpl("Rust", "new_content\n"));

        assert!(patched.is_some());
        let result = format!("{}", patched.unwrap());

        assert!(!result.contains("old_content"));
        assert!(result.contains("new_content"));
        assert!(result.contains(START_MARKER_PREFIX));
        assert!(result.contains(END_MARKER_PREFIX));
    }

    #[test]
    fn test_case_insensitive_matching() {
        let content = format!("{START_MARKER_PREFIX}rust\ntarget/\n{END_MARKER_PREFIX}rust\n");
        // Request with "Rust" casing while existing is "rust"
        let patched = Patcher::parse(&content).patch(tmpl("Rust", "target/\nnew_entry\n"));

        assert!(patched.is_some());
        let result = format!("{}", patched.unwrap());

        // Should update in place, syncing casing to "Rust" without duplicating the block
        assert_eq!(result.matches(START_MARKER_PREFIX).count(), 1);
        assert!(result.contains("# git-ignore-start: Rust"));
    }
}
