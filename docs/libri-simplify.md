# Libri simplification audit

Baseline: omp-parity 07c1754 plus chat-scroll-foot e0a63ae. Android excluded. No routes, tool names or stored formats change. Counts below are estimates until implementation; this is a bounded first pass, not a complete rewrite.

## Module map and extension costs

| Seam | Current interface and ownership | Cost to extend today |
|---|---|---|
| Tile | `tiles.tsx:16` Pane; binary split tree, persistence, drag and launcher; `App.tsx:338` supplies rendering | Add kind, name, icon, singleton policy, launcher/dock membership and App render case; server layout validation in `layout.rs:127`. At least three files. Keep layout independent of product rendering. |
| Harness | `model.rs:34` Harness; adapters produce model events; `rpc.rs:44` owns process protocol | Adapter plus model factory, host command discovery, catalog, sign-in/remedy and permission policy. Six or more modules. Protocol differences are real; provider policy should eventually be a descriptor, not a universal protocol wrapper. |
| Tool | `tools.rs:36` Tools and rmcp generated router; `servers.rs:24` scope determines access | Parameter schema, annotated method, scope policy and any backing domain module. Usually two or three files; the generated router already removes manual registration. |
| Page type | `library.rs` file discovery, `page_data.rs:104` versioned data; `apps.ts`/`card.tsx` MCP Apps runtime | HTML pages require no new host route; new native content kinds touch library classification and web dispatch. Prefer the existing sandboxed page extension before adding native types. |
| Memory | `hippo.rs:50` Hippo owns daemon connection; Keeper/Spool own recording; model calls consume view | Extend this client seam; do not make adapters know storage internals. |
| Worker | `task.rs` job operations; `lifecycle.rs:28,154` Phase and transition | Lifecycle is already explicit. Avoid replacing its enum with UI status flags. Legacy markers support rollback and cannot be deleted without a compatibility decision. |
| Temp | `scratchpad.rs`, web `scratchpad.ts`, App Temp presentation | State is shared across stream, reload and commands; working/stopping/id/queued are an implicit machine. Public wire format also serves Android. |
| Host/UI | `site.rs` HTTP dispatch; `App.tsx` product composition and settings | These are large ownership boundaries: 6527 and 5970 lines. Moving arbitrary fragments alone would produce shallow modules. Extract owned operations and state instead. |

## Ranked findings

1. **Repeated settings request lifecycle** — App.tsx:2105–2225 and 2401–2433. Four copies load optional resources, preserve old values on rejection, POST JSON, expose busy/error. Estimated 167 → 80 lines including a reusable hook; measured result is 101 net production lines removed. Two designs: (A) adopt TanStack Query, adding cache, invalidation and dependency policy; good when many shared caches need coordination, excessive for four uncached forms. (B) one resource hook owning request state and HTTP semantics; chosen. React hooks already exist; no new library needed. Preserve 404, error text, reload timing and setter return values. No pass-through per-resource hooks.
2. **Untyped sign-in transitions** — signin.rs:45–93. JSON patching permits arbitrary state/error combinations and spreads transition knowledge. Measured 159 → 197 production lines in signin.rs: 38 extra lines buy a private enum and named transition boundary. Waiting requires a device code; failure requires a reason. (A) serialized tagged enum as wire response: clean but changes optional field shape and risks Android. (B) private typed phase plus explicit projection to the existing JSON: chosen. Credentials remain with each harness. Do not silently fix the existing early-completion ordering race in a behaviour-preserving refactor.
3. **Settings ownership inside App** — same locations. Extracting the resource operation removes networking policy from composition; merely splitting each form into its own file would move complexity sideways. Included in item 1.
4. **Harness registration scattered** — model.rs:189, catalog.rs:677/690, host.rs:523, signin.rs:24/33. Candidate descriptor table for discovery, display and sign-in policy; protocol-specific factories remain explicit. Estimated 160 → 115 policy lines, requires broader adapter fixtures before implementation. Alternative giant Harness enum with every capability is tightly coupled; reject it. Defer in this pass.
5. **Layout synchronization implicit machine** — tiles.tsx:103–208: heard/version/sent/pending/byAgent/saveError. Separate synchronization state from split-tree editing. Estimated 105 → 100 lines plus tests. Reducer with explicit uninitialized/ready/saving states vs query library: reducer fits the custom long-poll/version semantics. Defer until concurrency fixtures cover conflicting local and remote edits; do not guess by rewriting effects.
6. **Temp state** — scratchpad.ts:1–100 and App Temp. working/stopping plus nullable id and queued text encode starting/running/stopping/idle. Model internally as a discriminated union, translate at HTTP boundary. Estimated 150 → 135 lines. Defer cross-platform changes until Android parity lands.
7. **Connection setup forms** — connections.tsx:94–150,326–350,393–450. busy/error/result flags and repeated POST handling. Shared settings resource is unsuitable for OAuth redirects and staged tests; separate setup reducer per flow rather than a universal form framework. Estimated 190 → 160. Defer to focused tests.
8. **Layout metadata duplication** — tiles.tsx:28/40/364/437; App render dispatch; server validation. One product tile descriptor may own labels/icons/cardinality; server still validates untrusted layout. Avoid coupling server schema to React. Small payoff alone, defer.
9. **Compatibility paths** — apps.ts:12 missing-api fallback; lifecycle.rs:1–12 legacy markers. These have documented consumers/rollback reasons, so they are not proven dead code. Removal needs a supported-version decision, not a speculative cleanup.
10. **Page/tool layering** — API transport (api.ts), rmcp routing, MCP Apps card lookup and page data each own distinct concerns. No proven dead pass-through module found in the inspected seams. Keep them.

## Patterns and dependencies

The existing tmux/Hyprland-style binary split tree is the right layout model; keep it. rmcp already provides tool registration. React supplies reducers/hooks; serde supplies typed Rust state and JSON projection. Adding a state-machine framework or a query cache would enlarge this pass without removing more complexity. The selected resource hook follows React ownership of stateful operations; the lifecycle enum follows the project's existing worker state-machine pattern.

## Implementation status

Audit published before edits. Implemented first layers:

- `caa9f6d`: one `settings.ts` hook replaces four request implementations in App.tsx; net 101 production lines removed. No resource-specific forwarding hooks remain.
- `b87d870`: private `Progress` enum replaces sign-in JSON patches, retaining the exact HTTP fields. Net 38 production lines added; two wire-format/transition tests add 39 lines.
- Net production reduction: **63 lines**. Tests and proof script add 109 lines; documentation and screenshots are counted separately. A state-machine refactor is useful even where it adds source lines.

Validation: settings proof passes optional 404, rejected save retaining edits, canonical save response, reload and viewport overflow at 1440px and 390px. Captures: `docs/libri-simplify-captures/settings-1440.png` and `settings-390.png`. The web build/typecheck and all 55 web tests passed. `check_changes.sh -- --max-jobs 1` passed, including Rust tests, clippy with warnings denied and rustfmt.

No Android edits, merges, deployments, real credential use or live memory writes. No visible behaviour or public API change. Full flake check remains a merge gate; not claimed here. The layout synchronizer, Temp reducer and harness registry remain follow-up work with the constraints above.
