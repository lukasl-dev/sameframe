{
  inputs,
  pkgs,
  system,
}:
let
  inherit (pkgs) lib;
  evaluate =
    settings:
    (inputs.nixpkgs.lib.nixosSystem {
      inherit system;
      modules = [
        inputs.self.nixosModules.sameframe
        {
          system.stateVersion = "26.05";
          services.sameframe = settings;
        }
      ];
    }).config;

  disabled = evaluate { };
  private = evaluate { enable = true; };
  public = evaluate {
    enable = true;
    public = true;
    host = "0.0.0.0";
    port = 8080;
    dataDir = "/var/lib/sameframe-test";
    openFirewall = true;
    package = pkgs.writeShellApplication {
      name = "sameframe-test";
      text = "exit 0";
    };
  };

  invalid = evaluate {
    enable = true;
    dataDir = "relative/state";
  };

  withArgs = evaluate {
    enable = true;
    extraArgs = [
      "--access-token-file"
      "token with spaces%and$dollars"
    ];
  };

  privateService = private.systemd.services.sameframe;
  publicService = public.systemd.services.sameframe;
in
assert !(disabled.systemd.services ? sameframe);
assert !(disabled.users.users ? sameframe);
assert privateService.environment.HOME == "/var/lib/sameframe";
assert privateService.environment.HOST == "127.0.0.1";
assert privateService.environment.PORT == "3000";
assert privateService.serviceConfig.WorkingDirectory == "/var/lib/sameframe";
assert privateService.serviceConfig.User == "sameframe";
assert private.users.users.sameframe.home == "/var/lib/sameframe";
assert private.systemd.tmpfiles.settings.sameframe."/var/lib/sameframe".d.mode == "0700";
assert !(lib.hasInfix "--public" privateService.serviceConfig.ExecStart);
assert private.networking.firewall.allowedTCPPorts == [ ];
assert
  withArgs.systemd.services.sameframe.serviceConfig.ExecStart
  == privateService.serviceConfig.ExecStart
  + " \"--access-token-file\" \"token with spaces%%and$$dollars\"";
assert publicService.environment.HOME == "/var/lib/sameframe-test";
assert publicService.environment.HOST == "0.0.0.0";
assert publicService.environment.PORT == "8080";
assert publicService.serviceConfig.ReadWritePaths == [ "/var/lib/sameframe-test" ];
assert lib.hasInfix "sameframe-test" publicService.serviceConfig.ExecStart;
assert lib.hasInfix "--public" publicService.serviceConfig.ExecStart;
assert public.networking.firewall.allowedTCPPorts == [ 8080 ];
assert lib.any (
  entry: !entry.assertion && lib.hasInfix "services.sameframe.dataDir" entry.message
) invalid.assertions;
pkgs.runCommand "sameframe-module-check" { } ''
  touch "$out"
''
