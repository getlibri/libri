# Outside apps as connectors

Libri's `/mcp` lets any MCP client in as a connector, as Claude's and
ChatGPT's custom connectors, Claude Code, Codex and Cursor expect.

## How it follows the spec

MCP authorization, revision 2026-07-28:

- A request with no token gets a 401 whose `WWW-Authenticate` names the
  protected resource metadata (RFC 9728) at
  `/.well-known/oauth-protected-resource/mcp`.
- Libri is its own authorization server (RFC 8414 metadata at
  `/.well-known/oauth-authorization-server`), as Linear's, Notion's and
  GitHub's hosted MCP servers are.
- Apps identify themselves by Client ID Metadata Document, the spec's first
  choice, or by dynamic registration (RFC 7591), which the spec deprecates
  but most apps still use.
- Codes need PKCE (S256), tokens are bound to `resource` (RFC 8707), and the
  browser goes back with `iss` (RFC 9207).
- Access tokens last an hour. Refresh tokens rotate on every use. Tokens
  are stored only as hashes.

## Where it departs

- The consent page is the person's own host, behind the gate every page
  sits behind (the tailnet, or a paired device). There is no account to sign
  in to. A ticket held server-side means a form posted from another site
  cannot let an app in.
- An app that only takes a header gets a key made in Settings, shown once,
  as GitHub's server takes a personal access token.
- `/mcp` accepts only connector tokens. A tailnet identity, a session or a
  device token is not a sign-in there, as the spec requires of a resource
  server.
- Tailscale's OAuth apps (alpha) could serve as the authorization server.
  They were not used: they sign in tailnet members only, so they do nothing
  for apps outside the tailnet.

## Modules

A connector's tools come in modules, picked at consent or in Settings. A
change takes effect on the connector's next call. The modules are memory,
workers, chats, and web/X/pictures/pages; a new connector starts with
memory and workers.

No tool answers, approves, deploys, merges or releases for the person.
`ask_answer` was removed because it answered as the person. Anything an app
sends lands under the connector's name:

- `message_send` arrives as a message note, as A2A messages do.
- `worker_start` and `worker_tell` say which app the words came from and
  that it is not the person.

## Pictures

`nix/drive_other_apps.py` (web) and `nix/drive_android_other_apps.py`
(emulator) make the pictures in `outside-door-captures/`.
