# Libri as NixOS services: one daemon per instance, each with its own
# config, written from `settings` to /etc/libri/<name>.toml (every key is in
# the README), and its own state, hippo included, in /var/lib/<name>. The
# unit is <name>.service, to extend with `systemd.services.<name>` as any
# NixOS unit. Pair a device with
# `sudo -u <user> libri --config /etc/libri/<name>.toml pair`.
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.libri.instances;
  toml = pkgs.formats.toml { };
  file = name: "libri/${name}.toml";
  port = i: lib.toInt (lib.last (lib.splitString ":" i.settings.daemon.listen));
  instance =
    { name, config, ... }:
    {
      options = {
        package = lib.mkOption {
          type = lib.types.either lib.types.package lib.types.path;
          default = self.packages.${pkgs.stdenv.hostPlatform.system}.libri;
          defaultText = lib.literalExpression "libri.packages.\${system}.libri";
          description = ''
            The build to run: a package, or the path of a link a deploy
            moves outside the system, which the unit reads at each start.
          '';
        };
        user = lib.mkOption {
          type = lib.types.str;
          default = name;
          description = "User it runs as, made here when it is the instance's own name.";
        };
        group = lib.mkOption {
          type = lib.types.str;
          default = config.user;
          description = "Group it runs as.";
        };
        settings = lib.mkOption {
          type = lib.types.submodule {
            freeformType = toml.type;
            options.daemon.listen = lib.mkOption {
              type = lib.types.str;
              default = "127.0.0.1:6780";
              description = "Address and port the daemon listens on.";
            };
          };
          default = { };
          description = "The config file, as the README lists its keys.";
        };
        browser = lib.mkOption {
          type = lib.types.nullOr lib.types.package;
          default = null;
          example = lib.literalExpression "pkgs.chromium";
          description = ''
            The host's Chromium: it renders a still of each published page
            for the Library's cards, and drives the agents' browser when
            `agentBrowser.enable` is on (pkgs.chromium if this is null).
            Null with the agents' browser off, the default, renders no stills
            and the cards show titles alone, keeping Chromium out of the
            closure.
          '';
        };
        harnesses = lib.mkOption {
          type = lib.types.listOf lib.types.package;
          default = [ ];
          example = lib.literalExpression "[ pkgs.claude-code pkgs.codex ]";
          description = "Harness CLIs on the daemon's PATH; each signs in once as its user.";
        };
        # Off, Xvfb is not in the system, nor Chromium unless `browser` names
        # one. On, its Chromium renders the Library's stills too.
        agentBrowser.enable = lib.mkEnableOption "Libri's browser for agents, Chromium on a private Xvfb display";
        openFirewall = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Open the listening port; pairing still guards every request.";
        };
      };
      config.settings = {
        state = lib.mkDefault "/var/lib/${name}";
        # One Chromium per host, for the stills and the agents' browser.
        browser = lib.mkIf (config.browser != null || config.agentBrowser.enable) {
          chromium = lib.mkDefault (lib.getExe (if config.browser != null then config.browser else pkgs.chromium));
          enable = lib.mkIf config.agentBrowser.enable true;
          xvfb = lib.mkIf config.agentBrowser.enable (lib.mkDefault "${pkgs.xvfb}/bin/Xvfb");
        };
      };
    };
  own = lib.filterAttrs (name: i: i.user == name) cfg;
in
{
  options.services.libri.instances = lib.mkOption {
    type = lib.types.attrsOf (lib.types.submodule instance);
    default = { };
    example.libri.harnesses = lib.literalExpression "[ pkgs.claude-code ]";
    description = "Libri daemons, by the name of their unit and state directory.";
  };

  config = lib.mkIf (cfg != { }) {
    assertions = [
      {
        assertion = lib.allUnique (lib.mapAttrsToList (_: i: i.settings.daemon.listen) cfg);
        message = "services.libri: each instance needs its own daemon.listen.";
      }
    ];
    environment.etc = lib.mapAttrs' (
      name: i: lib.nameValuePair (file name) { source = toml.generate "${name}-libri.toml" i.settings; }
    ) cfg;
    environment.systemPackages = lib.filter lib.isDerivation (
      lib.mapAttrsToList (_: i: i.package) cfg
    );
    users.users = lib.mapAttrs (name: i: {
      isSystemUser = true;
      inherit (i) group;
      # The harnesses keep their sign-ins under it.
      home = "/var/lib/${name}";
    }) own;
    users.groups = lib.mapAttrs' (_: i: lib.nameValuePair i.group { }) own;
    networking.firewall.allowedTCPPorts = lib.mapAttrsToList (_: port) (
      lib.filterAttrs (_: i: i.openFirewall) cfg
    );
    systemd.services = lib.mapAttrs (name: i: {
      description = lib.mkDefault "Libri (${name})";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      path = i.harnesses;
      restartTriggers = [ config.environment.etc.${file name}.source ];
      serviceConfig = {
        ExecStart = "${i.package}/bin/libri --config /etc/${file name}";
        User = i.user;
        Group = i.group;
        StateDirectory = name;
        StateDirectoryMode = "0700";
        Restart = lib.mkDefault "on-failure";
        NoNewPrivileges = lib.mkDefault true;
        ProtectSystem = lib.mkDefault "full";
        PrivateTmp = lib.mkDefault true;
      };
    }) cfg;
  };
}
