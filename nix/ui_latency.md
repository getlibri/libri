# UI latency: input to the frame after the page settles (2026-11-11)

Written by `nix/check_ui_latency.py --slow --table`. Each cell: median / worst ms over the runs, from the
input event to the frame after the page last changed for it (the end of any animation it started included).
`!!` over the 400 ms limit (an unthrottled median over it fails the check), `!` over the 100 ms target.
`first` is the median to the first frame that changed, unthrottled, the slower width: the gap between it and the
cell is animation or later work. `inp` is the browser's own interaction delay (Event Timing, at least 16 ms when
reported). `host` is the longest call to the stand-in host the interaction started; the stand-in answers at once.
`-` not reached at that width; `(no change)` nothing on screen changed. A name is the path of clicks to it.

| interaction | desk cpu/1 | desk cpu/4 | phone cpu/1 | phone cpu/4 | first | inp | host ms | host calls |
|---|---|---|---|---|---|---|---|---|
| Controls for Chat › Close Chat › Close Chat | !265 / 281 | !!1129 / 1146 | - | - | 232 | 296 | 454 | /api/layout /api/models /api/usage/report /api/work |
| New workspace (Alt Shift N) › Close Chat › Close Workspace 2 | !265 / 282 | !!1013 / 1014 | - | - | 248 | 288 | 423 | /api/library /api/models /api/usage/report |
| New workspace (Alt Shift N) › Chat, terminal and workers The chat, the | !299 / 299 | !!829 / 880 | - | - | 115 | 152 |  |  |
| New workspace (Alt Shift N) › Chat and Temp The chat, a temp chat besi | !332 / 332 | !!781 / 813 | - | - | 165 | 200 | 351 | /api/models /api/usage/report |
| New workspace (Alt Shift N) › Chat The chat alone | !299 / 315 | !!681 / 763 | - | - | 115 | 160 | 4 | /api/layout |
| New workspace (Alt Shift N) › Chat | !298 / 299 | !!664 / 664 | - | - | 116 | 160 | 4 | /api/layout |
| New workspace (Alt Shift N) › Chat (open beside the focused tile) | !132 / 148 | !!531 / 580 | - | - | 115 | 168 |  |  |
| New workspace (Alt Shift N) › Chat (Alt Shift 1). Double-click to rename; right-click for more. | !148 / 165 | !!530 / 563 | - | - | 131 | 176 |  |  |
| New workspace (Alt Shift N) › Library › Close Workspace 2 | !115 / 115 | !!497 / 512 | - | - | 98 | 136 | 4 | /api/work |
| New workspace (Alt Shift N) › Workers › Close Workspace 2 | !131 / 131 | !!496 / 529 | - | - | 114 | 144 | 5 | /api/layout /api/work |
| Side panel | - | - | !331 / 331 | !!495 / 496 | 64 | 80 | 138 | /api/capabilities /api/connections /api/guide /api/host /api/models /api/skills /api/worker-names |
| New workspace (Alt Shift N) › Terminal › Close Workspace 2 | !131 / 132 | !!492 / 511 | - | - | 98 | 144 | 4 | /api/layout |
| Open anything (Alt Shift Space) › Temp chat | !231 / 231 | !!457 / 459 | - | - | 64 | 96 | 83 | /api/models /api/usage/report |
| Redraw | !280 / 297 | !!456 / 474 | - | - | 63 | 40 | 240 | /api/run /api/work |
| New workspace (Alt Shift N) › Temp chat › Close Workspace 2 | !115 / 115 | !!446 / 497 | - | - | 99 | 136 | 5 | /api/layout |
| Edit › Send | !281 / 297 | !!441 / 441 | - | - | 64 | 96 | 95 | /api/run |
| Terminal (open beside the focused tile) | !114 / 114 | !!441 / 442 | - | - | 47 | 80 | 4 | /api/work |
| Controls for Chat › Split side by side | !230 / 246 | !!440 / 521 | - | - | 63 | 88 |  |  |
| send a message (until the bubble shows) | !270 / 285 | !!411 / 418 | !283 / 291 | !!434 / 461 | 58 | 72 | 106 | /api/run |
| Settings and usage | !248 / 264 | !!427 / 475 | - | - | 48 | 72 | 151 | /api/capabilities /api/connections /api/guide /api/host /api/models /api/skills /api/work /api/worker-names |
| Settings and usage › Hosts › Close the side panel | !264 / 281 | !!426 / 426 | - | - | 47 | 88 |  |  |
| Settings and usage › Theme Light › Close the side panel | !264 / 264 | !!426 / 426 | - | - | 47 | 80 |  |  |
| Create task › Promote | !280 / 281 | !!425 / 443 | - | - | 30 | 40 | 206 | /api/run |
| Alt Shift 1 (back to the first workspace) | !123 / 130 | !!412 / 440 | - | - | 97 | 136 |  |  |
| Settings and usage › Close the side panel | !264 / 264 | !!408 / 408 | - | - | 47 | 72 |  |  |
| Reply | !197 / 197 | !!404 / 407 | - | - | 47 | 80 | 4 | /api/work |
| bar: drag an item sideways (reorder) | - | - | !236 / 236 | !393 / 433 | 73 | 40 | 205 | /api/work |
| Settings and usage › Models › Close the side panel | !264 / 265 | !391 / 425 | - | - | 47 | 72 |  |  |
| Terminal (open beside the focused tile) › Copy Code › Close the terminal | 96 / 97 | !390 / 392 | - | - | 80 | 104 | 3 | /api/work |
| New workspace (Alt Shift N) › Terminal | !115 / 115 | !380 / 397 | - | - | 48 | 64 |  |  |
| bar: drop an item on × (close) | - | - | !220 / 236 | !364 / 389 | 104 | 64 | 194 | /api/work |
| Workers › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run | - | - | 98 / 98 | !362 / 363 | 65 | 56 | 247 | /api/models /api/usage/report /api/work/w0 /api/work/w0/choice /api/work/w0/read |
| Settings and usage › Theme Light | !247 / 248 | !359 / 359 | - | - | 14 | 64 |  |  |
| Terminal (open beside the focused tile) › Edit › Close the terminal | 97 / 98 | !359 / 375 | - | - | 81 | 112 | 2 | /api/work |
| Workers › Archived › Linnet Linnet batch check Comparing three quotes against the last run | - | - | 99 / 99 | !346 / 347 | 65 | 40 | 249 | /api/models /api/usage/report /api/work/w29 /api/work/w29/choice /api/work/w29/read |
| More for Chat › Save as template › More for Chat | 31 / 31 | !344 / 346 | - | - | 31 | 40 |  |  |
| Settings and usage › Theme Light › Theme Dark | !231 / 247 | !343 / 356 | - | - | 14 | 56 |  |  |
| Workers › Archived › Shrike Shrike batch check Comparing three quotes against the last run | - | - | 98 / 98 | !330 / 364 | 65 | 40 | 233 | /api/models /api/usage/report /api/work/w28 /api/work/w28/choice /api/work/w28/read |
| Workers (open beside the focused tile) › Archived › Shrike Shrike batch check Comparing three quotes against the last run | 81 / 98 | !326 / 376 | - | - | 48 | 72 | 221 | /api/models /api/usage/report /api/work/w28 /api/work/w28/choice /api/work/w28/read |
| Controls for Chat › Split stacked | 80 / 80 | !323 / 357 | - | - | 64 | 88 |  |  |
| More for Chat › Save as template › Open anything (Alt Shift Space) | 31 / 48 | !311 / 395 | - | - | 31 | 48 | 2 | /api/work |
| Temp chat (open beside the focused tile) | 81 / 81 | !311 / 325 | - | - | 64 | 88 | 81 | /api/models /api/usage/report /api/work |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run | 81 / 97 | !311 / 344 | - | - | 64 | 32 | 217 | /api/models /api/usage/report /api/work/w0 /api/work/w0/choice /api/work/w0/read |
| Workers (open beside the focused tile) | 81 / 81 | !308 / 324 | - | - | 64 | 96 |  |  |
| Workers (open beside the focused tile) › Archived › Linnet Linnet batch check Comparing three quotes against the last run | 80 / 81 | !305 / 327 | - | - | 64 | 40 | 211 | /api/models /api/usage/report /api/work/w29 /api/work/w29/choice /api/work/w29/read |
| bar: flick an item up (close) | - | - | !220 / 220 | !303 / 320 | 157 | 32 | 2 | /api/phone-layout |
| Library (open beside the focused tile) › Copy Code › Close Chat | 64 / 80 | !293 / 308 | - | - | 48 | 96 | 10 | /api/library |
| Library (open beside the focused tile) | 80 / 80 | !292 / 340 | - | - | 48 | 88 | 4 | /api/work |
| Terminal (open beside the focused tile) › Close the terminal | 64 / 80 | !292 / 293 | - | - | 48 | 88 |  |  |
| Terminal (open beside the focused tile) › Copy Code › Close Chat | 80 / 91 | !290 / 322 | - | - | 48 | 96 | 5 | /api/library |
| Workers (open beside the focused tile) › Controls for Workers › Close Workers | 80 / 81 | !290 / 294 | - | - | 64 | 104 |  |  |
| Alt Shift Enter (split the tile) | 86 / 93 | !280 / 301 | - | - | 60 | 80 | 4 | /api/work |
| Library (open beside the focused tile) › Controls for Library › Close Library | 65 / 81 | !278 / 292 | - | - | 48 | 88 |  |  |
| Library (open beside the focused tile) › Make something › Close Chat | 65 / 65 | !278 / 293 | - | - | 48 | 80 |  |  |
| Library (open beside the focused tile) › Edit › Close Chat | 65 / 81 | !277 / 290 | - | - | 48 | 88 | 5 | /api/work |
| Library (open beside the focused tile) › Plan 1 › Close Chat | 81 / 81 | !277 / 278 | - | - | 48 | 88 | 4 | /api/work |
| Terminal (open beside the focused tile) › Controls for Terminal › Close Chat | 80 / 80 | !276 / 291 | - | - | 64 | 88 |  |  |
| Terminal (open beside the focused tile) › Controls for Terminal › Close the terminal | 80 / 81 | !276 / 291 | - | - | 47 | 88 |  |  |
| Workers (open beside the focused tile) › Close Chat | 81 / 97 | !276 / 277 | - | - | 48 | 96 |  |  |
| Library (open beside the focused tile) › Controls for Library › Close Chat | 65 / 81 | !275 / 276 | - | - | 48 | 80 |  |  |
| Terminal (open beside the focused tile) › Close Chat | 64 / 97 | !275 / 275 | - | - | 48 | 104 |  |  |
| New workspace (Alt Shift N) › Chat, terminal and workers The chat, the › Close Workspace 2 | 81 / 97 | !274 / 275 | - | - | 64 | 104 | 5 | /api/work |
| Workers (open beside the focused tile) › Copy Code › Close Chat | 80 / 80 | !273 / 290 | - | - | 48 | 96 |  |  |
| Workers (open beside the focused tile) › Start a worker › Close | !180 / 181 | !266 / 289 | - | - | 30 | 40 |  |  |
| Terminal (open beside the focused tile) › Controls for Terminal › Close Terminal | 64 / 64 | !262 / 292 | - | - | 47 | 80 |  |  |
| Temp chat (open beside the focused tile) › Copy Code › Close Chat | 80 / 81 | !261 / 310 | - | - | 47 | 96 |  |  |
| Temp chat (open beside the focused tile) › Edit › Close Chat | 64 / 65 | !261 / 310 | - | - | 47 | 88 | 4 | /api/work |
| Terminal (open beside the focused tile) › Edit › Close Chat | 80 / 96 | !261 / 292 | - | - | 47 | 88 | 5 | /api/work |
| Library (open beside the focused tile) › plan-0 › Close Chat | 65 / 65 | !260 / 277 | - | - | 48 | 88 | 2 | /api/work |
| Side panel › Settings | - | - | !248 / 249 | !247 / 248 | 31 | 40 |  |  |
| Side panel › Settings › Appearance Light · 16px | - | - | !248 / 248 | !231 / 248 | 32 | 40 | 4 | /api/work |
| Side panel › Settings › Chat Pinned on | - | - | !248 / 249 | !247 / 248 | 32 | 40 |  |  |
| Side panel › Settings › Hosts box | - | - | !248 / 248 | !246 / 248 | 32 | 40 | 15 | /api/devices /api/host |
| Side panel › Settings › Models Opus 5.5 | - | - | !248 / 249 | !248 / 248 | 32 | 40 | 19 | /api/models /api/synopsis /api/work /api/worker-model |
| Side panel › Settings › Notifications Blocked | - | - | !248 / 248 | !231 / 248 | 31 | 40 | 4 | /api/work |
| Side panel › Hosts 1 | - | - | !232 / 249 | !247 / 247 | 32 | 40 | 16 | /api/devices /api/host |
| Side panel › Hosts 1 › Back to settings | - | - | !232 / 249 | !245 / 246 | 32 | 40 |  |  |
| Temp chat (open beside the focused tile) › Memory on: the next turn reads the memory and has your tools › Close Chat | 64 / 81 | !245 / 261 | - | - | 48 | 80 |  |  |
| Workers (open beside the focused tile) › Edit › Close Chat | 65 / 80 | !245 / 261 | - | - | 48 | 96 | 4 | /api/work |
| Library (open beside the focused tile) › Close Chat | 81 / 81 | !244 / 245 | - | - | 48 | 96 |  |  |
| Temp chat (open beside the focused tile) › Close Chat | 65 / 81 | !244 / 260 | - | - | 48 | 88 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat › Close Chat | 65 / 81 | !244 / 244 | - | - | 48 | 80 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat › Close Temp chat | 81 / 81 | !244 / 259 | - | - | 48 | 88 |  |  |
| Workers (open beside the focused tile) › Controls for Workers › Close Chat | 81 / 81 | !244 / 259 | - | - | 64 | 88 |  |  |
| Workers (open beside the focused tile) › Archived › Close Chat | 64 / 64 | !243 / 260 | - | - | 48 | 80 | 2 | /api/work |
| double-click a workspace tab (rename) | 80 / 81 | !243 / 325 | - | - | 80 | 88 | 4 | /api/work |
| Open anything (Alt Shift Space) › Chat | !178 / 181 | !242 / 275 | - | - | 31 | 48 |  |  |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Close Chat | 63 / 64 | !242 / 260 | - | - | 46 | 96 | 5 | /api/work |
| Reply › Edit | 64 / 64 | !240 / 242 | - | - | 47 | 72 |  |  |
| Side panel › Settings › Back | - | - | !232 / 249 | !230 / 247 | 32 | 40 |  |  |
| Side panel › Settings › Memory Working | - | - | !232 / 245 | !231 / 248 | 31 | 40 | 16 | /api/summarizer |
| Side panel › Settings › Workers 36 names | - | - | !232 / 248 | !231 / 248 | 32 | 40 | 4 | /api/work |
| Create task | !231 / 231 | !224 / 241 | - | - | 14 | 40 | 4 | /api/library |
| Settings and usage › Larger text | !164 / 180 | !226 / 240 | - | - | 14 | 80 |  |  |
| Settings and usage › Smaller text | !163 / 164 | !225 / 243 | - | - | 13 | 72 |  |  |
| Reply › Cancel reply | 64 / 431 | !223 / 240 | - | - | 47 | 72 | 19 | /api/library |
| Reply › Copy Code › Edit | 43 / 44 | !222 / 234 | - | - | 43 | 80 |  |  |
| New workspace (Alt Shift N) › Temp chat | 49 / 65 | !214 / 214 | - | - | 32 | 64 | 25 | /api/models /api/usage/report |
| Workers › Start a worker | - | - | 81 / 82 | !213 / 245 | 48 | 72 | 51 | /api/models /api/usage/report /api/work-choice |
| More for Chat › Rename › Open anything (Alt Shift Space) | 48 / 64 | !205 / 227 | - | - | 31 | 72 |  |  |
| Reply › Copy Code › Cancel reply | !164 / 214 | !204 / 375 | - | - | 47 | 80 | 4 | /api/work |
| Copy Code › Add to Main | - | - | !199 / 199 | !146 / 146 | 49 | 48 |  |  |
| Side panel › Light: switch to dark | - | - | !199 / 199 | !180 / 181 | 32 | 56 |  |  |
| New workspace (Alt Shift N) › Workers | 65 / 65 | !197 / 228 | - | - | 49 | 72 |  |  |
| New workspace (Alt Shift N) › Controls for the tile › More… | 65 / 66 | !196 / 214 | - | - | 49 | 64 |  |  |
| New workspace (Alt Shift N) › Open anything (Alt Shift Space) | 65 / 65 | !196 / 197 | - | - | 48 | 64 |  |  |
| Access › Accept edits Edits and file commands in | !181 / 181 | !191 / 207 | 65 / 65 | !113 / 128 | 31 | 40 | 65 | /api/access /api/models |
| Access › Manual Asks before edits and commands | !181 / 181 | !186 / 191 | 64 / 65 | !114 / 129 | 31 | 40 | 41 | /api/access /api/models |
| Add to Main › Temp chat › Memory on: the next turn reads the memory and has your tools | - | - | !182 / 182 | !180 / 197 | 32 | 32 | 16 | /api/phone-layout /api/scratchpad/memory |
| Side panel › Light: switch to dark › Dark: switch to light | - | - | !182 / 182 | !179 / 180 | 32 | 48 |  |  |
| Model, run in Claude Code › Opus 5.5 | !181 / 181 | !178 / 178 | 32 / 32 | 30 / 30 | 32 | 40 |  |  |
| New workspace (Alt Shift N) › Controls for the tile › Open anything (Alt Shift Space) | 65 / 65 | !181 / 181 | - | - | 48 | 64 |  |  |
| Temp chat (open beside the focused tile) › Memory on: the next turn reads the memory and has your tools › Memory off: the next turn has only the web | !181 / 181 | !179 / 261 | - | - | 15 | 40 | 16 | /api/scratchpad/memory /api/work |
| Workers › Main · Idle or done | - | - | !181 / 182 | !179 / 180 | 31 | 40 |  |  |
| Add to Main › Temp chat | - | - | 65 / 65 | !180 / 180 | 48 | 56 | 31 | /api/models /api/usage/report |
| New workspace (Alt Shift N) › Library | 49 / 49 | !180 / 198 | - | - | 49 | 64 |  |  |
| New workspace (Alt Shift N) › More… | 49 / 65 | !180 / 213 | - | - | 32 | 72 |  |  |
| Temp chat (open beside the focused tile) › Memory on: the next turn reads the memory and has your tools | !180 / 181 | !177 / 177 | - | - | 14 | 24 | 16 | /api/layout /api/scratchpad/memory |
| Thinking › Low | !180 / 181 | !176 / 177 | 65 / 65 | !113 / 129 | 32 | 40 | 46 | /api/effort /api/models |
| New workspace (Alt Shift N) › Chat and Temp The chat, a temp chat besi › Close Workspace 2 | 48 / 48 | !179 / 195 | - | - | 48 | 72 | 15 | /api/work |
| Temp chat (open beside the focused tile) › Copy Code › Copy | !179 / 180 | !155 / 190 | - | - | 29 | 24 |  |  |
| Workers (open beside the focused tile) › Copy Code › Copy | !179 / 179 | !157 / 189 | - | - | 29 | 40 |  |  |
| Library (open beside the focused tile) › Controls for Library › Move Library: drag onto another tile, or Alt Shift arrow | 47 / 47 | !178 / 210 | - | - | 31 | 48 | 2 | /api/work |
| New workspace (Alt Shift N) › Chat (Alt Shift 1). Double-click to rename; right-click for more. › Close Workspace 2 | 48 / 115 | !178 / 178 | - | - | 31 | 56 |  |  |
| Terminal (open beside the focused tile) › Copy Code › Copy | !178 / 179 | !171 / 188 | - | - | 29 | 40 |  |  |
| Workers (open beside the focused tile) › Controls for Workers › Move Workers: drag onto another tile, or Alt Shift arrow | 48 / 64 | !178 / 193 | - | - | 31 | 56 | 2 | /api/work |
| Controls for Chat › Split side by side › Close Chat | 47 / 47 | !177 / 192 | - | - | 30 | 56 |  |  |
| Workers | - | - | 65 / 65 | !177 / 178 | 48 | 56 |  |  |
| Terminal (open beside the focused tile) › Controls for Terminal › Move Terminal: drag onto another tile, or Alt Shift arrow | 47 / 64 | !176 / 192 | - | - | 31 | 56 |  |  |
| Thinking › Default | !164 / 181 | !175 / 175 | 32 / 32 | 29 / 29 | 32 | 40 |  |  |
| right-click a message (its menu) | !170 / 179 | !175 / 181 | - | - | 36 | 24 | 3 | /api/work |
| Alt Shift N (new workspace) | 47 / 67 | !169 / 171 | - | - | 30 | 48 |  |  |
| Library (open beside the focused tile) › Make something › Document | !165 / 180 | !160 / 161 | - | - | 14 | 32 |  |  |
| bar: tap outside (save the edit) | - | - | !165 / 165 | !162 / 179 | 31 | 32 | 4 | /api/work |
| Library (open beside the focused tile) › Make something › Web page | !164 / 165 | !161 / 161 | - | - | 14 | 32 |  |  |
| Model, run in Claude Code › Sonnet 5.5 | !164 / 180 | !159 / 160 | 65 / 65 | !113 / 128 | 32 | 40 | 39 | /api/model /api/models |
| Workers (open beside the focused tile) › Main · Idle or done | !164 / 164 | !159 / 160 | - | - | 14 | 32 | 3 | /api/layout |
| Library (open beside the focused tile) › Copy Code › Copy | !163 / 179 | !153 / 188 | - | - | 30 | 32 |  |  |
| bar: Undo after a close | - | - | 65 / 65 | !162 / 164 | 49 | 48 |  |  |
| Workers (open beside the focused tile) › Start a worker | 48 / 81 | !161 / 176 | - | - | 31 | 48 | 41 | /api/models /api/usage/report /api/work-choice |
| Library | - | - | 65 / 65 | !160 / 160 | 48 | 56 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat › Move Temp chat: drag onto another tile, or Alt Shift arrow | 48 / 64 | !160 / 161 | - | - | 31 | 64 | 2 | /api/work |
| Controls for Chat › Split side by side › Move Chat: drag onto another tile, or Alt Shift arrow | 48 / 97 | !159 / 259 | - | - | 31 | 72 | 2 | /api/layout |
| Workers › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Close its work | - | - | 49 / 65 | !159 / 179 | 49 | 72 |  |  |
| New workspace (Alt Shift N) | 47 / 47 | !158 / 175 | - | - | 31 | 48 |  |  |
| New workspace (Alt Shift N) › Close Chat | 49 / 49 | !146 / 163 | - | - | 32 | 48 |  |  |
| Workers › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Stop | - | - | 48 / 48 | !146 / 161 | 48 | 48 | 224 | /api/agent/w0/stop /api/work |
| Workspace: Main. Switch workspace › More for Main › More for Main | - | - | 48 / 49 | !145 / 146 | 48 | 48 |  |  |
| Thinking › Open anything (Alt Shift Space) | 32 / 48 | !144 / 145 | - | - | 14 | 56 |  |  |
| Workspace: Main. Switch workspace › New workspace | - | - | 48 / 49 | !144 / 146 | 48 | 48 |  |  |
| Access › Open anything (Alt Shift Space) | 31 / 48 | !143 / 145 | - | - | 14 | 56 |  |  |
| Library (open beside the focused tile) › Make something › Open anything (Alt Shift Space) | 64 / 65 | !143 / 143 | - | - | 47 | 56 |  |  |
| Controls for Chat › Open anything (Alt Shift Space) | 31 / 47 | !141 / 160 | - | - | 31 | 56 |  |  |
| Library (open beside the focused tile) › Copy Code › Open anything (Alt Shift Space) | 31 / 31 | !141 / 142 | - | - | 31 | 48 | 5 | /api/work |
| Library (open beside the focused tile) › Make something › Edit | 80 / 80 | !140 / 141 | - | - | 30 | 48 |  |  |
| Search | 31 / 31 | !140 / 141 | 47 / 48 | !110 / 112 | 31 | 48 | 117 | /api/sources |
| Library (open beside the focused tile) › Plan 1 › Edit | 62 / 64 | !139 / 140 | - | - | 29 | 48 | 2 | /api/work |
| Workers (open beside the focused tile) › Edit | 80 / 80 | !138 / 140 | - | - | 30 | 56 |  |  |
| New workspace (Alt Shift N) › Chat, terminal and workers The chat, the › Edit | !129 / 129 | !137 / 138 | - | - | 45 | 80 | 5 | /api/work |
| Terminal (open beside the focused tile) › Controls for Terminal › Edit | 80 / 80 | !137 / 139 | - | - | 30 | 48 |  |  |
| Add to Main › Temp chat › Add to Main | - | - | 49 / 49 | !130 / 131 | 49 | 56 |  |  |
| Copy Code › Scroll to bottom | 64 / 81 | 93 / 106 | 65 / 182 | !130 / 180 | 48 | 40 |  |  |
| Library › Plan 1 | - | - | 48 / 49 | !130 / 130 | 48 | 48 |  |  |
| Library › plan-0 › Back to the Library | - | - | 48 / 49 | !130 / 131 | 48 | 56 |  |  |
| Model, run in Claude Code › Add to Main | - | - | 48 / 48 | !130 / 146 | 32 | 56 |  |  |
| New workspace (Alt Shift N) › Chat, terminal and workers The chat, the › Close Chat | !130 / 131 | !110 / 111 | - | - | 31 | 80 |  |  |
| New workspace (Alt Shift N) › Chat, terminal and workers The chat, the › Close the terminal | !130 / 131 | !125 / 192 | - | - | 46 | 80 | 2 | /api/work |
| New workspace (Alt Shift N) › Controls for the tile › Close Chat | 49 / 49 | !130 / 131 | - | - | 32 | 40 |  |  |
| Workers › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Add to Main | - | - | 49 / 49 | !130 / 130 | 49 | 48 |  |  |
| Workspace: Main. Switch workspace › More for Main › Rename | - | - | 49 / 49 | !130 / 130 | 49 | 56 |  |  |
| Access › Add to Main | - | - | 49 / 49 | !129 / 163 | 32 | 48 |  |  |
| Add to Main › Chat | - | - | 48 / 49 | !129 / 130 | 48 | 48 |  |  |
| Library › plan-0 | - | - | 48 / 49 | !129 / 146 | 48 | 56 |  |  |
| Library › plan-0 › Add to Main | - | - | 48 / 49 | !129 / 130 | 48 | 48 |  |  |
| Workspace: Main. Switch workspace › New workspace › More for Workspace 2 | - | - | 49 / 49 | !129 / 145 | 49 | 48 |  |  |
| Library (open beside the focused tile) › Controls for Library › Open anything (Alt Shift Space) | 65 / 81 | !128 / 159 | - | - | 31 | 48 |  |  |
| Library (open beside the focused tile) › Plan 1 › Back to the Library | 32 / 48 | !128 / 144 | - | - | 31 | 48 |  |  |
| Library (open beside the focused tile) › plan-0 › Open anything (Alt Shift Space) | 31 / 31 | !128 / 144 | - | - | 31 | 48 |  |  |
| Model, run in Claude Code › Open anything (Alt Shift Space) | 47 / 48 | !128 / 128 | - | - | 15 | 56 |  |  |
| New workspace (Alt Shift N) › Chat (Alt Shift 1). Double-click to rename; right-click for more. › Workspace 2 (Alt Shift 2). Double-click to rename; right-click for more. | 47 / 98 | !128 / 128 | - | - | 31 | 40 |  |  |
| New workspace (Alt Shift N) › Terminal › Open anything (Alt Shift Space) | 31 / 31 | !128 / 143 | - | - | 31 | 48 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat › Full screen | 81 / 97 | !128 / 144 | - | - | 31 | 48 |  |  |
| Workspace: Main. Switch workspace | - | - | 48 / 48 | !128 / 128 | 48 | 56 |  |  |
| Workspace: Main. Switch workspace › New workspace › More for Main | - | - | 48 / 48 | !128 / 130 | 48 | 48 |  |  |
| Add to Main | - | - | 48 / 48 | !127 / 128 | 48 | 48 |  |  |
| Controls for Chat › Split side by side › Open anything (Alt Shift Space) | 81 / 81 | !127 / 129 | - | - | 31 | 48 | 2 | /api/layout |
| Open anything (Alt Shift Space) | 31 / 31 | !127 / 127 | - | - | 31 | 56 |  |  |
| Temp chat (open beside the focused tile) › Edit › Open anything (Alt Shift Space) | 31 / 47 | !127 / 143 | - | - | 31 | 48 |  |  |
| Workers (open beside the focused tile) › Archived › Open anything (Alt Shift Space) | 32 / 32 | !127 / 128 | - | - | 32 | 48 |  |  |
| Workers (open beside the focused tile) › Controls for Workers › Open anything (Alt Shift Space) | 80 / 81 | !127 / 128 | - | - | 31 | 48 |  |  |
| Workspace: Main. Switch workspace › Main Chat, Workers, Library | - | - | 48 / 49 | !127 / 129 | 48 | 56 |  |  |
| Controls for Chat › Split side by side › More… | 64 / 98 | !126 / 143 | - | - | 47 | 56 | 2 | /api/layout |
| Library (open beside the focused tile) › plan-0 › Edit | 63 / 79 | !126 / 126 | - | - | 29 | 48 | 2 | /api/work |
| Pin › Edit › Open anything (Alt Shift Space) | 31 / 48 | !126 / 128 | - | - | 31 | 56 |  |  |
| Temp chat (open beside the focused tile) › Copy Code › Open anything (Alt Shift Space) | 31 / 32 | !126 / 128 | - | - | 31 | 48 |  |  |
| Thinking › Add to Main | - | - | 49 / 49 | !126 / 129 | 32 | 48 |  |  |
| Thinking › More for Chat | 31 / 48 | !126 / 127 | - | - | 14 | 48 |  |  |
| Access › More for Chat | 31 / 31 | !125 / 125 | - | - | 14 | 48 |  |  |
| Alt Shift Space (open anything) | 31 / 41 | !125 / 127 | - | - | 31 | 56 |  |  |
| Library (open beside the focused tile) › Plan 1 › More for Chat | 31 / 31 | !125 / 125 | - | - | 31 | 40 |  |  |
| Temp chat (open beside the focused tile) › Edit | 64 / 80 | !125 / 125 | - | - | 30 | 48 |  |  |
| Terminal (open beside the focused tile) › Controls for Terminal › Open anything (Alt Shift Space) | 80 / 97 | !125 / 127 | - | - | 31 | 56 |  |  |
| Terminal (open beside the focused tile) › Copy Code › Open anything (Alt Shift Space) | 31 / 47 | !125 / 126 | - | - | 31 | 48 |  |  |
| Workers (open beside the focused tile) › Open anything (Alt Shift Space) | 31 / 31 | !125 / 159 | - | - | 31 | 48 | 2 | /api/work |
| Controls for Chat › More for Chat | 31 / 31 | !124 / 140 | - | - | 31 | 48 |  |  |
| Ctrl K (search) | 41 / 50 | !124 / 137 | 40 / 43 | !101 / 106 | 7 | 48 | 123 | /api/sources |
| Library (open beside the focused tile) › Controls for Library › Edit | 80 / 81 | !124 / 139 | - | - | 30 | 64 |  |  |
| More for Chat › Rename › Edit | 80 / 81 | !124 / 126 | - | - | 31 | 48 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat › Edit | 97 / 98 | !124 / 124 | - | - | 30 | 48 |  |  |
| Terminal (open beside the focused tile) › Controls for Terminal › Copy Code | 31 / 97 | !122 / 123 | - | - | 30 | 48 | 5 | /api/work |
| Thinking › Close Chat | 31 / 48 | !122 / 128 | - | - | 15 | 48 |  |  |
| Workers (open beside the focused tile) › Controls for Workers › Edit | 80 / 80 | !122 / 139 | - | - | 30 | 56 |  |  |
| Library (open beside the focused tile) › Plan 1 › Controls for Plan 1 | !121 / 131 | 6 / 8 (no change) | - | - | 126 | 0 | 2 | /api/layout |
| Library (open beside the focused tile) › plan-0 › Controls for plan-0 | !120 / 135 | 7 / 13 (no change) | - | - | 69 | 0 | 2 | /api/apps/view /api/layout |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Edit | 94 / 97 | !119 / 135 | - | - | 30 | 64 | 4 | /api/work |
| bar: hold an item (edit the bar) | - | - | 39 / 44 | !118 / 134 | 39 | 0 | 2 | /api/phone-layout |
| drag the split between two tiles | !102 / 111 | !117 / 117 | - | - | 32 | 48 | 161 | /api/work |
| Library › Add to Main | - | - | 49 / 49 | !114 / 130 | 49 | 48 |  |  |
| Library › Make something › Add to Main | - | - | 48 / 49 | !114 / 130 | 48 | 48 |  |  |
| Side panel › Settings › Agents Yours | - | - | 49 / 49 | !114 / 129 | 49 | 48 |  |  |
| Workers › Add to Main | - | - | 48 / 48 | !114 / 130 | 48 | 56 | 3 | /api/work |
| Workers › Archived › Add to Main | - | - | 48 / 49 | !114 / 147 | 48 | 48 |  |  |
| Workers › Start a worker › Close | - | - | 48 / 49 | !114 / 129 | 48 | 56 |  |  |
| Create task › Edit › Open anything (Alt Shift Space) | 48 / 48 | !113 / 125 | - | - | 48 | 48 |  |  |
| New workspace (Alt Shift N) › Library › Open anything (Alt Shift Space) | 32 / 32 | !113 / 130 | - | - | 32 | 40 |  |  |
| New workspace (Alt Shift N) › Workers › Open anything (Alt Shift Space) | 32 / 32 | !113 / 128 | - | - | 32 | 40 |  |  |
| Workspace: Main. Switch workspace › More for Main | - | - | 49 / 49 | !113 / 129 | 49 | 48 |  |  |
| Workspace: Main. Switch workspace › More for Main › Close workspace | - | - | 48 / 49 | !113 / 129 | 48 | 48 |  |  |
| Model, run in Claude Code › Close Chat | 31 / 32 | !112 / 126 | - | - | 14 | 56 |  |  |
| New workspace (Alt Shift N) › Temp chat › Open anything (Alt Shift Space) | 32 / 48 | !112 / 129 | - | - | 32 | 56 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat › Open anything (Alt Shift Space) | 97 / 98 | !112 / 126 | - | - | 31 | 48 |  |  |
| Access › Close Chat | 31 / 31 | !111 / 127 | - | - | 14 | 40 |  |  |
| Create task › Copy Code › Open anything (Alt Shift Space) | 31 / 48 | !111 / 112 | - | - | 31 | 48 | 3 | /api/work |
| Edit › Copy Code › Close Chat | 31 / 31 | !111 / 118 | - | - | 31 | 24 |  |  |
| Edit › Copy Code › Open anything (Alt Shift Space) | 31 / 31 | !111 / 112 | - | - | 31 | 48 | 4 | /api/work |
| Library (open beside the focused tile) › Edit › Open anything (Alt Shift Space) | 31 / 44 | !111 / 126 | - | - | 31 | 48 |  |  |
| Library (open beside the focused tile) › Plan 1 › Open anything (Alt Shift Space) | 31 / 31 | !111 / 142 | - | - | 31 | 48 |  |  |
| Library (open beside the focused tile) › plan-0 › Back to the Library | 31 / 45 | !111 / 127 | - | - | 31 | 56 |  |  |
| New workspace (Alt Shift N) › Terminal › Close the terminal | 31 / 32 | !111 / 112 | - | - | 31 | 40 |  |  |
| Pin › Copy Code › Open anything (Alt Shift Space) | 31 / 31 | !111 / 126 | - | - | 31 | 48 |  |  |
| Pin › Open anything (Alt Shift Space) | 31 / 32 | !111 / 127 | - | - | 31 | 48 |  |  |
| Reply › More for Chat | 31 / 98 | !111 / 128 | - | - | 31 | 48 |  |  |
| Reply › Open anything (Alt Shift Space) | 32 / 47 | !111 / 126 | - | - | 32 | 48 |  |  |
| Temp chat (open beside the focused tile) › Open anything (Alt Shift Space) | 31 / 47 | !111 / 128 | - | - | 31 | 56 | 2 | /api/work |
| Controls for Chat › Split side by side › Edit | 81 / 96 | !110 / 125 | - | - | 30 | 48 |  |  |
| Controls for Chat › Split side by side › More for Chat | 81 / 98 | !110 / 111 | - | - | 31 | 40 | 2 | /api/layout |
| Library (open beside the focused tile) › Edit | 64 / 81 | !110 / 138 | - | - | 30 | 48 | 8 | /api/library |
| Library (open beside the focused tile) › Open anything (Alt Shift Space) | 31 / 31 | !110 / 111 | - | - | 31 | 48 | 5 | /api/work |
| Model, run in Claude Code › More for Chat | 31 / 31 | !110 / 111 | - | - | 14 | 48 |  |  |
| New workspace (Alt Shift N) › Chat, terminal and workers The chat, the › Open anything (Alt Shift Space) | 47 / 97 | !110 / 141 | - | - | 47 | 80 |  |  |
| Pin › Copy Code › Close Chat | 31 / 31 | !110 / 111 | - | - | 31 | 40 |  |  |
| Pin › More for Chat | 31 / 32 | !110 / 127 | - | - | 31 | 40 |  |  |
| Reply › Copy Code › Open anything (Alt Shift Space) | 31 / 32 | !110 / 126 | - | - | 31 | 48 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat › Copy Code | 31 / 113 | !110 / 110 | - | - | 31 | 48 | 6 | /api/work |
| Temp chat (open beside the focused tile) › Edit › More for Chat | 31 / 31 | !110 / 111 | - | - | 31 | 48 |  |  |
| Terminal (open beside the focused tile) › Edit › Open anything (Alt Shift Space) | 31 / 31 | !110 / 127 | - | - | 31 | 48 |  |  |
| Terminal (open beside the focused tile) › Open anything (Alt Shift Space) | 30 / 30 | !110 / 126 | - | - | 30 | 48 | 2 | /api/work |
| Workers (open beside the focused tile) › Copy Code › Open anything (Alt Shift Space) | 31 / 47 | !110 / 112 | - | - | 31 | 64 |  |  |
| Workers (open beside the focused tile) › Edit › Open anything (Alt Shift Space) | 31 / 31 | !110 / 127 | - | - | 31 | 48 |  |  |
| Controls for Chat › Split side by side › Copy | 80 / 97 | !109 / 109 | - | - | 30 | 48 |  |  |
| Copy Code › Close Chat | 30 / 31 | !109 / 111 | - | - | 30 | 40 | 2 | /api/work |
| Library (open beside the focused tile) › Controls for Library › Copy Code | 47 / 113 | !109 / 125 | - | - | 31 | 56 | 12 | /api/work |
| More for Chat › Rename | 31 / 31 | !109 / 109 | - | - | 31 | 56 |  |  |
| More for Chat › Save as template › Close Chat | 31 / 64 | !109 / 377 | - | - | 31 | 24 |  |  |
| New workspace (Alt Shift N) › Chat (open beside the focused tile) › Open anything (Alt Shift Space) | 47 / 115 | !109 / 128 | - | - | 31 | 48 |  |  |
| New workspace (Alt Shift N) › Chat and Temp The chat, a temp chat besi › Close Chat | 81 / 98 | !109 / 125 | - | - | 31 | 40 |  |  |
| New workspace (Alt Shift N) › Chat and Temp The chat, a temp chat besi › Edit | 80 / 81 | !109 / 125 | - | - | 30 | 64 | 4 | /api/work |
| Pin › Edit › More for Chat | 31 / 32 | !109 / 126 | - | - | 31 | 40 |  |  |
| Settings and usage › Agents | 31 / 46 | !109 / 123 | - | - | 31 | 40 |  |  |
| Settings and usage › Models | 47 / 47 | !109 / 125 | - | - | 14 | 32 | 19 | /api/models /api/synopsis /api/worker-model |
| Temp chat (open beside the focused tile) › Copy Code | 31 / 114 | !109 / 141 | - | - | 30 | 40 | 4 | /api/work |
| Terminal (open beside the focused tile) › Controls for Terminal › More for Chat | 80 / 97 | !109 / 110 | - | - | 30 | 48 |  |  |
| Terminal (open beside the focused tile) › More for Chat | 31 / 81 | !109 / 124 | - | - | 31 | 48 |  |  |
| Workers (open beside the focused tile) › Archived › Edit | 64 / 80 | !109 / 109 | - | - | 30 | 48 |  |  |
| Workers (open beside the focused tile) › Controls for Workers › Copy Code | 31 / 96 | !109 / 138 | - | - | 31 | 64 | 10 | /api/work |
| Library (open beside the focused tile) › Plan 1 › Copy Code | 31 / 114 | !108 / 109 | - | - | 30 | 56 |  |  |
| More for Chat | 31 / 46 | !108 / 110 | - | - | 31 | 48 |  |  |
| More for Chat › Rename › Copy Code | 31 / 113 | !108 / 109 | - | - | 30 | 40 | 4 | /api/work |
| New workspace (Alt Shift N) › Chat, terminal and workers The chat, the › More for Workspace 2 | 30 / 97 | !108 / 109 | - | - | 30 | 80 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat › Copy | 80 / 96 | !108 / 124 | - | - | 30 | 48 |  |  |
| Temp chat (open beside the focused tile) › Memory on: the next turn reads the memory and has your tools › Edit | 80 / 96 | !108 / 124 | - | - | 30 | 48 |  |  |
| Terminal (open beside the focused tile) › Copy Code | 30 / 96 | !108 / 124 | - | - | 30 | 48 |  |  |
| Terminal (open beside the focused tile) › Edit | 95 / 97 | !108 / 152 | - | - | 30 | 48 |  |  |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Open anything (Alt Shift Space) | 30 / 31 | !108 / 124 | - | - | 30 | 64 |  |  |
| Controls for Chat › Split side by side › Copy Code | 47 / 113 | !107 / 123 | - | - | 31 | 48 | 4 | /api/work |
| Library (open beside the focused tile) › Controls for Library › Copy | 80 / 97 | !107 / 139 | - | - | 30 | 48 |  |  |
| Library (open beside the focused tile) › Make something › Copy | 80 / 80 | !107 / 124 | - | - | 30 | 48 |  |  |
| Workers (open beside the focused tile) › Copy Code | 30 / 97 | !107 / 108 | - | - | 30 | 48 | 14 | /api/work |
| Alt Shift F (tile full screen) | 26 / 29 | !106 / 115 | - | - | 26 | 24 |  |  |
| Library (open beside the focused tile) › Make something › Copy Code | 31 / 114 | !106 / 107 | - | - | 30 | 48 | 11 | /api/work |
| Workers (open beside the focused tile) › Controls for Workers › Copy | 79 / 80 | !105 / 124 | - | - | 30 | 64 |  |  |
| Library (open beside the focused tile) › plan-0 › Copy Code | 30 / 113 | !104 / 107 | - | - | 30 | 48 |  |  |
| Terminal (open beside the focused tile) › Controls for Terminal › Copy | 96 / 97 | !103 / 105 | - | - | 30 | 48 |  |  |
| bar: swipe to the next workspace | - | - | 37 / 37 | !103 / 104 | 223 | 0 | 3 | /api/phone-layout |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Copy Code | 30 / 95 | !101 / 104 | - | - | 29 | 48 |  |  |
| scroll the chat up (20 wheel steps) | 98 / 98 | 100 / 100 | 66 / 66 | 67 / 67 | 48 | 0 | 4 | /api/work |
| Alt Shift / (show the keys) | 24 / 37 | 98 / 119 | - | - | 24 | 40 |  |  |
| Side panel › Settings › Connections None | - | - | 49 / 49 | 98 / 128 | 32 | 40 | 26 | /api/builtins /api/connections/providers /api/outside |
| New workspace (Alt Shift N) › Temp chat › More for Workspace 2 | 32 / 32 | 97 / 130 | - | - | 32 | 40 |  |  |
| Edit › Open anything (Alt Shift Space) | 31 / 31 | 96 / 126 | - | - | 31 | 48 | 3 | /api/library |
| New workspace (Alt Shift N) › Chat and Temp The chat, a temp chat besi › Open anything (Alt Shift Space) | 32 / 98 | 96 / 127 | - | - | 32 | 48 |  |  |
| New workspace (Alt Shift N) › Terminal › More for Workspace 2 | 31 / 32 | 96 / 111 | - | - | 31 | 40 |  |  |
| New workspace (Alt Shift N) › Workers › More for Workspace 2 | 32 / 32 | 96 / 98 | - | - | 32 | 40 |  |  |
| Copy Code › Open anything (Alt Shift Space) | 31 / 47 | 95 / 128 | - | - | 31 | 56 |  |  |
| Create task › Copy Code › Close Chat | 32 / 32 | 95 / 103 | - | - | 32 | 32 |  |  |
| Create task › Copy Code › More for Chat | 31 / 32 | 95 / 110 | - | - | 31 | 40 |  |  |
| Create task › Edit › More for Chat | 31 / 31 | 95 / 111 | - | - | 31 | 48 |  |  |
| Create task › More for Chat | 32 / 48 | 95 / 111 | - | - | 32 | 48 | 3 | /api/work |
| Create task › Open anything (Alt Shift Space) | 31 / 32 | 95 / 137 | - | - | 31 | 48 | 4 | /api/library |
| Library (open beside the focused tile) › More for Chat | 31 / 115 | 95 / 110 | - | - | 31 | 40 |  |  |
| New workspace (Alt Shift N) › Chat (Alt Shift 1). Double-click to rename; right-click for more. › More for Chat | 32 / 114 | 95 / 95 | - | - | 31 | 48 |  |  |
| New workspace (Alt Shift N) › Chat (Alt Shift 1). Double-click to rename; right-click for more. › Open anything (Alt Shift Space) | 32 / 114 | 95 / 110 | - | - | 31 | 48 |  |  |
| New workspace (Alt Shift N) › Library › More for Workspace 2 | 32 / 32 | 95 / 96 | - | - | 32 | 32 |  |  |
| Pin › Copy Code › More for Chat | 31 / 31 | 95 / 111 | - | - | 31 | 48 |  |  |
| Temp chat (open beside the focused tile) › Memory on: the next turn reads the memory and has your tools › Open anything (Alt Shift Space) | 31 / 31 | 95 / 111 | - | - | 31 | 48 |  |  |
| Copy Code › More for Chat | 31 / 31 | 94 / 111 | - | - | 31 | 40 |  |  |
| Create task › Edit › Close Chat | 32 / 32 | 94 / 110 | - | - | 32 | 32 | 2 | /api/work |
| Edit › Copy Code › More for Chat | 32 / 32 | 94 / 94 | - | - | 32 | 48 |  |  |
| Library (open beside the focused tile) › Controls for Library › More for Chat | 81 / 81 | 94 / 94 | - | - | 31 | 40 |  |  |
| Library (open beside the focused tile) › Copy Code › More for Chat | 31 / 31 | 94 / 110 | - | - | 31 | 48 |  |  |
| Library (open beside the focused tile) › Edit › More for Chat | 31 / 31 | 94 / 109 | - | - | 31 | 40 |  |  |
| Library (open beside the focused tile) › Make something › More for Chat | 64 / 81 | 94 / 95 | - | - | 31 | 40 |  |  |
| Library (open beside the focused tile) › Plan 1 | 31 / 97 | 94 / 110 | - | - | 30 | 40 |  |  |
| Library (open beside the focused tile) › plan-0 | 31 / 31 | 94 / 111 | - | - | 31 | 40 |  |  |
| Library (open beside the focused tile) › plan-0 › More for Chat | 31 / 31 | 94 / 95 | - | - | 31 | 40 |  |  |
| More for Chat › Save as template | 31 / 31 | 94 / 94 | - | - | 31 | 40 |  |  |
| New workspace (Alt Shift N) › Chat (open beside the focused tile) › Close Chat | 31 / 115 | 94 / 95 | - | - | 31 | 32 |  |  |
| New workspace (Alt Shift N) › Chat (open beside the focused tile) › Close Workspace 2 | 31 / 31 | 94 / 95 | - | - | 31 | 32 | 2 | /api/work |
| New workspace (Alt Shift N) › Chat and Temp The chat, a temp chat besi › More for Workspace 2 | 31 / 97 | 94 / 95 | - | - | 31 | 40 |  |  |
| Pin › Close Chat | 31 / 31 | 94 / 110 | - | - | 31 | 32 | 2 | /api/work |
| Pin › Edit › Close Chat | 32 / 32 | 94 / 111 | - | - | 32 | 32 |  |  |
| Reply › Close Chat | 31 / 31 | 94 / 111 | - | - | 31 | 32 | 2 | /api/work |
| Reply › Copy Code › More for Chat | 31 / 31 | 94 / 110 | - | - | 31 | 48 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat › More for Chat | 81 / 98 | 94 / 95 | - | - | 31 | 48 |  |  |
| Temp chat (open beside the focused tile) › Copy Code › More for Chat | 31 / 31 | 94 / 95 | - | - | 31 | 40 |  |  |
| Temp chat (open beside the focused tile) › More for Chat | 31 / 114 | 94 / 109 | - | - | 31 | 48 |  |  |
| Terminal (open beside the focused tile) › Edit › More for Chat | 31 / 31 | 94 / 108 | - | - | 31 | 48 |  |  |
| Workers (open beside the focused tile) › Archived › More for Chat | 31 / 31 | 94 / 95 | - | - | 31 | 40 |  |  |
| Workers (open beside the focused tile) › Controls for Workers › More for Chat | 62 / 64 | 94 / 95 | - | - | 31 | 48 |  |  |
| Workers (open beside the focused tile) › Copy Code › More for Chat | 31 / 31 | 94 / 109 | - | - | 31 | 48 |  |  |
| Workers (open beside the focused tile) › Edit › More for Chat | 31 / 31 | 94 / 109 | - | - | 31 | 40 |  |  |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Close its work | 29 / 30 | 94 / 124 | - | - | 29 | 48 |  |  |
| Close Chat | 31 / 31 | 93 / 94 | - | - | 31 | 32 | 2 | /api/work |
| Create task › Close Chat | 31 / 31 | 93 / 94 | - | - | 31 | 32 | 2 | /api/work |
| More for Chat › Rename › Copy | 79 / 80 | 93 / 107 | - | - | 30 | 40 |  |  |
| Reply › Copy Code › Close Chat | 31 / 31 | 93 / 109 | - | - | 31 | 32 | 4 | /api/work |
| Settings and usage › Models › Codex on | 48 / 114 | 93 / 108 | - | - | 14 | 24 | 15 | /api/harness /api/models /api/work |
| Workers (open beside the focused tile) › More for Chat | 31 / 80 | 93 / 122 | - | - | 31 | 48 |  |  |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Stop | 29 / 30 | 93 / 107 | - | - | 29 | 56 | 186 | /api/agent/w0/stop /api/layout /api/work |
| Edit › Close Chat | 31 / 31 | 92 / 112 | - | - | 31 | 32 | 2 | /api/work |
| Edit › More for Chat | 32 / 32 | 92 / 111 | - | - | 32 | 40 | 5 | /api/work |
| Library (open beside the focused tile) › Copy Code | 30 / 113 | 92 / 107 | - | - | 29 | 48 | 12 | /api/work |
| Settings and usage › Usage | 31 / 47 | 92 / 92 | - | - | 14 | 32 | 13 | /api/usage/spend |
| Temp chat (open beside the focused tile) › Memory on: the next turn reads the memory and has your tools › Copy Code | 31 / 113 | 92 / 92 | - | - | 30 | 48 | 4 | /api/work |
| Terminal (open beside the focused tile) › Copy Code › More for Chat | 31 / 31 | 92 / 107 | - | - | 31 | 48 |  |  |
| Workers (open beside the focused tile) › Archived › Copy Code | 30 / 114 | 92 / 109 | - | - | 30 | 40 | 11 | /api/work |
| right-click a workspace tab (its menu) | 31 / 31 | 92 / 109 | - | - | 31 | 40 |  |  |
| Controls for Chat › Close Chat | 30 / 30 | 91 / 91 | - | - | 30 | 32 |  |  |
| New workspace (Alt Shift N) › Chat, terminal and workers The chat, the › Copy Code | 30 / 112 | 91 / 91 | - | - | 30 | 80 |  |  |
| Settings and usage › Models › Default | 47 / 114 | 91 / 93 | - | - | 14 | 24 | 14 | /api/harness /api/models /api/work |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › More for Chat | 30 / 30 | 90 / 92 | - | - | 30 | 64 |  |  |
| type 45 keys in the composer (slowest key) | 24 / 24 | 32 / 32 | 40 / 40 | 88 / 88 | 16 | 40 | 4 | /api/work |
| Create task › Edit › Cancel | 81 / 81 | 58 / 76 | - | - | 14 | 24 |  |  |
| Library (open beside the focused tile) › Make something › Close | 81 / 81 | 9 / 9 | - | - | 14 | 24 |  |  |
| Side panel › Settings › Usage Claude, ChatGPT | - | - | 49 / 49 | 81 / 95 | 32 | 40 | 16 | /api/usage/spend |
| Controls for Chat › Edit | 80 / 80 | 40 / 57 | - | - | 13 | 24 |  |  |
| Create task › Edit | 80 / 80 | 76 / 77 | - | - | 13 | 24 |  |  |
| Model, run in Claude Code › Copy | 80 / 80 | 77 / 77 | - | - | 14 | 24 |  |  |
| Model, run in Claude Code › Edit | 80 / 80 | 59 / 74 | - | - | 14 | 32 |  |  |
| New workspace (Alt Shift N) › Chat (open beside the focused tile) › Edit | 80 / 80 | 58 / 76 | - | - | 14 | 24 | 3 | /api/layout /api/work |
| Pin › Edit | 80 / 81 | 58 / 73 | - | - | 13 | 24 |  |  |
| Temp chat (open beside the focused tile) › Copy Code › Edit | 80 / 80 | 36 / 54 | - | - | 13 | 24 | 2 | /api/layout |
| New workspace (Alt Shift N) › Chat (Alt Shift 1). Double-click to rename; right-click for more. › Edit | 79 / 80 | 59 / 75 | - | - | 13 | 24 | 4 | /api/layout /api/work |
| New workspace (Alt Shift N) › Chat (open beside the focused tile) › More for Workspace 2 | 32 / 114 | 79 / 93 | - | - | 31 | 40 |  |  |
| New workspace (Alt Shift N) › Temp chat › Close Chat | 15 / 32 | 79 / 79 | - | - | 15 | 32 |  |  |
| Pin › Copy Code › Copy | 79 / 163 | 57 / 155 | - | - | 30 | 40 |  |  |
| Temp chat (open beside the focused tile) › Memory on: the next turn reads the memory and has your tools › More for Chat | 31 / 31 | 79 / 79 | - | - | 31 | 40 |  |  |
| Terminal (open beside the focused tile) › Copy Code › Edit | 79 / 96 | 66 / 72 | - | - | 12 | 40 | 4 | /api/layout /api/library |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Controls for Reed | 79 / 90 | 2 / 9 (no change) | - | - | 85 | 0 |  |  |
| Library (open beside the focused tile) › Copy Code › Edit | 78 / 80 | 38 / 72 | - | - | 13 | 24 | 2 | /api/layout |
| New workspace (Alt Shift N) › Chat (Alt Shift 1). Double-click to rename; right-click for more. › Close Chat | 31 / 31 | 78 / 78 | - | - | 31 | 40 | 3 | /api/work |
| Pin | 78 / 80 | 55 / 57 | - | - | 13 | 24 |  |  |
| Create task › Copy Code › Copy | 63 / 163 | 76 / 158 | - | - | 30 | 32 | 2 | /api/work |
| New workspace (Alt Shift N) › Chat and Temp The chat, a temp chat besi › Copy Code | 31 / 114 | 76 / 77 | - | - | 30 | 40 |  |  |
| New workspace (Alt Shift N) › Terminal › Close Chat | 31 / 32 | 76 / 77 | - | - | 31 | 32 |  |  |
| Settings and usage › Connections | 31 / 47 | 76 / 91 | - | - | 13 | 24 | 13 | /api/builtins /api/connections/providers /api/outside |
| Thinking › Copy | 64 / 80 | 76 / 76 | - | - | 14 | 24 |  |  |
| Thinking › Edit | 64 / 80 | 74 / 75 | - | - | 13 | 24 |  |  |
| Copy Code › Copy | 63 / 164 | 68 / 157 | - | - | 30 | 32 | 3 | /api/work |
| Reply › Copy Code › Copy | 63 / 162 | 67 / 172 | - | - | 29 | 32 |  |  |
| Workers (open beside the focused tile) › Copy Code › Edit | 63 / 63 | 67 / 75 | - | - | 12 | 40 | 2 | /api/layout |
| Access › Copy | 64 / 64 | 56 / 59 | - | - | 14 | 24 |  |  |
| Access › Edit | 64 / 80 | 38 / 75 | - | - | 13 | 24 |  |  |
| More for Chat › Save as template › Edit | 64 / 80 | 59 / 75 | - | - | 14 | 24 | 2 | /api/layout |
| New workspace (Alt Shift N) › Library › Close Chat | 16 / 32 | 64 / 80 | - | - | 16 | 24 |  |  |
| Controls for Chat › Copy | 63 / 80 | 38 / 57 | - | - | 30 | 24 |  |  |
| Edit | 63 / 79 | 54 / 74 | - | - | 13 | 24 |  |  |
| New workspace (Alt Shift N) › Workers › Close Chat | 31 / 32 | 62 / 78 | - | - | 31 | 24 |  |  |
| More for Chat › Save as template › Copy | 14 / 64 | 59 / 59 | - | - | 30 | 16 |  |  |
| Copy Code › Edit | 24 / 27 | 48 / 48 | - | - | 23 | 32 | 2 | /api/work |
| Workers › What the colours mean | - | - | 48 / 48 | 47 / 48 | 31 | 40 |  |  |
| Settings and usage › Hosts | 31 / 31 | 42 / 42 | - | - | 14 | 24 | 13 | /api/devices /api/host |
| hold a message (its menu) | - | - | 27 / 29 | 42 / 48 | 27 | 0 | 4 | /api/work |
| Create task › Copy Code › Edit | 24 / 28 | 41 / 49 | - | - | 11 | 32 | 3 | /api/work |
| Pin › Copy Code › Edit | 26 / 26 | 40 / 50 | - | - | 9 | 40 | 6 | /api/work |
| Edit › Copy Code › Edit | 27 / 80 | 38 / 38 | - | - | 12 | 32 | 2 | /api/work |
| Access | 14 / 31 | 24 / 25 | 32 / 48 | 29 / 46 | 32 | 40 |  |  |
| Library › Find in the Library | - | - | 32 / 32 | 30 / 30 | 32 | 40 |  |  |
| Library › Make something | - | - | 32 / 32 | 29 / 30 | 32 | 40 |  |  |
| Library › Make something › Close | - | - | 32 / 32 | 31 / 31 | 32 | 40 |  |  |
| Library › Make something › Document | - | - | 32 / 32 | 30 / 47 | 32 | 40 |  |  |
| Library › Make something › Web page | - | - | 32 / 32 | 30 / 30 | 32 | 40 |  |  |
| Model, run in Claude Code | 14 / 14 | 24 / 26 | 32 / 32 | 29 / 43 | 32 | 40 |  |  |
| Search › Newest | 14 / 14 | 25 / 26 | 32 / 32 | 28 / 30 | 32 | 32 |  |  |
| Side panel › Hosts 1 › Add a host By address or link | - | - | 32 / 32 | 31 / 31 | 32 | 32 |  |  |
| Workers › Archived | - | - | 32 / 32 | 30 / 30 | 32 | 40 |  |  |
| Workers › Watches2 | - | - | 32 / 32 | 29 / 29 | 32 | 40 |  |  |
| Claude use: Week 42% | 14 / 14 (no change) | 6 / 7 (no change) | 31 / 32 | 28 / 29 | 31 | 40 |  |  |
| Search › Close search | 14 / 14 | 9 / 91 | 31 / 32 | 29 / 30 | 31 | 48 |  |  |
| Settings and usage › Models › Sign in | 31 / 31 | 26 / 27 | - | - | 14 | 24 | 14 | /api/signin |
| Thinking | 14 / 14 | 24 / 26 | 31 / 32 | 28 / 29 | 31 | 40 |  |  |
| Controls for Chat › Close Chat › More… | 15 / 15 | 30 / 31 | - | - | 15 | 32 |  |  |
| Controls for Chat › Close Chat › Open anything (Alt Shift Space) | 15 / 15 | 30 / 31 | - | - | 15 | 32 |  |  |
| New workspace (Alt Shift N) › Close Chat › More… | 15 / 15 | 30 / 31 | - | - | 15 | 24 | 2 | /api/work |
| New workspace (Alt Shift N) › Close Chat › Open anything (Alt Shift Space) | 15 / 16 | 30 / 32 | - | - | 15 | 32 |  |  |
| Settings and usage › Hosts › Add a host By address or link | 14 / 14 | 26 / 26 | - | - | 14 | 32 |  |  |
| Settings and usage › Memory | 14 / 14 | 26 / 26 | - | - | 14 | 24 | 14 | /api/summarizer |
| Workers (open beside the focused tile) › Watches2 | 14 / 14 | 26 / 26 | - | - | 14 | 32 | 2 | /api/layout |
| Access › Copy Code | 14 / 114 | 25 / 41 | 15 / 15 (no change) | 12 / 13 (no change) | 14 | 40 |  |  |
| Create task › Cancel | 14 / 80 | 25 / 41 | - | - | 14 | 24 |  |  |
| Create task › Copy Code › Cancel | 14 / 114 | 24 / 75 | - | - | 14 | 24 |  |  |
| Pin › Unpin | 14 / 63 | 24 / 58 | - | - | 13 | 24 |  |  |
| Reply › Reply to Reply 59. The harness came back | 15 / 1231 | 24 / 309 | - | - | 15 | 24 | 3 | /api/work |
| Settings and usage › Notifications | 14 / 15 | 24 / 26 | - | - | 14 | 24 |  |  |
| Workers (open beside the focused tile) › Archived | 14 / 81 | 24 / 25 | - | - | 14 | 32 | 2 | /api/layout |
| Edit › Copy Code › Copy | 14 / 163 | 16 / 158 | - | - | 30 | 24 | 2 | /api/work |
| Side panel › Hosts 1 › Pair a device Full | - | - | 16 / 16 (no change) | 13 / 14 (no change) | 0 | 32 | 16 | /api/pair/new |
| Add to Main › Temp chat › Temp | - | - | 15 / 15 (no change) | 14 / 14 (no change) | 0 | 32 |  |  |
| Chat | - | - | 15 / 15 (no change) | 11 / 12 (no change) | 0 | 48 |  |  |
| Chat (open: go to it) | 15 / 15 (no change) | 11 / 11 (no change) | - | - | 0 | 32 |  |  |
| Controls for Chat › Close Chat › More for Chat | 15 / 16 | 14 / 14 | - | - | 15 | 24 |  |  |
| Copy Code › Copy Code | 14 / 64 | 9 / 31 | 15 / 199 | 13 / 14 (no change) | 199 | 32 |  |  |
| Edit › Copy Code › Copy Code | 15 / 64 | 12 / 56 | - | - | 30 | 32 |  |  |
| Library (open beside the focused tile) › Find in the Library | 15 / 98 | 11 / 27 | - | - | 15 | 32 | 4 | /api/work |
| Library (open beside the focused tile) › Library (open: go to it) | 15 / 99 | 12 / 13 (no change) | - | - | 99 | 32 |  |  |
| Model, run in Claude Code › Copy Code | 14 / 113 | 9 / 40 | 15 / 15 (no change) | 13 / 13 (no change) | 14 | 32 |  |  |
| New workspace (Alt Shift N) › Chat and Temp The chat, a temp chat besi › Access | 15 / 31 | 12 / 111 | - | - | 15 | 48 | 3 | /api/layout /api/work |
| New workspace (Alt Shift N) › Close Chat › More for Workspace 2 | 15 / 15 | 14 / 14 | - | - | 15 | 24 | 2 | /api/work |
| New workspace (Alt Shift N) › Close Chat › Workspace 2 (Alt Shift 1). Double-click to rename; right-click for more. | 15 / 16 (no change) | 14 / 14 (no change) | - | - | 0 | 16 |  |  |
| Pin › Copy | 15 / 63 | 8 / 41 | - | - | 30 | 16 |  |  |
| Search › Best | 14 / 14 (no change) | 10 / 10 (no change) | 15 / 15 (no change) | 12 / 13 (no change) | 0 | 32 |  |  |
| Side panel › Hosts 1 › Pair a device to read Read only | - | - | 15 / 16 (no change) | 14 / 15 (no change) | 0 | 32 | 17 | /api/pair/new |
| Temp chat (open beside the focused tile) › Temp chat (open: go to it) | 15 / 115 | 11 / 12 (no change) | - | - | 115 | 32 |  |  |
| Thinking › Copy Code | 14 / 113 | 9 / 57 | 15 / 15 (no change) | 11 / 12 (no change) | 14 | 40 |  |  |
| Workers › Workers28 | - | - | 15 / 15 (no change) | 13 / 14 (no change) | 0 | 32 |  |  |
| Controls for Chat › Copy Code | 14 / 114 | 7 / 52 | - | - | 31 | 24 |  |  |
| Copy | 14 / 64 | 9 / 37 | - | - | 30 | 24 |  |  |
| Copy Code | 13 / 113 | 7 / 38 | 14 / 15 (no change) | 10 / 11 (no change) | 29 | 32 |  |  |
| Create task › Copy | 14 / 80 | 10 / 59 | - | - | 30 | 24 |  |  |
| Create task › Copy Code | 14 / 113 | 10 / 54 | - | - | 29 | 16 |  |  |
| Create task › Copy Code › Copy Code | 14 / 81 | 11 / 76 | - | - | 31 | 16 |  |  |
| Create task › Edit › Copy | 14 / 64 | 10 / 59 | - | - | 30 | 24 | 4 | /api/work |
| Create task › Edit › Copy Code | 14 / 113 | 9 / 59 | - | - | 30 | 24 |  |  |
| Create task › Edit › Edit | 14 / 81 | 9 / 59 | - | - | 31 | 24 | 4 | /api/work |
| Edit › Copy | 14 / 64 | 11 / 76 | - | - | 30 | 24 |  |  |
| Edit › Copy Code | 14 / 114 | 9 / 59 | - | - | 30 | 24 |  |  |
| Edit › Edit | 14 / 80 | 9 / 76 | - | - | 30 | 24 |  |  |
| Library (open beside the focused tile) › Copy | 14 / 97 | 10 / 108 | - | - | 30 | 64 | 4 | /api/layout /api/work |
| Library (open beside the focused tile) › Copy Code › Copy Code | 14 / 80 | 10 / 37 | - | - | 30 | 16 |  |  |
| Library (open beside the focused tile) › Edit › Copy | 14 / 63 | 8 / 36 | - | - | 29 | 24 |  |  |
| Library (open beside the focused tile) › Edit › Copy Code | 14 / 114 | 7 / 56 | - | - | 30 | 24 |  |  |
| Library (open beside the focused tile) › Edit › Edit | 14 / 80 | 8 / 39 | - | - | 30 | 24 | 4 | /api/work |
| Library (open beside the focused tile) › Make something | 14 / 98 | 10 / 10 | - | - | 14 | 24 | 4 | /api/work |
| Library (open beside the focused tile) › Plan 1 › Copy | 14 / 64 | 9 / 124 | - | - | 31 | 40 | 2 | /api/layout |
| Library (open beside the focused tile) › plan-0 › Copy | 14 / 64 | 10 / 109 | - | - | 31 | 40 | 2 | /api/layout |
| More for Chat › Save as template › Copy Code | 14 / 113 | 8 / 42 | - | - | 30 | 16 |  |  |
| New workspace (Alt Shift N) › Chat (Alt Shift 1). Double-click to rename; right-click for more. › Copy | 14 / 96 | 9 / 58 | - | - | 29 | 16 |  |  |
| New workspace (Alt Shift N) › Chat (Alt Shift 1). Double-click to rename; right-click for more. › Copy Code | 14 / 114 | 10 / 60 | - | - | 30 | 16 |  |  |
| New workspace (Alt Shift N) › Chat (open beside the focused tile) › Copy | 14 / 96 | 10 / 55 | - | - | 30 | 16 |  |  |
| New workspace (Alt Shift N) › Chat (open beside the focused tile) › Copy Code | 14 / 114 | 8 / 41 | - | - | 30 | 16 |  |  |
| New workspace (Alt Shift N) › Chat and Temp The chat, a temp chat besi › Copy | 14 / 97 | 10 / 109 | - | - | 30 | 40 | 2 | /api/layout |
| Pin › Copy Code | 14 / 114 | 10 / 42 | - | - | 31 | 16 |  |  |
| Pin › Copy Code › Copy Code | 14 / 64 | 8 / 57 | - | - | 30 | 16 |  |  |
| Pin › Edit › Copy | 14 / 80 | 10 / 42 | - | - | 30 | 24 |  |  |
| Pin › Edit › Copy Code | 14 / 113 | 8 / 59 | - | - | 30 | 24 | 4 | /api/work |
| Pin › Edit › Edit | 14 / 64 | 9 / 58 | - | - | 30 | 24 | 4 | /api/work |
| Reply › Copy | 14 / 98 | 9 / 75 | - | - | 31 | 24 | 3 | /api/work |
| Reply › Copy Code | 14 / 113 | 9 / 56 | - | - | 30 | 24 |  |  |
| Reply › Copy Code › Copy Code | 14 / 80 | 9 / 58 | - | - | 30 | 16 |  |  |
| Settings and usage › Appearance | 14 / 14 (no change) | 9 / 9 (no change) | - | - | 0 | 16 |  |  |
| Settings and usage › Hosts › Pair a device Full | 14 / 14 (no change) | 8 / 8 (no change) | - | - | 0 | 16 | 14 | /api/pair/new |
| Settings and usage › Hosts › Pair a device to read Read only | 14 / 14 (no change) | 9 / 9 (no change) | - | - | 0 | 16 | 14 | /api/pair/new |
| Settings and usage › Models › A classifier decides each action; it rarely asks | 14 / 14 (no change) | 9 / 9 (no change) | - | - | 0 | 24 |  |  |
| Settings and usage › Models › Asks to go past the sandbox | 14 / 14 (no change) | 9 / 9 (no change) | - | - | 0 | 24 |  |  |
| Settings and usage › Models › ChatGPT / Codex Kimi Code Kilo | 14 / 14 (no change) | 7 / 9 (no change) | - | - | 0 | 24 | 5 | /api/work |
| Settings and usage › Models › Commands run on the machine | 14 / 14 (no change) | 9 / 10 (no change) | - | - | 0 | 24 |  |  |
| Settings and usage › Models › Default (haiku) Opus 5.5 Sonnet 5.5 Haik | 14 / 14 (no change) | 9 / 10 (no change) | - | - | 0 | 24 | 4 | /api/work |
| Settings and usage › Models › Its own GPT-6.1-Sol GPT-6-Luna | 14 / 114 | 9 / 10 (no change) | - | - | 114 | 16 |  |  |
| Settings and usage › Models › Its own Opus 5.5 Sonnet 5.5 Haiku 5.5 | 14 / 114 | 9 / 10 (no change) | - | - | 114 | 24 |  |  |
| Settings and usage › Models › Opus 5.5 Sonnet 5.5 Haiku 5.5 GPT-6.1-So | 14 / 14 (no change) | 9 / 9 (no change) | - | - | 0 | 24 | 4 | /api/work |
| Settings and usage › Models › Same as Chat Opus 5.5 Sonnet 5.5 Haiku 5 | 14 / 15 (no change) | 9 / 9 (no change) | - | - | 0 | 24 | 3 | /api/work |
| Temp chat (open beside the focused tile) › Copy | 14 / 97 | 9 / 92 | - | - | 30 | 40 | 3 | /api/layout /api/work |
| Temp chat (open beside the focused tile) › Copy Code › Copy Code | 14 / 63 | 9 / 42 | - | - | 30 | 16 |  |  |
| Temp chat (open beside the focused tile) › Edit › Copy | 14 / 63 | 8 / 39 | - | - | 29 | 24 |  |  |
| Temp chat (open beside the focused tile) › Edit › Copy Code | 14 / 113 | 8 / 58 | - | - | 29 | 24 |  |  |
| Temp chat (open beside the focused tile) › Edit › Edit | 14 / 64 | 10 / 75 | - | - | 30 | 24 | 4 | /api/work |
| Temp chat (open beside the focused tile) › Memory on: the next turn reads the memory and has your tools › Copy | 14 / 64 | 11 / 93 | - | - | 31 | 40 | 2 | /api/layout |
| Terminal (open beside the focused tile) › Copy | 14 / 97 | 7 / 104 | - | - | 30 | 40 | 4 | /api/layout /api/work |
| Terminal (open beside the focused tile) › Copy Code › Copy Code | 14 / 96 | 8 / 75 | - | - | 29 | 48 |  |  |
| Terminal (open beside the focused tile) › Edit › Copy | 14 / 79 | 8 / 91 | - | - | 29 | 40 |  |  |
| Terminal (open beside the focused tile) › Edit › Edit | 14 / 97 | 6 / 90 | - | - | 30 | 40 | 4 | /api/work |
| Terminal (open beside the focused tile) › Terminal (open: go to it) | 14 / 81 | 8 / 10 (no change) | - | - | 81 | 32 | 5 | /api/work |
| Workers (open beside the focused tile) › Archived › Copy | 14 / 63 | 8 / 76 | - | - | 30 | 40 | 2 | /api/layout |
| Workers (open beside the focused tile) › Copy | 14 / 79 | 6 / 92 | - | - | 29 | 40 | 2 | /api/layout |
| Workers (open beside the focused tile) › Copy Code › Copy Code | 14 / 79 | 9 / 58 | - | - | 29 | 16 |  |  |
| Workers (open beside the focused tile) › Edit › Copy | 14 / 80 | 8 / 39 | - | - | 30 | 32 |  |  |
| Workers (open beside the focused tile) › Edit › Copy Code | 14 / 96 | 9 / 41 | - | - | 30 | 32 | 4 | /api/work |
| Workers (open beside the focused tile) › Edit › Edit | 14 / 63 | 11 / 77 | - | - | 30 | 32 | 2 | /api/work |
| Workers (open beside the focused tile) › What the colours mean | 14 / 97 | 9 / 10 | - | - | 14 | 24 | 4 | /api/work |
| Workers (open beside the focused tile) › Workers (open: go to it) | 14 / 81 | 10 / 11 (no change) | - | - | 81 | 24 | 3 | /api/work |
| Workers (open beside the focused tile) › Workers28 | 14 / 81 | 9 / 10 (no change) | - | - | 81 | 16 | 4 | /api/work |
| New workspace (Alt Shift N) › Chat, terminal and workers The chat, the › Copy | 13 / 112 | 7 / 137 | - | - | 45 | 72 | 2 | /api/layout |
| Terminal (open beside the focused tile) › Edit › Copy Code | 13 / 96 | 8 / 54 | - | - | 29 | 48 |  |  |
| Workers (open beside the focused tile) › Reed · Thinking or writing Reed batch check Working · Comparing three quotes against the last run › Copy | 13 / 96 | 5 / 90 | - | - | 29 | 64 | 2 | /api/layout |
| Escape with nothing open | 12 / 16 (no change) | 12 / 13 (no change) | 5 / 9 (no change) | 10 / 16 (no change) | 0 | 16 |  |  |
| Terminal (open beside the focused tile) › Controls for Terminal | 6 / 14 (no change) | 11 / 11 (no change) | - | - | 0 | 0 |  |  |
| Temp chat (open beside the focused tile) › Controls for Temp chat | 10 / 14 (no change) | 9 / 16 (no change) | - | - | 0 | 0 |  |  |
| Workers (open beside the focused tile) › Controls for Workers | 9 / 12 (no change) | 4 / 16 (no change) | - | - | 0 | 0 |  |  |
| Controls for Chat | 3 / 8 (no change) | 8 / 11 (no change) | - | - | 0 | 0 |  |  |
| New workspace (Alt Shift N) › Controls for the tile | 7 / 10 (no change) | 6 / 8 (no change) | - | - | 0 | 0 | 2 | /api/layout |
| Library (open beside the focused tile) › Controls for Library | 6 / 9 (no change) | 6 / 8 (no change) | - | - | 0 | 0 |  |  |

## Not driven

- desk: Attach: opens the system's file picker
- desk: Settings and usage › Check for updates: reloads the page to the newest build
- desk: link: a link to another site
- phone: Attach: opens the system's file picker
- phone: Side panel › Settings › Check for updates: reloads the page to the newest build
- phone: link: a link to another site

## Source handlers whose written label no measured interaction carries (196 of 321)

Matched by the aria-label, title or button text written within six lines of the handler. Many carry a label
made at run time, or answer a state the stand-in host never shows, so are reached under another name or not
at all; read this list as where the crawl may have gaps, not as a list of gaps.

- App.tsx:362 onClick 
- App.tsx:412 onClick 
- App.tsx:476 onClick Picture
- App.tsx:477 onTouchStart Picture
- App.tsx:511 onClick 
- App.tsx:636 onClick 
- App.tsx:689 onClick Earlier
- App.tsx:776 onClick 
- App.tsx:789 onClick 
- App.tsx:938 onClick 
- App.tsx:974 onClick No connection yet
- App.tsx:1146 onClick This device
- App.tsx:1167 onChange Voice
- App.tsx:1182 onClick 
- App.tsx:1198 onChange Voice speed             
- App.tsx:1301 onClick Settings categories
- App.tsx:1370 onClick 
- App.tsx:1403 onClick 
- App.tsx:1667 onKeyDown 
- App.tsx:1675 onClick 
- App.tsx:1697 onClick 
- App.tsx:1838 onClick New task
- App.tsx:1839 onClick New task
- App.tsx:2036 onClick 
- App.tsx:2054 onChange Search models
- App.tsx:2077 onClick 
- App.tsx:2165 onClick 
- App.tsx:2210 onChange Your list of names
- App.tsx:2211 onClick Your list of names
- App.tsx:2231 onSubmit Find or add a name
- App.tsx:2248 onClick 
- App.tsx:2279 onChange Worker titles
- App.tsx:2421 onChange Image generation
- App.tsx:2441 onChange Permissions
- App.tsx:2452 onChange 
- App.tsx:2522 onClick 
- App.tsx:2557 onClick 
- App.tsx:2615 onClick 
- App.tsx:2704 onClick Attach
- App.tsx:2712 onClick 
- App.tsx:2745 onClick 
- App.tsx:2771 onClick Open the picture
- App.tsx:2926 onClick 
- App.tsx:2933 onClick 
- App.tsx:3176 onPointerDown 
- App.tsx:3177 onClick 
- App.tsx:3196 onKeyDown 
- App.tsx:3662 onClick Select text
- App.tsx:3665 onClick Select text
- App.tsx:3668 onClick Select text
- App.tsx:3923 onPointerDown 
- App.tsx:3925 onClick 
- App.tsx:4054 onClick 
- App.tsx:4127 onClick 
- App.tsx:4248 onClick Stop reading     
- App.tsx:4261 onClick 
- App.tsx:4316 onClick 
- App.tsx:4669 onClick 
- App.tsx:4712 onClick 
- App.tsx:5079 onClick 
- App.tsx:5095 onClick 
- App.tsx:5099 onClick 
- App.tsx:5333 onClick Clear the temp chat
- App.tsx:5360 onClick Clear the temp chat
- App.tsx:5455 onClick Back to Main                   
- App.tsx:5513 onSubmit New memory's name
- App.tsx:5546 onChange 
- App.tsx:5547 onChange 
- App.tsx:5580 onClick 
- App.tsx:5581 onClick 
- App.tsx:5667 onClick 
- App.tsx:5676 onClick 
- App.tsx:5680 onClick 
- App.tsx:5795 onSubmit 
- App.tsx:5813 onChange 
- App.tsx:5814 onKeyDown 
- App.tsx:5869 onClick Remove
- App.tsx:5907 onClick 
- App.tsx:5946 onClick 
- App.tsx:5957 onChange 
- App.tsx:5985 onClick Always allow               
- App.tsx:5989 onClick Always allow               
- App.tsx:6044 onClick Subject
- call.tsx:295 onClick 
- call.tsx:308 onClick Hang up
- card.tsx:299 onClick 
- connections.tsx:74 onClick Add another MCP server         
- connections.tsx:144 onClick 
- connections.tsx:148 onSubmit 
- connections.tsx:154 onChange Address of the page the server sent you to
- connections.tsx:206 onClick 
- connections.tsx:242 onClick 
- connections.tsx:261 onClick 
- connections.tsx:314 onClick 
- connections.tsx:367 onClick 
- connections.tsx:421 onSubmit 
- connections.tsx:439 onChange 
- connections.tsx:520 onClick Disconnect             
- connections.tsx:525 onClick Disconnect             
- connections.tsx:546 onSubmit 
- connections.tsx:553 onChange xAI API key
- connections.tsx:565 onClick Use this key               
- connections.tsx:638 onClick 
- connections.tsx:645 onClick 
- connections.tsx:698 onClick Let other apps in
- connections.tsx:709 onClick Web, X, pictures and pages
- dictate.tsx:98 onPointerDown 
- dictate.tsx:99 onClick 
- hold.tsx:43 onPointerDown 
- hold.tsx:60 onContextMenu 
- hold.tsx:147 onClick 
- hold.tsx:148 onContextMenu 
- hold.tsx:165 onClick 
- hold.tsx:170 onClick 
- home.tsx:42 onClick Needs you
- home.tsx:57 onClick Reading the calendar
- home.tsx:89 onClick Done today
- home.tsx:91 onClick Done today
- hosts.tsx:207 onClick 
- hosts.tsx:268 onClick Forget this host
- hosts.tsx:328 onSubmit Address or pairing link
- hosts.tsx:401 onClick Revoke             
- library.tsx:133 onSubmit 
- library.tsx:145 onClick 
- library.tsx:155 onChange 
- library.tsx:156 onKeyDown 
- library.tsx:204 onClick 
- library.tsx:218 onClick 
- library.tsx:333 onClick 
- library.tsx:351 onClick 
- memory.tsx:105 onClick 
- phone.tsx:482 onClick 
- phone.tsx:542 onClick 
- phone.tsx:552 onClick 
- phone.tsx:554 onClick 
- phone.tsx:576 onKeyDown Workspace name
- phone.tsx:585 onClick 
- phone.tsx:600 onClick 
- phone.tsx:630 onClick 
- pins.tsx:100 onClick Resume in chat               
- search.tsx:242 onClick 
- search.tsx:269 onClick 
- search.tsx:283 onChange 
- search.tsx:287 onKeyDown Search everything said
- search.tsx:385 onClick Earlier           
- search.tsx:391 onClick Earlier           
- search.tsx:424 onClick 
- skills.tsx:88 onSubmit Skill source repository
- skills.tsx:103 onClick Pull
- skills.tsx:135 onClick 
- skills.tsx:151 onClick 
- skills.tsx:176 onSubmit 
- skills.tsx:198 onClick Make a skill with the chat     
- skills.tsx:274 onPointerDown 
- skills.tsx:275 onClick 
- skills.tsx:317 onSubmit 
- split.tsx:42 onPointerDown Resize the panes
- split.tsx:51 onDoubleClick 
- split.tsx:52 onKeyDown 
- terminal.tsx:325 onClick 
- terminal.tsx:376 onClick 
- terminal.tsx:381 onClick 
- terminal.tsx:388 onClick Paste               
- terminal.tsx:394 onClick Paste               
- terminal.tsx:418 onPointerDown 
- terminal.tsx:419 onClick 
- tiles.tsx:318 onClick 
- tiles.tsx:654 onKeyDown Workspace name
- tiles.tsx:667 onClick 
- tiles.tsx:668 onDoubleClick 
- tiles.tsx:669 onContextMenu 
- tiles.tsx:686 onClick Rename, save as a template, close
- tiles.tsx:700 onClick Close the workspace: its tiles only, every chat goes on (Alt Shift W)
- tiles.tsx:709 onClick 
- tiles.tsx:718 onClick Dock
- tiles.tsx:726 onClick 
- tiles.tsx:732 onClick 
- tiles.tsx:860 onPointerDown Resize the tiles
- tiles.tsx:866 onDoubleClick 
- tiles.tsx:867 onKeyDown 
- tiles.tsx:945 onClick Move, split, full screen or close the tile
- tiles.tsx:1002 onClick Open here
- tiles.tsx:1008 onClick 
- tiles.tsx:1033 onKeyDown Template name
- tiles.tsx:1040 onClick 
- tiles.tsx:1114 onKeyDown 
- tiles.tsx:1151 onClick 
- tiles.tsx:1177 onClick 
- work.tsx:330 onClick 
- work.tsx:457 onSubmit 
- work.tsx:467 onChange Wake time
- work.tsx:544 onSubmit 
- work.tsx:558 onKeyDown 
- workTree.tsx:226 onClick 
- workTree.tsx:454 onClick 
- workTree.tsx:455 onDoubleClick 
