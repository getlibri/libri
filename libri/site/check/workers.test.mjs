// A worker card's Stop: "Stopping" lasts until the worker is done, and a
// worker back at work after can be stopped again.
import assert from "node:assert/strict";
import { test } from "node:test";

globalThis.localStorage = { getItem: () => null };
globalThis.location = new URL("http://libri.test/");
const { atWork, taskEvent } = await import("../src/chat.ts");

const stopping = { name: "Kerry", text: "work", tools: [], words: "", stopping: true, done: false };
const agent = { name: "Kerry", task: "work" };

test("a stopped worker that finishes is no longer stopping", () => {
  const [k] = taskEvent([stopping], { type: "SUBAGENT_FINISHED", subagentRunId: "Kerry" });
  assert.equal(k.done, true);
  assert.equal(k.stopping, false);
});

test("a worker that finished and starts again can be stopped again", () => {
  const done = taskEvent([stopping], { type: "SUBAGENT_FINISHED", subagentRunId: "Kerry" });
  const [k] = taskEvent(done, { type: "SUBAGENT_STARTED", subagentRunId: "Kerry", description: "more" });
  assert.equal(k.done, false);
  assert.equal(k.stopping, false);
});

test("the work list ends a Stop once the worker leaves it, and keeps one under way", () => {
  assert.equal(atWork([stopping], [agent])[0].stopping, true);
  const gone = atWork([stopping], []);
  assert.equal(gone[0].done, true);
  assert.equal(gone[0].stopping, false);
  const back = atWork([{ ...stopping, done: true }], [agent]);
  assert.equal(back[0].stopping, false);
});
