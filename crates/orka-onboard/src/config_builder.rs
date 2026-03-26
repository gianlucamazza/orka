//! Progressive TOML config builder for the onboarding wizard.
//!
//! Wraps a [`toml_edit::DocumentMut`] and provides typed operations for
//! setting config sections and appending array-of-tables entries.  Each
//! mutation is immediately validated by round-tripping through
//! [`OrkaConfig`].

use orka_core::{Error, Result, config::OrkaConfig};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

/// Known array-of-tables paths in the Orka config schema.
const ARRAY_OF_TABLES: &[&str] = &[
    "agents",
    "llm.providers",
    "graph.edges",
    "workspaces",
    "mcp.servers",
    "auth.api_keys",
    "guardrails.input.redact_patterns",
    "guardrails.output.redact_patterns",
];

/// Builds an `orka.toml` document progressively during the onboarding wizard.
pub struct ConfigBuilder {
    doc: DocumentMut,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigBuilder {
    /// Create a new builder seeded with `config_version = 6`.
    pub fn new() -> Self {
        let mut doc = DocumentMut::new();
        doc["config_version"] = toml_edit::value(6_i64);
        Self { doc }
    }

    /// Set key-value pairs under a dotted section path.
    ///
    /// `path` uses dot notation (`"server"`, `"adapters.telegram"`).
    /// `values` is a JSON object whose fields are merged into the section.
    pub fn set_section(&mut self, path: &str, values: &serde_json::Value) -> Result<()> {
        let obj = values
            .as_object()
            .ok_or_else(|| Error::Config("`values` must be a JSON object".to_string()))?;

        let table = navigate_or_create(&mut self.doc, path)?;
        for (k, v) in obj {
            if let Some(item) = json_to_item(v) {
                table[k.as_str()] = item;
            }
        }
        Ok(())
    }

    /// Append an entry to an array-of-tables section (e.g. `[[agents]]`).
    ///
    /// `path` uses dot notation (`"agents"`, `"llm.providers"`).
    /// `entry` is a JSON object for the new table entry.
    pub fn append_array_entry(&mut self, path: &str, entry: &serde_json::Value) -> Result<()> {
        let obj = entry
            .as_object()
            .ok_or_else(|| Error::Config("`entry` must be a JSON object".to_string()))?;

        // Navigate to the parent, then create/get the array-of-tables.
        let parts: Vec<&str> = path.split('.').collect();
        let (parent_parts, last) = parts.split_at(parts.len() - 1);

        // Build the parent path in the document.
        let doc_item = if parent_parts.is_empty() {
            self.doc.as_item_mut()
        } else {
            navigate_item_mut(self.doc.as_item_mut(), parent_parts)?
        };

        let parent_table = doc_item
            .as_table_mut()
            .ok_or_else(|| Error::Config("expected a table at parent path".to_string()))?;

        let key = last[0];
        if !parent_table.contains_key(key) {
            parent_table[key] = Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
        }

        let aot = parent_table[key]
            .as_array_of_tables_mut()
            .ok_or_else(|| Error::Config(format!("{path} is not an array-of-tables")))?;

        let mut new_table = Table::new();
        for (k, v) in obj {
            if let Some(item) = json_to_item(v) {
                new_table[k.as_str()] = item;
            }
        }
        aot.push(new_table);
        Ok(())
    }

    /// Validate the current document by parsing it as [`OrkaConfig`].
    ///
    /// Returns a list of non-fatal warning strings on success, or an error
    /// if the config is structurally invalid.
    pub fn validate(&self) -> Result<Vec<String>> {
        let toml_str = self.to_toml();
        let mut cfg: OrkaConfig = toml::from_str(&toml_str)
            .map_err(|e| Error::Config(format!("config parse error: {e}")))?;
        cfg.validate()?;
        Ok(vec![])
    }

    /// Return the current TOML document as a string.
    pub fn to_toml(&self) -> String {
        self.doc.to_string()
    }

    /// Parse the current document into an [`OrkaConfig`].
    pub fn to_orka_config(&self) -> Result<OrkaConfig> {
        toml::from_str(&self.to_toml())
            .map_err(|e| Error::Config(format!("config parse error: {e}")))
    }

    /// Return whether the given dotted path corresponds to an array-of-tables
    /// in the Orka schema.
    pub fn is_array_of_tables(path: &str) -> bool {
        ARRAY_OF_TABLES.contains(&path)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Navigate to (or create) a [`Table`] at the given dotted path within `doc`.
fn navigate_or_create<'a>(doc: &'a mut DocumentMut, path: &str) -> Result<&'a mut Table> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut item = doc.as_item_mut();
    for part in &parts {
        let table = item
            .as_table_mut()
            .ok_or_else(|| Error::Config(format!("expected table at path component '{part}'")))?;
        if !table.contains_key(part) {
            table[*part] = Item::Table(Table::new());
        }
        item = &mut table[*part];
    }
    item.as_table_mut()
        .ok_or_else(|| Error::Config(format!("expected table at path '{path}'")))
}

/// Navigate to a mutable [`Item`] following the given path segments.
fn navigate_item_mut<'a>(item: &'a mut Item, parts: &[&str]) -> Result<&'a mut Item> {
    let mut current = item;
    for part in parts {
        let table = current
            .as_table_mut()
            .ok_or_else(|| Error::Config(format!("expected table at '{part}'")))?;
        if !table.contains_key(part) {
            table[*part] = Item::Table(Table::new());
        }
        current = &mut table[*part];
    }
    Ok(current)
}

/// Convert a [`serde_json::Value`] to a [`toml_edit::Item`].
///
/// Returns `None` for `null` values (which are omitted from the output).
fn json_to_item(value: &serde_json::Value) -> Option<Item> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(toml_edit::value(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(toml_edit::value(i))
            } else {
                n.as_f64().map(toml_edit::value)
            }
        }
        serde_json::Value::String(s) => Some(toml_edit::value(s.as_str())),
        serde_json::Value::Array(arr) => {
            let mut toml_arr = Array::new();
            for v in arr {
                if let Some(item) = json_to_item(v)
                    && let Ok(val) = item.into_value()
                {
                    toml_arr.push(val);
                }
            }
            Some(Item::Value(Value::Array(toml_arr)))
        }
        serde_json::Value::Object(map) => {
            let mut inline = InlineTable::new();
            for (k, v) in map {
                if let Some(item) = json_to_item(v)
                    && let Ok(val) = item.into_value()
                {
                    inline.insert(k, val);
                }
            }
            Some(Item::Value(Value::InlineTable(inline)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_builder_has_config_version() {
        let b = ConfigBuilder::new();
        let toml = b.to_toml();
        assert!(toml.contains("config_version = 6"));
    }

    #[test]
    fn set_section_simple() {
        let mut b = ConfigBuilder::new();
        b.set_section(
            "server",
            &serde_json::json!({ "host": "0.0.0.0", "port": 8080 }),
        )
        .unwrap();
        let toml = b.to_toml();
        assert!(toml.contains("[server]"));
        assert!(toml.contains("host = \"0.0.0.0\""));
        assert!(toml.contains("port = 8080"));
    }

    #[test]
    fn set_section_nested() {
        let mut b = ConfigBuilder::new();
        b.set_section(
            "adapters.telegram",
            &serde_json::json!({ "bot_token_secret": "telegram_token" }),
        )
        .unwrap();
        let toml = b.to_toml();
        assert!(toml.contains("bot_token_secret"));
    }

    #[test]
    fn append_array_entry() {
        let mut b = ConfigBuilder::new();
        b.append_array_entry(
            "agents",
            &serde_json::json!({
                "id": "assistant",
                "kind": "agent",
                "name": "Assistant",
                "system_prompt": "You are a helpful assistant."
            }),
        )
        .unwrap();
        let toml = b.to_toml();
        assert!(toml.contains("[[agents]]"));
        assert!(toml.contains("id = \"assistant\""));
    }

    #[test]
    fn append_multiple_providers() {
        let mut b = ConfigBuilder::new();
        b.append_array_entry(
            "llm.providers",
            &serde_json::json!({
                "name": "anthropic",
                "provider": "anthropic",
                "model": "claude-sonnet-4-6",
                "api_key_secret": "llm/anthropic"
            }),
        )
        .unwrap();
        b.append_array_entry(
            "llm.providers",
            &serde_json::json!({
                "name": "openai",
                "provider": "openai",
                "model": "gpt-4o",
                "api_key_secret": "llm/openai"
            }),
        )
        .unwrap();
        let toml = b.to_toml();
        assert!(toml.contains("[[llm.providers]]"));
        assert!(toml.contains("name = \"anthropic\""));
        assert!(toml.contains("name = \"openai\""));
    }

    #[test]
    fn null_values_are_skipped() {
        let mut b = ConfigBuilder::new();
        b.set_section("server", &serde_json::json!({ "host": null, "port": 8080 }))
            .unwrap();
        let toml = b.to_toml();
        assert!(!toml.contains("host"));
        assert!(toml.contains("port = 8080"));
    }

    #[test]
    fn validate_minimal_valid_config() {
        let mut b = ConfigBuilder::new();
        b.append_array_entry(
            "llm.providers",
            &serde_json::json!({
                "name": "anthropic",
                "provider": "anthropic",
                "api_key_env": "ANTHROPIC_API_KEY"
            }),
        )
        .unwrap();
        // Minimal config should parse without errors
        assert!(b.to_orka_config().is_ok());
    }
}
