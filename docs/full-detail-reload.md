# Full detail after reload

Hippo messages may carry `display: {version: 1, result?, metrics?}` beside
`text`. Tool results retain the final structured result; answer rows retain
reported context, speed and usage. The spool includes the same field and the
`keep_display` command writes it with the text under the existing row key.
Old rows and unknown display versions render as plain text.

The model reads text, as before. Display fields do not change message size,
summary input, zoom text, search text or the bounded view. They are not raw
omp events and do not create an omp session store. Image blocks refer to
Libri's existing file library instead of embedding image bytes in memory.

The browser's history reducer and Android's shared Rust client restore the
same structured-result field that streaming fills. Existing tool and metric
renderers draw both. Android resolves image references through its host's
file URL, with the same credentials as other kept pictures.

Validation:

- Hippo disk-reload test verifies presentation survives while rendered text
  and summary byte count stay identical.
- Keeper spool test verifies the text and diff share one echo row and that
  the view contains no diff.
- Browser and shared-client reducer tests rebuild rich results from history
  without live events; client results pair by row key even out of order.
- `nix/check_omp_rendering.py` uses a throwaway Libri and an omp RPC stand-in
  which edits a scratch file. At desktop and phone widths it restarts both
  daemons, opens a fresh page, and verifies diff, inline image and measured
  context/speed. Saved image records contain file references without base64;
  a marker present only in display metadata never enters model requests.

This proof uses no real provider account or model call. Existing chats cannot
recover display data that was discarded before this change. No phone runtime
or custom omp extension-widget renderer is covered by the browser proof.

Scope: this change restores the hippo-owned main conversation served by
`/api/history`. Temp and independent worker threads still read plain task
logs through `chat_keyed`; their separate history routes do not yet retain
structured display records. The main conversation's completed subagent
cards and reports are covered by the scratch proof.
