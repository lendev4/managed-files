use anyhow::{Result, ensure};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub backup_extension: Option<String>,
    pub files: Vec<ManagedFile>,
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported manifest version {}",
            self.version
        );
        validate_backup_extension(self.backup_extension.as_deref())?;
        for file in &self.files {
            validate_backup_extension(
                file.backup_extension
                    .resolve(self.backup_extension.as_deref()),
            )?;
            if matches!(file.mode, Mode::Merge) {
                ensure!(
                    matches!(file.content, Content::Json { .. } | Content::Toml { .. }),
                    "{}: merge mode only supports JSON and TOML",
                    file.target.display()
                );
                if let Content::Json { json } = &file.content {
                    ensure!(
                        json.is_object(),
                        "{}: JSON merge content must be an object",
                        file.target.display()
                    );
                }
            }
            if let Content::Toml { toml } = &file.content {
                ensure!(
                    toml.is_object(),
                    "{}: TOML content must be a table",
                    file.target.display()
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "RawFile")]
pub struct ManagedFile {
    pub target: PathBuf,
    pub mode: Mode,
    pub precedence: Precedence,
    pub backup_extension: BackupExtension,
    pub content: Content,
}

#[derive(Debug, Default)]
pub enum BackupExtension {
    #[default]
    Inherit,
    Override(Option<String>),
}

impl BackupExtension {
    pub fn resolve<'a>(&'a self, global: Option<&'a str>) -> Option<&'a str> {
        match self {
            Self::Inherit => global,
            Self::Override(extension) => extension.as_deref(),
        }
    }
}

impl<'de> Deserialize<'de> for BackupExtension {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<String>::deserialize(deserializer).map(Self::Override)
    }
}

fn validate_backup_extension(extension: Option<&str>) -> Result<()> {
    if let Some(extension) = extension {
        ensure!(
            !extension.is_empty() && !extension.contains(['/', '\\', '\0']),
            "backup_extension must be a nonempty filename suffix without separators or NUL"
        );
    }
    Ok(())
}

// Preserve field presence even for JSON null, and let serde reject duplicate
// fields and unknown keys before converting the content into an enum.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    target: PathBuf,
    mode: Mode,
    #[serde(default)]
    precedence: Precedence,
    #[serde(default)]
    backup_extension: BackupExtension,
    #[serde(default, deserialize_with = "present")]
    source: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    text: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    json: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    toml: Option<Value>,
}

impl TryFrom<RawFile> for ManagedFile {
    type Error = anyhow::Error;

    fn try_from(raw: RawFile) -> Result<Self> {
        let count = [&raw.source, &raw.text, &raw.json, &raw.toml]
            .iter()
            .filter(|value| value.is_some())
            .count();
        ensure!(
            count == 1,
            "{}: define exactly one of source, text, json, or toml",
            raw.target.display()
        );
        let content = if let Some(source) = raw.source {
            Content::Source {
                source: serde_json::from_value(source)?,
            }
        } else if let Some(text) = raw.text {
            Content::Text {
                text: serde_json::from_value(text)?,
            }
        } else if let Some(json) = raw.json {
            Content::Json { json }
        } else {
            Content::Toml {
                toml: raw.toml.expect("one content field was checked"),
            }
        };
        Ok(Self {
            target: raw.target,
            mode: raw.mode,
            precedence: raw.precedence,
            backup_extension: raw.backup_extension,
            content,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Seed,
    Merge,
    Replace,
    ReplaceReadonly,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Precedence {
    #[default]
    Existing,
    Declared,
}

#[derive(Debug)]
pub enum Content {
    Source { source: PathBuf },
    Text { text: String },
    Json { json: Value },
    Toml { toml: Value },
}
