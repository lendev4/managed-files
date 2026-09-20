use anyhow::{Context, Result, bail, ensure};
use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use tempfile::NamedTempFile;

use crate::formats::json::merge_values;
use crate::formats::toml::{merge_document, render_toml};
use crate::manifest::{Content, Manifest, Mode};

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    contents: Vec<u8>,
    identity: Identity,
}

#[derive(Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    mode: u32,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    links: u64,
}

impl From<&fs::Metadata> for Identity {
    fn from(m: &fs::Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            mode: m.mode(),
            size: m.size(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
            links: m.nlink(),
        }
    }
}

fn snapshot(path: &Path) -> Result<Option<Snapshot>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file(),
        "{}: target or backup must be a regular file (symlinks are not supported)",
        path.display()
    );
    ensure!(
        metadata.nlink() == 1,
        "{}: hard-linked targets and backups are not supported",
        path.display()
    );
    let contents = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let identity = Identity::from(&metadata);
    ensure!(
        Identity::from(&fs::symlink_metadata(path)?) == identity,
        "{} changed while being read; retry application",
        path.display()
    );
    Ok(Some(Snapshot { contents, identity }))
}

// Resolve existing parent symlinks without following the leaf. Resolve missing
// directories recursively so aliases collide even before directories exist.
fn resolve_directory(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            ensure!(
                fs::metadata(path)?.is_dir(),
                "{} is not a directory",
                path.display()
            );
            fs::canonicalize(path).with_context(|| format!("failed to resolve {}", path.display()))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let parent = path.parent().context("directory has no parent")?;
            Ok(resolve_directory(parent)?.join(path.file_name().context("directory has no name")?))
        }
        Err(error) => Err(error.into()),
    }
}

fn resolve_target(path: &Path) -> Result<PathBuf> {
    ensure!(
        !path.as_os_str().is_empty() && path.file_name().is_some(),
        "target must name a file"
    );
    ensure!(
        !path.as_os_str().as_encoded_bytes().ends_with(b"/"),
        "{}: target must not end with a directory separator",
        path.display()
    );
    ensure!(
        !path.components().any(|p| matches!(p, Component::ParentDir)),
        "{}: parent traversal (..) is not allowed in targets",
        path.display()
    );
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(
        resolve_directory(absolute.parent().context("target has no parent")?)?
            .join(absolute.file_name().context("target has no filename")?),
    )
}

#[derive(Debug)]
struct Destination {
    requested: PathBuf,
    resolved: PathBuf,
    before: Option<Snapshot>,
}

impl Destination {
    fn inspect(requested: PathBuf) -> Result<Self> {
        let resolved = resolve_target(&requested)?;
        let before = snapshot(&resolved)?;
        Ok(Self {
            requested,
            resolved,
            before,
        })
    }

    fn check(&self) -> Result<()> {
        ensure!(
            resolve_target(&self.requested)? == self.resolved,
            "{}: parent directory changed; retry application",
            self.requested.display()
        );
        ensure!(
            snapshot(&self.resolved)? == self.before,
            "{} changed since preparation; retry application",
            self.requested.display()
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Created,
    Merged,
    Replaced,
    Skipped,
}

impl Action {
    pub fn label(self, dry_run: bool) -> &'static str {
        match (self, dry_run) {
            (Self::Created, false) => "created",
            (Self::Created, true) => "would create",
            (Self::Merged, false) => "merged",
            (Self::Merged, true) => "would merge",
            (Self::Replaced, false) => "replaced",
            (Self::Replaced, true) => "would replace",
            (Self::Skipped, _) => "skipped",
        }
    }
}

#[derive(Debug)]
pub struct PlannedFile {
    target: Destination,
    backup: Option<Destination>,
    contents: Vec<u8>,
    mode: u32,
    pub action: Action,
}

impl PlannedFile {
    pub fn target(&self) -> &Path {
        &self.target.requested
    }
    pub fn backup(&self) -> Option<&Path> {
        self.backup.as_ref().map(|b| b.requested.as_path())
    }

    fn check(&self) -> Result<()> {
        self.target.check()?;
        if let Some(backup) = &self.backup {
            backup.check()?;
        }
        Ok(())
    }

    pub fn apply(&self) -> Result<()> {
        self.check()?;
        if matches!(self.action, Action::Skipped) {
            return Ok(());
        }
        // Prepare the replacement before touching the backup. Its final mode is
        // set before rename, including for read-only files.
        let temp = prepare_write(&self.target.resolved, &self.contents, self.mode)?;
        self.check()?;
        if let Some(backup) = &self.backup {
            let original = self
                .target
                .before
                .as_ref()
                .expect("backup requires an existing target");
            let backup_temp = prepare_write(&backup.resolved, &original.contents, 0o600)?;
            self.check()?;
            persist(backup_temp, backup)?;
        }
        self.target.check()?;
        persist(temp, &self.target)
    }
}

pub struct Plan {
    pub files: Vec<PlannedFile>,
}

impl Plan {
    /// Read, render and validate everything before creating directories or files.
    pub fn prepare(manifest: &Manifest, manifest_path: &Path) -> Result<Self> {
        manifest.validate()?;
        let mut files = Vec::new();
        let mut destinations = Vec::new();
        let mut inputs = vec![fs::canonicalize(manifest_path)?];
        for file in &manifest.files {
            let prepared = (|| -> Result<PlannedFile> {
                let target = Destination::inspect(file.target.clone())?;
                destinations.push((target.requested.clone(), target.resolved.clone()));
                let declared = render_content(&file.content)?;
                if let Content::Source { source } = &file.content {
                    inputs.push(fs::canonicalize(source)?);
                }
                let skipped = matches!(file.mode, Mode::Seed) && target.before.is_some();
                let contents = if !skipped && matches!(file.mode, Mode::Merge) {
                    if let Some(before) = &target.before {
                        let text = std::str::from_utf8(&before.contents)
                            .context("existing content is not UTF-8")?;
                        match &file.content {
                            Content::Json { json } => {
                                let existing = serde_json::from_str(text)
                                    .context("failed to parse existing JSON")?;
                                ensure!(
                                    matches!(existing, serde_json::Value::Object(_)),
                                    "existing JSON merge content must be an object"
                                );
                                render_json(&merge_values(json, &existing, file.precedence))?
                            }
                            Content::Toml { toml } => {
                                merge_document(toml, text, file.precedence)?.into_bytes()
                            }
                            _ => unreachable!("manifest validation checks merge content"),
                        }
                    } else {
                        declared
                    }
                } else {
                    declared
                };
                let existing_mode = target
                    .before
                    .as_ref()
                    .map_or(0o600, |s| s.identity.mode & 0o777);
                let mode = match file.mode {
                    Mode::Replace => existing_mode | 0o200,
                    Mode::ReplaceReadonly => existing_mode & !0o222,
                    _ => existing_mode,
                };
                let action = if skipped {
                    Action::Skipped
                } else if target.before.is_none() {
                    Action::Created
                } else if matches!(file.mode, Mode::Merge) {
                    Action::Merged
                } else {
                    Action::Replaced
                };
                let reserved_backup = if !matches!(file.mode, Mode::Seed) {
                    manifest
                        .backup_extension
                        .as_ref()
                        .map(|extension| {
                            let mut name = file
                                .target
                                .file_name()
                                .context("target has no filename")?
                                .to_os_string();
                            name.push(extension);
                            Destination::inspect(file.target.with_file_name(name))
                        })
                        .transpose()?
                } else {
                    None
                };
                if let Some(backup) = &reserved_backup {
                    destinations.push((backup.requested.clone(), backup.resolved.clone()));
                }
                let backup = if target.before.is_some() {
                    reserved_backup
                } else {
                    None
                };
                Ok(PlannedFile {
                    target,
                    backup,
                    contents,
                    mode,
                    action,
                })
            })()
            .with_context(|| format!("failed to prepare {}", file.target.display()))?;
            files.push(prepared);
        }
        for (index, destination) in destinations.iter().enumerate() {
            for other in &destinations[..index] {
                ensure!(
                    !overlaps(&destination.1, &other.1),
                    "conflicting destinations: {} and {}",
                    destination.0.display(),
                    other.0.display()
                );
            }
            for input in &inputs {
                ensure!(
                    !overlaps(&destination.1, input),
                    "{} conflicts with input {}",
                    destination.0.display(),
                    input.display()
                );
            }
        }
        let plan = Self { files };
        plan.check()?;
        Ok(plan)
    }

    pub fn check(&self) -> Result<()> {
        for file in &self.files {
            file.check()?;
        }
        Ok(())
    }
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

fn render_json(value: &serde_json::Value) -> Result<Vec<u8>> {
    let mut rendered = serde_json::to_vec_pretty(value)?;
    rendered.push(b'\n');
    Ok(rendered)
}

fn render_content(content: &Content) -> Result<Vec<u8>> {
    match content {
        Content::Source { source } => {
            ensure!(
                fs::metadata(source)
                    .with_context(|| format!("failed to inspect source {}", source.display()))?
                    .is_file(),
                "source {} must be a regular file",
                source.display()
            );
            fs::read(source).with_context(|| format!("failed to read source {}", source.display()))
        }
        Content::Text { text } => Ok(text.as_bytes().to_vec()),
        Content::Json { json } => render_json(json),
        Content::Toml { toml } => Ok(render_toml(toml)?.into_bytes()),
    }
}

fn prepare_write(target: &Path, contents: &[u8], mode: u32) -> Result<NamedTempFile> {
    let parent = target.parent().context("target has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(contents)?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    temp.as_file().sync_all()?;
    Ok(temp)
}

fn persist(temp: NamedTempFile, destination: &Destination) -> Result<()> {
    // Atomic no-clobber creation closes the check/create race for new files.
    let result = if destination.before.is_none() {
        temp.persist_noclobber(&destination.resolved)
    } else {
        temp.persist(&destination.resolved)
    };
    match result {
        Ok(_) => Ok(()),
        Err(error) => bail!(
            "failed to write {}: {}",
            destination.requested.display(),
            error.error
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn plan(dir: &Path, mode: &str, backup: bool) -> Plan {
        let manifest_path = dir.join("manifest.json");
        fs::write(&manifest_path, "{}").unwrap();
        let manifest = serde_json::from_value(json!({
            "version": 1,
            "backup_extension": if backup { Some(".bak") } else { None },
            "files": [{"target": dir.join("config"), "mode": mode, "text": "declared"}]
        }))
        .unwrap();
        Plan::prepare(&manifest, &manifest_path).unwrap()
    }

    #[test]
    fn refuses_content_permission_and_inode_changes_after_preparation() {
        for mutation in ["content", "permissions", "inode", "symlink", "removed"] {
            let dir = tempdir().unwrap();
            let target = dir.path().join("config");
            fs::write(&target, "original").unwrap();
            let plan = plan(dir.path(), "replace", true);
            match mutation {
                "content" => fs::write(&target, "concurrent edit").unwrap(),
                "permissions" => {
                    fs::set_permissions(&target, fs::Permissions::from_mode(0o400)).unwrap()
                }
                "inode" => {
                    fs::rename(&target, dir.path().join("old")).unwrap();
                    fs::write(&target, "original").unwrap();
                }
                "symlink" => {
                    fs::remove_file(&target).unwrap();
                    fs::write(dir.path().join("other"), "other").unwrap();
                    symlink("other", &target).unwrap();
                }
                _ => fs::remove_file(&target).unwrap(),
            }
            assert!(plan.files[0].apply().is_err(), "{mutation}");
            assert!(!dir.path().join("config.bak").exists());
            if mutation == "content" {
                assert_eq!(fs::read_to_string(&target).unwrap(), "concurrent edit");
            }
            if mutation == "symlink" {
                assert!(target.is_symlink());
            }
            if mutation == "removed" {
                assert!(!target.exists());
            }
        }
    }

    #[test]
    fn refuses_new_targets_and_backup_changes_after_preparation() {
        for mode in ["seed", "replace", "replace-readonly"] {
            let dir = tempdir().unwrap();
            let plan = plan(dir.path(), mode, true);
            fs::write(dir.path().join("config"), "created concurrently").unwrap();
            assert!(plan.files[0].apply().is_err());
            assert_eq!(
                fs::read_to_string(dir.path().join("config")).unwrap(),
                "created concurrently"
            );
        }
        for existing in [false, true] {
            let dir = tempdir().unwrap();
            fs::write(dir.path().join("config"), "original").unwrap();
            if existing {
                fs::write(dir.path().join("config.bak"), "old backup").unwrap();
            }
            let plan = plan(dir.path(), "replace", true);
            fs::write(dir.path().join("config.bak"), "concurrent backup").unwrap();
            assert!(plan.files[0].apply().is_err());
            assert_eq!(
                fs::read_to_string(dir.path().join("config")).unwrap(),
                "original"
            );
            assert_eq!(
                fs::read_to_string(dir.path().join("config.bak")).unwrap(),
                "concurrent backup"
            );
        }
    }

    #[test]
    fn refuses_parent_symlink_changes_after_preparation() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("a")).unwrap();
        fs::create_dir(dir.path().join("b")).unwrap();
        let link = dir.path().join("link");
        symlink("a", &link).unwrap();
        let plan = plan(&link, "seed", false);
        fs::remove_file(&link).unwrap();
        symlink("b", &link).unwrap();
        assert!(plan.files[0].apply().is_err());
        assert!(!dir.path().join("a/config").exists());
        assert!(!dir.path().join("b/config").exists());
    }

    #[test]
    fn no_clobber_persistence_protects_files_created_after_final_check() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("config");
        let destination = Destination::inspect(target.clone()).unwrap();
        let temp = prepare_write(&target, b"declared", 0o600).unwrap();
        destination.check().unwrap();
        fs::write(&target, "concurrent").unwrap();
        assert!(persist(temp, &destination).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "concurrent");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn competing_plans_cannot_silently_apply_a_stale_snapshot() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("config"), "original").unwrap();
        let first = plan(dir.path(), "replace", true);
        let second = plan(dir.path(), "replace", true);
        first.files[0].apply().unwrap();
        assert!(second.files[0].apply().is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("config.bak")).unwrap(),
            "original"
        );
    }
}
