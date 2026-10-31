use redb::TableDefinition;

pub const TEMPLATES_TABLE: TableDefinition<&str, &str> = TableDefinition::new("templates");
pub const TEMPLATES_OIDS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("template_oids");
pub const VERSIONS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("versions");
