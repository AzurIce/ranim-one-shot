{
  # Authoring workspace for the *next* one-shot example.
  #
  # Delivered examples each carry their own frozen flake (flake.nix +
  # flake.lock) in their directory. When delivering a new example, copy one of
  # those flakes into its directory, point its ranim input at the example's
  # Cargo.toml rev, and run `nix flake lock` inside it. Bump the pin below
  # (and the matching nightly) when starting new work.
  description =
    "ranim-one-shot — authoring workspace for the next frozen one-shot ranim example";

  inputs = {
    ranim.url =
      "github:Azurice/ranim/09d67d0f456c3124cc4e466f407369800f490845";

    # Reuse ranim's own toolchain/ecosystem pins so the CLI, the examples and
    # CI all build with the same nixpkgs/crane/rust versions.
    nixpkgs.follows = "ranim/nixpkgs";
    flake-utils.follows = "ranim/flake-utils";
    crane.follows = "ranim/crane";
    rust-overlay.follows = "ranim/rust-overlay";
  };

  outputs =
    { self, ranim, nixpkgs, flake-utils, crane, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        inherit (pkgs) lib;

        # Keep in sync with ranim's CI lint/build jobs
        # (.github/workflows/build.yml pins nightly-2026-08-01).
        craneLib = (crane.mkLib pkgs).overrideToolchain (p:
          p.rust-bin.nightly."2026-08-01".default.override {
            extensions = [ "rust-src" "rustfmt" "clippy" ];
          });

        # ranim's flake.packages.ranim-cli omits the root crate's sources
        # from its crane fileset and fails to build; build the CLI here from
        # the full pinned source tree instead.
        ranim-cli = craneLib.buildPackage {
          src = ranim;
          strictDeps = true;
          cargoExtraArgs = "-p ranim-cli";
          doCheck = false;
        };

        rustToolchain = pkgs.rust-bin.nightly."2026-08-01".default.override {
          extensions = [ "rust-src" "rustfmt" "clippy" ];
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
