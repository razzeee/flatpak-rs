{
  description = "Experimental flatpak-rs CLI, service, and Linux VM acceptance test";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/02c087f3fe673c9e317f206a627cb6b910336dbe";
  outputs = { self, nixpkgs }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs { inherit system; };
    package = pkgs.rustPlatform.buildRustPackage {
      pname = "flatpak-rs";
      version = "0.1.0";
      src = pkgs.lib.cleanSource ./.;
      cargoLock.lockFile = ./Cargo.lock;
      nativeBuildInputs = [ pkgs.pkg-config ];
      buildInputs = [ pkgs.ostree pkgs.glib ];
      # Real-process tests may consult user-session tools; isolate their HOME.
      preCheck = ''
        export HOME="$TMPDIR/test-home"
        mkdir -p "$HOME"
      '';
      postInstall = ''
        mkdir -p $out/lib/systemd/user
        cp systemd/flatpak-rs.socket $out/lib/systemd/user/
        substitute systemd/flatpak-rs.service $out/lib/systemd/user/flatpak-rs.service \
          --replace-fail /usr/libexec/flatpak-rs-service $out/bin/flatpak-rs-service
      '';
    };
  in {
    packages.${system}.default = package;
    checks.${system} = {
      rust = package.overrideAttrs (old: {
        pname = "flatpak-rs-rust-checks";
        nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.rustfmt pkgs.clippy pkgs.actionlint ];
        postBuild = ''
          actionlint .github/workflows/ci.yml
          cargo fmt --all -- --check
          cargo check --locked --all-targets
          cargo clippy --locked --all-targets -- -D warnings
        '';
      });
      service = import ./tests/vm.nix { inherit pkgs package; };
    };
  };
}
