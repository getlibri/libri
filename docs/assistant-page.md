# The assistant page: Chat · Tasks · Library

This is a starting point, approved on 2026-11-04, to be modified with use.
The model is the Claude mobile app's Chat / Tasks /
Make: Libri takes the structure, not the names. Rewrite this file as the
design changes; it describes what stands, not its history.

## Phone (web and Android the same)

Three places on the bottom bar, plus Pinned while something is pinned:

- **Chat**: the main chat. Temp is a button in the chat's top bar, beside
  search and the terminal, and opens over the chat as before.
- **Tasks** (a working name, "Tasks for now"): the workers, then the watches
  and reminders.
  - **+** starts a worker straight from the person's words, not through main
    (`POST /api/work`). Like every worker it forks main's memory. Its report
    waits on its card; main gets a one-line note that it exists and is not set
    going by the report. A failed run still reaches main. Deploys still need
    the person's word in main.
  - A watch's hold menu has Snooze and Delete, which edit the book with no
    model call (`/api/watches/{id}/snooze`, `/delete`).
- **Library**: the pages `show_page` published (newest first) and the book's
  lists. A page opens in place, sandboxed as the host serves it. + sends the
  chat a request to make one.

## Wide screen

- The Library is a section of the left column: the five latest, and All for
  the gallery.
- An item opens in the pane beside the chat, Temp's slot. Opening Temp there
  replaces it, and the other way round.
- Tasks is the existing workers column, with the watches under the workers.

## Rules

- No hardwired use cases. Starters are requests worded as the person would
  word them; rich cards come from MCP Apps.
- Don't copy Claude's labels (Make, Artifacts, Get started, Up next, Ready for
  review).
- Stock harnesses: nothing here is specific to one harness.

## Not built yet

- Groups as planned: Waiting on you (For review / approval / an answer),
  Working, Queued (added, not started), Done (collapsed). Tasks still shows
  Waiting, Working, Napping, Sleeping, Archived.
- Quick capture: a box whose words the agent files as a list item, reminder or
  note, confirmed in one line with undo. Open questions: where a note lives,
  which model files it, and what undo reverses.
