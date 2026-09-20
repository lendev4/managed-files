{
  lib,
  rustPlatform,
}:

let
  manifest = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = manifest.package.name;
  version = manifest.package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../LICENSE
      ../README.md
      ../src
      ../tests
    ];
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  meta = {
    description = manifest.package.description;
    homepage = manifest.package.repository;
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "managed-files";
  };
}
