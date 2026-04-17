{pkgs ? import <nixpkgs> {}}:
with pkgs;
  mkShell rec {
    nativeBuildInputs = [
      evcxr
      jupyter
      # presenterm
      rust-script
      pstree
      nix-index
      pkg-config
    ];
    LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath nativeBuildInputs;
  }
