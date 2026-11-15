# services.libri.instances on a machine with nothing else: two instances,
# each with the hippo it starts under a user of its own. A device pairs with
# the first, the page loads, one turn through a stand-in Claude reaches the
# first's memory once and the second's not at all.
{ pkgs, self }:
let
  claude = pkgs.writeScriptBin "claude" ''
    #!${pkgs.lib.getExe pkgs.python3}
    ${builtins.readFile ./stand_in_claude.py}
  '';
in
pkgs.testers.runNixOSTest {
  name = "libri-service";
  nodes.machine = {
    imports = [ self.nixosModules.default ];
    services.libri.instances = {
      libri.harnesses = [ claude ];
      second = {
        harnesses = [ claude ];
        settings.daemon.listen = "127.0.0.1:6781";
      };
    };
    systemd.services.libri.environment.STAND_IN_LOG = "/var/lib/libri/stand-in.jsonl";
    environment.systemPackages = [
      pkgs.curl
      pkgs.jq
    ];
    virtualisation.memorySize = 2048;
  };
  testScript = ''
    import json

    site = "http://127.0.0.1:6780"
    hippo = "sudo -u libri HIPPO_DIR=/var/lib/libri/hippo hippo"
    words = "the words of one full turn"

    for name, port in (("libri", 6780), ("second", 6781)):
        machine.wait_for_unit(f"{name}.service")
        machine.wait_for_open_port(port)
        machine.wait_until_succeeds(f"test -S /var/lib/{name}/hippo/hippo.sock", timeout=30)
        machine.succeed(f"pgrep -u {name} -x hippo")

    machine.succeed(f"test \"$(curl -s -o /dev/null -w '%{{http_code}}' {site}/api/devices)\" = 401")
    link = machine.succeed("sudo -u libri libri --config /etc/libri/libri.toml pair").strip()
    secret = link.split("#pair=")[1].split()[0]
    paired = json.loads(machine.succeed(
        f"curl -sf -H 'content-type: application/json' -d '{{\"secret\": \"{secret}\", \"name\": \"test\"}}' {site}/api/pair"
    ))
    bearer = f"-H 'authorization: Bearer {paired['token']}'"

    page = machine.succeed(f"curl -sf {bearer} {site}/")
    assert "<div id=\"root\"" in page, page[:300]

    run = json.dumps({"messages": [{"id": "m1", "role": "user", "content": words}]})
    machine.succeed(f"curl -sf {bearer} -H 'content-type: application/json' -d '{run}' {site}/api/run -o /dev/null")

    machine.wait_until_succeeds(f"{hippo} tail 200 | grep -q 'claude answers'", timeout=30)
    tail = machine.succeed(f"{hippo} tail 200")
    held = [json.loads(l) for l in tail.splitlines() if l.startswith("{")]
    assert sum(words in m["text"] for m in held if m["kind"] == "user") == 1, held
    assert sum("claude answers" in m["text"] for m in held if m["kind"] == "talk") == 1, held
    second = machine.succeed("sudo -u second HIPPO_DIR=/var/lib/second/hippo hippo tail 200")
    assert words not in second, second
  '';
}
