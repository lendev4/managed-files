{
  description = "Declarative management of mutable configuration files";

  inputs = {
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    {
      self,
      nixpkgs,
      home-manager,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forAllSystems = nixpkgs.lib.genAttrs systems;

      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          managed-files = pkgs.callPackage ./nix/package.nix { };
          default = self.packages.${system}.managed-files;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          meta.description = self.packages.${system}.managed-files.meta.description;
          program = "${self.packages.${system}.managed-files}/bin/managed-files";
        };
      });

      checks = forAllSystems (system: {
        package = self.packages.${system}.managed-files;
        noctalia = import ./nix/tests/noctalia.nix {
          pkgs = pkgsFor system;
          package = self.packages.${system}.managed-files;
        };
        home-manager = import ./nix/tests/home-manager.nix {
          pkgs = pkgsFor system;
          inherit home-manager;
          package = self.packages.${system}.managed-files;
        };
      });

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
            ];
          };
        }
      );

      homeModules = {
        managed-files = import ./nix/home-manager.nix;
        default = self.homeModules.managed-files;
      };
    };
}
