{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    types
    ;
  cfg = config.managedFiles;
  package = pkgs.callPackage ./package.nix { };
  precedenceType = types.enum [
    "existing"
    "declared"
  ];

  fileModule = types.submodule (
    { config, name, ... }: {
      options = {
        enable = mkOption {
          type = types.bool;
          default = true;
          description = "Whether to manage this file. Disabled entries are omitted from validation and application.";
        };

        mode = mkOption {
          type = types.enum [
            "seed"
            "merge"
            "replace"
            "replace-readonly"
          ];
          default =
            if config.json != null || config.toml != null then
              "merge"
            else
              throw "managedFiles entry ${name}: mode must be specified for text or source content.";
          defaultText = lib.literalExpression ''"merge" for json/toml; required for text/source'';
          description = "How managed-files should manage this file.";
        };

        source = mkOption {
          type = types.nullOr types.path;
          default = null;
          description = "Source file to use as the file contents.";
        };

        text = mkOption {
          type = types.nullOr types.lines;
          default = null;
          description = "Literal text contents.";
        };

        json = mkOption {
          type = types.nullOr types.attrs;
          default = null;
          description = "Structured JSON contents. Defaults to merge mode.";
        };

        toml = mkOption {
          type = types.nullOr types.attrs;
          default = null;
          description = "Structured TOML contents. Defaults to merge mode.";
        };

        precedence = mkOption {
          type = precedenceType;
          default = cfg.defaults.precedence;
          defaultText = lib.literalExpression "config.managedFiles.defaults.precedence";
          description = "Which side wins when merge encounters conflicting values. Existing-only keys survive either choice.";
        };

        backupExtension = mkOption {
          type = types.nullOr types.str;
          default = cfg.backupExtension;
          defaultText = lib.literalExpression "config.managedFiles.backupExtension";
          description = "Backup suffix for this file, overriding the global default. Set null to disable backups. Seed mode never creates backups.";
        };
      };
    }
  );

  mkEntries =
    namespace: base: files:
    lib.mapAttrsToList (name: file: {
      inherit name file;
      option = "managedFiles.${namespace}.${name}";
      target = "${lib.removeSuffix "/" base}/${name}";
    }) (lib.filterAttrs (_: file: file.enable) files);

  entries =
    mkEntries "files" config.home.homeDirectory cfg.files
    ++ mkEntries "xdgConfigFiles" config.xdg.configHome cfg.xdgConfigFiles;

  contentCount =
    file:
    builtins.length (
      builtins.filter (value: value != null) [
        file.source
        file.text
        file.json
        file.toml
      ]
    );

  fileAssertions = lib.concatMap (
    {
      name,
      file,
      option,
      ...
    }:
    [
      {
        assertion =
          name != "" && lib.all (part: part != "" && part != "." && part != "..") (lib.splitString "/" name);
        message = "${option}: use a relative file path without empty, . or .. components.";
      }
      {
        assertion = contentCount file == 1;
        message = "${option} must define exactly one of source, text, json, or toml.";
      }
      {
        assertion = file.mode != "merge" || file.json != null || file.toml != null;
        message = "${option}: merge mode only supports json or toml content.";
      }
      {
        assertion = validBackupExtension file.backupExtension;
        message = "${option}.backupExtension must be null or a nonempty filename suffix without separators.";
      }
    ]
  ) entries;

  mkManifestFile =
    {
      target,
      file,
      option,
      ...
    }:
    let
      content =
        if file.source != null then
          { source = "${file.source}"; }
        else if file.text != null then
          { text = file.text; }
        else if file.json != null then
          { json = file.json; }
        else if file.toml != null then
          { toml = file.toml; }
        else
          throw "${option}: no content configured";
    in
    {
      inherit target;
      inherit (file) mode precedence;
      backup_extension = file.backupExtension;
    }
    // content;

  manifestFile = pkgs.writeText "managed-files-manifest.json" (
    builtins.toJSON {
      version = 1;
      backup_extension = cfg.backupExtension;
      files = map mkManifestFile entries;
    }
  );

  validBackupExtension =
    extension:
    extension == null
    || (extension != "" && !(lib.hasInfix "/" extension) && !(lib.hasInfix "\\" extension));
in
{
  options.managedFiles = {
    enable = mkEnableOption "mutable managed files" // {
      default = entries != [ ];
      defaultText = lib.literalExpression "true when at least one file is enabled";
      description = "Whether to manage files. Enabled automatically when files are configured; set false to disable all management.";
    };

    defaults.precedence = mkOption {
      type = precedenceType;
      default = "existing";
      description = "Default merge precedence for both files and xdgConfigFiles. Individual entries may override it.";
    };

    backupExtension = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Default backup suffix for all files. Backups are disabled by default. Each file may override this setting.";
    };

    package = mkOption {
      type = types.package;
      default = package;
      description = "managed-files package to use.";
    };

    files = mkOption {
      type = types.attrsOf fileModule;
      default = { };
      description = "Files to manage, with names relative to the home directory.";
    };

    xdgConfigFiles = mkOption {
      type = types.attrsOf fileModule;
      default = { };
      description = "Files to manage, with names relative to xdg.configHome (usually ~/.config).";
    };
  };

  config = mkIf cfg.enable {
    assertions = fileAssertions ++ [
      {
        assertion = validBackupExtension cfg.backupExtension;
        message = "managedFiles.backupExtension must be null or a nonempty filename suffix without separators.";
      }
      {
        assertion =
          builtins.length (lib.unique (map (entry: entry.target) entries)) == builtins.length entries;
        message = "managedFiles.files and managedFiles.xdgConfigFiles must not manage the same target.";
      }
    ];

    home.packages = [ cfg.package ];

    home.activation.managedFiles = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
      verboseEcho "Applying managed mutable files"
      if [[ -n "''${DRY_RUN_CMD:-}" ]]; then
        ${lib.getExe cfg.package} apply ${manifestFile} --dry-run
      else
        run ${lib.getExe cfg.package} apply ${manifestFile}
      fi
    '';
  };
}
