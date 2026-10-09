{ inputs }:
{
  name = "sameframe-service";

  nodes.machine =
    { lib, pkgs, ... }:
    {
      imports = [ inputs.self.nixosModules.sameframe ];

      services.sameframe.enable = true;
      environment.systemPackages = [
        pkgs.curl
        pkgs.jq
      ];

      specialisation.public.configuration.services.sameframe = {
        public = lib.mkForce true;
        dataDir = lib.mkForce "/var/lib/sameframe-public";
      };
    };

  testScript = ''
    machine.start()
    machine.wait_for_unit("sameframe.service")
    machine.wait_for_open_port(3000)

    machine.succeed("test $(stat -c %a /var/lib/sameframe) = 700")
    machine.succeed("test $(stat -c %a /var/lib/sameframe/access-token) = 600")
    machine.succeed("test $(stat -c %U /var/lib/sameframe/access-token) = sameframe")
    machine.succeed("test ! -e /var/lib/sameframe/.sameframe")
    machine.succeed("cp /var/lib/sameframe/access-token /tmp/original-token")

    machine.succeed("test $(curl -s -o /tmp/locked.html -w '%{http_code}' http://127.0.0.1:3000/) = 401")
    machine.succeed("grep -q access-form /tmp/locked.html")
    machine.succeed("jq -Rs '{token: rtrimstr(\"\\n\")}' /var/lib/sameframe/access-token | curl --fail -s -c /tmp/cookies -H 'Origin: http://127.0.0.1:3000' -H 'Content-Type: application/json' --data-binary @- http://127.0.0.1:3000/api/access")
    machine.succeed("curl --fail -s -b /tmp/cookies http://127.0.0.1:3000/ | grep -q create-room")

    machine.succeed("systemctl restart sameframe.service")
    machine.wait_for_open_port(3000)
    machine.succeed("cmp /tmp/original-token /var/lib/sameframe/access-token")
    machine.succeed("curl --fail -s -b /tmp/cookies http://127.0.0.1:3000/api/access")

    machine.succeed("/run/current-system/specialisation/public/bin/switch-to-configuration test")
    machine.wait_for_unit("sameframe.service")
    machine.wait_for_open_port(3000)
    machine.succeed("curl --fail -s http://127.0.0.1:3000/ | grep -q create-room")
    machine.succeed("test -d /var/lib/sameframe-public && test ! -e /var/lib/sameframe-public/access-token")
    machine.succeed("cmp /tmp/original-token /var/lib/sameframe/access-token")
  '';
}
