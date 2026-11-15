# Libri: the server, its page, hippo, and the wiki. `nix flake check` runs each crate's
# tests and clippy with warnings denied, and rustfmt over the tree.
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Unstable no longer supports Intel macOS; retain its last supported channel.
    nixpkgs-darwin.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";
    crate2nix = {
      url = "github:nix-community/crate2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Rust's standard library for Android, which nixpkgs' rustc lacks.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-darwin,
      crate2nix,
      rust-overlay,
    }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      rustTool = import ./nix/rust-tool.nix nixpkgs.lib crate2nix pkgs;
      # The server's crate without the page beside it, which builds on its
      # own: an edit to the page would otherwise recompile the server.
      libriSrc = nixpkgs.lib.fileset.toSource {
        root = ./libri;
        fileset = nixpkgs.lib.fileset.difference ./libri ./libri/site;
      };

      # hippo's compactions read Libri's own prompt (libri/src/prompt.txt):
      # one file, which the repository links into hippo for Cargo and the
      # build copies in here, since a crate's source holds no link outside it.
      hippoSrc = pkgs.runCommand "hippo-src" { } ''
        cp -r ${
          nixpkgs.lib.fileset.toSource {
            root = ./hippo;
            fileset = nixpkgs.lib.fileset.difference ./hippo ./hippo/src/memory/prompt.txt;
          }
        } $out
        chmod -R u+w $out
        cp ${./libri/src/prompt.txt} $out/src/memory/prompt.txt
      '';

      aarch64Tool = import ./nix/rust-tool.nix nixpkgs.lib crate2nix pkgs.pkgsCross.aarch64-multiplatform;

      # The release tarball for a machine with no Nix (README): static
      # binaries, the page, and the script that installs them.
      bundleFor =
        arch: tool:
        let
          daemon = tool {
            src = libriSrc;
            static = true;
          };
          hippo = tool {
            src = hippoSrc;
            static = true;
          };
          wiki = tool {
            src = ./wiki;
            static = true;
          };
        in
        pkgs.runCommand "libri-bundle" { nativeBuildInputs = [ pkgs.gnutar ]; } ''
          name=libri-${daemon.crateVersion or "0.1.0"}-linux-${arch}
          mkdir -p $name/bin $name/share/libri $out
          cp ${pkgs.lib.getExe daemon} ${pkgs.lib.getExe hippo} ${pkgs.lib.getExe wiki} $name/bin/
          cp -r ${site} $name/share/libri/site
          cp ${./nix/install.sh} $name/install.sh
          echo Linux-${arch} > $name/machine
          chmod -R u+w $name
          tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -czf $out/$name.tar.gz $name
        '';

      site = pkgs.buildNpmPackage {
        pname = "libri-site";
        version = "0";
        src = ./libri/site;
        npmDepsHash = "sha256-1+48X24KvJJMV1u5V1mGJhle1oUv22Nx9EYafgGlWIo=";
        # rolldown and tsgo ship prebuilt Linux binaries.
        nativeBuildInputs = [
          pkgs.autoPatchelfHook
          pkgs.brotli
          pkgs.resvg
        ];
        buildInputs = [ pkgs.stdenv.cc.cc.lib ];
        doCheck = true;
        checkPhase = "npm run check";
        # Home screens want the icon as PNG at these sizes.
        installPhase = ''
          cp -r dist $out
          # Excalidraw's fonts beside the page's files (site/src/board.tsx
          # points EXCALIDRAW_ASSET_PATH here), so a whiteboard never
          # fetches from a CDN.
          mkdir -p $out/_site/excalidraw
          cp -r node_modules/@excalidraw/excalidraw/dist/prod/fonts $out/_site/excalidraw/fonts
          for s in 180 192 512; do
            resvg -w $s -h $s public/icon/app.svg $out/icon/app-$s.png
          done
          # Each text asset beside its brotli and gzip copies, which the
          # host serves to a browser that takes them (site.rs, share.rs):
          # the editor's code is megabytes, a phone fetches it over the
          # tailnet.
          find $out/_site -type f \( -name '*.js' -o -name '*.css' -o -name '*.svg' -o -name '*.json' -o -name '*.wasm' \) -size +1k \
            -exec brotli -k -q 11 {} + -exec gzip -k -9 {} +
        '';
      };
    in
    {
      packages =
        nixpkgs.lib.genAttrs [ "aarch64-linux" "aarch64-darwin" "x86_64-darwin" ] (
          target:
          let
            targetPkgs =
              (if target == "x86_64-darwin" then nixpkgs-darwin else nixpkgs).legacyPackages.${target};
          in
          {
            hippo = targetPkgs.rustPlatform.buildRustPackage {
              pname = "hippo";
              version = "0.1.0";
              src = self;
              cargoRoot = "hippo";
              buildAndTestSubdir = "hippo";
              cargoLock.lockFile = ./hippo/Cargo.lock;
            };
          }
        )
        // {
          ${system} = rec {
            daemon = rustTool { src = libriSrc; };
            hippo = rustTool { src = hippoSrc; };
            # The assistant's notes, an Obsidian vault served over MCP.
            wiki = rustTool { src = ./wiki; };
            inherit site;
            # Libri on its own: `libri` (the daemon and its commands), the
            # hippo it keeps its memory in, the wiki it keeps its notes in, and
            # the page. Runs from one config
            # file; see the README.
            libri =
              pkgs.runCommand "libri"
                {
                  nativeBuildInputs = [ pkgs.makeWrapper ];
                  meta.mainProgram = "libri";
                }
                ''
                  mkdir -p $out/bin
                  makeWrapper ${pkgs.lib.getExe daemon} $out/bin/libri \
                    --set-default LIBRI_PACKAGED_SITE ${site} \
                    --set LIBRI_COMMIT ${self.shortRev or self.dirtyShortRev or "unknown"} \
                    --prefix PATH : ${
                      pkgs.lib.makeBinPath [
                        hippo
                        wiki
                      ]
                    } \
                    --suffix PATH : ${
                      pkgs.lib.makeBinPath [
                        pkgs.git
                        # Local dictation: ffmpeg reads a recording's audio,
                        # tar and bzip2 unpack a downloaded speech model.
                        pkgs.ffmpeg-headless
                        pkgs.gnutar
                        pkgs.bzip2
                      ]
                    }
                  ln -s ${pkgs.lib.getExe hippo} $out/bin/hippo
                  ln -s ${pkgs.lib.getExe wiki} $out/bin/wiki
                  ln -s ${site} $out/site
                '';
            bundle = bundleFor "x86_64" rustTool;
            # The UI driver (nix/drive_ui.py): the page driven and measured against
            # a throwaway Libri, with golden pictures; the hill climb's scorecard.
            # Run from the checkout, `nix run .#ui-driver`, to compare with and
            # keep the goldens in nix/ui_goldens; `-- --help` for the rest.
            ui-driver = pkgs.writeShellApplication {
              name = "ui-driver";
              runtimeInputs = [
                (pkgs.python3.withPackages (p: [
                  p.playwright
                  p.pillow
                ]))
              ];
              text = ''
                export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
                export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
                goldens=()
                [ -d nix/ui_goldens ] && goldens=(--goldens "$PWD/nix/ui_goldens")
                case "''${1:-}" in
                "" | -*) exec python3 ${./nix}/drive_ui.py ${libri} "''${goldens[@]}" "$@" ;;
                *) exec python3 ${./nix}/drive_ui.py "$@" "''${goldens[@]}" ;;
                esac
              '';
            };
            # Every interaction with the page within 400 ms (nix/check_ui_latency.py):
            # the built site, crawled and timed in Chromium at 1440 and 390.
            # A browser check, so out of `nix flake check`; the release runs it
            # after the checks. Fails on any median over 400 ms unthrottled,
            # timed again alone first, on a page that never loads or a crawl
            # that finds far too little, and on a share that times nothing for
            # five minutes or runs past its limit; the whole run stops at
            # `-- --limit SECONDS` (default 90 minutes).
            # `-- --slow --table FILE` adds the 4x
            # slowed processor and writes the full table.
            ui-latency = pkgs.writeShellApplication {
              name = "ui-latency";
              runtimeInputs = [
                pkgs.util-linux
                (pkgs.python3.withPackages (p: [ p.playwright ]))
              ];
              text = ''
                # One timing run at a time (nix/with_check_lock.sh). The timing
                # lock alone: the gate builds nothing once started, and holding
                # the builds lock for its half hour stopped every build on the host.
                if [ -z "''${LIBRI_TIMING_HELD:-}" ]; then
                  exec ${./nix/with_check_lock.sh} --timing-only "$0" "$@"
                fi
                export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
                export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
                # The site beside its libri: the stand-in host asks it for the tiles.
                exec python3 ${./nix}/check_ui_latency.py ${libri}/site --inventory ${./libri/site/src} "$@"
              '';
            };
            # The page's browser checks: Playwright runs of the built site that
            # need a real browser, which the nix build sandbox can't give
            # them, so they stay out of `nix flake check` and the release runs
            # them instead. `nix run .#browser-checks -- SHOTS_DIR` runs each
            # against this commit's Libri, prints its result, and fails if
            # any failed. A check joins the list once it passes on main; one
            # left off is stale and is fixed before it joins.
            browser-checks = pkgs.writeShellApplication {
              name = "browser-checks";
              runtimeInputs = [
                pkgs.coreutils
                pkgs.util-linux
                (pkgs.python3.withPackages (p: [
                  p.playwright
                  p.pillow
                ]))
              ];
              text = ''
                # One timing run at a time, with no build beside it
                # (nix/with_check_lock.sh --timing): builds beside it slow
                # first page loads past their timeouts.
                if [ -z "''${LIBRI_TIMING_HELD:-}" ]; then
                  exec ${./nix/with_check_lock.sh} --timing "$0" "$@"
                fi
                export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
                export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
                shots=$(realpath -m "''${1:-browser-checks}")
                mkdir -p "$shots"
                failed=()
                # A check that runs past this many seconds fails rather than
                # holding the lock: headless WebKit can stop drawing frames and
                # wait on one forever. timeout signals the check's whole process
                # group, so its browsers go with it.
                limit=''${LIBRI_CHECK_LIMIT:-600}
                # Each check with the arguments its usage line names.
                output=$(mktemp)
                trap 'rm -f "$output"' EXIT
                attempt() {
                  local check=$1
                  shift
                  timeout --kill-after=30 "$limit" python3 ${./nix}/browser_check_run.py ${./nix}/"$check".py "$@" 2>&1 | tee "$output" || return "''${PIPESTATUS[0]}"
                }
                run() {
                  local name=$1 rc=0
                  shift
                  echo "== $name"
                  attempt "$name" "$@" || rc=$?
                  # A first page load can hang past even 60 s on a loaded
                  # host, and the same check then passes when run alone.
                  # Until the cause is found and fixed (which
                  # removes this rerun), a check that failed only on a
                  # Page.goto timeout runs once more, and passes if that run
                  # does. Any other failure, or a second one, still fails it.
                  if ((rc != 0 && rc != 124 && rc != 137)) && grep -q 'Page.goto: Timeout' "$output"; then
                    echo "== $name: Page.goto timed out (the first-load hang); running it again alone"
                    rc=0
                    attempt "$name" "$@" || rc=$?
                    if ((rc == 0)); then
                      echo "== $name passed on its rerun (first-load hang override)"
                      return
                    fi
                  fi
                  if ((rc == 0)); then
                    echo "== $name passed"
                  elif ((rc == 124 || rc == 137)); then
                    echo "== $name FAILED (still running after ''${limit} s)"
                    failed+=("$name")
                  else
                    echo "== $name FAILED"
                    failed+=("$name")
                  fi
                }
                run check_phone_workspaces ${libri}/site "$shots/phone_workspaces"
                run check_chat_steady ${libri}/site
                run check_page_data ${libri} "$shots/page_data"
                run check_page_links ${libri} "$shots/page_links"
                run check_usage_page ${libri}/bin/libri ${libri}/site "$shots/usage_page"
                run check_omp_rendering ${libri} ${libri}/site "$shots/omp_rendering"
                run check_edge_fade ${libri}/site "$shots/edge_fade"
                run check_side_panel ${libri}/site "$shots/side_panel"
                run check_settings_fit ${libri}/site "$shots/settings_fit"
                run check_settings_resource ${libri} ${libri}/site "$shots/settings_resource"
                run check_whiteboard ${libri} "$shots/whiteboard"
                run check_terminal_field ${libri} "$shots/terminal_field"
                run check_composer_agents ${libri}/site "$shots/composer_agents"
                run check_tile_menu ${libri}/site "$shots/tile_menu"
                run check_temps ${libri} "$shots/temps"
                run check_workspace_close ${libri} "$shots/workspace_close"
                run check_memory_tabs ${libri} "$shots/memory_tabs"
                run check_slash_picks ${libri} ${libri}/site "$shots/slash_picks"
                run check_card_opens ${libri} "$shots/card_opens"
                run check_peers_settings ${libri} ${libri}/site "$shots/peers_settings"
                # The voice detector a call's ear runs, which the host
                # otherwise fetches on first use.
                run check_call_chat ${libri} ${
                  pkgs.fetchurl {
                    url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
                    sha256 = "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6";
                  }
                }
                run check_system_tile ${libri} "$shots/system_tile"
                if ((''${#failed[@]})); then
                  echo "browser checks failed: ''${failed[*]} (pictures in $shots)"
                  exit 1
                fi
                echo "browser checks pass (pictures in $shots)"
              '';
            };
            bundle-aarch64 = bundleFor "aarch64" aarch64Tool;
            # Libri as anyone installs it. Each instance offers the harnesses its
            # own PATH holds.
            default = libri;
            # The throwaway VM run_endurance_test.sh boots. Not a check: it runs
            # for hours and only by hand.
            endurance = pkgs.lib.makeOverridable (import ./nix/endurance/package.nix) {
              inherit nixpkgs pkgs libri;
            };
          };
        };
      nixosModules.default = import ./nix/module.nix self;

      # `nix develop .#android` builds the client core and the Android app;
      # `.#android-emulator` adds an emulator to run it on.
      devShells.${system} = {
        android = import ./nix/android.nix { inherit nixpkgs rust-overlay system; };
        android-emulator = import ./nix/android.nix {
          inherit nixpkgs rust-overlay system;
          emulator = true;
        };
      };

      checks.${system} = {
        # The module evaluates to a service, on a machine with nothing else.
        module =
          (nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              self.nixosModules.default
              {
                services.libri.instances.libri.settings.daemon.listen = "0.0.0.0:6780";
                boot.loader.grub.enable = false;
                fileSystems."/" = {
                  device = "none";
                  fsType = "tmpfs";
                };
                system.stateVersion = "26.11";
              }
            ];
          }).config.systemd.units."libri.service".unit;
        # The service on a bare machine: up, paired, one full turn, kept once.
        service = import ./nix/check_service.nix { inherit pkgs self; };
        daemon = rustTool {
          src = libriSrc;
          check = true;
          # The worktree tests make repositories of their own.
          nativeCheckInputs = [ pkgs.git ];
        };
        # Every message reaches hippo once, whichever harness runs the turn.
        memory =
          pkgs.runCommand "memory-check"
            {
              nativeBuildInputs = [ pkgs.python3 ];
            }
            ''
              python3 ${./nix}/check_memory.py ${self.packages.${system}.libri}
              touch $out
            '';
        # Two assistants pair, message each other, unpair and pair again.
        peers =
          pkgs.runCommand "peers-check"
            {
              nativeBuildInputs = [ pkgs.python3 ];
            }
            ''
              python3 ${./nix}/check_peers.py ${self.packages.${system}.libri}
              touch $out
            '';
        # The person's words reach the turn under way, in order.
        delivery =
          pkgs.runCommand "delivery-check"
            {
              nativeBuildInputs = [ pkgs.python3 ];
            }
            ''
              python3 ${./nix/check_delivery.py} ${self.packages.${system}.libri}
              touch $out
            '';
        # Every app has a view for each kind of tile the host declares where
        # it runs, and code for each row its menu names (tiles.rs).
        tiles =
          pkgs.runCommand "tiles-check"
            {
              nativeBuildInputs = [ pkgs.python3 ];
            }
            ''
              python3 ${./nix/check_tile_coverage.py} ${self.packages.${system}.libri}/bin/libri ${self}
              touch $out
            '';
        # A named memory's chat reads and keeps its own memory alone.
        memories =
          pkgs.runCommand "memories-check"
            {
              nativeBuildInputs = [ pkgs.python3 ];
            }
            ''
              python3 ${./nix}/check_memories.py ${self.packages.${system}.libri}
              touch $out
            '';
        # Other apps reach the memory and chats over MCP, once allowed.
        outside =
          pkgs.runCommand "outside-check"
            {
              nativeBuildInputs = [ pkgs.python3 ];
            }
            ''
              python3 ${./nix}/check_outside.py ${self.packages.${system}.libri}
              touch $out
            '';
        hippo = rustTool {
          src = hippoSrc;
          check = true;
        };
        wiki = rustTool {
          src = ./wiki;
          check = true;
        };
        client = rustTool {
          src = ./client;
          check = true;
        };
        inherit site;
        # No names or paths of a particular install (nix/check_names.sh).
        names =
          pkgs.runCommand "names-check"
            {
              nativeBuildInputs = [
                pkgs.findutils
                pkgs.gnugrep
              ];
            }
            ''
              sh ${./nix/check_names.sh} ${self} && touch $out
            '';
        # No browser confirm/alert/prompt in the page (nix/check_dialogs.sh).
        dialogs =
          pkgs.runCommand "dialogs-check" { nativeBuildInputs = [ pkgs.gnugrep ]; }
            ''
              sh ${./nix/check_dialogs.sh} ${self} && touch $out
            '';
        rustfmt = pkgs.runCommand "rustfmt-check" { nativeBuildInputs = [ pkgs.rustfmt ]; } ''
          cd ${self} && rustfmt --edition 2024 --check $(find . -name '*.rs' -not -path '*/node_modules/*') && touch $out
        '';
      };
      formatter.${system} = pkgs.nixfmt;
    };
}
