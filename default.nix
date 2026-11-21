# Builds todo-txt-rs with the nixpkgs pinned in npins/ (see README.md).
let
  sources = import ./npins;
  pkgs = import sources.nixpkgs { };
  cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
in
pkgs.rustPlatform.buildRustPackage {
  pname = "todo-txt-rs";
  version = cargoToml.package.version;

  # Only the files that go into the build, so target/ and result/ are not hashed.
  src = pkgs.lib.fileset.toSource {
    root = ./.;
    fileset = pkgs.lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./src
    ];
  };

  cargoLock.lockFile = ./Cargo.lock;

  meta = {
    description = cargoToml.package.description;
    homepage = cargoToml.package.repository;
    license = pkgs.lib.licenses.mit;
    mainProgram = "todo";
  };
}
