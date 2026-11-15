# `.#endurance`: the VM and the report writer, for run_endurance_test.sh.
{
  nixpkgs,
  pkgs,
  libri,
  # The build the instance is born on, upgraded from and rolled back to.
  previous ? libri,
  memoryMiB ? 6144,
}:
let
  machine = nixpkgs.lib.nixosSystem {
    inherit (pkgs.stdenv.hostPlatform) system;
    specialArgs = { inherit libri previous memoryMiB; };
    modules = [ ./vm.nix ];
  };
in
pkgs.runCommand "libri-endurance" { } ''
  mkdir -p $out/bin
  ln -s ${machine.config.system.build.vm}/bin/run-*-vm $out/bin/run_endurance_vm
  { echo '#!${pkgs.lib.getExe pkgs.python3}'; cat ${./write_endurance_report.py}; } \
    > $out/bin/write_endurance_report
  chmod +x $out/bin/write_endurance_report
''
