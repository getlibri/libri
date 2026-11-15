# wiki

An agent's notes, kept as an Obsidian vault and served over MCP.

`wiki serve DIR [--read-only]` speaks MCP on stdio with four tools:

- `read`: one note whole, or one section of it (`Ada#Work`).
- `list`: every note by folder and name, with its size and the day it last
  changed.
- `search`: every line and note name matching a case-insensitive regex.
- `write`: one note whole, made or replaced; empty text removes it. Not
  offered with `--read-only`.

There is no prompt of its own and no hook: the tool descriptions say what the
vault is for, and the model reads and writes when it decides to.

The vault is a plain folder kept by Obsidian's conventions, so the app opens
it as it is:

- A note is `Name.md`, in the vault or any folder of it, and goes by its name
  alone. Links are `[[Name]]`, `[[Name#Heading]]` and `[[Name|shown text]]`;
  a Markdown link to a note's file is refused. Since links find notes by
  name (without regard to case), no two notes share one: writing a new note
  under a name another folder already holds is refused.
- Properties are YAML frontmatter between `---` lines (`tags`, `aliases`,
  `created`, `sources` with the memory's message ids); `#tags` inline too.
- Attachments sit in `attachments/` and embed as `![[file.png]]`; the tools
  write notes only.
- `.obsidian/` is the app's: no note is read from or written there, nor from
  any hidden folder.

Each write goes through a file renamed into place, so Obsidian or a reader
never sees half a note. There is no rename or move: the tools that would
need to rewrite the links pointing at a note, as Obsidian does, are left for
when they are needed.

Libri runs it for each turn beside its own server (`libri/src/servers.rs`):
the main chat writes, workers, Temp and quiet turns read (`--read-only`),
and a memory-off turn or a blind worker gets none. The vault is
`wiki.vault` in the deployment's config, else `wiki/` in the state
directory.
