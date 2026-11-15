# Ghostty's terminal library at the commit the libghostty-vt crate pins,
# which the terminal keeps its screens in. nixpkgs packages a later commit
# whose C API the crate's bindings no longer match (ghostty_terminal_new
# changed), so this rebuilds the nixpkgs package at the crate's commit with
# the Zig and dependencies that commit names. Remove it once a crate release
# and nixpkgs agree on a commit.
{
  lib,
  stdenv,
  libghostty-vt,
  fetchFromGitHub,
  callPackage,
  zig_0_15,
}:
libghostty-vt.overrideAttrs (
  finalAttrs: old: {
    version = "0.2.2-a887df4";
    src = fetchFromGitHub {
      owner = "ghostty-org";
      repo = "ghostty";
      # libghostty-vt-sys 0.2.2, build.rs GHOSTTY_COMMIT.
      rev = "a887df42c56f6de86c0fe6da9c4eeca37931e083";
      hash = "sha256-1Zz65SCk3rkJ9+Q0MmyNOTNiDSLBRIHRd3IvFM4iNXw=";
    };
    deps = callPackage "${finalAttrs.src}/build.zig.zon.nix" { inherit zig_0_15; };
    nativeBuildInputs = map (p: if p.pname or "" == "zig" then zig_0_15 else p) old.nativeBuildInputs;
    # Zig links its own musl; left to detect, it takes the build machine's.
    zigBuildFlags =
      map (
        f:
        if builtins.match "--system|/nix/store/.*" f != null && f != "--system" then
          "${finalAttrs.deps}"
        else
          f
      ) old.zigBuildFlags
      ++ lib.optional stdenv.hostPlatform.isMusl "-Dtarget=${stdenv.hostPlatform.parsed.cpu.name}-linux-musl";
  }
)
