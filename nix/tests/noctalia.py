"""Exercise the real, pinned Noctalia palette updater around CLI merges."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib

binary, apply_script = sys.argv[1:]
target = Path(os.environ["STARSHIP_CONFIG"])
palette = Path(os.environ["XDG_CACHE_HOME"]) / "noctalia/starship-palette.toml"
palette.parent.mkdir(parents=True, exist_ok=True)
manifest = Path("manifest.json")
manifest.write_text(json.dumps({
    "version": 1,
    "files": [{
        "target": str(target), "mode": "merge", "precedence": "declared",
        "toml": {"add_newline": False, "package": {"disabled": True}},
    }],
}))


def theme(color):
    palette.write_text(f'[palettes.noctalia]\nblue = "{color}"\n')
    subprocess.run(["bash", apply_script], check=True)


def apply():
    subprocess.run([binary, "apply", str(manifest)], check=True)


def verify(color):
    text = target.read_text()
    parsed = tomllib.loads(text)
    assert parsed["add_newline"] is False
    assert parsed["package"]["disabled"] is True
    assert parsed["palette"] == "noctalia"
    assert parsed["palettes"]["noctalia"]["blue"] == color
    begin = "# >>> NOCTALIA STARSHIP PALETTE >>>"
    end = "# <<< NOCTALIA STARSHIP PALETTE <<<"
    assert text.count(begin) == text.count(end) == 1
    block = text.split(begin, 1)[1].split(end, 1)[0]
    assert "[package]" not in block
    assert "add_newline" not in block


# Noctalia creates its config first; managed-files appends a new table after
# the marker, and a subsequent real theme update must keep that table.
theme("#123456")
apply()
verify("#123456")
theme("#abcdef")
verify("#abcdef")
apply()
verify("#abcdef")
theme("#fedcba")
verify("#fedcba")

# Also support the reverse order on a fresh installation.
target.unlink()
apply()
theme("#654321")
verify("#654321")
apply()
verify("#654321")
