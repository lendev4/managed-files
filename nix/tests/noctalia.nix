{ pkgs, package }:
let
  applyScript = pkgs.fetchurl {
    url = "https://raw.githubusercontent.com/noctalia-dev/noctalia/960d4d46bd3149a556504e7c40ecd87abb251a36/assets/templates/starship/apply.sh";
    hash = "sha256-OPwKP4PUs00RdYaNojg0YP4TFRugxPnpJy31fWO2LT8=";
  };
in
pkgs.runCommand "managed-files-noctalia-test"
  {
    nativeBuildInputs = [
      pkgs.python3
      pkgs.bash
      pkgs.gawk
      pkgs.gnused
      pkgs.gnugrep
    ];
  }
  ''
    export HOME="$TMPDIR/home"
    export XDG_CONFIG_HOME="$HOME/.config"
    export XDG_CACHE_HOME="$HOME/.cache"
    export STARSHIP_CONFIG="$XDG_CONFIG_HOME/starship.toml"
    python3 ${./noctalia.py} ${pkgs.lib.getExe package} ${applyScript}
    touch "$out"
  ''
