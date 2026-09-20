use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::{TempDir, tempdir};

fn apply(dir: &TempDir, files: Value, backup: Value) -> Output {
    let manifest = dir.path().join("manifest.json");
    fs::write(
        &manifest,
        json!({"version": 1, "backup_extension": backup, "files": files}).to_string(),
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_managed-files"))
        .current_dir(dir.path())
        .arg("apply")
        .arg(manifest)
        .output()
        .unwrap()
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn modes_backups_and_repeated_application() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source");
    fs::write(&source, b"source\0bytes").unwrap();
    let files = json!([
        {"target":"nested/seed", "mode":"seed", "text":"initial"},
        {"target":"replace", "mode":"replace", "source":source},
        {"target":"readonly", "mode":"replace-readonly", "text":"fixed"}
    ]);
    success(apply(&dir, files.clone(), json!(".bak")));
    fs::write(dir.path().join("nested/seed"), "edited").unwrap();
    fs::write(dir.path().join("replace"), "edited").unwrap();
    success(apply(&dir, files.clone(), json!(".bak")));
    success(apply(&dir, files.clone(), json!(".bak")));
    success(apply(&dir, files, json!(null)));
    assert_eq!(
        fs::read_to_string(dir.path().join("nested/seed")).unwrap(),
        "edited"
    );
    assert!(!dir.path().join("nested/seed.bak").exists());
    assert_eq!(
        fs::read(dir.path().join("replace")).unwrap(),
        b"source\0bytes"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("replace.bak")).unwrap(),
        "source\0bytes"
    );
    assert!(
        fs::metadata(dir.path().join("readonly"))
            .unwrap()
            .permissions()
            .readonly()
    );
    assert!(
        !fs::metadata(dir.path().join("replace"))
            .unwrap()
            .permissions()
            .readonly()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(dir.path().join("replace"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}

#[test]
fn structured_merges_cover_precedence_and_creation() {
    for precedence in ["existing", "declared"] {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("config.json"),
            r#"{"editor":{"size":18,"local":true},"items":[2]}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "# keep me\n[editor]\nsize = 18\nlocal = true\n",
        )
        .unwrap();
        let declared = json!({"editor":{"size":12,"theme":"dark"},"items":[1]});
        success(apply(
            &dir,
            json!([
                {"target":"config.json","mode":"merge","precedence":precedence,"json":declared},
                {"target":"config.toml","mode":"merge","precedence":precedence,"toml":declared},
                {"target":"new.json","mode":"merge","json":declared},
                {"target":"new.toml","mode":"merge","toml":declared}
            ]),
            json!(".bak"),
        ));
        let value: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("config.json")).unwrap())
                .unwrap();
        assert_eq!(
            value["editor"]["size"],
            if precedence == "existing" { 18 } else { 12 }
        );
        assert_eq!(
            value["items"],
            if precedence == "existing" {
                json!([2])
            } else {
                json!([1])
            }
        );
        assert_eq!(value["editor"]["local"], true);
        assert_eq!(value["editor"]["theme"], "dark");
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        let value: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(
            value["editor"]["size"].as_integer(),
            Some(if precedence == "existing" { 18 } else { 12 })
        );
        assert!(text.contains("# keep me"));
        assert!(value["editor"]["local"].as_bool().unwrap());
        assert!(dir.path().join("config.json.bak").exists());
        assert!(dir.path().join("config.toml.bak").exists());
        assert!(!dir.path().join("new.json.bak").exists());
        assert_eq!(
            serde_json::from_str::<Value>(
                &fs::read_to_string(dir.path().join("new.json")).unwrap()
            )
            .unwrap(),
            declared
        );
        assert_eq!(
            toml::from_str::<toml::Value>(
                &fs::read_to_string(dir.path().join("new.toml")).unwrap()
            )
            .unwrap()["editor"]["size"]
                .as_integer(),
            Some(12)
        );
    }
}

#[test]
fn malformed_existing_files_are_preserved() {
    for (name, content) in [
        ("bad.json", json!({"json":{}})),
        ("bad.toml", json!({"toml":{}})),
    ] {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(name), "[invalid").unwrap();
        let mut file = json!({"target":name,"mode":"merge"});
        file.as_object_mut()
            .unwrap()
            .extend(content.as_object().unwrap().clone());
        let output = apply(&dir, json!([file]), json!(".bak"));
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(name));
        assert_eq!(
            fs::read_to_string(dir.path().join(name)).unwrap(),
            "[invalid"
        );
        assert!(!dir.path().join(format!("{name}.bak")).exists());
    }
}

#[test]
fn invalid_manifests_fail_without_writing() {
    for manifest in [
        json!({"version":2,"files":[{"target":"out","mode":"replace","text":"bad"}]}).to_string(),
        json!({"version":1,"files":[{"target":"out","mode":"unknown","text":"bad"}]}).to_string(),
        json!({"version":1,"files":[{"target":"out","mode":"merge","text":"bad"}]}).to_string(),
        "not json".into(),
    ] {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("manifest.json"), manifest).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_managed-files"))
            .current_dir(dir.path())
            .args(["apply", "manifest.json"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
        assert!(!dir.path().join("out").exists());
    }
}

fn failure(output: Output, expected: &str) {
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "preflight must not report applied files"
    );
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains(expected), "expected {expected:?} in {error}");
}

#[test]
fn entire_manifest_is_validated_before_any_mutation() {
    for (bad, message) in [
        (
            json!({"target":"bad","mode":"replace","text":"a","json":{}}),
            "exactly one",
        ),
        (
            json!({"target":"bad","mode":"replace","text":null,"json":{}}),
            "exactly one",
        ),
        (
            json!({"target":"bad","mode":"replace","text":"a","typo":true}),
            "unknown field",
        ),
        (json!({"target":"bad","mode":"replace"}), "exactly one"),
        (
            json!({"target":"bad","mode":"merge","text":"a"}),
            "only supports",
        ),
        (
            json!({"target":"bad","mode":"merge","json":[]}),
            "must be an object",
        ),
        (
            json!({"target":"bad","mode":"replace","toml":{"bad":null}}),
            "TOML",
        ),
        (
            json!({"target":"bad","mode":"replace","source":"missing"}),
            "source",
        ),
        (
            json!({"target":"bad","mode":"merge","json":{}}),
            "existing JSON",
        ),
        (
            json!({"target":"bad","mode":"merge","toml":{}}),
            "existing TOML",
        ),
        (
            json!({"target":"../escape","mode":"replace","text":"x"}),
            "parent traversal",
        ),
    ] {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("bad"), "[invalid").unwrap();
        fs::write(dir.path().join("existing"), "original").unwrap();
        failure(
            apply(
                &dir,
                json!([
                    {"target":"new/first","mode":"replace","text":"new"},
                    {"target":"existing","mode":"replace","text":"changed"},
                    bad
                ]),
                json!(".bak"),
            ),
            message,
        );
        assert!(!dir.path().join("new").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("existing")).unwrap(),
            "original"
        );
        assert!(!dir.path().join("existing.bak").exists());
    }
}

#[test]
fn invalid_backup_suffixes_and_overlapping_paths_are_rejected() {
    for suffix in ["", "/backup", "../backup", "\\backup", "\0"] {
        let dir = tempdir().unwrap();
        failure(
            apply(
                &dir,
                json!([{"target":"new/config","mode":"replace","text":"x"}]),
                json!(suffix),
            ),
            "backup_extension",
        );
        assert!(!dir.path().join("new").exists());
    }
    for second in [
        "config",
        "./config",
        "config/child",
        "config.bak",
        "config.bak/child",
    ] {
        let dir = tempdir().unwrap();
        failure(
            apply(
                &dir,
                json!([
                    {"target":"config","mode":"replace","text":"x"},
                    {"target":second,"mode":"replace","text":"y"}
                ]),
                json!(".bak"),
            ),
            "conflicting destinations",
        );
        assert!(!dir.path().join("config").exists());
    }
}

#[test]
fn inputs_cannot_be_overwritten_by_targets_or_backups() {
    for target in ["manifest.json", "source", "source-parent"] {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("source-parent")).unwrap();
        fs::write(dir.path().join("source-parent/data"), "source").unwrap();
        fs::write(dir.path().join("source"), "source").unwrap();
        let output = apply(
            &dir,
            json!([
                {"target":"out","mode":"replace","source":"source"},
                {"target":"other","mode":"replace","source":"source-parent/data"},
                {"target":target,"mode":"replace","text":"x"}
            ]),
            json!(null),
        );
        assert!(!output.status.success());
        assert!(!dir.path().join("out").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("source")).unwrap(),
            "source"
        );
    }
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("config.bak"), "source").unwrap();
    failure(
        apply(
            &dir,
            json!([{"target":"config","mode":"replace","source":"config.bak"}]),
            json!(".bak"),
        ),
        "conflicts with input",
    );
}

#[test]
fn dry_run_reports_actions_and_backups_without_changes() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("seed"), "user").unwrap();
    fs::write(dir.path().join("replace"), "old").unwrap();
    fs::write(dir.path().join("merge"), "{}").unwrap();
    let files = json!([
        {"target":"seed","mode":"seed","text":"default"},
        {"target":"replace","mode":"replace","text":"new"},
        {"target":"merge","mode":"merge","json":{"key":true}},
        {"target":"new/readonly","mode":"replace-readonly","text":"fixed"}
    ]);
    let manifest = json!({"version":1,"backup_extension":".bak","files":files});
    fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
    let before = fs::metadata(dir.path().join("replace")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_managed-files"))
        .current_dir(dir.path())
        .args(["apply", "--dry-run", "manifest.json"])
        .output()
        .unwrap();
    let report = String::from_utf8(output.stdout.clone()).unwrap();
    success(output);
    assert_eq!(
        report,
        "skipped seed\nwould replace replace (backup: replace.bak)\nwould merge merge (backup: merge.bak)\nwould create new/readonly\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("replace")).unwrap(),
        "old"
    );
    let after = fs::metadata(dir.path().join("replace")).unwrap();
    assert_eq!(
        (before.ino(), before.mode(), before.mtime_nsec()),
        (after.ino(), after.mode(), after.mtime_nsec())
    );
    assert!(!dir.path().join("new").exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
    let output = apply(&dir, files, json!(".bak"));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "skipped seed\nreplaced replace (backup: replace.bak)\nmerged merge (backup: merge.bak)\ncreated new/readonly\n"
    );
    success(output);
}

#[test]
fn dry_run_rejects_invalid_existing_content() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("config"), "invalid").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        json!({"version":1,"files":[
            {"target":"new/config","mode":"replace","text":"x"},
            {"target":"config","mode":"merge","json":{}}
        ]})
        .to_string(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_managed-files"))
        .current_dir(dir.path())
        .args(["apply", "manifest.json", "--dry-run"])
        .output()
        .unwrap();
    failure(output, "existing JSON");
    assert!(!dir.path().join("new").exists());
}

#[test]
fn permissions_are_preserved_with_explicit_mode_adjustments() {
    use std::os::unix::fs::PermissionsExt;
    for (mode, initial, expected) in [
        ("seed", 0o640, 0o640),
        ("merge", 0o440, 0o440),
        ("replace", 0o440, 0o640),
        ("replace", 0o750, 0o750),
        ("replace-readonly", 0o764, 0o544),
    ] {
        let dir = tempdir().unwrap();
        let target = dir.path().join("config");
        fs::write(&target, "{}").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(initial)).unwrap();
        success(apply(
            &dir,
            json!([{"target":"config","mode":mode,"json":{"a":true}}]),
            json!(".bak"),
        ));
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            expected,
            "{mode}"
        );
        if mode != "seed" {
            assert_eq!(
                fs::metadata(dir.path().join("config.bak"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn symlinks_and_hard_links_are_rejected_but_parent_symlinks_work() {
    use std::os::unix::fs::symlink;
    for mode in ["seed", "merge", "replace", "replace-readonly"] {
        for kind in ["symlink", "dangling", "hardlink", "directory"] {
            let dir = tempdir().unwrap();
            fs::write(dir.path().join("real"), "{}").unwrap();
            let target = dir.path().join("config");
            match kind {
                "symlink" => symlink("real", &target).unwrap(),
                "dangling" => symlink("missing", &target).unwrap(),
                "hardlink" => fs::hard_link(dir.path().join("real"), &target).unwrap(),
                _ => fs::create_dir(&target).unwrap(),
            }
            let output = apply(
                &dir,
                json!([{"target":"config","mode":mode,"json":{}}]),
                json!(".bak"),
            );
            assert!(!output.status.success(), "{mode}: {kind}");
            assert_eq!(fs::read_to_string(dir.path().join("real")).unwrap(), "{}");
            assert!(!dir.path().join("config.bak").exists());
        }
    }
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("real")).unwrap();
    symlink("real", dir.path().join("link")).unwrap();
    failure(
        apply(
            &dir,
            json!([
                {"target":"real/nested/config","mode":"replace","text":"x"},
                {"target":"link/nested/config","mode":"replace","text":"y"}
            ]),
            json!(null),
        ),
        "conflicting destinations",
    );
    assert!(!dir.path().join("real/nested").exists());
    success(apply(
        &dir,
        json!([{"target":"link/nested/config","mode":"seed","text":"x"}]),
        json!(null),
    ));
    assert_eq!(
        fs::read_to_string(dir.path().join("real/nested/config")).unwrap(),
        "x"
    );
}

#[test]
fn unsafe_backup_destinations_fail_before_replacing_files() {
    use std::os::unix::fs::symlink;
    for kind in ["symlink", "dangling", "hardlink", "directory"] {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("config"), "old").unwrap();
        fs::write(dir.path().join("other"), "other").unwrap();
        let backup = dir.path().join("config.bak");
        match kind {
            "symlink" => symlink("other", &backup).unwrap(),
            "dangling" => symlink("missing", &backup).unwrap(),
            "hardlink" => fs::hard_link(dir.path().join("other"), &backup).unwrap(),
            _ => fs::create_dir(&backup).unwrap(),
        }
        let output = apply(
            &dir,
            json!([{"target":"config","mode":"replace","text":"new"}]),
            json!(".bak"),
        );
        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(dir.path().join("config")).unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("other")).unwrap(),
            "other"
        );
    }
}

#[test]
fn strict_parsing_rejects_unknown_and_duplicate_fields_but_accepts_json_null() {
    for manifest in [
        r#"{"version":1,"files":[],"typo":true}"#,
        r#"{"version":1,"version":1,"files":[]}"#,
        r#"{"version":1,"files":[{"target":"out","mode":"replace","text":"a","text":"b"}]}"#,
        r#"{"version":1,"files":[{"target":"out","mode":"replace","text":null}]}"#,
    ] {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("manifest.json"), manifest).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_managed-files"))
            .current_dir(dir.path())
            .args(["apply", "manifest.json"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!dir.path().join("out").exists());
    }
    let dir = tempdir().unwrap();
    success(apply(
        &dir,
        json!([{"target":"out","mode":"replace","json":null}]),
        json!(null),
    ));
    assert_eq!(
        fs::read_to_string(dir.path().join("out")).unwrap(),
        "null\n"
    );
}

#[test]
fn target_paths_must_name_files_and_source_symlinks_are_supported() {
    use std::os::unix::fs::symlink;
    for target in ["", ".", "/", "new/"] {
        let dir = tempdir().unwrap();
        let output = apply(
            &dir,
            json!([
                {"target":"first","mode":"seed","text":"x"},
                {"target":target,"mode":"seed","text":"x"}
            ]),
            json!(null),
        );
        assert!(!output.status.success(), "{target:?}");
        assert!(!dir.path().join("first").exists());
    }
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("source"), "contents").unwrap();
    symlink("source", dir.path().join("source-link")).unwrap();
    success(apply(
        &dir,
        json!([{"target":"out","mode":"replace","source":"source-link"}]),
        json!(null),
    ));
    assert_eq!(
        fs::read_to_string(dir.path().join("out")).unwrap(),
        "contents"
    );
}
