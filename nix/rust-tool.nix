# Builds one of this repository's crates, named and versioned by its
# Cargo.toml. crate2nix turns Cargo.lock into one buildRustCrate derivation
# per dependency, so an edit rebuilds only the crate itself. `check = true`
# gives the `nix flake check` gate instead: the tests run, and clippy lints
# the crate with warnings denied. `static = true` links it against musl, for
# the release tarball that must run with no Nix on the machine.
lib: crate2nix: pkgs:
{
  src,
  static ? false,
  env ? { },
  check ? false,
  nativeCheckInputs ? [ ],
}:
let
  inherit (lib.attrsets) genAttrs optionalAttrs;
  inherit (lib.lists) subtractLists;

  target = if static then pkgs.pkgsStatic else pkgs;

  manifest = (lib.trivial.importTOML "${src}/Cargo.toml").package;

  # Generated on the build machine: a cross `pkgs` would run its own compiler
  # here, which cannot run.
  cargoNix = (import "${crate2nix}/tools.nix" { pkgs = pkgs.buildPackages; }).generatedCargoNix {
    inherit (manifest) name;
    inherit src;
  };

  # crate2nix counts rmcp's optional reqwest dependency as rmcp's `reqwest`
  # feature, which asks for reqwest's TLS on aws-lc. Cargo does not, and
  # aws-lc is in no lock file: these crates keep ring, as Cargo builds them.
  # Remove once crate2nix keeps `dep:reqwest` apart from that feature.
  ringOnly =
    genAttrs
      [
        "hyper-rustls"
        "reqwest"
        "rustls"
        "rustls-webpki"
        "tokio-rustls"
      ]
      (
        _: crate: {
          features = subtractLists [ "__rustls-aws-lc-rs" "aws-lc-rs" "aws_lc_rs" ] crate.features;
        }
      );

  # Fixes for dependencies that expect something buildRustCrate lacks.
  crateOverrides =
    target.defaultCrateOverrides
    // ringOnly
    // {
      # Cargo sets CARGO_CRATE_NAME; buildRustCrate does not.
      rmcp = _: { env.CARGO_CRATE_NAME = "rmcp"; };
      # UniFFI's macros read it to name the scaffolding.
      libri-client = _: { env.CARGO_CRATE_NAME = "libri_client"; };
      # sherpa-onnx's C API from nixpkgs, in place of the build script's
      # download of a prebuilt archive.
      # Its build script also copies the libraries beside Cargo's profile
      # directory, which buildRustCrate has none of, so it is replaced by the
      # three lines that matter: link the C API, and find it at run time.
      # Remove once the crate skips the copy when SHERPA_ONNX_LIB_DIR is set.
      sherpa-onnx-sys = _: {
        postPatch = ''
          cat > build.rs <<EOF
          fn main() {
              println!("cargo:rustc-link-search=native=${target.sherpa-onnx}/lib");
              println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");
              println!("cargo:rustc-link-arg=-Wl,-rpath,${target.sherpa-onnx}/lib");
          }
          EOF
        '';
      };
      # Ghostty's terminal library, prebuilt, in place of the build script's
      # git clone and Zig build.
      libghostty-vt-sys = _: {
        nativeBuildInputs = [ target.buildPackages.pkg-config ];
        buildInputs = [ (target.callPackage ./libghostty-vt.nix { }).dev ];
      };
    };

  # The nixpkgs SQLite the default override links wants zlib, which a static
  # link must find itself.
  linkage = lib.optionalAttrs static {
    ${manifest.name} = _: { buildInputs = [ target.zlib ]; };
  };

  # The crate built with `attrs` added to its own derivation only.
  # buildRustCrate gives every crate one codegen unit, which runs a whole
  # crate's optimiser on one core: half an hour for libri when checks run
  # side by side. Sixteen let LLVM spread it, for code a server never misses.
  build =
    attrs:
    (target.callPackage "${cargoNix}/default.nix" {
      # A static build leaves out default features that link shared
      # libraries (libri's local speech engine).
      rootFeatures = if static then [ ] else [ "default" ];
      buildRustCrateForPkgs =
        pkgs: crate:
        pkgs.buildRustCrate.override { defaultCrateOverrides = crateOverrides // linkage; } (
          crate // optionalAttrs (crate.crateName == manifest.name) ({ codegenUnits = 16; } // attrs)
        );
    }).rootCrate.build;
in
if check then
  let
    # The gate wants verdicts, not fast code: the crate itself compiles
    # unoptimised (its dependencies stay the release builds already cached),
    # and without the full debuginfo buildRustCrate gives a debug build,
    # which costs rustc memory and time and which no test failure prints.
    # Tests keep line tables, for their backtraces.
    harness = build {
      release = false;
      buildTests = true;
      extraRustcOpts = [ "-C debuginfo=line-tables-only" ];
    };
  in
  pkgs.linkFarm "${manifest.name}-check" {
    # The test harness alone, run from the source tree as Cargo runs it.
    # crate2nix's runTests also compiles the crate's binaries, to copy beside
    # the harness for tests that run them; none of these tests do.
    tests = pkgs.stdenvNoCC.mkDerivation {
      name = "${manifest.name}-tests";
      # The harness's own source, unpacked where it was compiled, so
      # CARGO_MANIFEST_DIR still leads to the crate's test fixtures.
      inherit (harness) src;
      nativeBuildInputs = nativeCheckInputs;
      buildPhase = ''
        set -o pipefail
        export RUST_BACKTRACE=1
        # A hung test fails the check in minutes instead of holding the
        # build lock for hours: the whole harness gets ten minutes (it runs in
        # seconds), and libtest has named any test running past a minute by
        # then ("has been running for over 60 seconds").
        for test in ${harness}/tests/*; do
          status=0
          timeout -k 10 600 "$test" 2>&1 | tee -a $out || status=$?
          if [ "$status" -eq 124 ] || [ "$status" -eq 137 ]; then
            echo "$test: a test hung past ten minutes (see the tests running over 60 seconds above)" >&2
          fi
          [ "$status" -eq 0 ] || exit "$status"
        done
      '';
      dontInstall = true;
    };
    clippy = build {
      release = false;
      useClippy = true;
      lints.rust.warnings = "deny";
      extraRustcOpts = [ "-C debuginfo=0" ];
      # buildRustCrate builds binaries through make without exporting the
      # driver, so they would quietly compile with plain rustc.
      preBuild = "export RUSTC_DRIVER";
    };
  }
else
  build { inherit env; }
