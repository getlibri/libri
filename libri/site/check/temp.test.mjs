// Temp's own state against the host: its asks answer through their own
// cards, and a read under way when the person acted does not undo it.
import assert from "node:assert/strict";
import { test } from "node:test";

globalThis.localStorage = { getItem: () => null, setItem: () => {} };
globalThis.location = new URL("http://libri.test/");

// The host, as each test sets it: an answer per request, by method and path.
let host;
globalThis.fetch = async (url, init = {}) => host(`${init.method ?? "GET"} ${new URL(url, location).pathname.slice(1)}`, init);
const json = (v, status = 200) => new Response(JSON.stringify(v), { status, headers: { "content-type": "application/json" } });

const { closeScratchpad, readScratchpad, replyScratchpad, scratchpadNow, setScratchpadMemory, stopScratchpad } = await import("../src/scratchpad.ts");

const ASK = { id: "scratchpad-ask-1", reason: "ask", metadata: { tool: "AskUserQuestion", input: {} } };
const pad = (over = {}) => ({ id: "scratchpad-1", text: "", working: true, words: "", thinking: "", stopping: false, queued: [], memory: true, asks: [ASK], ...over });

/** The store with one open ask, as the host last said. */
async function asking() {
  host = (req) => (req === "GET api/scratchpad" ? json(pad()) : json({}));
  await readScratchpad();
  assert.equal(scratchpadNow().asks.length, 1);
}

test("an ask the turn took back goes from Temp's own cards", async () => {
  await asking();
  host = (req) => (req === "POST api/run" ? new Response("", { status: 409 }) : json(pad()));
  await replyScratchpad(ASK.id, true);
  assert.equal(scratchpadNow().asks.length, 0);
});

test("an answer that fails leaves the card to try again", async () => {
  await asking();
  let seen;
  host = (req) => {
    if (req !== "POST api/run") return json(pad());
    seen = scratchpadNow().asks[0].busy;
    return new Response("", { status: 500 });
  };
  await replyScratchpad(ASK.id, true);
  assert.equal(seen, true, "busy while the host has it");
  assert.equal(scratchpadNow().asks[0].busy, false);
});

test("an answer the host takes is sent with the interrupt's id", async () => {
  await asking();
  let body;
  host = (req, init) => {
    if (req === "POST api/run") body = JSON.parse(init.body);
    return req === "POST api/run" ? new Response("", { status: 202 }) : json(pad({ asks: [] }));
  };
  await replyScratchpad(ASK.id, false);
  assert.deepEqual(body.resume, [{ interruptId: ASK.id, status: "cancelled" }]);
  assert.deepEqual(body.messages, []);
});

/** A read the host answers only once let go, with the temp chat as it was. */
function slowRead() {
  let go;
  const held = new Promise((r) => (go = r));
  host = async (req) => {
    if (req !== "GET api/scratchpad") return json({});
    await held;
    return json(pad({ asks: [], memory: true }));
  };
  const read = readScratchpad();
  return async () => (go(), await read);
}

test("a read under way does not bring back a temp chat closed meanwhile", async () => {
  const finish = slowRead();
  await closeScratchpad();
  await finish();
  assert.equal(scratchpadNow().id, null);
  assert.equal(scratchpadNow().working, false);
});

test("a read under way does not flip Memory back", async () => {
  const finish = slowRead();
  await setScratchpadMemory(false);
  await finish();
  assert.equal(scratchpadNow().memory, false);
});

test("a stop refused because the turn ended reads the host and clears the dots", async () => {
  await asking();
  host = (req) =>
    req === "POST api/scratchpad/stop" ? new Response("", { status: 409 }) : json(pad({ working: false, asks: [] }));
  await stopScratchpad();
  await new Promise((r) => setTimeout(r, 0)); // let the read it starts land
  assert.equal(scratchpadNow().working, false);
  assert.equal(scratchpadNow().stopping, false);
});
