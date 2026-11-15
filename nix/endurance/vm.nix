# The throwaway machine run_endurance_test.sh boots: dev's Libri and hippo,
# the hostile rig and a stand-in harness, and nothing else. Its store is an
# image of its own closure, not the host's store, and its network reaches only
# the one port forwarded in from the host's loopback, so it holds no
# credentials and no test in it can reach a live service. /tmp/shared is the
# only way out: the plan comes in there and the results go out.
{
  pkgs,
  lib,
  modulesPath,
  libri,
  previous,
  memoryMiB,
  ...
}:
let
  python = pkgs.python3;
  suite = lib.fileset.toSource {
    root = ../hostile;
    fileset = ../hostile;
  };
in
{
  imports = [ "${modulesPath}/virtualisation/qemu-vm.nix" ];

  system.stateVersion = "25.11";
  networking.hostName = "libri-endurance";
  documentation.enable = false;
  nix.enable = false;

  virtualisation = {
    memorySize = memoryMiB;
    cores = 6;
    diskSize = 16384;
    graphics = false;
    useNixStoreImage = true;
    mountHostNixStore = false;
    restrictNetwork = true;
    # Hands freed guest pages back, so the host carries what the guest uses,
    # not the most it ever touched.
    qemu.options = [ "-device virtio-balloon-pci,free-page-reporting=on" ];
  };

  # The console is the way in by hand; nothing is reachable from outside.
  services.getty.autologinUser = "root";

  environment.systemPackages = [
    libri
    python
    pkgs.nftables
  ];

  systemd.services.libri-endurance = {
    description = "Libri endurance suite";
    wantedBy = [ "multi-user.target" ];
    after = [ "multi-user.target" ];
    path = [
      libri
      python
      pkgs.coreutils
      pkgs.e2fsprogs
      pkgs.iproute2
      pkgs.nftables
      pkgs.procps
      pkgs.util-linux
    ];
    environment = {
      LIBRI_BUILD = "${libri}";
      LIBRI_PREVIOUS = "${previous}";
      LIBRI_NIGHT_BASE = "/var/lib/libri-endurance";
      ENDURANCE_OUTPUT = "/tmp/shared";
    };
    serviceConfig = {
      Type = "oneshot";
      ExecStart = "${lib.getExe python} ${suite}/endurance.py";
      StateDirectory = "libri-endurance";
      TimeoutStartSec = "infinity";
      StandardOutput = "journal+console";
      StandardError = "journal+console";
    };
    # A hands-on boot keeps running; a suite's end ends the machine.
    unitConfig = {
      SuccessAction = "poweroff";
      FailureAction = "poweroff";
    };
  };
}
