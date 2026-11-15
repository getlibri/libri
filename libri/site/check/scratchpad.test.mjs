// The scratchpad's thread off its log: what a turn did stands in order, and
// promoting carries the words alone.
import assert from "node:assert/strict";
import { test } from "node:test";

globalThis.localStorage = { getItem: () => null, setItem: () => {} };
globalThis.location = new URL("http://libri.test/");
const { fromAgentChat } = await import("../src/chat.ts");
const { wordsOf } = await import("../src/scratchpad.ts");

const log = [
  "1|user: the first words",
  "2|talk: Looking.",
  "3|tool: web_search {\"query\":\"x\"}",
  "4|echo: secret result",
  "5|talk: Found it.",
  "6|file: chart.png",
  "7|user: and more",
  "8|talk: Done.",
].join("\n");

test("the log reads as the thread, the first words among the rest", () => {
  const { tools, words } = fromAgentChat(log, true);
  assert.deepEqual(
    tools.map((t) => (t.told !== undefined ? `me ${t.told}` : t.file ? `file ${t.file}` : t.name)),
    ["me the first words", "web_search", "file chart.png", "me and more"],
  );
  assert.equal(tools[1].before, "Looking.");
  assert.equal(tools[1].result, "secret result");
  assert.equal(words, "Done.");
});

test("promoting carries the words, not a call or its result", () => {
  const talk = wordsOf(fromAgentChat(log, true), "Libri");
  assert.equal(talk, "Me: the first words\n\nLibri: Looking.\n\nLibri: Found it.\n\nMe: and more\n\nLibri: Done.");
  assert.doesNotMatch(talk, /secret|web_search|chart/);
});
