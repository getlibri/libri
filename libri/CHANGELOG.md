# Changelog

Each release gets an entry here and a `vX.Y.Z` tag, newest first. Releases
before 0.9.23 were not tagged.

## 0.9.85 (2026-11-15)

- Check rigs: keep the host's memory alerts out of a throwaway Libri's chat
- Browser checks: rerun a check once alone when it failed only on a Page.goto timeout
- Give every browser check's page loads 60 s, not Playwright's 30
- Wrap the memory gate's comment
- Count a cgroup's inactive page cache as room in the memory gate
- Fly and fold the agents' dots in 200 ms, not 280, on web and Android

## 0.9.84 (2026-11-15)

- Fold and fly the agents' dots in 280 ms, not 340, on web and Android
- Open a worker's chat at once from the column; the rail's fold no longer runs ahead of it
- Revert "Revert the agents-rail merge 41305e21: opening a worker's row from the column takes 415 ms against the 400 ms gate; the rail comes back once that is fixed"

## 0.9.83 (2026-11-15)

- Revert the agents-rail merge 41305e21: opening a worker's row from the column takes 415 ms against the 400 ms gate; the rail comes back once that is fixed
- Address peers by their person, and say when a shared answer is lost
- Agents: dots run one rail from the bar, round a corner, up to their rows

## 0.9.82 (2026-11-14)

- Android agents: measure the bar and the column on the screen, hide the bar once the dots stand over it
- Android agents: dots fly above the column, fold ends when they are home; drives keep their shots folder

## 0.9.81 (2026-11-14)

- Judge the client a proxy on loopback names in X-Real-IP
- Docked chat heads fade only between the pill row and the talk
- Temp and memory chat heads dock like the main chat's
- Workers popover: solid panel, brand-coloured marks, bar circles slide then rise

## 0.9.80 (2026-11-14)

- Add --port, --listen and --instance, and tailnet entries for one device

## 0.9.79 (2026-11-14)

- Phone foot: the inner gap, composer to bar and disc to tabs, back up to 10px (10dp) from the send button's inset (6px, 5dp), which read as squeezed; the outer 14 stays; web and Android alike
- Chat: draw a talk from its newest 40 messages and earlier ones a page at a time as the person scrolls up, the top row kept in place; a quote's jump draws the whole talk first. Opening a Chat tile beside another over a long history rendered every message's markdown at once (Open anything › Chat, desk: median 331 ms, worst 481)
- 400 ms gate: reach the phone bar's edit mode as the person does, through the hold's menu and its Edit the bar
- Composer agents: only workers at work or asking the person; a done one leaves the circles and the column
- tiles.rs: the browser and system kinds keep no tabs
- Browser tile: its tabs on the shared tab strip
- ui-latency: crawl the site inside the Libri package, so the stand-in answers /api/tiles; drive_android_phone_workspaces says why an install failed
- Android: the agents' column lays out from the app's one WindowOrigin
- The agents' column: a row's time after its words, so the row reads the same as time passes
- A workspace's Undo stands alone, out of Recently closed until it lapses; checks wait on the whole state the host keeps, and find workspaces as tabs
- client: Recently closed sorted by key
- The agents' column: its words haloed in paper over the talk, its foot at the column's end
- check_memory_tabs: a chat's memories as tabs, wide and on a phone, against a throwaway Libri
- Android: a closed workspace's time as the client's unsigned seconds
- The agents rise out of the composer as a column: each circle travels to its slot, its name, model and state to its left, the talk behind dimmed; folded, they travel back. Reduced motion, at once
- A chat tile's memories as tabs along its top, the memory pill gone from the composer: + opens one not open or makes a new one; /memory and the Workers tree open or bring forward its tab; a composer says Message and the name of whom it talks to
- Closing a workspace keeps it under Recently closed for 30 days: an Undo where it was for 3 seconds, Reopen and Delete after; a shell no workspace names any more ends
- One tab strip for every tab row: the workspace bar, the dock and the whiteboard's boards share its pill, soft-fill mark and ×; a workspace's menu on right-click, no chevron; the phone bar marks its item by fill alone
- Composer: whole outline under opaque agent circles, no backdrop blur to roughen its corners; a layer order puts popovers over the circles

## 0.9.78 (2026-11-14)

- Browser checks: hold the builds lock again; builds beside them slowed check_call_chat's first load past 30 s. The 400 ms gate keeps the timing lock alone
- System monitor: a disk is nearly full under 5% free, or under 10 GiB and 20% free; the bare 10 GiB floor called every small disk full, and its alert on the VM test's 1 GB disk raced the test's own turn
- browser-checks: say why it takes the timing lock alone
- ui-latency: a share's progress is an interaction finished, timed or not driven; a share keeps its results as it goes, so one stopped keeps what it found; a share that exits badly fails the run
- ui-latency: the crawl names each screen it could not open, and a short crawl's failure says whether any were
- ui-latency: the timing lock alone, a share starts only with room below its cgroup's memory cap, and a stall names the memory state

## 0.9.77 (2026-11-14)

- Browser checks: run check_settings_resource
- Bring check_settings_resource up to today's Settings
- Polish Settings on web and Android
- List the servers the host adds through harness.mcp at /api/connections/host

## 0.9.76 (2026-11-14)

- 400 ms gate: time the site beside its built libri, so the stand-in serves the tiles; a site with no libri beside it fails
- check_peers_settings: answer Libri's own revoke confirm
- A saved Grok call leaves out the turns that went to the chat: the request and the voice's words around its answer are already in the chat; what Grok handled alone is kept
- Browser checks: each fails past a wall-clock limit; drives take the timing lock and a short TMPDIR
- Grok call driver: a live call through xAI with the stand-in chat, by hand; call.tsx says how both kinds of call run
- Calls on Android: hold the microphone in a foreground service so a call goes on with the screen off, offer the call whenever the host can take one, and show the words as they are heard
- Calls: hear through the host's speech-to-text session and time each stage
- Call check: record the call socket in the page, hear the answer once, and run with the browser checks
- Add a host system monitor: System tile, system tool and alerts
- Client tests: the host that is down is one no stand-in can take
- Calls: transcribe the utterance as audio/wav, the kind stt.rs takes
- The client keeps its own notes through a read of the history, in their place: a stream's reconnect took a command's answer away
- Client tests: a wait that runs out says what the session last showed
- Browser tile: rustfmt
- Client tests: the stand-in host serves until the test lets go, and busy tests give it room
- Client tests: the stand-in host's budget leaves the event stream out, and every wait has one deadline; a hung test harness fails the check in ten minutes
- Composer: drop the unused voice read
- Order the speech module
- Calls hear through the dictation choice and speak with the host's own Kokoro voice when it is here; the call check sets its stand-in as the hearing service
- Call: offer it whenever the host can call, not only with xAI's voice; the check finds the Call button by its prefix
- speech: while-let for complete sentences; read PCM pairs with as_chunks
- Format the call code
- speech: collect the spoken pieces at the end of a reply
- Calls talk with the chat: the host hears, the chat answers, the voice speaks it
- Browser tile: the page fills a phone's place, cut from Chromium's wider window; only a tile in sight sets the page's size
- drive_android_phone_workspaces: open the Temp on the bar; the plus opens a new one beside it
- drive_android_phone_workspaces: says whether Temp left the bar
- drive_android_phone_workspaces: a menu a slow drag opened is the hold's
- drive_android_phone_workspaces: a hold waits for the item menu's Edit the bar
- Clear clippy's findings in the speech code
- Android: TempChat is @Stable, so Compose needs no stability field an inner class lacks under R8 (the app crashed on opening)
- Order the tts module
- Browser tile: a still page shows in the tile; the check covers a 2.625x phone and a page that fills the tile
- Format stt.rs
- Live dictation writes each utterance down whole, with the speech before the detector's start, so a live stream's words no longer stall or lose their first word
- After the rebase: message_peer in the tool lists, the client formatted, the Android app asks for the tiles on every start
- Browser tile, follow-ups: frames at the device's pixel ratio, a logins grant keeps the tabs open, the person's tabs reopen after a restart
- Write dictation down on the host by default, with xAI or an OpenAI-compatible API as choices
- Libri's own confirm and notice in place of the browser's (dialog.tsx; Android's ConfirmDialog), every confirm and alert moved to them, checks.dialogs and a browser guard against native dialogs; the terminal's input field is never seen and sits on the cursor
- Phone Settings: a row's name keeps one line; check_settings_fit asserts Hosts and every Settings page fit 390px
- check_temps: several Temps against a throwaway Libri; the phone drive holds for the item menu
- Android: tiles from the host's list, an item menu on a hold, a Temp per tab; tiles-check
- Web: tiles from the host's list; menus' rows by id; a Temp per tile
- Several Temps at once, each its own: routes, run state, asks and turns by Temp id
- Tiles declared once: tiles.rs, /api/tiles, layout checks and tool words from it, tile_open

## 0.9.75 (2026-11-14)

- Bound what the workers can hold: the cgroup caps are tested, and heavy jobs wait for room instead of each other
- Sessions compact at a host-wide point, 200k tokens unless picked: Claude Code's autoCompactWindow, Codex's model_auto_compact_token_limit, set in Settings
- A quiet tool's processes are sampled at the window's ends, not every second
- Listings read the worker folders they hold, never looking each up by name
- Whiteboard: the tile's notch sits over a board as over any tile (the tabs below it, Excalidraw's layers isolated in the canvas); check_whiteboard drives it
- Reveal: name no one in its notes
- Browser tile: Libri's own browser, live and shared with agents, in a tile
- drive_android_phone_workspaces: swipe from Library to the side panel
- drive_android_phone_workspaces: Main ends as Chat and Library
- check_phone_workspaces: two items stand alike, centred; no name in a comment
- Android drives reach every worker from the bar's plus; the phone workspaces drive starts from Chat and Library
- Temp stays one tile until the host holds several Temps: opening it again finds the open one
- Tile menus, as Tern's pane menu: a right click in a tile, or on its notch; a hold on a phone's bar item
- terminal.rs: rustfmt
- Open any place again: each its own tile; a terminal beside another a shell of its own
- Drop the Workers place: the composer's agents hold its work; the circles grow to the composer's width
- Phone foot: the inner gap is the send button's inset (6px, 5dp) between composer and bar and disc and tabs; no edge on the bar; a far tab slides as a neighbour's
- Peers: each pair of assistants has its own credential, asked for by one and approved by the other in Settings > Hosts > Peers; revoking stops it on both sides; message_peer lets an assistant write to a paired one; a host on the old shared token keeps its pair
- A call's card shows by itself only when showing it is the point: its tool says so (is.libri/opens) on the person's own turn, or the model asks (show_card); a lookup is its row alone, which opens the card on asking. What the person opens in a chat comes into view, and what they close keeps its row in place; a card's link opens in the browser on Android
- check_card_opens.py and the Android drive: a card shows only when showing it is the point, an opened row comes into view, a closed one keeps its place
- ui-latency: a share's progress is a timing that came back; a share whose browser is gone exits; each share and the whole run have hard limits; test hooks that make a share hang
- Format after removing the past view
- Phone side panel: the page left in view closes it by a tap or a swipe left, outside the inert page; check_side_panel in WebKit and Chromium
- Remove message folding: Fold all, the folded chat and the past view's lines, on web and Android; /api/past and /api/zoom go with them
- Chat header: drop the Fold all button; Fold all stays in the hold menu

## 0.9.74 (2026-11-13)

- Whiteboard: a tile with no board shows the first and asks for it once, only when it is the one on screen, so a phone's Whiteboard item added with no board no longer loops until React takes the page down; a WebKit iPhone pass in check_whiteboard
- Two check locks: builds run side by side under a shared builds lock; timing runs (browser checks, the gate, emulator drives) run one at a time with no build beside them
- Whiteboard: ctrl+wheel and pinch on a board zoom the board, not the chat's text; exports keep their file names on Android; the board page never scrolls; checks for zoom, drags and the phone's kept board
- Whiteboard on phones: drags stay on the board, the board opens from a page of its own kept alive in the app, assets go compressed, a picked board is the workspace's on phone web too, menus sit above the canvas, and files go in and out of the app's web views
- Work cards read a log's start and end, and an unchanged log not at all
- ui-latency: the host-wide check lock replaces its own; fail on a run that finds under 540 interactions
- ui-latency: today's crawl counts, and the lock and failures in the flake's description
- ui-latency: one run per machine, fail when the page never loads or the crawl finds far too little, and stop a stalled share rather than hang

## 0.9.73 (2026-11-13)

- check_page_data: a composer poll during the page's reload reads nothing instead of failing
- Find a worker by its ID in one look, not by reading every worker's markers
- Phone bar two thirds of the composer's height at rest (59px, 54dp); the one 14px gap kept
- One host-wide check lock: check_changes, browser-checks, ui-latency and the Android release build run under nix/with_check_lock.sh

## 0.9.72 (2026-11-13)

- Whiteboard on Android: the host's tile page in a web view as a place of the bar, the board picked kept as that place's own, one Whiteboard item per workspace
- Board links: a view or edit link to one board, lapsing or not and taken back from the Share menu, served on a listener of its own that serves nothing else
- Whiteboard: notes wrap their words, a tile claims its board only when shown, opening a board writes nothing, and check_whiteboard drives it all
- Whiteboard tile: Excalidraw over host-kept boards, tabs per tile, exports, and live sync by the shared-object op log
- Whiteboards on the host: boards kept element by element, an API for the Whiteboard tile, and board tools for the agent
- Stand-in models read the words after the device's mark
- The device's mark in Temp, memory chats and a worker's page, taken off their bubbles; the memory check covers marks on change only
- The person's device: a mark when it changes, read back beside each message's time
- Install-name scan as the flake's checks.names, so the release's flake check enforces it; check_changes runs the same script
- Browser-check timeout notes name no particular install

## 0.9.71 (2026-11-13)

- Release browser checks: 60 s per step in the three that allowed 15, as check_chat_steady does, so a loaded host doesn't time out a page load
- check_edge_fade: allow 30 s per step, as the other chat checks do, so a loaded host doesn't time out its first page load
- Android: the bar as tall as its own composer at rest (81dp)
- check_phone_workspaces: tabs beside the composer-tall disc
- Phone foot: one 14px gap at the sides, between composer and bar and under the bar; the bar as tall as the composer
- drive_android_call_cards.py: a call's card, its tool call, full screen, a kept card's pill, a sent picture and /memory on the emulator; the memory chat's send button at the right
- Android opens a named memory's chat, from the Workers tree and /memory <name>; /help lists the harness's own commands
- Web /model, /effort and /harness read and set the pick as the pickers do; a worker thread's pickers set its own choice
- Android draws a call's MCP Apps card in the chat, and a kept image goes out as a picture
- Workers by their models' picker names, and the composer's tree anchored to its edge
- Composer agents: the outline breaks where the circles sit, nothing laid over it
- drive_android_composer_agents.py: the composer's agents on an emulator, 1, 3 and 7 at work, light and dark
- Composer: the agents at work as circles in its outline, and their tree risen out of it
- Outside door: who the assistant is from the rules' Identity section
- Outside door: memory through hippo2's recall and open, in place of the old memory tools
- Ring a module off in the frame's line, and record the door's design with its pictures
- Offer an app only its own connectors at sign-in, and picture Other apps on web and Android
- Let outside apps in as connectors, each signed in by OAuth or a key and limited to its modules

## 0.9.70 (2026-11-13)

- The hold's cap counts the dock; check_thread_room: a few lines' message in Temp stays below the pills mid-turn
- The held room never lifts the person's last message above the talk's first line, mid-turn too
- Chat: the top fades in just below the status bar, the talk clear behind the header's buttons
- Terminal: on a phone the keyboard waits for a tap instead of rising as the terminal opens
- A command that answers nothing says it ran; check_memory covers the harness's commands and /clear
- Name no one in the prompt: who the assistant is opens the rules as an Identity section, the host's facts are HOST.md in Settings, and tool text goes to the tools
- Android: /clear and /new start a new session, /memory, and the harness's own commands in the / menu
- Slash commands: /clear and /new start a new session, /memory, and the harness's own commands sent as typed
- browser-checks: add check_edge_fade, which passes on main
- Add browser-checks: the Playwright checks a release runs against its built site

## 0.9.69 (2026-11-12)

- Chat edges hold still in a fling: drawn outside the scroll on web and the drift on Android

## 0.9.68 (2026-11-12)

- check_memory: match zoom as a whole word, not inside the temporary path
- Chat: hidden under the status bar, fading in under the header's buttons
- Add an Android driver picturing the chat's foot
- Chat: clear down to the dock, the talk dimmed beneath it, as Claude Duet's

## 0.9.67 (2026-11-12)

- check_memory: a worker is told what <chat> holds when its session starts new
- check_memory waits for a worker's session to be its own
- Name a Claude session before it starts, and date each worker log entry
- Format the client's exports
- A worker that can't copy its head's thread starts with the newest of its chat
- A Claude session past its context window gives way to a new one
- The stand-in Claude's sessions hold their thread, as the real one's do
- The memory's server goes by its program's name, hippo2
- Start a harness anew when another ran the chat since
- Check the memory as it is now: one session per chat, hippo2 beside it
- Resume a Muse session per chat while its profile and model hold
- Drop hippo.record: Libri's hippo keeps Libri's record alone
- Resume the page stream from its last event instead of reading the chat again
- Keep an OpenCode session per chat while its model and permissions hold
- Take ACP sessions up again per chat: session/resume, else session/load
- Keep one omp and one Pi session per chat and worker
- Keep one Codex thread per chat and worker: thread/resume, thread/fork
- Write no summaries: hippo keeps Libri's record alone, and the compactor service goes
- Drop the memory tools of Libri's server: memory_zoom, memory_search, memory_date, work_zoom, work_search
- Keep one Claude Code session per chat and worker, resumed each turn
- Hand every turn the memory's MCP server, hippo2 by default (memory.command)

## 0.9.66 (2026-11-11)

- Workers: each keeps scratch in its own folder, TMPDIR to its harness, never /tmp; model and usage listings start in an empty folder under the state directory
- Memory check: read worker names from the spawn reply now that each carries its model and level
- Workers: exact model and thinking level at start, from the harness's own report

## 0.9.65 (2026-11-11)

- Tests: drop install names from two fixtures so the leak guard passes
- Watches: a tap opens a watch's card in place to show all it holds; the host lists its look arguments, tests, fields, sharing and push, and names its worker

## 0.9.64 (2026-11-11)

- Only a dated message rises: a worker's or Temp's undated log no longer animates in when opened
- Record the UI latency table after the fixes: 536 interactions, none over 400 ms
- UI latency check: time a miss again alone before failing, reach what a person reaches
- Chat: a message rises once, only when it arrives; Scroll to bottom and a quote's jump land at once
- Record the first full UI latency table: 444 interactions, 15 over 400 ms
- Add a UI latency check: every reachable control, input to settled frame, 400 ms limit
- Chat checks: serve the endpoints the page reads now; width opens Temp from the dock, the side columns being gone
- Chat checks: open Temp from the plus on phones, reach Settings through the side panel; Simple keeps the words before the person's reply
- Chat checks: find the talk and composer by data-surface and .floating
- Android: one chat renderer (ChatTalk) for the main chat, Temp and worker threads
- Web: one chat renderer for the main chat, Temp, memory chats and worker threads

## 0.9.63 (2026-11-10)

- Android: drive the Workers tile's three tabs on an emulator in both themes
- Workers tile: Watches and Archived each get a tab; Archived shows its count only once opened

## 0.9.62 (2026-11-10)

- Browser: drive the person's own browser through an extension relay for tasks with their logins, and cap the agents' Chromium in a cgroup of its own

## 0.9.61 (2026-11-10)

- Memory check: wait 60 s, as a turn under the full check's load can take more than 20 s to start
- Comments: drop personal names and dates from design notes
- Workers tile: watches and archived move to a second tab; Main reads as the tree's root
- Check that a thread's room stays within a view when a turn folds mid-run
- Claude models: ask Claude Code for its list rather than LIBRI_MODELS
- Threads: count the spacer only when it stands inside the talk, so Temp's room stops growing on itself
- Web search through the host's key proxy socket; Libri holds no search key
- Phone Android: a dropped item hits the ✕ where the finger is; the emulator check covers the edited bar
- Phone Android: no page heads, one floating make, the bar edited in place
- Phone web: no page heads, one floating make, the bar edited in place
- Phone: terminal keys clear the system bar; tab labels shrink to fit

## 0.9.60 (2026-11-10)

- Phone bar: the same side margins as the composer, so it is never wider

## 0.9.59 (2026-11-10)

- Chat: fade the talk out above the composer, not at the screen's foot

## 0.9.58 (2026-11-10)

- Phone bar: size the web pill as Android does, padding included

## 0.9.57 (2026-11-10)

- Phone bar: a workspace disc, three items fill the pill, the plus at its end

## 0.9.56 (2026-11-10)

- Give a phone's item labels the tab's whole width
- Phone workspaces on Android, as on the web
- Phone workspaces on the web: the bar at the foot is a workspace

## 0.9.55 (2026-11-10)

- Android: keep the chat field focused while switching keyboards

## 0.9.54 (2026-11-09)

- Run a restarted turn again only when it was cut before its answer
- Hold the foot through WebKit's clamp, with anchoring off at the foot
- Keep the chat on its foot, and run the talk under its header and dock

## 0.9.53 (2026-11-09)

- Android: the Workers page is the same tree as on the web: memories over their workers, with cards showing name, model, title and status, activity colours, a glow while working and taps to fold.

## 0.9.52 (2026-11-09)

- Skills from a git repository: set a source in Settings → Agents (such as owner/skills); Libri pulls it on start, hourly and on request, keeps the last copy if a pull fails, and installs its skills beside your own for every harness it launches
- Skills list with on/off switches in the Agents pane; name clashes are left out and noted
- The `$` skill menu is back in the main and worker composers, and on Android

## 0.9.51 (2026-11-09)

- Worker cards: each worker shows its name, model, title and a periodic status; colour shows what it is doing (amber waiting on you, red failed, dim idle); a lane glows while its agent works, and dashes run along it while agents message each other

## 0.9.50 (2026-11-09)

- Named memories: a memory picker beside the model picker; each memory (Main, Jobs, Tutor, ...) keeps its own hippo store and chat
- Settings gains an Agents pane: shared rules plus an optional agent.md per memory; memories can be added, renamed and deleted
- Memory chats have their own memory tools and approval asks
- Any memory's chat can start workers; a worker forks that memory, hangs from it and reports to its chat
- The Workers tile is a tree by memory: forking lanes that glow while a worker talks, amber while it waits on you; click folds, double-click opens
- Full detail survives a reload of the main chat

## 0.9.49 (2026-11-08)

- omp is the default harness for general work, except Claude Code; ChatGPT, Kimi, Kilo and Qwen sign in through omp from Settings
- Chat detail: Simple shows only messages and final replies, Standard adds tools, Full adds omp's diffs, todos and context; Full's extra detail does not yet survive a reload
- Android matches the web's detail modes and omp presentation
- An idle chat reopens on its last line on reload and return
- Settings load as one resource; device sign-in shows its steps plainly
- The omp memory plugin is gone; Libri is the one way to run hippo

## 0.9.48 (2026-11-07)

- A failed layout load no longer overwrites the saved layout; failed saves retry and show their status
- Library links survive a reload
- Settings shows the version and an update button
- Clearing Temp restores it when the host refuses
- Phone: tile controls clear the pane title; terminal tiles get phone keys
- Saved template names stay distinct
- Chat: no blank room under a finished reply, no jump while history loads
- Code keywords meet contrast in both themes; the Workers button no longer covers timestamps

## 0.9.47 (2026-11-07)

- Wiki: an Obsidian vault of notes beside memory; the main chat reads and writes it, workers and Temp read it

## 0.9.46 (2026-11-07)

- Memory keeps its model processes running between summaries instead of starting a new one each time, which cuts their CPU use about fourfold

## 0.9.45 (2026-11-07)

- The main chat can now search its memory for exact words, beside zooming
- The prompt lists the memory tools without rules for when to use each

## 0.9.44 (2026-11-07)

- Drop the blank rows above a terminal read
- Let the app clear the temp chat from another origin
- Shorten the instructions line and keep the prompt in one file
- Format the terminal tools
- Let the agent type into and read the terminal the person watches
- Drop the rule limiting subagents to when the user asks

## 0.9.44 (2026-11-07)

- The agent can open a terminal tile, type into it and read its screen (terminal_open, terminal_send, terminal_read); workers and watches can only read.
- Clear in the temp chat now clears it on the host from app.libri.is, and says so if the host refuses.
- One prompt file serves both turns and memory compaction; the rule limiting subagents to when asked is gone.

## 0.9.43 (2026-11-07)

- The agent can read and change the workspace layout (layout, layout_change, layout_undo); open screens follow live, and an agent change shows an Undo chip

## 0.9.42 (2026-11-07)

- Workspaces are yours: + adds an empty one, rename it in place, close it (chats and workers stay), and save one as a template to start others from.
- Each tile's controls hang as a small notch from the middle of its top edge, covering nothing until opened.

## 0.9.41 (2026-11-07)

- A reloaded chat opens on its last line, with no blank screen under it, on the web and Android.
- One port for everyone on a host: a tailnet login with its own Libri is passed on to it.
- Settings and every config key are described in one page, /docs/settings.md, which the assistant reads.

## 0.9.40 (2026-11-07)

- Tiles on the web: workspaces in the left pill, a dock on the right and a launcher (Alt Shift Space) for the rest. Each tile has its own pill to move, split, go fullscreen or close, and layouts are kept on the host.
- A host can let in only the tailnet logins it is for, signed in with no pairing.
- What's new: a device is told what changed since the version it last saw.
- Libri's own notices (a restart, memory coming back) show as system lines on the web and Android.
- A long chat no longer opens blank when its column changes width.
- Settings lists every setting and where it applies; one bad access default no longer resets the rest.
- The event stream pulses each second, so a lost connection can be noticed and resumed.

## 0.9.39 (2026-11-07)

- Libri as an MCP server for other apps: memory and chats at `/mcp` and `libri mcp` on stdio, behind a switch in Settings > Other apps.
- Settings > Usage on the web and Android: subscription limits with a pace mark, and spend at API prices by caller, model, maker, harness and payment.
- omp: subagents run as their own threads, running output shows as it prints, and its tools read as Libri's.
- The chat, worker and Temp threads hold their place: a tool row ending no longer drops the talk, and reading higher up while a reply streams moves nothing.
- Content fades out under the floating bars instead of being cut off, on the web and Android.
- Page links open in the Library in place, and a page published live opens by itself.
- Library cards show thumbnails again, rendered with the host's Chromium.
- hippo follows UniiChat: one prompt, the person's rules on every summary call, Haiku high by default with no fallback, and only what passes through Libri is recorded.

## 0.9.38 (2026-11-06)

- Home on Android and mobile web: a place left of Chat with today's date, what needs you, the calendar, work running and done today.
- Android: Back from Calendar or Mail comes to the chat; only Back from the chat leaves the app.

## 0.9.37 (2026-11-06)

- Android: pages, MCP apps and Calendar keep scrolling when the finger drifts sideways.
- Settings > Connections: signing in to Fastmail turns on Mail and Calendar; each tool is Allow, Ask or Off, with writes on Ask and reads on Allow by default.
- Android signs in through its own is.libri return; a browser the server refuses gets a paste-back.

## 0.9.36 (2026-11-06)

- A reply no longer shows twice when two messages arrive during one turn.
- Quick-reply chips and Reply appear on a new answer straight away, without a reload.
- A turn reads in order, live and after a reload: words, the tool call's line, then the next words.
- Tasks + opens the full composer in a sheet, with model, thinking and access pickers.

## 0.9.35 (2026-11-06)

- Reply quotes a message; Edit copies an old message into the composer and sends it as a new one, in main, Temp and tasks.
- "Send to Work" is now "Create task".
- Quick-reply chips sit under the message that asks; tapping one replies to it.
- Mail and Calendar take whatever access their sign-in grants; the Read picker is gone.

## 0.9.34 (2026-11-06)

- Calendar built-in: connect Fastmail from Settings and see your agenda in the sidebar, beside Chat. Mail and Calendar start on.
- Phone sidebar pushes the chat aside instead of covering it; the bottom bar floats as a pill; Hosts and Memory move into Settings; full-width rules are gone.

## 0.9.33 (2026-11-06)

- Merge a finished worker into main chat: its messages join main as ordinary messages with their original times; the brief and final report stay out.
- Memory summaries follow the UniiChat prompt closely, and copied ids and dates are cut from the start of each line.

## 0.9.32 (2026-11-06)

- Android and phone web: the Tasks, Library and Pinned heads and the
  full-width rules are gone; spacing parts the sections.
- Library pages are interactive: a page keeps data both you and the agent
  change, the agent sees your edits at its next turn, and a page's message
  lands in the composer. On Android a page opens alone, in the app's theme.
- The household's shared shelf, mailboxes and seat are gone; A2A notes land
  in the chat.

## 0.9.31 (2026-11-06)

- Android and web: each assistant reply sits on its faint fill at every
  width, so back-to-back replies read apart, as on the web since 0.9.30.

## 0.9.30 (2026-11-05)

- Web: the chat's head docks to the top; on a screen 1280px and wider its
  controls carry their names. At 1024px the chat's cell holds one pane at a
  time: the chat, Temp, the terminal or a worker.
- Web: an open thread's task card joins the thread only while it slides out.
- Web: each tool call shows the lines it changed and the time it took, and a
  folded run its totals.
- Web and Android: the Library is a gallery of cards, grouped by when they
  changed: two columns on a phone, a grid on a wide screen, lists last. A
  page's card shows a still of it where the host renders stills, which is off
  unless `daemon.browser` names a Chromium (the NixOS module's `browser`
  option, null by default); otherwise the card shows the page's title.
- Connection sign-ins return through https://libri.is/connect/back, which
  sends the browser on to the address it reached Libri at, so a sign-in
  started on a phone finishes there. `connections.return_page` names another
  page; empty keeps the old return to 127.0.0.1.
- Android: the app id is now `is.libri` (was `li.libri`). Android installs
  this release as a new app beside the old one; it does not update it.
  Install 0.9.30, sign in, then uninstall the old Libri. Obtainium users: add
  Libri again from your host's /android/ page and remove the old entry, since
  Obtainium tracks an app by its id.

## 0.9.29 (2026-11-05)

- Web: a calmer page: lighter paper, firmer small text, one table frame,
  framed user bubbles, named tool targets, one glyph for a folded tool run.
- Web: the Library heads the left rail, with host and memory at its foot; on
  a wide screen the Library lives there alone, not in the chat's pill.
- Web: at 1024px the workers stay in reach when Temp or the terminal covers
  the chat.
- Web and Android: the Tasks and Library plus floats at the lower right of its
  column; Android gets a Temp back arrow and lists Archived last.

## 0.9.28 (2026-11-04)

- Settings has an Add connection form on the web and Android: any MCP server, by URL or by a command run on the host, signed in by OAuth or by pasted headers. A new connection starts on; tools can be withheld from every harness.
- Fastmail moves onto the same list as an ordinary connection, with send_email withheld.
- An address that answers with a web page instead of the host now says so, rather than failing as "invalid type: null".

## 0.9.27 (2026-11-04)

- On a desktop, the pill at the top right holds a Library button that opens
  the Library as a column beside the chat. Search and Fold all move to a
  pill at the top left.
- The talk is as wide as the composer: assistant text and user bubbles line
  up with its edges.

## 0.9.26 (2026-11-04)

- Working is a book whose pages flip over and back, 18 px tall, on the page
  and in the Android app, in place of three bouncing dots. With reduced
  motion (the page) or animations off (Android) it stays the shut book, a
  flat bar.

## 0.9.25 (2026-11-04)

- Libri is a standalone product: the scripts that deploy and prove one
  particular install moved out of the repository, tests, evals and fault rigs
  use placeholder names, and install paths are settings
  (`LIBRI_ANDROID_KEYS`, `ENDURANCE_GROUP_LIMIT`, `HIPPO_FAULT_SEED`).
- `check_changes.sh` fails on names and paths of a particular install.

## 0.9.24 (2026-11-04)

- Web: iOS Safari no longer rubber-bands the whole page when a drag runs past
  the chat's end.
- Web: code blocks draw with their message, so a message's kept height is true
  and the chat no longer jumps as a code block nears the view.

## 0.9.23 (2026-11-04)

- Android: the app id is now `li.libri` (was `is.su.libri`). Android installs
  this release as a new app beside the old one; it does not update it. Install
  0.9.23, sign in, then uninstall the old Libri. Obtainium users: add Libri
  again from your host's /android/ page and remove the old entry, since Obtainium
  tracks an app by its id.

## 0.9.22 (2026-11-04)

- Library and Tasks pages: published pages and lists, watches with snooze and
  delete, workers started from Tasks, and Temp in the chat's top bar.

## 0.9.21 (2026-11-04)

- The mark is mirrored: the top page curves and the bottom lies straight.
- The worker that starts a deploy no longer holds the deploy's quiet wait.

## 0.9.20 (2026-11-04)

- A worker's results pair with their calls by id (older logs by order).
- Stop on a Claude subagent stops it by the call that started it.
- Waiting tags, worker branch and worktree self-cleaning, MCP Apps fixes,
  napping, and a second account running as a stock Libri install.
