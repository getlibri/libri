# Libri settings

Generated from `libri/src/settings.rs` and `libri/src/host.rs`; `cargo test` fails while this page differs.

## In the app

Host settings are kept on the host, so every device on it shares them; device settings stay on each device.

| Setting | Where | What it does | Takes | Default | Changed by |
|---|---|---|---|---|---|
| `model` | host | The main chat's model, and with it its harness, from the next turn. | a {harness, model} of GET /api/models | the default harness's default model | POST /api/model {harness, model} |
| `effort` | host | How hard the main chat's model thinks, from the next turn. | one of the levels GET /api/models lists for the model | the harness's own default | POST /api/effort {effort} |
| `access` | host | How freely the main chat's harness acts: its own permission mode and sandbox. Holds only on the harness it was picked on. | the modes GET /api/models lists for the harness | the harness's default (access.defaults) | POST /api/access {access?, sandbox?} |
| `access.defaults` | host | What each harness starts on, by harness id. | per harness, one of its modes and sandboxes | each harness's own default | POST /api/access/default {harness, access?, sandbox?} |
| `harnesses.off` | host | The harnesses switched off; one off is offered nowhere. | harness ids installed here, one per line | the ones off at first | POST /api/harness {harness, on} |
| `harness.default` | host | The harness a new chat and a new worker run on. | an installed harness that is switched on | the deployment's harness.default | POST /api/harness {harness, default: true} |
| `harness.models` | host | Each harness's default model, by harness id. | per harness, one of its models in GET /api/models | each harness's own default | POST /api/harness {harness, model} |
| `others` | host | The harness that runs other makers' models. | an installed harness that reaches other makers | the first that does | POST /api/others {others} |
| `temp.model` | host | Temp's model, apart from the main chat's. | a {harness, model} of GET /api/models | the main chat's | POST /api/model {harness, model, temp: true} |
| `temp.effort` | host | How hard Temp's model thinks. | one of the levels GET /api/models lists for Temp's model | the harness's own default | POST /api/effort {effort, temp: true} |
| `temp.memory` | host | Whether Temp reads the memory. Off, it holds the web tools alone and no rules. | on, off | on | POST /api/scratchpad/memory {on} |
| `worker.model` | host | The model a new worker starts on. | a {harness, model} of GET /api/models | the main chat's | POST /api/worker-model {harness, model} |
| `worker.names` | host | The names new workers are drawn from. | a list of names | Libri's own list | POST /api/worker-names |
| `synopsis.model` | host | The model that writes a chat's title and a worker's one-line job. | a {harness, model} of GET /api/models | a small model on the default harness | POST /api/synopsis {harness, model} |
| `image.model` | host | The model that makes pictures. | one of GET /api/capabilities's image choices | the first on offer | POST /api/capabilities {capability: "image", provider, model} |
| `voice` | host | The voice replies are read aloud and calls held in. | one of GET /api/voices | the deployment's x.voice | POST /api/voice {voice} |
| `forget` | host | How long a Libri that forgets keeps what is said, in minutes. Only where the deployment sets tools.forget. | 10, 30, 60, 180, 720, 1440, 4320, 10080 | the deployment's tools.forget | POST /api/forget {minutes} |
| `compact.at` | host | How large a chat's session grows, in tokens, before its harness compacts it: the main chat, Temp, memory chats and workers alike (Claude Code and Codex; omp, Pi and OpenCode keep their own rule). | a whole number of tokens, 20000 or more | the deployment's compact.at, else 200000 | POST /api/compact {tokens} (null for the default) |
| `usage.shown` | host | Whether every composer shows the subscription usage rings. | on, off | on | POST /api/usage {shown} |
| `rules` | host | The person's rules (AGENTS.md), added to every prompt but memory-off Temp's. | text up to 64 KB | none | POST /api/guide {text} |
| `host_facts` | host | What is true of this host alone (HOST.md), added to every prompt after the person's rules, but a named memory's and memory-off Temp's. | text up to 64 KB | none | POST /api/guide/host {text} |
| `theme` | device | Light or dark. | light, dark; on Android also system | light on the web, system on Android | the theme switch in Settings |
| `text.size` | device | The talk's text size; the page around it keeps its own. | 13 to 29 px on the web (its pinch, 0.8 to 1.8 times 16 px); 12 to 24 px on Android | 16 px | pinch, or the size stepper in Settings |
| `voice.speed` | device | How fast the assistant speaks read aloud and on calls. | 0.5 to 3 in steps of 0.1; a call takes 0.7 to 1.5 | 1 | the speed control in Settings |
| `sides` | device | The width of each side column on a wide screen. | 240 to 480 px each | 390 px | drag a column's edge |
| `pins` | device | The chats and pages pinned, and whether the pinned list shows. | a list; shown or hidden | none; shown | pin or unpin |
| `hosts` | device | The hosts this device knows and the one open. | host addresses | the host that served the page | Settings, Hosts |

## The deployment's config

TOML, read from `--config FILE`, else `$XDG_CONFIG_HOME/libri/config.toml`. Every key is optional; an unknown key is an error. A setting already in the environment wins over the file.

| Key | Environment | What it does |
|---|---|---|
| `name` | LIBRI_NAME | What the chat calls the assistant where a deployment names it; unset, `Libri`. The host itself goes by the machine's hostname. No prompt names the assistant: who it is opens the person's rules, as their Identity section. |
| `app` | LIBRI_APP | What the installed app is called; unset, the name. |
| `person` | LIBRI_PERSON | Who it works for; unset, `$USER`. |
| `state` | STATE_DIRECTORY | Where the host keeps its state; unset, `$XDG_STATE_HOME/libri` (`libri-NAME` for `--instance NAME`). |
| `style` | LIBRI_STYLE | A line on tone. |
| `workdir` | LIBRI_WORKDIR | The harnesses' working directory. |
| `daemon.listen` | LIBRI_SITE | The address it listens on; unset, `127.0.0.1:6780`. `--listen ADDRESS:PORT` sets it for one run, `--port PORT` its port alone. |
| `daemon.url` | LIBRI_SITE_URL | Its public address, for push and pairing links. |
| `daemon.site` | LIBRI_SITE_ROOT | The web app it serves; unset, the packaged one. |
| `daemon.pages` | LIBRI_PAGES | Where the pages the assistant publishes are kept; unset, `<state>/pages`. |
| `daemon.pages_url` | LIBRI_PAGES_URL | Where those pages are served. |
| `daemon.open` | LIBRI_OPEN | `true` lets every request in unpaired; unset, false. |
| `daemon.origins` | LIBRI_ORIGINS | Comma-separated origins of the app served elsewhere that may use this host (`https://app.libri.is`); unset, none. |
| `daemon.tailnet` | LIBRI_TAILNET | Comma-separated tailnet logins (`ada@example.com`) this host is for; `ada@example.com/box` names only that login's device `box`, for a machine shared by people its owner's login does not stand for. Set, each request's peer is looked up in the local tailscaled (`whois`), anyone else is refused, and those listed count as signed in with no pairing; loopback stays in. Unset, no tailnet gate. |
| `daemon.forward` | LIBRI_FORWARD | Comma-separated `login=http://address` pairs: a tailnet login whose requests, WebSocket upgrades included, pass whole to that person's own Libri, so one address serves each person their own. Needs tailscaled as `daemon.tailnet` does. |
| `daemon.tailscale_socket` | LIBRI_TAILSCALE_SOCKET | tailscaled's LocalAPI socket; unset, `/var/run/tailscale/tailscaled.sock`. |
| `connections.return_page` | LIBRI_RETURN_PAGE | The page a connection's OAuth sign-in returns through, which sends the browser on to the address it reached Libri at; unset, `https://libri.is/connect/back`. Empty: every sign-in returns to `127.0.0.1`, and another device pastes the address it ended on. |
| `hippo.dir` | HIPPO_DIR | The memory's directory; unset, `<state>/hippo`. |
| `hippo.serve` | LIBRI_HIPPO_SERVE | Whether this daemon runs that hippo as a child; unset, true when `hippo.dir` is left default. |
| `hippo.shared` | LIBRI_HIPPO_SHARED | When the hippo keeps other chats too: the title this daemon's chat goes under. |
| `memory.command` | LIBRI_MEMORY_COMMAND | The memory every turn recalls from: an MCP server on stdio, as a command line, handed to every harness beside Libri's own tools. Unset, `hippo2 mcp` when `hippo2` is on the PATH; empty, none. |
| `wiki.vault` | LIBRI_WIKI | The folder of the wiki, the assistant's notes: an Obsidian vault the `wiki` program serves beside hippo, written by the main chat and read by workers and Temp; unset, `wiki/` in the state directory. |
| `harness.default` | LIBRI_BACKEND | The harness a new chat and a new worker run on until the person picks one; unset, omp when found, else the first found. |
| `harness.model` | LIBRI_MODEL | The Claude model until the person picks one. |
| `harness.effort` | LIBRI_EFFORT | The thinking effort until the person picks one. |
| `harness.efforts` | LIBRI_EFFORTS | Comma-separated efforts offered where a harness has no list of its own. |
| `harness.coding` | LIBRI_CODING | `0` leaves Claude Code without its own shell and file tools, for an assistant with Libri's alone. |
| `harness.instructions` | LIBRI_INSTRUCTIONS | Read once, to fill an empty HOST.md (Settings > Agents, This host) when a release before 0.9.70 kept the host's facts here; nothing reads it after. Goes once no deployment sets it. |
| `harness.identity` | LIBRI_IDENTITY | Read once, to open the person's rules (AGENTS.md) with an Identity section when a release before 0.9.70 kept who the assistant is here; nothing reads it after. Goes once no deployment sets it. |
| `harness.mcp` | LIBRI_MCP | MCP servers added to Libri's own on every harness, in Claude Code's `mcpServers` shape. |
| `harness.mcp_withheld` | LIBRI_MCP_WITHHELD | MCP servers no turn may reach, for a host whose harness configs hold some for a person at a terminal; unset, none. |
| `harness.claude.command` | LIBRI_CLAUDE | Claude Code's command; unset, `claude`. |
| `harness.codex.command` | LIBRI_CODEX | Codex's command; unset, `codex`. |
| `harness.opencode.command` | LIBRI_OPENCODE | OpenCode's command; unset, `opencode`. |
| `harness.opencode.port` | LIBRI_OPENCODE_PORT | A fixed port for OpenCode's server; unset, any free one. |
| `harness.omp.command` | LIBRI_OMP | omp's command; unset, `omp`. |
| `harness.pi.command` | LIBRI_PI | Pi's command; unset, `pi`. |
| `harness.muse.command` | LIBRI_MUSE | Muse's command; unset, `muse`. |
| `harness.acp.command` | LIBRI_ACP | An ACP agent's command line. |
| `harness.http.url` | LIBRI_URL | A chat-completions endpoint. |
| `tools.forget` | LIBRI_FORGET | Minutes after which a chat is forgotten; set, the person picks among the lengths in Settings. |
| `tools.search_socket` | LIBRI_SEARCH_SOCKET | A Unix socket serving Parallel's search with the host's key added. |
| `tools.exa_socket` | LIBRI_EXA_SOCKET | A Unix socket serving Exa's search with the host's key added; set, search offers Exa. |
| `tools.brave_socket` | LIBRI_BRAVE_SOCKET | A Unix socket serving Brave's web search with the host's key added; set, search offers Brave. |
| `tools.usage` | LIBRI_USAGE_UNUSED | Accepted and ignored: Libri reads subscription limits itself. |
| `tools.mail` | LIBRI_MAIL | The mail accounts file. |
| `tools.imap` | LIBRI_IMAP | The IMAP server mail is read from. |
| `tools.smtp` | LIBRI_SMTP | The SMTP server mail is sent through. |
| `x.voice` | LIBRI_VOICE | The voice until the person picks one; xAI itself is connected in Settings, Connections. |
| `computer.run` | LIBRI_RUN | The address of the assistant's computer, for `run_code`; unset, no `run_code`. |
| `computer.runs` | LIBRI_RUNS | The directory that computer shares with Libri for each run's files. |
| `browser.enable` | LIBRI_AGENT_BROWSER | `true` gives agents Libri's own browser; unset, off. |
| `browser.chromium` | LIBRI_CHROMIUM | Chromium's command; unset, `chromium`. Named, it also renders a still of each Library page for its card. |
| `browser.xvfb` | LIBRI_XVFB | Xvfb's command, for the browser's hidden display; unset, `Xvfb`. |
| `browser.relay` | LIBRI_BROWSER_RELAY | The address of a DevTools relay to the person's own browser (an extension relay such as OpenCode Browse's, `http://127.0.0.1:19988`); unset, agents use Libri's browser alone. |
| `browser.relay_token` | LIBRI_BROWSER_RELAY_TOKEN | The file holding the token the relay asks for. |
| `browser.use` | LIBRI_BROWSER_USE | Where a task with the person's logins browses: `logins` (unset), their own browser while it is connected, else Libri's; `own`, Libri's only; `yours`, their own only. |
| `share.listen` | LIBRI_SHARE_LISTEN | Where board links are served (address:port), on a listener that serves them and nothing else; a public name points here. Off unless set. |
| `share.url` | LIBRI_SHARE_URL | The public address board links are made under, e.g. `https://share.example.net`; without it no link can be made. |
| `compact.at` | LIBRI_COMPACT_AT | The context size, in tokens, at which every harness compacts a chat's session (Claude Code's autoCompactWindow, Codex's model_auto_compact_token_limit); the person's pick in Settings goes first; unset, 200000. |
| `a2a.token_file` | LIBRI_A2A_TOKEN | Turns on A2A: a file of the bearer token other agents present and this one sends. Off unless set. |
| `a2a.peers` | LIBRI_A2A | Other agents this one may send to, `Name=url` each. |
