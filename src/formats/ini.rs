use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

use crate::manifest::Precedence;

fn scalar_to_string(value: &Value) -> Result<String> {
    match value {
        Value::String(s) => {
            ensure!(
                !s.contains(['\n', '\r', '\0']),
                "INI values must not contain newlines or NUL"
            );
            Ok(s.clone())
        }
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => bail!("INI values must be strings, numbers, or booleans"),
    }
}

fn validate_key(key: &str) -> Result<()> {
    ensure!(!key.is_empty(), "INI keys must not be empty");
    ensure!(
        !key.contains(['=', ':', '[', ']', '\n', '\r', '\0']),
        "INI key {key:?} contains a reserved character"
    );
    Ok(())
}

fn validate_section(section: &str) -> Result<()> {
    ensure!(!section.is_empty(), "INI section names must not be empty");
    ensure!(
        !section.contains(['[', ']', '\n', '\r', '\0', ';', '#']),
        "INI section {section:?} contains a reserved character"
    );
    Ok(())
}

// Normalized content model: global keys plus named sections, insertion order.
// Merge output is always re-rendered in this canonical form; comments, blank
// lines and original separators are intentionally not preserved.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct IniData {
    globals: Vec<(String, String)>,
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl IniData {
    fn section_mut(&mut self, name: &str) -> &mut Vec<(String, String)> {
        if let Some(pos) = self.sections.iter().position(|(s, _)| s == name) {
            &mut self.sections[pos].1
        } else {
            self.sections.push((name.to_string(), Vec::new()));
            &mut self.sections.last_mut().expect("just pushed").1
        }
    }

    fn upsert(keys: &mut Vec<(String, String)>, key: &str, value: String) {
        if let Some(slot) = keys.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value;
        } else {
            keys.push((key.to_string(), value));
        }
    }
}

fn parse_declared(value: &Value) -> Result<IniData> {
    let top = value.as_object().context("INI content must be a table")?;
    let mut data = IniData::default();
    for (key, item) in top {
        match item {
            Value::Object(inner) => {
                validate_section(key)?;
                for (inner_key, inner_value) in inner {
                    validate_key(inner_key)?;
                    data.section_mut(key)
                        .push((inner_key.clone(), scalar_to_string(inner_value)?));
                }
            }
            Value::Array(_) => bail!("INI section {key:?} must be a table of scalar values"),
            Value::Null => {
                bail!("INI key {key:?} must be a string, number, boolean, or section table")
            }
            scalar if scalar.is_string() || scalar.is_number() || scalar.is_boolean() => {
                validate_key(key)?;
                data.globals.push((key.clone(), scalar_to_string(scalar)?));
            }
            _ => bail!("INI key {key:?} must be a string, number, boolean, or section table"),
        }
    }
    Ok(data)
}

fn render_data(data: &IniData) -> String {
    let mut out = String::new();
    for (key, val) in &data.globals {
        out.push_str(&format!("{key} = {val}\n"));
    }
    for (section, keys) in &data.sections {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("[{section}]\n"));
        for (key, val) in keys {
            out.push_str(&format!("{key} = {val}\n"));
        }
    }
    out
}

pub fn render_ini(value: &Value) -> Result<String> {
    Ok(render_data(&parse_declared(value)?))
}

/// Strip an inline `;`/`#` comment: the marker only counts when preceded by a
/// space or tab, so values like `foo;bar` survive.
fn strip_inline_comment(value: &str) -> &str {
    let mut prev_is_blank = false;
    for (idx, ch) in value.char_indices() {
        if (ch == ';' || ch == '#') && prev_is_blank {
            return value.get(..idx).map(str::trim_end).unwrap_or(value);
        }
        prev_is_blank = ch == ' ' || ch == '\t';
    }
    value
}

fn parse_existing(existing: &str) -> Result<IniData> {
    ensure!(
        !existing.contains('\0'),
        "failed to parse existing INI: NUL byte"
    );
    let mut data = IniData::default();
    let mut current: Option<String> = None;
    for raw in existing.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            let Some(close) = raw.find(']') else {
                bail!("failed to parse existing INI: unbalanced section header {raw:?}");
            };
            let Some(open) = raw.find('[') else {
                bail!("failed to parse existing INI: unbalanced section header {raw:?}");
            };
            let Some(inside) = raw.get(open + 1..close) else {
                bail!("failed to parse existing INI: unbalanced section header {raw:?}");
            };
            let name = inside.trim().to_string();
            ensure!(
                !name.is_empty(),
                "failed to parse existing INI: empty section name in {raw:?}"
            );
            let Some(rest_raw) = raw.get(close + 1..) else {
                bail!("failed to parse existing INI: unbalanced section header {raw:?}");
            };
            let rest = rest_raw.trim();
            ensure!(
                rest.is_empty() || rest.starts_with([';', '#']),
                "failed to parse existing INI: unexpected content after section header {raw:?}"
            );
            if !data.sections.iter().any(|(s, _)| s == &name) {
                data.sections.push((name.clone(), Vec::new()));
            }
            current = Some(name);
        } else {
            let Some(pos) = raw.find(['=', ':']) else {
                bail!("failed to parse existing INI: unexpected line {raw:?}");
            };
            let Some(left) = raw.get(..pos) else {
                bail!("failed to parse existing INI: unexpected line {raw:?}");
            };
            let key = left.trim().to_string();
            ensure!(
                !key.is_empty(),
                "failed to parse existing INI: empty key in {raw:?}"
            );
            let Some(after) = raw.get(pos + 1..) else {
                bail!("failed to parse existing INI: unexpected line {raw:?}");
            };
            let value = strip_inline_comment(after.trim()).to_string();
            match &current {
                None => IniData::upsert(&mut data.globals, &key, value),
                Some(section) => IniData::upsert(data.section_mut(section), &key, value),
            }
        }
    }
    Ok(data)
}

pub fn merge_ini(declared: &Value, existing: &str, precedence: Precedence) -> Result<String> {
    let declared = parse_declared(declared)?;
    let existing = parse_existing(existing)?;
    let mut merged = existing;
    for (key, value) in &declared.globals {
        match merged.globals.iter_mut().find(|(k, _)| k == key) {
            Some(slot) if matches!(precedence, Precedence::Declared) => slot.1 = value.clone(),
            Some(_) => {}
            None => merged.globals.push((key.clone(), value.clone())),
        }
    }
    for (section, keys) in &declared.sections {
        let target = merged.section_mut(section);
        for (key, value) in keys {
            match target.iter_mut().find(|(k, _)| k == key) {
                Some(slot) if matches!(precedence, Precedence::Declared) => {
                    slot.1 = value.clone();
                }
                Some(_) => {}
                None => target.push((key.clone(), value.clone())),
            }
        }
    }
    Ok(render_data(&merged))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_ini() -> Result<()> {
        let value = json!({
            "global_key": "top",
            "port": 8080,
            "enabled": true,
            "server": {"host": "example.com", "port": 80}
        });
        let rendered = render_ini(&value)?;
        assert!(rendered.contains("global_key = top"));
        assert!(rendered.contains("port = 8080"));
        assert!(rendered.contains("enabled = true"));
        assert!(rendered.contains("[server]"));
        assert!(rendered.contains("host = example.com"));
        Ok(())
    }

    #[test]
    fn rejects_non_table_and_nested_values() {
        assert!(render_ini(&json!([1])).is_err());
        assert!(render_ini(&json!({"s": {"nested": {"deep": 1}}})).is_err());
        assert!(render_ini(&json!({"s": {"list": [1]}})).is_err());
        assert!(render_ini(&json!({"s": {"v": null}})).is_err());
        assert!(render_ini(&json!({"v": null})).is_err());
    }

    #[test]
    fn existing_values_win() -> Result<()> {
        let declared = json!({"server": {"host": "declared", "added": true}, "global": "declared"});
        let existing = "# keep me\ninvite = existing\nglobal = existing\n\n[server]\n# comment\nhost = existing\n";
        let merged = merge_ini(&declared, existing, Precedence::Existing)?;
        assert_eq!(
            merged,
            "invite = existing\nglobal = existing\n\n[server]\nhost = existing\nadded = true\n"
        );
        Ok(())
    }

    #[test]
    fn declared_values_win_and_normalize() -> Result<()> {
        let declared = json!({"server": {"host": "declared"}});
        let existing =
            "[server] # section comment\nhost = existing ; keep this comment\nother = 1\n";
        let merged = merge_ini(&declared, existing, Precedence::Declared)?;
        assert_eq!(merged, "[server]\nhost = declared\nother = 1\n");
        Ok(())
    }

    #[test]
    fn appends_new_sections_and_keys() -> Result<()> {
        let declared = json!({"new_section": {"key": "v"}, "server": {"fresh": "yes"}});
        let existing = "[server]\nhost = h\n";
        let merged = merge_ini(&declared, existing, Precedence::Existing)?;
        assert_eq!(
            merged,
            "[server]\nhost = h\nfresh = yes\n\n[new_section]\nkey = v\n"
        );
        Ok(())
    }

    #[test]
    fn merge_is_idempotent() -> Result<()> {
        let declared = json!({"g": "1", "s": {"a": "1", "b": 2}});
        let existing = "; header\ng = old\n[s]\na = old\n";
        for precedence in [Precedence::Existing, Precedence::Declared] {
            let once = merge_ini(&declared, existing, precedence)?;
            let twice = merge_ini(&declared, &once, precedence)?;
            assert_eq!(once, twice);
        }
        Ok(())
    }

    #[test]
    fn duplicates_last_win_case_sensitive() -> Result<()> {
        let merged = merge_ini(
            &json!({}),
            "[s]\nkey = first\nkey = second\nKey = upper\n",
            Precedence::Existing,
        )?;
        assert_eq!(merged, "[s]\nkey = second\nKey = upper\n");
        Ok(())
    }

    #[test]
    fn rejects_malformed_existing() {
        assert!(merge_ini(&json!({}), "no separator here", Precedence::Existing).is_err());
        assert!(merge_ini(&json!({}), "[unclosed", Precedence::Existing).is_err());
    }

    #[test]
    fn colon_separator_is_accepted_and_normalized() -> Result<()> {
        let declared = json!({"s": {"k": "new"}});
        let merged = merge_ini(&declared, "[s]\nk : old\n", Precedence::Declared)?;
        assert_eq!(merged, "[s]\nk = new\n");
        Ok(())
    }
}
