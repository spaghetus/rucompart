{pkgs ? import <nixpkgs> {}}:
with pkgs;
  mkShell {
    buildInputs = [
      evcxr
      jupyter
      # presenterm
      rust-script
      pstree
    ];
  }
