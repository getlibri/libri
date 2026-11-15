# Libri

Libri is an agentic user agent: what a browser is to the web, Libri is to AI
agents. It is one assistant that runs on top of whichever coding agents you
already have installed (Claude Code, Codex, OpenCode and others), with its
own memory, tools and workers that stay the same whichever agent and model
answers a given turn.

It is a small daemon you run on your own machine. It serves a web page and
talks to an Android app, and it gives the assistant:

- **Any harness, any model.** Each installed CLI is offered as it is, with
  your own login, settings, skills and MCP servers. Switch harness or model
  per chat.
- **Episodic memory.** `hippo` keeps every message and folds older ones into
  summaries, so the assistant remembers across chats and harnesses.
- **Notes.** `wiki` keeps the assistant's notes as an Obsidian vault, which
  it writes and searches when it sees fit and you can open in Obsidian.
- **Workers.** The assistant can start named sub-agents for longer jobs,
  each in its own chat, that report back when done or when they need you.
- **Approvals.** Tool calls that need a yes come to you as cards, on the web
  or your phone.
- **Watches and reminders**, published pages, voice, and Web Push.

## Status

Libri is early, used daily by one household. Expect rough edges.

| Area | State |
|---|---|
| Linux x86_64 and aarch64 | in use |
| Web page in Chromium-based browsers | in use |
| Android app | in use; build it yourself, no store release; connects by address with no pairing yet |
| Claude Code | in use |
| Codex, OpenCode, omp, Muse Code | supported, less tested |
| Pi | supported, off by default |
| Other ACP agents (Gemini CLI, Grok Build, ...) | supported through ACP, untested |
| Firefox, Safari | untested |
| iOS | no app; the web page on iOS Safari is untested |
| macOS, Windows | untested; the daemon is built and tested on Linux only |

## Install

Libri needs Linux and at least one harness CLI installed and signed in as the
user Libri runs as. No harness comes with it.

### With Nix

    nix run github:getlibri/libri            # listens on 127.0.0.1:6780
    nix run github:getlibri/libri -- pair    # prints a one-time sign-in link

### Without Nix

Build the release tarball on any machine with Nix, then install it on the
target, with no root and nothing else needed:

    nix build .#bundle                 # or .#bundle-aarch64
    sh install.sh result/libri-*-linux-x86_64.tar.gz
    libri service install              # a systemd user unit, started now and at login
    libri pair

`install.sh` also takes a URL, and puts `libri`, `hippo` and `wiki` under
`~/.local/share/libri` with links in `~/.local/bin` (`LIBRI_INSTALL_DIR`
changes the first). `libri service remove` takes the unit away again.

### On NixOS

    inputs.libri.url = "github:getlibri/libri";
    # in a configuration:
    imports = [ libri.nixosModules.default ];
    services.libri.instances.libri = {
      settings.daemon.listen = "100.101.102.103:6780";
      harnesses = [ pkgs.claude-code ];
      # agentBrowser.enable = true;  # Libri's browser, with Chromium and Xvfb
    };

Pair with `sudo -u libri libri --config /etc/libri/libri.toml pair`. Each
instance is a systemd unit of that name with its own user, state directory
and config file.

### Port and a second instance

    libri --port 6790                       # this run on another port
    libri --listen 0.0.0.0:6780             # or another address and port

A second Libri for the same user, with its own config, state and memory,
takes a name with `--instance`:

    libri --instance work --port 6781                     # run it
    libri --instance work --port 6781 service install     # or as its own service
    libri --instance work pair                            # pair a device with it

`--instance work` reads `$XDG_CONFIG_HOME/libri-work/config.toml`, keeps its
state in `$XDG_STATE_HOME/libri-work` and installs as `libri-work.service`;
with no instance, everything stays under `libri` as before. Give each
instance its own port, here or as `listen` in its config: the flags win over
the environment and the file. On NixOS, each entry of
`services.libri.instances` is one already.

## Signing in

`libri pair` prints a one-time link, good for ten minutes. Open it on the
device: it trades the secret in the link for a token of its own.
`pair --read` makes a link for a device that may look but change nothing;
`libri devices` lists paired devices and `libri revoke ID` unpairs one.
Other clients send the token as `Authorization: Bearer`.

A fresh install answers on its own machine only. To reach it from elsewhere,
set `listen` under `[daemon]` (say `"0.0.0.0:6780"`) and restart; `pair` then
prints a link for each address. The Android app has no pairing yet and needs
`open = true`, which lets in everyone who can reach the port, terminal
included: set that only on a private network such as a tailnet.

## The app from anywhere

The page is the same whether the host serves it or it is served on its own,
as at app.libri.is: on its own it opens empty, and Settings adds each host
by its address, kept in that browser, as Paseo's app adds a daemon. A bare
address (`100.64.0.7`, `box`) means the host's plain-http daemon on port
6780. The host must listen where the browser can reach it and let the page
in:

    [daemon]
    listen = "0.0.0.0:6780"
    origins = "https://app.libri.is,http://app.libri.is"

Keep that port to your tailnet (or LAN) with the firewall: nothing becomes
public, the page is fetched from the app's address, and every call after
goes from the browser straight to the host. The app is served both ways
because browsers differ: Chromium reaches a private or tailnet address only
from the https copy, after asking once whether the page may reach devices
on your network; Safari refuses a plain-http address from an https page, so
it uses the http copy, which has no microphone or notifications. A copy that
cannot reach the host says so and links to the other, the host list along.
A pairing token, if the host needs one, stays in that browser.

## Configuration

TOML, read from `--config FILE`, else `$XDG_CONFIG_HOME/libri/config.toml`.
Every key is optional; an unknown key is an error. A setting already in the
environment wins over the file.

    name = "Libri"
    person = "Ada"
    [daemon]
    listen = "0.0.0.0:6780"
    url = "https://box.tailnet.ts.net"
    [harness]
    default = "claude"

Every key, what it does and its default: [`libri/docs/settings.md`](libri/docs/settings.md),
alongside the settings changed in the app. A running host serves the same
page at `/docs/settings.md`, and the assistant reads it, with which keys the
host sets, through its `settings` tool. The page is generated from
`libri/src/host.rs` and `libri/src/settings.rs` (`libri docs`); a test fails
while it differs.

## Browser

The browser is off unless `browser.enable = true`, and then needs Chromium
and Xvfb: on NixOS set the instance's `agentBrowser.enable` and the
module brings both; elsewhere install them from the system's packages
(`chromium` and `xvfb` or `xorg-x11-server-Xvfb`). Off, it costs nothing:
neither is installed or started. On, the daemon starts a Chromium of its own
when an agent first asks for a page, hidden on a private virtual display, and
stops it after 15 minutes unused. Agents get `browser_open`, `browser_look`,
`browser_click`, `browser_type`, `browser_press`, `browser_scroll`,
`browser_hand_over` and `browser_logins`. The chat draws their calls as a live card: the page as
it is now, and Take control, which hands the browser to the person (for a
sign-in, a code, a payment) until they hand it back. An agent's
`browser_hand_over` shows as Waiting, with a push.
On a phone the page is laid out at the phone's size while the person holds
it. The Android app opens a call's card full screen from its Open, by the
site's `#card=<tool>` page.

DevTools runs over a pipe, so the browser has no port; agents reach it by a
socket in the state folder, the person through the signed-in page.

One browser, one profile (`<state>/browser/profile`), as in ChatGPT Atlas.
The profile keeps what the person signs in to, but an agent browses without
those logins: its tab runs in a throwaway browser context that sends none of
the profile's cookies and keeps nothing, so it may open any public site. When
a task needs a signed-in site, the agent asks with `browser_logins`, naming
the site and why; the card asks the person, and Allow adds the site to the
list (`<state>/browser/sites.json`) and restarts the browser with the logins
loaded, for that agent alone. A login the person lacks they make through
Take control, and it is kept. While the logins are loaded the daemon refuses
to load any page off the list (in any tab or frame), other agents wait, and
handing control back from a page off the list blanks it. The agent gives the
logins up with `browser_logins` `done`; the person removes sites, or ends the
use, from the card; the browser going idle ends it too, and the next task
asks again. Sites that refuse automated browsers (Google's sign-in among
them) refuse it here as well. Cookies sit unencrypted in the state folder,
readable only by the daemon's user.

## Other apps

Libri is also an MCP server for other apps: Claude (its custom connectors,
on the web, desktop and phone), ChatGPT, Claude Code, Codex, Cursor or any
MCP client adds it as a connector. The daemon serves it at `/mcp`
(streamable HTTP); each app comes in as a connector of its own, which the
person names, lets use some modules of tools, and revokes, in Settings >
Connections > Other apps.

An app signs in as the MCP authorization spec has it: `/mcp` answers 401
with its protected resource metadata, Libri is its own OAuth 2.1
authorization server (client ID metadata documents, or dynamic
registration), and the person allows the app on Libri's own consent page,
behind the same gate as every page. An app that only takes a header gets a
key instead, made in Settings and shown once:

```sh
claude mcp add --transport http libri http://<host>/mcp                 # signs in by OAuth
claude mcp add --transport http libri http://<host>/mcp \
  --header "Authorization: Bearer <key>"                                # or with a key
codex mcp add libri --url http://<host>/mcp
```

An app that starts a command rather than reaching an address runs `libri
mcp` on the host, which relays stdio to the daemon, signed in by a key in
`LIBRI_CONNECTOR_KEY`; given a URL (`libri mcp http://<host>`) it relays to
another host.

The modules, each switched on per connector:

- Memory: `memory_recall`, `memory_open`, read only: hippo2's recall and
  open over the person's main memory, offered where the memory is hippo2.
- Workers: `workers_list`, `worker_read`, `worker_start`, `worker_tell`. A
  worker started or told here is told it was by the app, not the person.
- Chats: `chats_list`, `messages_read`, `events_wait` (new messages from a
  cursor on, waiting up to five minutes), `asks_list` (what waits on the
  person, to see), and `message_send`, which lands in the main chat as a
  message from the connector, never as the person's own word.
- Web, X, pictures and pages: `web_search`, `x_search`, `make_image`,
  `show_page` and `pages`, each only where it has something behind it.

Nothing offered answers for the person, approves, deploys, merges or
releases; shell, files, code, the browser and the connections' secrets are
never offered.

## Harnesses

`libri/src/model.rs` defines the `Harness` trait; each adapter maps its
harness's events onto the same steps, which `agui.rs` turns into AG-UI.
Every harness found installed is offered: Claude Code, Codex, OpenCode, omp, Pi and Muse Code,
each by its usual command on PATH or the path its `harness.*.command` names.
LIBRI_BACKEND picks the default, omp when unset and installed; the person switches among them with
`/harness` on the page. The main chat's model is the person's
pick; a worker takes it at spawn, or the model `spawn` names, and holds its own.

Settings > Models > Harnesses switches each of those on or off, picks the
default harness and each harness's default model. Only harnesses switched on
show in the pickers; one stays on. Pi starts switched off, unless it is the
host's default; switching it on holds. A harness on but not signed in shows
greyed, with how to sign it in, and a pick of it is refused rather than
failing a turn.

Each harness runs as its own install has it: the person's login, settings,
skills, plugins, MCP servers and repository instructions all hold, and a turn
adds Libri's tools, instructions and memory through the harness's own flags,
with the harness's memory off. Only hippo's summaries bypass all that.

Muse Code's turns keep their sessions in Libri's state (`muse-data`), not
in `~/.local/share/muse`, and Libri removes each when its turn ends: Muse picks
a permission profile only on a host that keeps sessions, and hippo reads the
person's. Muse's own skills and plugins under `~/.local/share/muse` are
therefore not loaded on Libri's turns; its login and settings are.

Pi never asks before a tool runs, so its modes are tool sets: Read only
(`--tools read,grep,find,ls`) and All tools, Pi's own default.

OpenCode must be version 2 (`curl -fsSL https://opencode.ai/v2/install | bash`).
The 1.x line, which `opencode.ai/install` still serves, has no `models --server`
and offers no models through its `/api` server.

`libri harness-check` runs one fixed exchange through the configured
harness and reports what it carries (text, thinking, tool calls, results,
approvals, resume, cancel, usage).

`nix/check_fresh_home.py` proves a fresh install: each harness unmodified, in a
HOME that holds only its login, through real turns, tools, workers, memory, an
over-long turn that compacts natively and the message after it. It spends real
model calls, so it stays out of `nix flake check`.

## Standards

| Layer | Standard | State |
|---|---|---|
| Server to harness | Native adapters (Claude Code stream-json, Codex app-server, OpenCode server, omp rpc-ui, Pi rpc, Muse Code MSP) and ACP for the rest (Grok Build, Gemini CLI…) | in use |
| Server to screen | AG-UI events and interrupts | in use |
| Tools to every harness | MCP | in use |
| Tool cards on the page | MCP Apps (`io.modelcontextprotocol/ui`), web; Android planned | in use |
| Agent instructions | AGENTS.md | in use |
| Notifications | Web Push (VAPID) | in use |
| Sign-in | Device pairing (one-time link, revocable bearer tokens) | in use |
| Passkeys | WebAuthn | planned |
| Skills | SKILL.md | planned |

## Build from source

    nix build .#libri          # or .#daemon, .#hippo, .#wiki, .#site on their own
    nix build .#bundle         # the release tarball
    ./check_changes.sh         # the checks covering what changed since origin/main
    nix flake check            # every check, including end-to-end runs of the release build

The Android app: `nix develop .#android`, then `gradle assembleDebug` in
`android/`.

Layout:

- `libri/`: the daemon and its web page (`libri/site`).
- `hippo/`: episodic memory; see `hippo/README.md`.
- `wiki/`: the notes' MCP server; see `wiki/README.md`.
- `client/`, `android/`: the Android app and the Rust client core under it.
- `nix/`: the module, packaging and end-to-end checks.

## License

Apache License 2.0; see `LICENSE` and `NOTICE`.
