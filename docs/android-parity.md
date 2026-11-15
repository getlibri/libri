# Android chat parity

Based on `omp-parity` 07c1754 with `chat-scroll-foot` e0a63ae merged.
This is a branch for review. No release version, APK publication or deployment.

## Behaviour

- Settings → Chat → Chat detail: Simple, Standard and Full. The preference
  stays on the device and covers main chat, Temp and worker threads for every
  harness. Simple shows user messages and completed final replies, with one
  quiet working indicator. It hides narration, thinking, tool calls/results,
  diffs, todos and subagent details. Standard retains existing folds. Full
  initially opens the individual thinking/tool cards; they remain foldable.
- Full retains original tool arguments and renders native omp edit diffs and tool images, todos, subagent reports,
  measured context tokens/window and output tokens per second. Unknown
  measurements are omitted. Android uses the shared Rust reducer and existing
  Compose components, theme, Coil and JSON dependencies.
- Live presentation survives history acknowledgements by stable row and call
  identity. Temp and worker logs carry call IDs, so repeated identical calls
  cannot receive each other's images or diffs. Temporary chat metadata is
  selected by its actual thread ID.
- Settings → Models starts omp's ChatGPT, Kimi Code and Kilo device flows,
  displays the browser code and polls completion. Credentials stay in omp.
  Waiting sign-ins resume when that settings page is reopened. Copilot, xAI,
  GLM and Meta retain terminal instructions; Meta subscriptions use Muse.
  No Anthropic or Google subscription login is offered through omp.
- Reversed chat lists release temporary held height when a turn finishes and
  when opened idle. Returning or reloading cannot retain the previous run's
  blank space beneath the reply. An active reader remains unpinned.

## Verification

`./check_changes.sh` passed, including Rust tests, clippy, rustfmt and web
checks. `nix develop .#android --command bash -c 'cd android && gradle
:app:assembleDebug --console=plain'` built both arm64 and x86_64 libraries
and the debug APK.

`nix/check_android_parity.py` drives a debug APK on a prepared emulator,
using throwaway Libri/hippo state and an omp RPC fixture. It uses no real
model, account, credential store or memory. Run inside the Android emulator
shell with a built server package, debug APK and emulator serial. Captures
are in `android-parity-captures/`.

The emulator parity driver passed main, Temp and worker coverage in every requested mode, all three scratch device flows, and long-chat idle reload/return. Focused final checks also passed preference selection through the UI and persistence, original Full-mode arguments, and return during an active turn.

The core regression tests cover live/reloaded narration, rich results and
metrics, acknowledgement preservation and isolation of worker metadata.

## Limits

- Rich results and measurements heard live remain available in that app
  session. Restarting the app cannot recover metadata discarded from old
  bounded-text history. This matches the web branch's limitation.
- Extension widgets and custom omp semantic renderers are not translated.
- Completed real-provider authorization requires the account holder; the
  emulator signs in only against a deterministic scratch fixture.
- The emulator is Android 16 x86_64. Physical phone behaviour and a signed
  release update still require release verification. No APK is published.

## Captures

- [Simple](android-parity-captures/simple.png), [working quietly](android-parity-captures/simple-live.png), [reloaded](android-parity-captures/simple-reloaded.png)
- [Standard](android-parity-captures/standard.png)
- [Full image and metrics](android-parity-captures/full.png), [earlier tools and diff](android-parity-captures/full-older-0.png)
- [Full Temp](android-parity-captures/full-temp.png), [Full worker](android-parity-captures/full-worker.png)
- [Temp reloaded in Simple](android-parity-captures/simple-temp-reloaded.png), [worker reloaded in Simple](android-parity-captures/simple-worker-reloaded.png)
- [Device sign-in](android-parity-captures/signin-chatgpt.png), [completed](android-parity-captures/signin-done.png)

- [Full original arguments](android-parity-captures/full-arguments.png), [detail settings](android-parity-captures/detail-settings.png), [return during a turn](android-parity-captures/simple-active-return.png)
