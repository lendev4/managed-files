import json
import os
from pathlib import Path
import sys
import tomllib

home = Path(os.environ["HOME"])
phase = sys.argv[1]
if phase == "prepare":
    manifest = json.loads(Path("manifest.json").read_text())
    assert manifest["version"] == 1
    assert manifest["backup_extension"] == ".mfbak"
    no_backups = json.loads(Path("no-backups.json").read_text())
    assert no_backups["backup_extension"] is None
    assert no_backups["files"] == manifest["files"]
    files = {Path(f["target"]).name: f for f in manifest["files"]}
    assert set(files) == {"seed", "replace", "readonly", "settings.json", "settings.toml"}
    assert files["seed"]["text"] == "initial"
    assert files["replace"]["source"].startswith("/nix/store/")
    assert Path(files["replace"]["source"]).read_text() == "from source\n"
    assert files["readonly"]["mode"] == "replace-readonly"
    assert files["settings.json"]["json"] == {"size": 12, "theme": "dark"}
    assert files["settings.json"]["precedence"] == "existing"
    assert files["settings.json"]["mode"] == "merge"
    assert files["settings.toml"]["toml"] == {"size": 12, "theme": "dark"}
    assert files["settings.toml"]["precedence"] == "declared"
    assert files["settings.toml"]["mode"] == "merge"
    default_xdg = json.loads(Path("default-xdg.json").read_text())
    assert default_xdg["files"] == [{
        "target": "/build/home/.config/app/config.json",
        "mode": "merge", "precedence": "existing", "json": {},
    }]
    ux = json.loads(Path("ux.json").read_text())
    ux_files = {f["target"]: f for f in ux["files"]}
    assert set(ux_files) == {
        "/build/home/declared.json", "/build/home/existing.json",
        "/build/home/replace.json", "/build/custom-config/nested/settings.toml",
    }
    for file in ux["files"]:
        assert file["mode"] == ("replace" if file["target"].endswith("replace.json") else "merge")
        assert file["precedence"] == ("existing" if file["target"].endswith("existing.json") else "declared")
        # Preserve the hierarchy and keep custom XDG paths separate from home.
        target = home / "ux" / Path(file["target"]).relative_to("/build")
        file["target"] = str(target)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("size = 18\nlocal = true\n" if target.suffix == ".toml" else '{"size":18,"local":true}')
    Path("ux-runtime.json").write_text(json.dumps(ux))
    for file in manifest["files"]:
        assert file["target"].startswith("/build/home/")
        file["target"] = str(home / Path(file["target"]).name)
    Path("runtime-manifest.json").write_text(json.dumps(manifest))
    (home / "replace").write_text("old")
    (home / "settings.json").write_text('{"size":18,"local":true}')
    (home / "settings.toml").write_text('# preserved\nsize = 18\nlocal = true\n')
elif phase == "dry-run":
    preview = Path("dry-run.log").read_text()
    assert "would create /build/home/seed" in preview
    assert "would create /build/home/readonly" in preview
    assert (home / "replace").read_text() == "old"
    assert (home / "settings.json").read_text() == '{"size":18,"local":true}'
    assert not (home / "seed").exists()
    assert not (home / "readonly").exists()
    assert not list(home.glob("*.mfbak"))
elif phase == "ux":
    ux_home = home / "ux/home"
    assert json.loads((ux_home / "declared.json").read_text()) == {"size":12,"added":True,"local":True}
    assert json.loads((ux_home / "existing.json").read_text()) == {"size":18,"added":True,"local":True}
    assert json.loads((ux_home / "replace.json").read_text()) == {"size":12}
    assert tomllib.loads((home / "ux/custom-config/nested/settings.toml").read_text()) == {"size":12,"added":True,"local":True}
    assert not list((home / "ux").rglob("disabled.*"))
else:
    assert (home / "seed").read_text() == ("initial" if phase == "verify" else "edited")
    assert (home / "replace").read_text() == "from source\n"
    assert (home / "replace.mfbak").read_text() == ("old" if phase == "verify" else "from source\n")
    assert (home / "readonly").read_text() == "fixed"
    assert (home / "readonly").stat().st_mode & 0o222 == 0
    assert (home / "replace").stat().st_mode & 0o200
    assert json.loads((home / "settings.json").read_text()) == {"size":18,"theme":"dark","local":True}
    text = (home / "settings.toml").read_text()
    assert tomllib.loads(text) == {"size":12,"theme":"dark","local":True}
    assert "# preserved" in text
    (home / "seed").write_text("edited")
