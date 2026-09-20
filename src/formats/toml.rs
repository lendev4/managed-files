use anyhow::{Context, Result};
use serde_json::Value;
use toml_edit::{DocumentMut, Item, Table, TableLike};

use crate::manifest::Precedence;

pub fn render_toml(value: &Value) -> Result<String> {
    let toml_value: toml::Value = serde_json::from_value(value.clone())
        .context("failed to convert structured value to TOML")?;

    toml::to_string_pretty(&toml_value).context("failed to render TOML")
}

pub fn merge_document(declared: &Value, existing: &str, precedence: Precedence) -> Result<String> {
    let mut existing_doc = existing
        .parse::<DocumentMut>()
        .context("failed to parse existing TOML")?;

    let declared_text = render_toml(declared)?;

    let declared_doc = declared_text
        .parse::<DocumentMut>()
        .context("failed to parse declared TOML")?;

    // Parsed declarations have their own table positions. Reusing those
    // positions can insert unrelated tables inside existing comment blocks.
    let mut next_position = 0;
    visit_tables(existing_doc.as_table_mut(), false, &mut |table, _| {
        next_position = next_position.max(table.position().unwrap_or(0) + 1);
    });
    let first_new_position = next_position;
    merge_tables(
        existing_doc.as_table_mut(),
        declared_doc.as_table(),
        precedence,
        false,
        &mut next_position,
    );

    // toml_edit keeps footer comments at document end. Keep them with the
    // original content instead of letting newly appended tables precede them.
    let mut first_appended = None;
    visit_tables(
        existing_doc.as_table_mut(),
        false,
        &mut |table, is_array| {
            if let Some(position) = table.position()
                && position >= first_new_position
                && (is_array
                    || (!table.is_dotted()
                        && (!table.is_implicit() || !table.get_values().is_empty())))
            {
                first_appended =
                    Some(first_appended.map_or(position, |first: isize| first.min(position)));
            }
        },
    );
    if let Some(position) = first_appended {
        let trailing = existing_doc
            .trailing()
            .as_str()
            .unwrap_or_default()
            .to_owned();
        existing_doc.set_trailing("");
        visit_tables(existing_doc.as_table_mut(), false, &mut |table, _| {
            if table.position() == Some(position) {
                let prefix = table
                    .decor()
                    .prefix()
                    .and_then(|p| p.as_str())
                    .unwrap_or("\n");
                let prefix = format!("{trailing}{prefix}");
                table.decor_mut().set_prefix(prefix);
            }
        });
    }

    Ok(existing_doc.to_string())
}

fn visit_tables(table: &mut Table, is_array: bool, visit: &mut impl FnMut(&mut Table, bool)) {
    visit(table, is_array);
    for (_, item) in table.iter_mut() {
        match item {
            Item::Table(child) => visit_tables(child, false, visit),
            Item::ArrayOfTables(array) => {
                for child in array.iter_mut() {
                    visit_tables(child, true, visit);
                }
            }
            _ => {}
        }
    }
}

fn inserted_item(declared: &Item, inline: bool, next_position: &mut isize) -> Item {
    let mut item = declared.clone();
    if inline {
        item.make_value();
    } else {
        let mut assign = |table: &mut Table, _: bool| {
            table.set_position(Some(*next_position));
            *next_position += 1;
        };
        match &mut item {
            Item::Table(table) => visit_tables(table, false, &mut assign),
            Item::ArrayOfTables(array) => {
                for table in array.iter_mut() {
                    visit_tables(table, true, &mut assign);
                }
            }
            _ => {}
        }
    }
    item
}

fn merge_tables(
    existing: &mut dyn TableLike,
    declared: &dyn TableLike,
    precedence: Precedence,
    inline: bool,
    next_position: &mut isize,
) {
    for (key, declared_item) in declared.iter() {
        match existing.get_mut(key) {
            Some(existing_item) => {
                merge_items(
                    existing_item,
                    declared_item,
                    precedence,
                    inline,
                    next_position,
                );
            }

            None => {
                existing.insert(key, inserted_item(declared_item, inline, next_position));
            }
        }
    }
}

fn merge_items(
    existing: &mut Item,
    declared: &Item,
    precedence: Precedence,
    inline: bool,
    next_position: &mut isize,
) {
    let child_inline = existing.is_inline_table();
    if let (Some(existing_table), Some(declared_table)) =
        (existing.as_table_like_mut(), declared.as_table_like())
    {
        merge_tables(
            existing_table,
            declared_table,
            precedence,
            child_inline,
            next_position,
        );

        return;
    }

    if matches!(precedence, Precedence::Declared) {
        let mut replacement = inserted_item(declared, inline, next_position);
        if let (Some(old), Some(new)) = (existing.as_value(), replacement.as_value_mut()) {
            *new.decor_mut() = old.decor().clone();
        }
        if let (Some(old), Some(new)) = (
            existing.as_array_of_tables(),
            replacement.as_array_of_tables_mut(),
        ) && let Some(first) = old.iter().next()
        {
            // Arrays are replaced as units, but keep the section's original
            // location and leading comments instead of moving it on every run.
            for table in new.iter_mut() {
                visit_tables(table, true, &mut |table, _| {
                    table.set_position(first.position())
                });
            }
            if let Some(table) = new.iter_mut().next() {
                *table.decor_mut() = first.decor().clone();
            }
        }
        *existing = replacement;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_toml() -> Result<()> {
        let value = json!({
            "theme": "dark",
            "editor": {
                "fontSize": 18
            }
        });

        let rendered = render_toml(&value)?;

        assert!(rendered.contains("theme = \"dark\""));

        assert!(rendered.contains("[editor]"));

        assert!(rendered.contains("fontSize = 18"));

        Ok(())
    }

    #[test]
    fn existing_values_win() -> Result<()> {
        let declared = json!({
            "theme": "dark",
            "editor": {
                "fontSize": 12
            }
        });

        let existing = r#"
theme = "light"

[editor]
fontSize = 18
"#;

        let merged = merge_document(&declared, existing, Precedence::Existing)?;

        assert!(merged.contains("theme = \"light\""));

        assert!(merged.contains("fontSize = 18"));

        Ok(())
    }

    #[test]
    fn declared_values_win() -> Result<()> {
        let declared = json!({
            "theme": "dark",
            "editor": {
                "fontSize": 12
            }
        });

        let existing = r#"
theme = "light"

[editor]
fontSize = 18
"#;

        let merged = merge_document(&declared, existing, Precedence::Declared)?;

        assert!(merged.contains("theme = \"dark\""));

        assert!(merged.contains("fontSize = 12"));

        Ok(())
    }

    #[test]
    fn adds_missing_values() -> Result<()> {
        let declared = json!({
            "theme": "dark",
            "editor": {
                "fontSize": 12,
                "autosave": true
            }
        });

        let existing = r#"
[editor]
fontSize = 18
"#;

        let merged = merge_document(&declared, existing, Precedence::Existing)?;

        assert!(merged.contains("theme = \"dark\""));

        assert!(merged.contains("fontSize = 18"));

        assert!(merged.contains("autosave = true"));

        Ok(())
    }

    #[test]
    fn preserves_comments() -> Result<()> {
        let declared = json!({
            "theme": "dark",
            "editor": {
                "fontSize": 12,
                "autosave": true
            }
        });

        let existing = r#"
# This comment should survive
theme = "light" # user choice

[editor]
# My font size
fontSize = 18
"#;

        let merged = merge_document(&declared, existing, Precedence::Existing)?;

        assert!(merged.contains("# This comment should survive"));

        assert!(merged.contains("# user choice"));

        assert!(merged.contains("# My font size"));

        Ok(())
    }

    #[test]
    fn inline_and_standard_tables_merge_recursively() -> Result<()> {
        let declared = json!({"editor": {
            "size": 12, "added": true,
            "nested": {"changed": "declared", "new": {"enabled": true}},
            "plugins": ["declared"]
        }});
        for existing in [
            "editor = { size = 18, local = true, nested = { changed = 'existing', local = 1 }, plugins = ['existing'] } # keep\n",
            "[editor]\nsize = 18 # keep\nlocal = true\nplugins = ['existing']\n[editor.nested]\nchanged = 'existing'\nlocal = 1\n",
            "editor.size = 18 # keep\neditor.local = true\neditor.nested = { changed = 'existing', local = 1 }\neditor.plugins = ['existing']\n",
        ] {
            for precedence in [Precedence::Existing, Precedence::Declared] {
                let merged = merge_document(&declared, existing, precedence)?;
                let parsed: toml::Value = toml::from_str(&merged)?;
                let prefer_declared = matches!(precedence, Precedence::Declared);
                assert_eq!(
                    parsed["editor"]["size"].as_integer(),
                    Some(if prefer_declared { 12 } else { 18 })
                );
                assert_eq!(parsed["editor"]["local"].as_bool(), Some(true));
                assert_eq!(parsed["editor"]["added"].as_bool(), Some(true));
                assert_eq!(parsed["editor"]["nested"]["local"].as_integer(), Some(1));
                assert_eq!(
                    parsed["editor"]["nested"]["new"]["enabled"].as_bool(),
                    Some(true)
                );
                let expected = if prefer_declared {
                    "declared"
                } else {
                    "existing"
                };
                assert_eq!(
                    parsed["editor"]["nested"]["changed"].as_str(),
                    Some(expected)
                );
                assert_eq!(parsed["editor"]["plugins"][0].as_str(), Some(expected));
                assert!(merged.contains("# keep"));
                assert_eq!(merge_document(&declared, &merged, precedence)?, merged);
            }
        }
        Ok(())
    }

    #[test]
    fn inline_replacements_and_insertions_remain_valid_values() -> Result<()> {
        let existing =
            "config = { scalar = 1, table = { old = true }, list = [1], local = true }\n";
        let declared = json!({"config": {
            "scalar": {"nested": {"value": 2}},
            "table": "replacement", "list": [{"id": 1}, {"id": 2}],
            "new": {"value": 3}
        }});
        let merged = merge_document(&declared, existing, Precedence::Declared)?;
        let parsed: toml::Value = toml::from_str(&merged)?;
        let mut expected = declared;
        expected["config"]["local"] = json!(true);
        assert_eq!(serde_json::to_value(parsed)?, expected);
        assert!(merged.starts_with("config = {"));
        Ok(())
    }

    #[test]
    fn new_tables_follow_existing_footer_comments() -> Result<()> {
        let existing = "palette = 'noctalia'\n\n# BEGIN GENERATED\n[palettes.noctalia]\nblue = '#123456'\n# END GENERATED\n";
        let declared = json!({"add_newline": false, "package": {"disabled": true}, "custom": {"nested": {"value": 1}}});
        let merged = merge_document(&declared, existing, Precedence::Declared)?;
        let end = merged.find("# END GENERATED").unwrap();
        assert!(merged.find("[package]").unwrap() > end);
        assert!(merged.find("[custom.nested]").unwrap() > end);
        let parsed: toml::Value = toml::from_str(&merged)?;
        assert_eq!(parsed["package"]["disabled"].as_bool(), Some(true));
        assert_eq!(
            parsed["palettes"]["noctalia"]["blue"].as_str(),
            Some("#123456")
        );
        assert_eq!(merged.matches("# END GENERATED").count(), 1);
        assert_eq!(
            merge_document(&declared, &merged, Precedence::Declared)?,
            merged
        );
        Ok(())
    }

    #[test]
    fn inserted_tables_do_not_interleave_existing_sections() -> Result<()> {
        let existing = "# BEGIN GENERATED\n[palette.first]\ncolor = 'red'\n[palette.second]\ncolor = 'blue'\n# END GENERATED\n\n[user]\nlocal = true\n";
        let declared = json!({"new": {"enabled": true}, "user": {"nested": {"enabled": true}}, "items": [{"id":1}, {"id":2}]});
        let merged = merge_document(&declared, existing, Precedence::Declared)?;
        let end = merged.find("# END GENERATED").unwrap();
        for header in ["[new]", "[user.nested]", "[[items]]"] {
            assert!(merged.find(header).unwrap() > end, "{merged}");
        }
        let parsed: toml::Value = toml::from_str(&merged)?;
        assert_eq!(parsed["palette"]["first"]["color"].as_str(), Some("red"));
        assert_eq!(parsed["items"].as_array().unwrap().len(), 2);
        assert_eq!(
            merge_document(&declared, &merged, Precedence::Declared)?,
            merged
        );
        Ok(())
    }
}
