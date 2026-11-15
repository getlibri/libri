# hippo

Episodic memory for an AI agent: every chat it has ever had, kept word for
word, and folded into one fixed-size summary that fits in its context.

hippo follows an agent's transcripts as they are written, logs each message,
and keeps a binary tree of summaries over the log. The tree's frontier is the
**view**: the whole history as one block of `id+n|text` lines within a byte
budget. Recent lines cover one message each, older lines cover more. The
agent reads the view at session start, and drills into any line with `zoom`.
Nothing is ever forgotten; it only gets further away.

The design is Victor Taelin's [OptChat](https://gist.github.com/VictorTaelin/91837951a5ce5b38f341ec1ba1df6449).
hippo is one memory with two modes:

- **Chat mode** (`hippo serve chat`) is OptChat's own shape. A chat agent
  logs each message over the socket, and the view is its whole context:
  every turn is the view plus the new message. This suits an assistant,
  whose turns are far apart and which needs the gist more than the exact
  words. It records what the agent logs and nothing else: Libri runs its
  own hippo this way unless told to record all (`hippo.record`).
- **Code mode** (`hippo serve code`) puts the same memory beside the coding
  harnesses you already use. The harness keeps its own raw context and its
  own compaction; hippo covers everything before it, across every session.
  It records every session the harnesses save under the service's home,
  whoever started it: Libri's `hippo.record = "all"`.

A subagent forks the memory. Each of Libri's workers takes the view as it
stands at its spawn and goes on alone under `workers/<id>` in the store: a log
that follows the worker's own, with ids going on from main's, the same tree,
views and compactor. A run reads only that view, and the two never resync. Its
messages never enter the main memory, only its report does.
`src/memory/work.rs` holds what is particular to it, which is not much.

The memory (`src/memory`: log, tree, compactor, view, socket) is the same in
both. Code mode adds `src/code`, which reads the transcripts and turns them
into messages.

Code mode never replaces Claude Code, Codex or OpenCode:
it reads the transcripts they write, builds the tree beside them, and hands
the view back as a file they load as a rule, so a session opens with the
whole history already in context and no tool call. The compactor runs
through the `claude` CLI on a plain subscription, so there is no API bill;
any chat-completions endpoint works as well.

It is about 7,000 lines of Rust with seven dependencies and has been the
memory of a household of agents since 2026. It is developed as part of
Libri, whose daemon runs it as a child process.

## How it works

**Recording.** In chat mode the agent logs each message with the socket's
`log` command (kinds `user`, `talk`, `note`). In code mode a watcher follows Claude Code, Codex, Muse Code and OpenCode transcripts
(OpenCode through its SQLite database) and appends each message to an
append-only day-file log. Secrets of known shapes (API keys, tokens, private
keys, named passwords) are masked before they are written. Nothing else is
dropped; tool calls and results are kept too.

**Compaction.** Messages are the leaves of a binary tree. Each message is
compressed to a line of at most 512 bytes; each pair of lines is merged into
one line covering both, and so on upward. Lines are tagged by kind (`user`,
`talk`, `tool`, `echo`, `note`). A node, once built, is never rewritten.
The compactor calls a model with the compactions' own view (so calls share a
cached prefix), a ruler of 512 dashes for scale, and the step. When a merge's
two halves come from different chats, the step says so, so the model does not
read one chat as a reply to the other.

**The view.** Built nodes tile the log, oldest first, under a 128 KB budget
(about 60k tokens). It changes only by appending each new message and
merging the most due pairs; a merged part never splits, so consecutive views
share all but their end and stay cached. Merges come in batches: past the
budget the view folds down to 64 KB, then only grows.

A batch merges only pairs whose parent is built, so while the compactor is
paused or behind, the view outgrows its budget. What it prints and publishes,
and the context sent to the compactor (32 KB), never do: past the budget the
oldest lines give way to the fewest tree nodes covering them, each shown by
its summary or, if unbuilt, as `0+1024|(older history, past the view's
budget: zoom it or search)`; `zoom` opens either.

While `state/view-override.md` exists in the store, `view.md` holds its text
instead of the view, from the next start or message on; a repair uses it to
keep the history out of every new context. Like a pause, it outlives
restarts, and `status` names it.

**Reading.** `hippo view` prints the view; `zoom <id> <n>` opens a line into
its two halves, down to the whole message; `search <regex>` reads every
message word for word; `date <id>` says when. The service answers over a
unix socket in a few milliseconds.

## Running it

```
hippo serve chat|code         the service, and the only writer
hippo view                    the whole history as one-line summaries
hippo zoom <id> <n>           open line id+n; n = 1 prints message id whole
hippo date <id>               date and time of message id
hippo search <regex>          search every message, word for word
hippo status                  what the service is doing
hippo pause / resume          stop or restart the compactor's model calls
code mode only:
hippo audit [YYYY-MM-DD]      check a day of the log against the transcripts
hippo import [--since DATE]   bootstrap an empty store from past transcripts
hippo replay <store> <from> <to>
                              rebuild a scratch store from a date range
```

The service reads its configuration from the environment:

| Variable | Meaning |
| --- | --- |
| `HIPPO_DIR` | the store: log, SQLite tree, socket, `view.md` |
| `HIPPO_ARCHIVE` | older transcripts, for `import` |
| `HIPPO_BACKEND` | `claude` (the `claude` CLI, on a subscription), `http` (any chat-completions endpoint, with `HIPPO_URL`, `HIPPO_KEY_FILE` and `HIPPO_HTTP_EXTRA`) `codex` (`HIPPO_CODEX`), or `libri`: each call goes to the Libri beside the store over `compactor.sock`, which runs it on its Claude harness; default `claude` |
| `HIPPO_KEY_FILE` | a file holding the `http` endpoint's bearer key |
| `HIPPO_MODEL`, `HIPPO_EFFORT` | the compactor's model and reasoning effort; default `haiku`, `high` |
| `HIPPO_LEVELS` | per-level overrides, `level=backend/model/effort` separated by spaces, say `0=claude/haiku/xhigh` |
| `HIPPO_VIEW` | the view budget in bytes, line labels included, default 128000 |
| `HIPPO_AGENT` | the agent's name, in the compactor's prompt; default Libri |

In chat mode the service follows nothing; the agent writes the log through
`log`, and a store nothing writes to (a replayed scratch store, say) is
served and compacted as it stands. In code mode `log` is refused: the
transcripts are the record, and a logged `user` line would put words in the
user's mouth.

`nix build` builds it; `nix flake check` runs the tests, clippy with warnings
denied, and rustfmt. `cargo build` works too.

## What was measured

- Five-minute prompt caching instead of one hour cut the cost per compactor
  call by about two thirds.
- Haiku, Sonnet, Opus and two OpenAI models were replayed over the same 512
  messages and judged blind, by level. Haiku at high effort matched Sonnet at
  compressing single messages, once a guard held each line to its message's
  kind, and invented at merges; hence `HIPPO_LEVELS`.
- A real history line as the scale example got copied into summaries as fact;
  so did an invented one. The scale is now a ruler of dashes.
- Chat labels on tree lines proved unreliable and were dropped for the
  session check on merges.

## License

MIT.
