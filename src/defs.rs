use redb::TableDefinition;

pub const TEMPLATES_TABLE: TableDefinition<&str, &str> = TableDefinition::new("templates");
pub const TEMPLATES_OIDS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("template_oids");
pub const METADATA_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata");

pub const START_MARKER_PREFIX: &str = "# git-ignore-start: ";
pub const END_MARKER_PREFIX: &str = "# git-ignore-end: ";
