# omp presentation and account integration

Reference: omp 18.16.0 RPC implementation, Tern 0.9.12 screenshots and
semantic-renderer experiment, and Libri's existing shared chat components.

## Initial rendering gaps

| Capability | Libri before this branch | Work required |
| --- | --- | --- |
| Composer chips | Model, effort, permissions and attachments already present | Retain shared composer |
| Thinking | Foldable reasoning, including streaming | Detail setting |
| Tool cards | Foldable input, edit diffs, output, errors and elapsed time | Full mode opens individual cards |
| Partial output | Streaming shell output already present | Keep visible in Full |
| Inline images | Tool pictures copied to outbox and displayed | Verify at phone width |
| Todo / plan | omp todo events translated to TodoWrite | Verify shared renderer |
| Subagents | Lifecycle, nested calls and reports already translated | Verify shared renderer |
| Context | Memory health and subscription meters, no model context meter | Carry actual request context separately from cumulative usage |
| Speed | No per-turn token/s display | Carry output usage and measured generation duration |
| Detail | One fixed rendering | Simple / Standard / Full |

## Account probes

Real omp processes used isolated HOME directories and named `--profile`s.
No sign-in was completed and no real credential store was accessed.

| Provider | Actual RPC behavior |
| --- | --- |
| ChatGPT / Codex device | Browser URL and device code |
| Kimi Code | Browser URL and device code |
| Kilo | Browser URL and code instructions |
| Copilot | RPC explicitly rejects initial interactive prompts |
| xAI | API-key dashboard, followed by credential input |
| GLM / zai | API-key dashboard, followed by credential input |
| Qwen | Requests copied OAuth token or API key |
| Meta | Meta Model API key dashboard; not Muse account login |

Muse's own `login --help` confirms Meta-account browser-code login. Keep
Meta subscriptions on Muse. Anthropic and Google subscription logins are
excluded from omp's Libri sign-in choices. No credentials are copied to
Libri: supported device flows run omp's own `login` RPC command.

Provider authorization completion requires the person's account; scratch
integration checks use an omp stand-in to prove URL/code delivery, state
polling, completion and rejection of excluded providers without signing in.

The device-login process disables extension and project discovery. omp
18.16 requires a model even before login on a fresh profile, so the login
process names a bootstrap model without ever prompting it. Remove that
selector when upstream allows login without model selection.

## Default routing

omp is first among general-purpose harnesses, including OpenAI models even
when Codex is installed. Explicit saved harness choices remain in force.
Claude stays on Claude Code; Meta account models stay on Muse. Google
subscription providers are not offered through omp; Google's API-key
provider remains available. Codex remains an explicit harness choice.

Validated with the catalog unit checks and a throwaway Libri catalog:
omp default, OpenAI offered through omp with Codex installed, restricted
login providers absent, Claude Code present; desktop and phone captures.

## Chat detail

Settings → Chat → Chat detail has Simple, Standard and Full. The browser
keeps this display preference, shared by main chat, Temp and worker threads
regardless of harness. Standard preserves the existing folds. Simple hides
all tool calls, results, diffs, todo and subagent cards, thinking and progress
narration. It shows only user messages and final assistant text, with one
quiet working indicator during a turn. Replies appear when the turn completes
so text that later becomes progress narration never flashes into view. Full opens individual thinking and tool
cards, including inputs, diffs, errors and output. A person can collapse an
individual card in Full.

Browser proofs exercise a streamed Claude-shaped trace through the common
renderer at 1440 and 390 pixels, switch all three modes through Settings,
check visible output and thinking, persisted choice and horizontal overflow.
Native Android's separate renderer is not changed by this browser setting.

## Full rendering and telemetry

Native omp edit results now retain their structured diff, per-file changes
and diagnostics. Tool image blocks render directly inside their cards,
including Temp and worker threads; they do not depend on a separate file
being sent after the answer. Inputs remain visible for omp's different edit
formats. Existing partial output, todo phases and subagent lifecycle/report
rendering are retained.

omp's `get_state` supplies actual context tokens, context window and output
tokens per second. Full shows these below the answer with a context bar.
Cumulative billed input/output is labelled separately. Unknown measurements
stay absent. The same usage row works for other harnesses that report token
usage; no generation speed is invented for harnesses that do not report it.
Statistics replies do not count as model progress in the idle monitor.

The RPC browser proof uses a deterministic omp stand-in, not a paid model:
thinking, native edit diff, partial shell output, todo phase, a subagent's
report, a decoded image, context 32K / 128K, 42.5 tok/s and token totals,
checked at desktop and phone widths. Real model and completed account-login
smokes remain for the maintainer's account after branch approval.

## Remaining limits

- This branch changes the web client, including phone web. Android's
  separate native renderer and settings are not part of this branch.
- Copilot and credential-entry providers use omp's terminal login; this
  omp release refuses those interactive/secret prompts through RPC.
- Structured native diffs, images and measurements are live details. Old
  memory rows keep their existing bounded text; they cannot recover rich
  metadata discarded by earlier versions.
- Existing output limits remain. Full controls presentation, not the
  harness's or memory store's output retention.
- Extension widgets and custom semantic renderers are not translated;
  Libri retains its own Pages and approval interface.

## Review captures

These are browser screenshots of the scratch app and deterministic harness
fixtures. Synthetic isolation markers in user bubbles are masked.

- [Device sign-in on phone](omp-parity-captures/signin-phone.png)
- [Simple on phone](omp-parity-captures/detail-simple-phone.png)
- [Standard on phone](omp-parity-captures/detail-standard-phone.png)
- [Full native edit diff on phone](omp-parity-captures/omp-diff-phone.png)
- [Full image, subagent and metrics on phone](omp-parity-captures/omp-full-phone.png)
- [Full on desktop](omp-parity-captures/omp-full-desktop.png)

Validation: `./check_changes.sh` passed. Rust: 469 passed, 1 ignored.
Scratch browser/API proofs: `nix/check_omp_signin.py`,
`nix/check_omp_catalog.py`, `nix/check_chat_detail.py`, and
`nix/check_omp_rendering.py` all passed. Run browser proofs with the
Playwright environment used by `nix run .#ui-driver`; the rendering proof
also uses Pillow, already included there.

Simple also hides pre-tool narration stored as separate messages, both live
and after history reload. This has reducer checks and a browser reload
check on desktop and phone. The final web suite passes 55 tests.

Simple answer-only follow-up: `check_chat_detail.py` exercises live and reloaded
main chat; `check_simple_threads.py` exercises live and reloaded Temp and worker
threads, preserving earlier final replies. Both cover desktop and 390px.
Standard and Full keep their existing presentation.
