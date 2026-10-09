{ self }:
{
  config,
  lib,
  pkgs,
  utils,
  ...
}:
let
  cfg = config.services.sameframe;
in
{
  options.services.sameframe = {
    enable = lib.mkEnableOption "Sameframe";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.sameframe;
      defaultText = lib.literalExpression "sameframe.packages.\${pkgs.stdenv.hostPlatform.system}.sameframe";
      description = "Sameframe package to run.";
    };

    host = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "Listening address.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 3000;
      description = "Listening TCP port.";
    };

    public = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Allow visitors without a site access token. Room permissions still apply.";
    };

    extraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [
        "--access-token-file"
        "another-token"
      ];
      description = "Additional command-line arguments. Do not put secrets here; they enter the Nix store.";
    };

    dataDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/sameframe";
      description = "Service home and working directory. Stores the generated access-token file.";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Open the listening TCP port in the firewall.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion =
          lib.hasPrefix "/" cfg.dataDir
          && cfg.dataDir != "/"
          && !lib.hasPrefix "${builtins.storeDir}/" cfg.dataDir;
        message = "services.sameframe.dataDir must be an absolute runtime directory outside the Nix store.";
      }
    ];

    users.users.sameframe = {
      isSystemUser = true;
      group = "sameframe";
      home = cfg.dataDir;
    };
    users.groups.sameframe = { };

    systemd.tmpfiles.settings.sameframe.${cfg.dataDir}.d = {
      mode = "0700";
      user = "sameframe";
      group = "sameframe";
    };

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];

    systemd.services.sameframe = {
      description = "Sameframe";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      environment = {
        HOME = cfg.dataDir;
        HOST = cfg.host;
        PORT = toString cfg.port;
      };

      serviceConfig = {
        User = "sameframe";
        Group = "sameframe";
        WorkingDirectory = cfg.dataDir;
        ExecStart = utils.escapeSystemdExecArgs (
          [ (lib.getExe cfg.package) ] ++ lib.optional cfg.public "--public" ++ cfg.extraArgs
        );
        Restart = "on-failure";
        RestartSec = 5;
        UMask = "0077";

        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectHome = true;
        ProtectSystem = "strict";
        ReadWritePaths = [ cfg.dataDir ];
      };
    };
  };
}
