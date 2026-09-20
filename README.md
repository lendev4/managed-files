# managed-files

Manage mutable configuration files from Home Manager or a JSON manifest.
Files are written atomically as regular files, with parent directories created
as needed.

| Mode | On application |
| --- | --- |
| `seed` | Create a missing file; leave an existing file untouched. |
| `merge` | Recursively merge structured JSON or TOML into existing content, or create the file. |
| `replace` | Write the declared content every time, leaving the file writable by its owner. |
| `replace-readonly` | Write the declared content every time, then remove write permissions. |

JSON is pretty-printed; TOML merges standard and
inline tables recursively, preserves comments on untouched content, and appends
new table sections after existing footer comments. JSON merge requires objects
on both sides; TOML content must
be a table. Malformed existing JSON/TOML stops preparation before any files are
written.

## Merge precedence

`precedence` decides which value wins when the file on disk and your declaration
set the same key differently. It only affects `merge` mode.
**Existing** means the current content on disk, including edits made by you or
the application. **Declared** means the `json` or `toml` content you specify in
Home Manager or the CLI manifest.

| Precedence | When both sides set the same key | Use it to… |
| --- | --- | --- |
| `"existing"` (default) | Keep the value already on disk. | Supply defaults while allowing the application or user to change them. |
| `"declared"` | Write the value from your declaration. | Enforce selected settings each time managed-files runs. |

For example, suppose the file on disk contains:

```toml
theme = "light"
font_size = 18
```

And you declare:

```nix
managedFiles.xdgConfigFiles."my-app/settings.toml" = {
  precedence = "declared"; # or "existing", which is the default
  toml = {
    theme = "dark";
    autosave = true;
  };
};
```

The resulting file contains:

| Setting | With `"existing"` | With `"declared"` |
| --- | --- | --- |
| `theme` (both sides) | `"light"` | `"dark"` |
| `font_size` (only on disk) | `18` | `18` |
| `autosave` (only declared) | `true` | `true` |

With `"existing"`, later changes to your declared `theme` also leave the on-disk
value alone. With `"declared"`, each run restores your declared `theme` if it was
changed on disk. This happens when managed-files runs, not continuously.

Objects and tables merge recursively, so precedence applies to individual
nested settings too. Arrays are kept or replaced as a whole, never combined.
If one side has a table/object and the other has a different type, the winning
side supplies the whole value at that key.

Keys found only on disk survive within merged objects/tables, even with
`"declared"`; use `replace` to make the entire file match your declaration.
Removing a key from your declaration does not delete it from disk. If the file
is missing, either precedence creates it from your declared content.

## Home Manager

Add the flake input:

```nix
inputs = {
  managed-files = {
    url = "github:lendev4/managed-files";
    inputs.nixpkgs.follows = "nixpkgs";
  };
};
```

Import the Home Manager module:

```nix
imports = [ inputs.managed-files.homeModules.default ];
```

### Declare files

Declare the files to manage:

```nix
{
  managedFiles = {
    xdgConfigFiles = {
      "my-app/notes.txt" = {
        mode = "seed";
        text = "Edit me!\n";
      };
      "my-app/settings.json" = {
        json = { theme = "dark"; editor.fontSize = 14; };
      };
      "my-app/settings.toml" = {
        precedence = "declared";
        backupExtension = ".mfbak"; # opt in for this file only
        toml = { editor.autosave = true; };
      };
      "my-app/template.txt" = {
        mode = "replace";
        source = ./template.txt;
      };
      "my-app/policy.txt" = {
        mode = "replace-readonly";
        text = "Managed by Home Manager\n";
      };
    };
  };
}
```

Use `managedFiles.files` for paths relative to your home directory and
`managedFiles.xdgConfigFiles` for paths relative to `config.xdg.configHome`
(usually `~/.config`). Both collections use the same options. Each enabled entry
must specify exactly one of `source`, `text`, `json`, or `toml`.

- JSON and TOML entries default to `mode = "merge"`; other modes can still be
  selected explicitly. Text and source entries require an explicit mode.
- The module enables automatically when at least one entry is enabled. Set
  `managedFiles.enable = false` to disable all management. No files means no
  package installation or activation by default.
- Each entry supports `enable = false`, including declarations inherited from
  other modules. Disabled entries are omitted from validation and application;
  disabling an entry does not delete its existing file.
- [Merge precedence](#merge-precedence) defaults to `"existing"`. Set
  `managedFiles.defaults.precedence = "declared"` to change the default for both
  collections; an entry's own `precedence` overrides it.

For example, disable an inherited declaration with
`managedFiles.xdgConfigFiles."my-app/settings.json".enable = false;`.
Declaring the same target through both collections is an error.

Backups are disabled by default. Opt in per file with `backupExtension`:

```nix
managedFiles.xdgConfigFiles."my-app/settings.toml" = {
  backupExtension = ".mfbak";
  toml.editor.autosave = true;
};
```

To enable backups globally, set `managedFiles.backupExtension = ".mfbak";`.
Individual files can choose another suffix or set `backupExtension = null;`
to disable backups. This works for both `files` and `xdgConfigFiles`.
Existing backups are not deleted when backups are disabled.

Paths cannot contain empty, `.` or `..` components. Activation runs after Home
Manager's write boundary. Home Manager dry runs execute the CLI preview, so you
can see individual planned changes. `managedFiles.package` can override the
executable package.

### Example: Starship with Noctalia theme colors

Noctalia's [Starship integration](https://github.com/noctalia-dev/noctalia/blob/960d4d46bd3149a556504e7c40ecd87abb251a36/assets/templates/starship/apply.sh)
updates a marked palette block and selects the `noctalia` palette while
preserving the rest of the config. With that integration enabled, managed-files
can manage your prompt settings in the same writable file:

```nix
{
  # Install Starship and enable shell integration without generating its config.
  programs.starship.enable = true;

  managedFiles.xdgConfigFiles."starship.toml" = {
    precedence = "declared";
    toml = {
      add_newline = false;
      command_timeout = 1000;
      package.disabled = true;
    };
  };
}
```

Home Manager activation applies your declared settings while retaining the
palette. Noctalia theme changes update the palette while retaining your
settings. No custom activation script or additional remerge hook is needed.

Leave `palette` and `palettes.noctalia` to Noctalia, and leave Home Manager's
`programs.starship.settings` and `programs.starship.presets` unset so they do
not generate a competing store-backed config. This example assumes the default
`~/.config/starship.toml` location; if you use a custom location, point both
tools at the same file. The XDG shorthand follows `config.xdg.configHome`.
When migrating an existing store-backed config, replace
its symlink with a writable regular file before using managed-files.

## CLI

Save this as `manifest.json` and run `managed-files apply manifest.json`
(or `nix run github:lendev4/managed-files -- apply manifest.json` without
installing it; use `nix run . -- apply manifest.json` from this repository):

```json
{
  "version": 1,
  "files": [
    { "target": "config/notes.txt", "mode": "seed", "text": "Edit me!\n" },
    { "target": "config/settings.json", "mode": "merge", "backup_extension": ".mfbak", "json": { "theme": "dark" } },
    { "target": "config/template.txt", "mode": "replace", "text": "Template\n" },
    { "target": "config/policy.txt", "mode": "replace-readonly", "text": "Managed\n" }
  ]
}
```

The defaults above are Home Manager conveniences: CLI manifest entries still
require `mode`, and omitted `precedence` means `"existing"`.
CLI paths are relative to the current working directory or absolute. Backups
are disabled by default. A top-level `backup_extension` sets the default for
all files. Each file can override it with its own `backup_extension`: omit the
field to inherit, use a suffix string to enable, or `null` to disable.
When enabled, merge and
replace modes copy the previous contents to `<target><extension>` before
writing, overwriting an existing **regular-file** backup. Seed never creates a
backup. The extension must be nonempty and contain no path separators.

Preview the same plan without writing files, backups, or directories:

```console
$ managed-files apply manifest.json --dry-run
would create config/notes.txt
would merge config/settings.json (backup: config/settings.json.mfbak)
would replace config/template.txt
would create config/policy.txt
```

Output depends on which files already exist. Normal application reports
`created`, `merged`, `replaced`, or `skipped` after each successful action.
Previews validate and render the full manifest too, and return a nonzero exit
status on errors. A preview does not guarantee that later writes will succeed.

## Filesystem behavior

- **Validate first:** unknown or duplicate entry fields, ambiguous content,
  invalid modes, missing sources, unrenderable content, and malformed merge
  files fail before application starts. Even skipped seed declarations must be
  valid. Only manifest version 1 is accepted.
- **Paths:** duplicate or overlapping targets, backup paths, and inputs are
  rejected, including aliases through parent-directory symlinks. Backup names
  are reserved even for currently missing targets. CLI targets cannot contain
  `..`. Neither the manifest nor a source can also be a destination.
- **Links:** targets and backups must be regular files with one hard link.
  Symlinks, including dangling links, are rejected even for seed. Parent
  directory symlinks and source symlinks are supported.
- **Permissions:** new files and backups start at `0600`; new read-only files
  use `0400`. Rewrites preserve existing rwx bits, with `replace` adding owner
  write and `replace-readonly` removing all write bits. Seed leaves existing
  files untouched. Atomic replacement creates a new inode owned by the running
  user; special mode bits, ACLs, and extended attributes are not preserved.
- **Concurrent edits:** application checks content, file identity, permissions,
  timestamps, and resolved parent paths against the prepared plan before
  writing. A detected change stops application. Creating new targets/backups
  uses atomic no-clobber persistence, so files created concurrently survive.
  Existing-file replacement still has a small check-to-rename race: this is
  not a lock shared with applications or a security boundary against hostile
  filesystem changes. Close applications that actively write these files
  when you need guaranteed coordination.

The full plan is held in memory. Application is atomic per file, not a
transaction across the manifest: runtime errors stop further writes but do
not roll back earlier files. A backup may have been updated before a later
target-write failure. Backups contain the pre-application contents and are
replaced atomically with owner-only permissions.

## Development

Use `nix develop`, then `cargo test` and `cargo clippy --all-targets -- -D warnings`.
`nix flake check` builds the package, runs the Rust tests, and evaluates the real
Home Manager module, checks its assertions and generated manifests, and applies
its dry-run activation and repeated application in an isolated build.
It also runs the pinned Noctalia Starship updater before and after merges to
verify that theme updates preserve custom prompt settings.
`nix fmt` formats Nix files. While new files are untracked, use
`nix flake check path:.` to include them.
The flake exposes `packages.default`, `apps.default`, `devShells.default`, and
`homeModules.default`, with Linux packages for x86_64 and aarch64.

## License

[MIT](LICENSE).
