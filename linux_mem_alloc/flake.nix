{
  # Frozen workspace for the linux_mem_alloc one-shot example.
  #
  # The ranim rev below must match the rev in Cargo.toml; CI checks that
  # Cargo.toml, this file and flake.lock all agree.
  description =
    "linux_mem_alloc — \"Where Does Memory Come From?\" (frozen one-shot ranim example)";

  inputs.ranim.url =
    "github:Azurice/ranim/09d67d0f456c3124cc4e466f407369800f490845";

  # Reuse the pinned ranim's own nixpkgs/crane/rust-overlay pins so the CLI,
  # the toolchain and this example all build on one consistent ecosystem.
  inputs.nixpkgs.follows = "ranim/nixpkgs";
  inputs.flake-utils.follows = "ranim/flake-utils";
  inputs.crane.follows = "ranim/crane";
  inputs.rust-overlay.follows = "ranim/rust-overlay";

  outputs =
    { self, ranim, nixpkgs, flake-utils, crane, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        inherit (pkgs) lib;

        # Keep in sync with the toolchain pinned by this ranim rev
        # (ranim's CI pins nightly-2026-08-01).
        rustToolchain = pkgs.rust-bin.nightly."2026-08-01".default.override {
          extensions = [ "rust-src" "rustfmt" "clippy" ];
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain (p: rustToolchain);

        # ranim's own flake.packages.ranim-cli omits the root crate's sources
        # from its crane fileset and fails to build; build the CLI here from
        # the full pinned source tree instead.
        ranim-cli = craneLib.buildPackage {
          src = ranim;
          strictDeps = true;
          cargoExtraArgs = "-p ranim-cli";
          doCheck = false;
        };
      in
      {
        packages = { inherit ranim-cli; };

        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            ranim-cli
            pkgs.ffmpeg
          ] ++ lib.optionals pkgs.stdenv.isLinux [
            pkgs.vulkan-loader
            pkgs.wayland
            pkgs.libxkbcommon
            pkgs.libX11
          ];

          # The renderer (wgpu) needs these libraries at runtime.
          shellHook = lib.optionalString pkgs.stdenv.isLinux ''
            export LD_LIBRARY_PATH="${
              lib.makeLibraryPath [
                pkgs.vulkan-loader
                pkgs.wayland
                pkgs.libxkbcommon
                pkgs.libX11
              ]
            }''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
          '';
        };
      });
}
