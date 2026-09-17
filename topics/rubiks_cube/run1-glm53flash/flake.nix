{
  # Authoring workspace for the *next* one-shot example.
  #
  # Delivered runs each carry their own frozen flake (flake.nix + flake.lock)
  # in their run directory. When delivering a run, copy this flake into the
  # run directory, point its ranim input at the run's delivered rev, and run
  # `nix flake lock` inside it. Bump the pin below (and the matching nightly)
  # when starting new work.
  description =
    "ranim-one-shot — authoring workspace for the next frozen one-shot ranim example";

  inputs = {
    ranim.url = "github:AzurIce/ranim/40d15be64edf5c04a78e2db75908e4a192a8e942";

    # Reuse ranim's own ecosystem pins so the CLI, the examples and CI all
    # build with the same nixpkgs/rust versions.
    nixpkgs.follows = "ranim/nixpkgs";
    flake-utils.follows = "ranim/flake-utils";
    rust-overlay.follows = "ranim/rust-overlay";
  };

  outputs =
    { self, ranim, nixpkgs, flake-utils, rust-overlay }:
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

        # Since ranim#211, ranim's own packages.ranim-cli builds the CLI
        # correctly and wraps it with the runtime libraries winit/wgpu/rodio
        # dlopen — no local crane build needed here anymore.
        ranim-cli = ranim.packages.${system}.ranim-cli;

        # The wrapped CLI carries these itself; the shell hook below still
        # exposes them for plain `cargo run` of an example inside the shell.
        previewRuntimeLibs = [
          pkgs.vulkan-loader
          pkgs.wayland
          pkgs.libxkbcommon
          pkgs.libX11
          pkgs.libGL
        ];
      in
      {
        packages = { inherit ranim-cli; };

        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            ranim-cli
            pkgs.ffmpeg
          ] ++ lib.optionals pkgs.stdenv.isLinux previewRuntimeLibs;

          # The renderer (wgpu) needs these libraries at runtime.
          shellHook = lib.optionalString pkgs.stdenv.isLinux ''
            export LD_LIBRARY_PATH="${
              lib.makeLibraryPath previewRuntimeLibs
            }''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
          '';
        };
      });
}
