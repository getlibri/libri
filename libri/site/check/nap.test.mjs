// A nap's presets, as T3 Code's snooze has them: what is past is left out,
// two in the same minute are shown once, and the soonest comes first.
import assert from "node:assert/strict";
import { test } from "node:test";

const { napPresets, wakeWords, localInput } = await import("../src/nap.ts");

const words = (now) => napPresets(now).map((p) => p.label.split(",")[0]);

test("a weekday morning offers all five, soonest first", () => {
  const now = new Date(2026, 10, 3, 10, 30); // Tuesday
  const naps = napPresets(now);
  assert.deepEqual(words(now), ["1 hour", "3 hours", "This evening", "Tomorrow", "Monday"]);
  assert.equal(naps[0].until, Math.floor(now.getTime() / 1000) + 3600);
  assert.equal(new Date(naps[4].until * 1000).getDay(), 1);
  assert.ok(naps.every((p, i) => i === 0 || p.until > naps[i - 1].until));
});

test("the evening is left out once it has passed", () => {
  assert.deepEqual(words(new Date(2026, 10, 3, 19, 0)), ["1 hour", "3 hours", "Tomorrow", "Monday"]);
});

test("on a Sunday tomorrow is Monday, shown once", () => {
  assert.deepEqual(words(new Date(2026, 10, 8, 10, 0)), ["1 hour", "3 hours", "This evening", "Tomorrow"]);
});

test("on a Monday it is the next one", () => {
  const monday = napPresets(new Date(2026, 10, 2, 10, 0)).at(-1);
  assert.equal(new Date(monday.until * 1000).getDate(), 9);
});

test("a wake time reads as the hour today, else the day", () => {
  const now = new Date(2026, 10, 3, 10, 0);
  const s = (d) => Math.floor(d.getTime() / 1000);
  assert.doesNotMatch(wakeWords(s(new Date(2026, 10, 3, 18, 0)), now), /tomorrow/);
  assert.match(wakeWords(s(new Date(2026, 10, 4, 9, 0)), now), /^tomorrow /);
  assert.match(wakeWords(s(new Date(2026, 10, 20, 9, 0)), now), /9:00/);
  assert.equal(localInput(new Date(2026, 0, 5, 7, 4)), "2026-01-05T07:04");
});
