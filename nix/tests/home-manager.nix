{
  pkgs,
  home-manager,
  package,
}:
let
  inherit (pkgs) lib;
  evaluateWith =
    settings: extraModules:
    home-manager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        ../home-manager.nix
        {
          home.username = "test";
          home.homeDirectory = "/build/home";
          home.stateVersion = "26.05";
          managedFiles = {
            inherit package;
          }
          // settings;
        }
      ]
      ++ extraModules;
    };
  evaluate = settings: evaluateWith settings [ ];
  files = {
    "seed" = {
      mode = "seed";
      text = "initial";
    };
    "replace" = {
      mode = "replace";
      source = ./source.txt;
    };
    "readonly" = {
      mode = "replace-readonly";
      text = "fixed";
      backupExtension = null;
    };
    "settings.json" = {
      json = {
        size = 12;
        theme = "dark";
      };
    };
    "settings.toml" = {
      precedence = "declared";
      toml = {
        size = 12;
        theme = "dark";
      };
    };
  };
  enabled = evaluate {
    inherit files;
    backupExtension = ".mfbak";
  };
  disabled = evaluate {
    enable = false;
    files.bad.text = "no mode needed while disabled";
  };
  noBackups = evaluate {
    enable = true;
    backupExtension = null;
    inherit files;
  };
  empty = evaluate { };
  allDisabled = evaluate {
    files.bad = {
      enable = false;
    };
    xdgConfigFiles.bad = {
      enable = false;
      text = "unused";
    };
  };
  defaultXdg = evaluate { xdgConfigFiles."app/config.json".json = { }; };
  ux =
    evaluateWith
      {
        defaults.precedence = "declared";
        files = {
          "declared.json" = {
            backupExtension = ".local-backup";
            json = {
              size = 12;
              added = true;
            };
          };
          "existing.json" = {
            precedence = "existing";
            json = {
              size = 12;
              added = true;
            };
          };
          "replace.json" = {
            mode = "replace";
            json.size = 12;
          };
          "disabled.json" = {
            mode = "seed";
            text = "inherited";
          };
        };
        xdgConfigFiles = {
          "nested/settings.toml".toml = {
            size = 12;
            added = true;
          };
          "nested/settings.toml".backupExtension = ".toml-backup";
          "disabled.toml" = {
            enable = false;
          };
        };
      }
      [
        { xdg.configHome = "/build/custom-config"; }
        { managedFiles.files."disabled.json".enable = false; }
      ];
  rejects = settings: !(builtins.tryEval (evaluate settings).activationPackage.drvPath).success;
  activation = enabled.config.home.activation.managedFiles;
  assertionsPass = settings: lib.all (a: a.assertion) (evaluate settings).config.assertions;
  invalid =
    file:
    !(builtins.tryEval
      (evaluate {
        enable = true;
        files.bad = file;
      }).activationPackage.drvPath
    ).success;
  # Extract the manifest argument from the real activation entry, preserving
  # its store dependency. No duplicate implementation of manifest generation.
  manifestOf =
    evaluated:
    lib.last (
      lib.splitString " " (
        lib.last (
          lib.filter (s: lib.hasInfix "run " s && lib.hasInfix " apply " s) (
            lib.splitString "\n" evaluated.config.home.activation.managedFiles.data
          )
        )
      )
    );
in
assert assertionsPass {
  enable = true;
  inherit files;
};
assert !(disabled.config.home.activation ? managedFiles);
assert !(lib.elem package disabled.config.home.packages);
assert !empty.config.managedFiles.enable;
assert !(empty.config.home.activation ? managedFiles);
assert !(lib.elem package empty.config.home.packages);
assert !allDisabled.config.managedFiles.enable;
assert !(allDisabled.config.home.activation ? managedFiles);
assert enabled.config.managedFiles.enable;
assert defaultXdg.config.managedFiles.enable;
assert ux.config.managedFiles.enable;
assert rejects { files.bad.text = "requires an explicit mode"; };
assert rejects { xdgConfigFiles.bad.source = ./source.txt; };
assert rejects {
  files.".config/duplicate.json".json = { };
  xdgConfigFiles."duplicate.json".json = { };
};
assert rejects {
  files.bad = {
    json = { };
    backupExtension = "";
  };
};
assert rejects {
  xdgConfigFiles.bad = {
    toml = { };
    backupExtension = "/bad";
  };
};
assert lib.elem package enabled.config.home.packages;
assert lib.elem "writeBoundary" activation.after;
assert invalid { mode = "seed"; };
assert invalid {
  mode = "replace";
  text = "a";
  json = { };
};
assert invalid {
  mode = "merge";
  text = "a";
};
assert invalid {
  mode = "merge";
  source = ./source.txt;
};
assert lib.all
  (
    target:
    !(builtins.tryEval
      (evaluate {
        enable = true;
        files.${target} = {
          mode = "seed";
          text = "x";
        };
      }).activationPackage.drvPath
    ).success
  )
  [
    ""
    "/absolute"
    "../escape"
    "a/../escape"
    "a//b"
    "a/./b"
  ];
assert lib.all
  (
    backupExtension:
    !(builtins.tryEval
      (evaluate {
        enable = true;
        inherit backupExtension;
      }).activationPackage.drvPath
    ).success
  )
  [
    ""
    "/backup"
    "\\backup"
  ];
pkgs.runCommand "managed-files-home-manager-test" { nativeBuildInputs = [ pkgs.python3 ]; } ''
  export HOME="$TMPDIR/home"
  mkdir -p "$HOME"
  # The fixture's home is /build/home. Nix sandbox builds use /build, but
  # substitute it in the runner to also support non-sandboxed builds.
  cp ${manifestOf enabled} manifest.json
  cp ${manifestOf noBackups} no-backups.json
  cp ${manifestOf defaultXdg} default-xdg.json
  cp ${manifestOf ux} ux.json
  python3 ${./verify.py} prepare
  verboseEcho() { echo "$@"; }
  run() { "$1" "$2" "$PWD/runtime-manifest.json"; }
  DRY_RUN_CMD=echo
  (
    ${activation.data}
  ) > dry-run.log
  cat dry-run.log
  python3 ${./verify.py} dry-run
  unset DRY_RUN_CMD
  ${activation.data}
  python3 ${./verify.py} verify
  ${activation.data}
  python3 ${./verify.py} repeated
  ${lib.getExe package} apply ux-runtime.json
  python3 ${./verify.py} ux
  touch "$out"
''
