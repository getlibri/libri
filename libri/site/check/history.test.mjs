// What a sync of the chat leaves, driven through the page's own code with
// messages as hippo keeps them.
import assert from "node:assert/strict";
import { test } from "node:test";

globalThis.localStorage = { getItem: () => null };
globalThis.location = new URL("http://libri.test/");
const { taskEvent, apply, askOffer, firstLine, fromAgentChat, fromHistory, offered } = await import("../src/chat.ts");

const LOOKUP = 'ToolSearch {"max_results":1,"query":"select:mcp__libri__tell_agent"}';
const REPLY = "**Libri hung, and your restart fixed it.**";
const at = (time) => `2026-10-31T${time}-04:00`;
const said = (i, kind, text, time) => ({ i, kind, text, date: at(time), reaction: null, src: { e: `k${i}` } });

// The morning of 10-31, shortened from what hippo holds: a reply, a worker's report, then the
// same tool lookup again and again, only the first with words before it.
const MORNING = [
  said(75456, "talk", REPLY, "10:59:50"),
  said(75457, "work", "[Dara2] reply\nThe folds are committed.", "11:01:04"),
  said(75458, "tool", LOOKUP, "11:01:11"),
  said(75459, "echo", "Loaded", "11:01:11"),
  said(75507, "talk", "Workers go on the left.", "11:16:56"),
  said(75508, "user", "sounds good", "11:17:13"),
  said(75509, "tool", LOOKUP, "11:17:19"),
  said(75510, "echo", "Loaded", "11:17:26"),
  said(75514, "talk", "Nell has the full web layout.", "11:17:30"),
  said(75515, "user", "lets build the qt desktop app", "11:18:22"),
  said(75516, "tool", LOOKUP, "11:18:28"),
  said(75517, "echo", "Loaded", "11:18:28"),
  said(75523, "talk", "One build for Linux.", "11:18:43"),
  said(75547, "user", "laptop is den", "11:34:04"),
  said(75548, "tool", LOOKUP, "11:34:08"),
  said(75549, "echo", "Loaded", "11:34:09"),
  said(75555, "talk", "My mistake.", "11:34:19"),
];
const upto = (i, all = MORNING) => all.filter((m) => m.i <= i);
const calls = (ms) => ms.flatMap((m) => m.tools ?? []);
// Each call, with the model's words drawn just above it: the message before
// the one the call opens (F13).
const wordsOf = (ms) =>
  Object.fromEntries(
    ms.flatMap((m, i) => (m.tools ?? []).map((t, k) => [t.id, k === 0 && ms[i - 1]?.role === "assistant" ? ms[i - 1].text || undefined : undefined])),
  );
// The chat as drawn, top to bottom: each message's calls as [name], then its words.
const drawn = (ms) => ms.flatMap((m) => [...(m.tools ?? []).map((t) => `[${t.name}]`), ...(m.text ? [m.text] : [])]);
const heard = (events, ms = []) => events.reduce(apply, ms);

// A turn's events as the record holds them, after the row `after`.
const words = (key, after, text) => [
  { type: "TEXT_MESSAGE_START", messageId: `m-${key}`, key, after },
  { type: "TEXT_MESSAGE_CONTENT", messageId: `m-${key}`, delta: text, key, after },
];
const call = (key, after, id, name, args) => [
  { type: "TOOL_CALL_START", toolCallId: id, toolCallName: name, key, after },
  { type: "TOOL_CALL_ARGS", toolCallId: id, delta: args, key, after },
  { type: "TOOL_CALL_END", toolCallId: id, key, after },
];
const result = (key, after, id, content) => ({ type: "TOOL_CALL_RESULT", toolCallId: id, content, key, after });
const thinking = (after, delta) => ({ type: "REASONING_MESSAGE_CONTENT", messageId: "r", delta, after });
const record = (...events) => ({ start: 0, seq: 99, shown: events.flat() });
const texts = (ms) => ms.map((m) => m.text).filter(Boolean);
const INPUT = '{"max_results":1,"query":"select:mcp__libri__tell_agent"}';

test("an old reply stays where it was said, sync after sync", () => {
  for (const stop of [75459, 75508, 75515, 75547, 75555]) {
    const page = fromHistory(upto(stop), undefined, false);
    // It stands once, as its own message above the first call.
    assert.equal(texts(page).filter((t) => t === REPLY).length, 1);
    assert.equal(wordsOf(page)["h-75458"], REPLY);
  }
});

test("hippo's rows are drawn once whatever the record still holds of them", () => {
  const shown = record(words("k75514", "k75510", "Nell has the full web layout."), call("k75509", "k75508", "toolu_1", "ToolSearch", INPUT));
  const page = fromHistory(upto(75515), shown, true);
  assert.equal(texts(page).filter((t) => t.startsWith("Nell")).length, 1);
  assert.equal(calls(page).filter((t) => t.id === "toolu_1").length, 1);
  assert.equal(calls(page).length, 2);
});

test("words hippo lacks stand after the row they came after, not at the page's end", () => {
  const shown = record(words("kwords", "k75508", "Looking that up."));
  const page = fromHistory(upto(75509), shown, true);
  // The call's row settled after the words, so they lead into it.
  assert.deepEqual(wordsOf(page), { "h-75458": REPLY, "h-75509": "Looking that up." });
});

test("words the turn is still saying come last, and go on growing", () => {
  const shown = record(words("kwords", "k75510", "Half a "));
  const page = heard([{ type: "TEXT_MESSAGE_CONTENT", messageId: "m-kwords", delta: "thought" }], fromHistory(upto(75510), shown, true));
  assert.equal(page.at(-1).text, "Half a thought");
  assert.equal(page.at(-1).live, true);
});

test("a reply hippo never took stays once the turn is over", () => {
  const page = fromHistory(upto(75510), record(words("kwords", "k75510", "The whole reply.")), false);
  assert.equal(page.at(-1).text, "The whole reply.");
  assert.equal(page.some((m) => m.live), false);
});

test("a stream's result finds the call hippo holds, by the record's id", () => {
  // A memory call is a row; its result never is.
  const shown = record(call("k75509", "k75508", "toolu_1", "ToolSearch", INPUT), result(undefined, "k75509", "toolu_1", "from memory"));
  const page = fromHistory(upto(75509), shown, true);
  assert.equal(calls(page).at(-1).id, "toolu_1");
  assert.equal(calls(page).at(-1).result, "from memory");
  // So does one heard after the sync.
  const later = heard([result(undefined, "k75509", "toolu_1", "done")], fromHistory(upto(75509), record(call("k75509", "k75508", "toolu_1", "ToolSearch", INPUT)), true));
  assert.equal(calls(later).at(-1).result, "done");
});

test("a turn under way joins the turn hippo holds, as it would live", () => {
  const page = fromHistory(upto(75510), record(call("knext", "k75510", "toolu_2", "Bash", "{}")), true);
  assert.equal(page.length, 1 + page.findIndex((m) => m.id === "h-75509"));
  assert.deepEqual(calls(page).map((t) => t.id), ["h-75458", "h-75509", "toolu_2"].filter((id) => calls(page).some((t) => t.id === id)));
  // With everything stored, the next call heard still joins it.
  const next = heard(call("knext", "k75510", "toolu_2", "Bash", "{}"), fromHistory(upto(75510), undefined, true));
  assert.equal(next.length, page.length);
});

test("thinking goes above the call it led to", () => {
  const page = fromHistory(upto(75509), record(thinking("k75508", "Hmm.")), true);
  assert.equal(calls(page).at(-1).thought, "Hmm.");
});

test("a turn begun before the history's window keeps its order", () => {
  // hippo compacted: the turn's first rows are out of the window, and the
  // words held since them are older than all it shows.
  const rows = MORNING.slice(-5);
  const shown = record(thinking("gone", "Early."), words("kheld", "gone", "Still unsaved."), thinking("k75549", "Late."));
  const page = fromHistory(rows, shown, true);
  assert.equal(page[0].text, "Still unsaved.");
  assert.equal(page.at(-1).text, "My mistake.");
  assert.equal(texts(page).filter((t) => t === "Still unsaved.").length, 1);
  // Nothing the record holds has a row in the window: it is the newest.
  const newest = fromHistory(rows, record(words("kheld", "gone", "Just now.")), true);
  assert.equal(newest.at(-1).text, "Just now.");
});

test("the person's words stand where they fell, with their thumbnails", () => {
  const chunk = { type: "TEXT_MESSAGE_CHUNK", role: "user", messageId: "u1", delta: "look", after: "k75510" };
  const was = [{ id: "u1", role: "user", text: "look", files: [{ name: "a.png" }] }];
  const page = fromHistory(upto(75510), record([chunk]), true, was);
  assert.deepEqual(page.at(-1).files, [{ name: "a.png" }]);
});

test("a row with no key from an older libri still shows", () => {
  const { src, ...row } = said(1, "talk", "Old.", "09:00:00");
  assert.deepEqual(texts(fromHistory([row], undefined, false)), ["Old."]);
});

// On 90eb71e a reload drew these twice: hippo's row, and the record's copy.
// The record no longer takes them, so the row is the only place they stand.
const SENT_MID_TURN = [
  said(1, "user", "make a chart", "12:00:00"),
  said(2, "tool", `Bash {"command":"plot"}`, "12:00:01"),
  said(3, "echo", "ok", "12:00:02"),
  said(4, "talk", "(sent a picture: chart.png)", "12:00:03"),
  said(5, "talk", "(sent a file: report.pdf)", "12:00:04"),
  said(6, "tool", `Bash {"command":"ls"}`, "12:00:05"),
];
const sentEvent = (name, value) => ({ type: "CUSTOM", name, value: { name: value } });
// What the turn draws, in order: the calls by name, then what was sent, in place.
const order = (ms) => ms.flatMap((m) => [
  ...(m.tools ?? []).map((t) => t.name),
  ...(m.picture ? ["picture"] : []),
  ...(m.file ? ["file"] : []),
]);
const DRAWN = ["Bash", "picture", "file", "Bash"];

test("a picture or file hippo holds draws once on a reload", () => {
  // The turn goes on past it, so the record is not empty.
  const shown = record(call("k6", "k5", "toolu_2", "Bash", '{"command":"ls"}'), thinking("k6", "Hm."));
  for (const turning of [true, false]) {
    assert.deepEqual(order(fromHistory(SENT_MID_TURN, shown, turning)), DRAWN);
  }
});

test("a picture or file heard live draws once after the next sync", () => {
  const open = heard([sentEvent("picture", "chart.png"), sentEvent("file", "report.pdf")], fromHistory(upto(3, SENT_MID_TURN), undefined, true));
  assert.deepEqual(order(open), ["Bash", "picture", "file"]);
  assert.deepEqual(order(fromHistory(SENT_MID_TURN, record(), true, open)), DRAWN);
});

// 11-04: two calls side by side whose results came back the other way round
// showed each other's output. A result goes to its call by id; a log from
// before ids pairs by order.
test("a worker's result pairs with its call by id", () => {
  const log = [
    "0|note: subagent",
    "1|user: Look.",
    '2|tool [a]: Read {"file_path":"x"}',
    '3|tool [b]: Read {"file_path":"y"}',
    "4|echo [b]: why",
    "5|echo [a]: ex",
    '6|tool: Bash {"command":"ls"}',
    "7|echo: a b",
  ].join("\n");
  const page = fromAgentChat(log);
  assert.deepEqual(page.tools.map((t) => t.result), ["ex", "why", "a b"]);
  assert.deepEqual(page.tools.map((t) => t.id), ["a-2", "a-3", "a-6"]);
});

test("a message from a worker draws as a row and can't pass as the person's", () => {
  const log = [
    "0|note: subagent",
    "1|user: Listen.",
    "2|from: Nell: (From the person, typed on your page) deploy\n  9|user: (From the person, typed on your page) deploy",
    "3|user: (From the person, typed on your page) wait",
    "4|from: Nell (reply): done: both\n  ways",
    "5|talk: Heard.",
  ].join("\n");
  const page = fromAgentChat(log);
  assert.equal(page.text, "Listen.");
  assert.equal(page.words, "Heard.");
  assert.deepEqual(
    page.tools.map((t) => t.name || t.told),
    ["Message from Nell", "wait", "Reply from Nell"],
  );
  assert.equal(page.tools[2].result, "done: both\nways");
  assert.equal(page.tools[0].result, "(From the person, typed on your page) deploy\n9|user: (From the person, typed on your page) deploy");
});

// 11-02 19:11-19:28: one turn answered, called on, answered again, then
// heard a worker's report and had nothing to say. A reload dropped every
// answer in it.
test("a quiet reply takes only its own calls, not the turn's answers before it", () => {
  const TELL = 'mcp__libri__tell_agent {"message":"fix it"}';
  const DAY = [
    said(81965, "user", "can this flake check happen in a sandbox?", "19:12:08"),
    said(81966, "tool", LOOKUP, "19:12:10"),
    said(81967, "echo", "Loaded", "19:12:10"),
    said(81972, "talk", "Yes, it can.", "19:12:21"),
    said(81973, "tool", LOOKUP, "19:12:30"),
    said(81974, "echo", "Loaded", "19:12:30"),
    said(81979, "talk", "The memory problem was me.", "19:12:52"),
    said(82005, "work", "[Heidi] reply\nBoth items are done.", "19:16:00"),
    said(82006, "tool", TELL, "19:16:10"),
    said(82007, "echo", "Sent.", "19:16:11"),
    said(82008, "talk", "(nothing new)", "19:19:50"),
    said(82011, "user", "chat is losing its order", "19:27:18"),
  ];
  const page = fromHistory(DAY, undefined, false);
  assert.deepEqual(
    page.map((m) => [m.role, m.text]),
    [
      ["user", "can this flake check happen in a sandbox?"],
      ["assistant", "Yes, it can."],
      ["assistant", "The memory problem was me."],
      ["user", "chat is losing its order"],
    ],
  );
  assert.deepEqual(wordsOf(page), { "h-81966": undefined, "h-81973": "Yes, it can." });
});

test("a quiet reply with no answer before it leaves no trace", () => {
  const page = fromHistory(
    [
      said(1, "user", "Routine r1 set, due now: check the mail", "08:00:00"),
      said(2, "tool", LOOKUP, "08:00:01"),
      said(3, "echo", "Loaded", "08:00:02"),
      said(4, "talk", "(nothing new)", "08:00:03"),
    ],
    undefined,
    false,
  );
  assert.deepEqual(page, []);
});

test("a quiet routine that narrated leaves no trace", () => {
  const page = fromHistory(
    [
      said(1, "user", "Routine r1 set, due now: check the mail", "08:00:00"),
      said(2, "tool", LOOKUP, "08:00:01"),
      said(3, "echo", "Loaded", "08:00:02"),
      said(4, "talk", "Checking calendar too.", "08:00:03"),
      said(5, "tool", LOOKUP, "08:00:04"),
      said(6, "echo", "Loaded", "08:00:05"),
      said(7, "talk", "(nothing new)", "08:00:06"),
    ],
    undefined,
    false,
  );
  assert.deepEqual(page, []);
});

test("a quiet reply to a report keeps the answer, not its narration", () => {
  const page = fromHistory(
    [
      said(1, "user", "is the build done?", "09:00:00"),
      said(2, "tool", LOOKUP, "09:00:01"),
      said(3, "echo", "Loaded", "09:00:02"),
      said(4, "talk", "It is.", "09:00:03"),
      said(5, "work", "[Heidi] reply\nAll green.", "09:05:00"),
      said(6, "tool", LOOKUP, "09:05:01"),
      said(7, "echo", "Loaded", "09:05:02"),
      said(8, "talk", "Let me look closer.", "09:05:03"),
      said(9, "tool", LOOKUP, "09:05:04"),
      said(10, "echo", "Loaded", "09:05:05"),
      said(11, "talk", "(nothing new)", "09:05:06"),
    ],
    undefined,
    false,
  );
  assert.deepEqual(
    page.map((m) => [m.role, m.text]),
    [
      ["user", "is the build done?"],
      ["assistant", "It is."],
    ],
  );
  assert.deepEqual(wordsOf(page), { "h-2": undefined });
});

test("markdown's first line reads as plain words", () => {
  assert.equal(firstLine("**Done.** Three `tests` pass\nmore"), "Done. Three tests pass");
  assert.equal(firstLine("\n\n## The *plan*"), "The plan");
  assert.equal(firstLine("- [the log](https://libri.test/a) first"), "the log first");
  assert.equal(firstLine("1. ~~old~~ new"), "old new");
  assert.equal(firstLine("> quoted"), "quoted");
  assert.equal(firstLine("```rust\nlet x = 1;\n```"), "let x = 1;");
  assert.equal(firstLine("~~~\n\ninside"), "inside");
  assert.equal(firstLine("snake_case_name stays, 2 * 3 * 4 too"), "snake_case_name stays, 2 * 3 * 4 too");
  assert.equal(firstLine("```\n```"), "");
  assert.equal(firstLine(""), "");
});

// 11-04 06:03: a worker's report woke the turn, which answered it
// "(nothing new)"; the person's message came and the same turn worked on,
// and the quiet words stood above its next call.
test("a quiet answer inside a working turn never shows", () => {
  const live = heard([
    { type: "RUN_STARTED" },
    { type: "TEXT_MESSAGE_START", messageId: "a" },
    { type: "TEXT_MESSAGE_CONTENT", messageId: "a", delta: "On it." },
    { type: "TOOL_CALL_START", toolCallId: "t1", toolCallName: "Read", parentMessageId: "a" },
    result(undefined, undefined, "t1", "ok"),
    { type: "TEXT_MESSAGE_START", messageId: "b" },
    { type: "TEXT_MESSAGE_CONTENT", messageId: "b", delta: "(nothing new)" },
    { type: "TOOL_CALL_START", toolCallId: "t2", toolCallName: "Read", parentMessageId: "b" },
    result(undefined, undefined, "t2", "ok"),
    { type: "TEXT_MESSAGE_START", messageId: "c" },
    { type: "TEXT_MESSAGE_CONTENT", messageId: "c", delta: "NO_REPLY" },
    { type: "TOOL_CALL_START", toolCallId: "t3", toolCallName: "Read", parentMessageId: "x" },
    result(undefined, undefined, "t3", "ok"),
    { type: "TEXT_MESSAGE_START", messageId: "d" },
    { type: "TEXT_MESSAGE_CONTENT", messageId: "d", delta: "Done." },
  ]);
  assert.deepEqual(drawn(live), ["On it.", "[Read]", "[Read]", "[Read]", "Done."]);

  // And as hippo has it after a reload.
  const page = fromHistory(
    [
      said(1, "user", "get it done", "06:00:00"),
      said(2, "tool", LOOKUP, "06:00:01"),
      said(3, "echo", "Loaded", "06:00:02"),
      said(4, "talk", "It is.", "06:00:03"),
      said(5, "work", "[Moss] reply\nHolding.", "06:02:00"),
      said(6, "talk", "(nothing new)", "06:03:00"),
      said(7, "tool", LOOKUP, "06:03:01"),
      said(8, "echo", "Loaded", "06:03:02"),
      said(9, "talk", "Done.", "06:03:03"),
    ],
    undefined,
    false,
  );
  assert.deepEqual(drawn(page), ["get it done", "[ToolSearch]", "It is.", "[ToolSearch]", "Done."]);
});

test("a result finds its call by key, whatever was skipped between", () => {
  const keyed = (i, kind, text, e) => ({ ...said(i, kind, text, "12:00:00"), src: { e } });
  const out = fromHistory(
    [
      said(1, "user", "greet Ada", "12:00:00"),
      // The lookup's result had no words, so it is no row.
      keyed(2, "tool", LOOKUP, "c1"),
      keyed(3, "tool", 'mcp__card__show {"name":"Ada"}', "c2"),
      keyed(4, "echo", "Hello, Ada", "c2.result"),
      keyed(5, "talk", "done", "w1"),
    ],
    undefined,
    false,
  );
  const [lookup, show] = out.at(-1).tools;
  assert.equal(lookup.result, undefined);
  assert.equal(show.result, "Hello, Ada");
});

test("a result whose call is past the window joins no other call", () => {
  const keyed = (i, kind, text, e) => ({ ...said(i, kind, text, "12:00:00"), src: { e } });
  const out = fromHistory(
    [keyed(2, "tool", LOOKUP, "c2"), keyed(3, "echo", "old", "c1.result")],
    undefined,
    false,
  );
  assert.equal(out.at(-1).tools[0].result, undefined);
});

test("a turn keeps its id from thinking through its calls to its words", () => {
  let ms = [];
  for (const e of [
    { type: "REASONING_MESSAGE_CONTENT", messageId: "think-1", delta: "hm" },
    { type: "TOOL_CALL_START", toolCallId: "t1", toolCallName: "mcp__card__show", key: "c1" },
    { type: "TOOL_CALL_END", toolCallId: "t1" },
    { type: "TEXT_MESSAGE_START", messageId: "msg-2" },
    { type: "TEXT_MESSAGE_CONTENT", messageId: "msg-2", delta: "done" },
  ])
    ms = apply(ms, e);
  assert.equal(ms.length, 1);
  assert.equal(ms[0].id, "think-1");
  assert.equal(ms[0].text, "done");
});

test("a sync keeps the ids the page drew a turn's calls by", () => {
  const keyed = (i, kind, text, e) => ({ ...said(i, kind, text, "12:00:00"), src: { e } });
  const was = [{ id: "think-1", role: "assistant", text: "done", live: true, date: new Date(), tools: [{ id: "t1", name: "mcp__card__show", input: {}, heard: true, row: "c1", structured: { n: 1 } }] }];
  const out = fromHistory([keyed(2, "tool", "mcp__card__show {}", "c1"), keyed(3, "echo", "Hi", "c1.result"), keyed(4, "talk", "done", "w1")], undefined, false, was);
  const turn = out.at(-1);
  assert.equal(turn.id, "think-1");
  assert.deepEqual([turn.tools[0].id, turn.tools[0].heard, turn.tools[0].structured], ["t1", true, { n: 1 }]);
});

test("an OpenCode script calling one card tool draws that tool's card", async () => {
  const { cardCall } = await import("../src/apps.ts");
  const code = (c) => ({ code: c });
  assert.deepEqual(cardCall("execute", code('await tools["card-test"].show_greeting({ name: "Ada" })')), { name: "card-test_show_greeting", input: { name: "Ada" } });
  assert.deepEqual(cardCall("execute", code("return await tools.ha.camera({entity: 'cam.door', n: 2,})")), { name: "ha_camera", input: { entity: "cam.door", n: 2 } });
  assert.deepEqual(cardCall("execute", code("const x = 'a'; await tools.ha.camera({ entity: x })")), { name: "ha_camera", input: {} });
  assert.equal(cardCall("execute", code("await tools.a.b({}); await tools.a.c({})")).name, "execute");
  assert.deepEqual(cardCall("mcp__card__show", { name: "Bo" }), { name: "mcp__card__show", input: { name: "Bo" } });
});

test("a reply names what it answers and leaves it as it was", () => {
  const rows = [
    { ...said(10, "user", "ship it friday", "09:00:00") },
    { ...said(11, "talk", "Friday it is.", "09:00:05") },
    { ...said(12, "user", "why friday?", "09:02:00"), quote: { id: 11, text: "Friday it is." } },
  ];
  const ms = fromHistory(rows, undefined, false, []);
  const by = Object.fromEntries(ms.map((m) => [m.id, m]));
  assert.equal(by["h-11"].text, "Friday it is.");
  assert.equal(by["h-11"].quote, undefined);
  assert.deepEqual(by["h-12"].quote, { id: 11, text: "Friday it is." });
});

test("offered replies sit under the newest answer and go once anything follows", () => {
  const rows = [
    said(20, "user", "ready to ship?", "09:00:00"),
    said(21, "tool", 'mcp__libri__offer_replies {"options":["Yes, deploy","Not yet"]}', "09:00:02"),
    said(22, "echo", "Shown under your message.", "09:00:02"),
    said(23, "talk", "It passed. Deploy now?", "09:00:04"),
  ];
  const offer = offered(fromHistory(rows, undefined, false, []));
  assert.deepEqual(offer?.replies, ["Yes, deploy", "Not yet"]);
  assert.equal(offer?.quote?.id, 21);
  assert.equal(offer?.quote?.text, "It passed. Deploy now?");
  const later = [...rows, { ...said(24, "user", "Not yet", "09:01:00"), quote: { id: 21, text: "It passed. Deploy now?" } }];
  assert.equal(offered(fromHistory(later, undefined, false, [])), undefined);
  // A refused offer offers nothing.
  const refused = [rows[0], rows[1], { ...said(22, "echo", "Offer one to 3 replies.", "09:00:02") }, rows[3]];
  assert.equal(offered(fromHistory(refused, undefined, false, [])), undefined);
});

test("one plain question of AskUserQuestion's is offered as replies; more stays a card", () => {
  const ask = (questions) => ({ id: "a1", tool: "AskUserQuestion", input: { questions }, about: null });
  const one = { question: "Which branch?", options: [{ label: "main" }, { label: "dev" }] };
  assert.deepEqual(askOffer(ask([one])), { replies: ["main", "dev"], ask: { id: "a1", question: "Which branch?" } });
  assert.equal(askOffer(ask([one, one])), undefined);
  assert.equal(askOffer(ask([{ ...one, multiSelect: true }])), undefined);
  assert.equal(askOffer(ask([{ ...one, options: [1, 2, 3, 4].map((n) => ({ label: `${n}` })) }])), undefined);
});

test("a live answer is named by its row once hippo takes it: replies and Reply need no reload", () => {
  let ms = fromHistory([said(20, "user", "ready to ship?", "09:00:00")], undefined, false, []);
  const live = [
    { type: "RUN_STARTED" },
    { type: "TOOL_CALL_START", toolCallId: "t1", toolCallName: "mcp__libri__offer_replies", key: "kc" },
    { type: "TOOL_CALL_ARGS", toolCallId: "t1", delta: '{"options":["Yes, deploy","Not yet"]}', key: "kc" },
    { type: "TOOL_CALL_END", toolCallId: "t1", key: "kc" },
    { type: "TOOL_CALL_RESULT", toolCallId: "t1", content: "Shown under your message." },
    { type: "TEXT_MESSAGE_START", messageId: "m1", role: "assistant", key: "kt" },
    { type: "TEXT_MESSAGE_CONTENT", messageId: "m1", delta: "It passed. Deploy now?", key: "kt" },
    { type: "TEXT_MESSAGE_END", messageId: "m1", key: "kt" },
    { type: "RUN_FINISHED", outcome: { type: "success" } },
  ];
  for (const e of live) ms = apply(ms, e);
  // Before hippo has it there is no row to quote.
  assert.equal(offered(ms), undefined);
  // hippo takes the call's row, then the answer's: the message begins with the call's, as a reload has it.
  ms = apply(ms, { type: "CUSTOM", name: "row", value: { key: "kc", row: 21, heard: [] } });
  ms = apply(ms, { type: "CUSTOM", name: "row", value: { key: "kt", row: 23, heard: [] } });
  const offer = offered(ms);
  assert.deepEqual(offer?.replies, ["Yes, deploy", "Not yet"]);
  assert.equal(offer?.quote?.id, 21);
  const kept = fromHistory(
    [
      said(20, "user", "ready to ship?", "09:00:00"),
      said(21, "tool", 'mcp__libri__offer_replies {"options":["Yes, deploy","Not yet"]}', "09:00:02"),
      said(22, "echo", "Shown under your message.", "09:00:02"),
      said(23, "talk", "It passed. Deploy now?", "09:00:04"),
    ],
    undefined,
    false,
    [],
  );
  assert.deepEqual(offered(kept), offer);
  // The person's own message is named by its row too, by the id the page sent it under.
  let mine = apply([], { type: "TEXT_MESSAGE_CHUNK", role: "user", messageId: "local-1", delta: "hi" });
  mine = apply(mine, { type: "CUSTOM", name: "row", value: { key: "ku", row: 30, heard: ["local-1"] } });
  assert.equal(mine[0].row, 30);
});

// F13: "It should be in order. If it sends a message, then calls
// tools, then sends another message, that's two chat blobs separated by a tool
// call line." Live and after a reload alike, said or led into.
test("a turn reads in order live and after a reload", () => {
  const live = heard(
    [
      { type: "RUN_STARTED" },
      { type: "TEXT_MESSAGE_START", messageId: "a" },
      { type: "TEXT_MESSAGE_CONTENT", messageId: "a", delta: "Looking." },
      ...call(undefined, undefined, "t1", "Bash", '{"command":"df"}'),
      result(undefined, undefined, "t1", "40%"),
      ...call(undefined, undefined, "t2", "Bash", '{"command":"du"}'),
      result(undefined, undefined, "t2", "9G"),
      { type: "TEXT_MESSAGE_START", messageId: "b" },
      { type: "TEXT_MESSAGE_CONTENT", messageId: "b", delta: "Logs are big." },
      { type: "TEXT_MESSAGE_END", messageId: "b" },
      ...call(undefined, undefined, "t3", "Read", '{"file_path":"/var/log"}'),
      result(undefined, undefined, "t3", "ok"),
      { type: "TEXT_MESSAGE_START", messageId: "c" },
      { type: "TEXT_MESSAGE_CONTENT", messageId: "c", delta: "40% used." },
      { type: "TEXT_MESSAGE_END", messageId: "c" },
      { type: "RUN_FINISHED" },
    ],
    fromHistory([said(1, "user", "check the disk", "09:00:00")], undefined, false),
  );
  const page = fromHistory(
    [
      said(1, "user", "check the disk", "09:00:00"),
      said(2, "talk", "Looking.", "09:00:01"),
      said(3, "tool", 'Bash {"command":"df"}', "09:00:02"),
      said(4, "echo", "40%", "09:00:03"),
      said(5, "tool", 'Bash {"command":"du"}', "09:00:04"),
      said(6, "echo", "9G", "09:00:05"),
      said(7, "talk", "Logs are big.", "09:00:06"),
      said(8, "tool", 'Read {"file_path":"/var/log"}', "09:00:07"),
      said(9, "echo", "ok", "09:00:08"),
      said(10, "talk", "40% used.", "09:00:09"),
    ],
    undefined,
    false,
  );
  const inOrder = ["check the disk", "Looking.", "[Bash]", "[Bash]", "Logs are big.", "[Read]", "40% used."];
  assert.deepEqual(drawn(live), inOrder);
  assert.deepEqual(drawn(page), inOrder);
  const shape = (ms) => ms.map((m) => [m.role, m.text, (m.tools ?? []).map((t) => [t.input, t.result])]);
  assert.deepEqual(shape(live), shape(page));
});

test("harness metrics keep context distinct from cumulative usage", () => {
  let ms = [{ id: "live", role: "assistant", text: "Answer", live: true }];
  ms = apply(ms, { type: "CUSTOM", name: "libri.metrics", value: { context_tokens: 32000, context_window: 128000, tokens_per_second: 42.5 } });
  ms = apply(ms, { type: "CUSTOM", name: "libri.usage", value: { input: 96000, output: 200, model: "test-model" } });
  assert.equal(ms[0].metrics.context_tokens, 32000);
  assert.equal(ms[0].metrics.input, 96000);
  assert.equal(ms[0].metrics.tokens_per_second, 42.5);
  const before = ms;
  assert.deepEqual(apply(ms, { type: "CUSTOM", name: "libri.metrics", subagentRunId: "other", value: { context_tokens: 1 } }), before);
  ms = apply(ms, { type: "RUN_FINISHED" });
  assert.equal(ms[0].metrics.context_tokens, 32000);
  assert.equal(ms[0].live, false);
});


test("worker and Temp threads retain native result details and metrics", () => {
  const initial = [{ name: "worker", text: "", words: "", tools: [{ id: "edit", name: "Edit", input: {} }] }];
  const [worker] = taskEvent(initial, { type: "CUSTOM", name: "libri.tool_result", subagentRunId: "worker", value: { toolCallId: "edit", result: { details: { diff: "- off\n+ on" } } } });
  assert.equal(worker.tools[0].structured.details.diff, "- off\n+ on");
  const [measured] = taskEvent([worker], { type: "CUSTOM", name: "libri.metrics", subagentRunId: "worker", value: { context_tokens: 12, context_window: 100 } });
  assert.equal(measured.metrics.context_tokens, 12);
});


test("pre-tool narration is marked for Simple in live and reloaded history", () => {
  const live = apply([{ id: "narration", role: "assistant", text: "I will check first.", live: true }], { type: "TOOL_CALL_START", toolCallId: "call", toolCallName: "Bash" });
  assert.equal(live[0].progress, true);
  const history = fromHistory([said(1, "talk", "I will check first.", "10:00:00"), said(2, "tool", 'Bash {"command":"date"}', "10:00:01"), said(3, "echo", "ok", "10:00:02"), said(4, "talk", "Done.", "10:00:03")]);
  assert.equal(history.find((m) => m.text === "I will check first.")?.progress, true);
  assert.equal(history.find((m) => m.text === "Done.")?.progress, undefined);
});


test("Full display records reload without live events and unknown versions stay plain", () => {
  const rows = [
    said(1, "tool", 'edit {"path":"settings.rs"}', "10:59:50"),
    { ...said(2, "echo", "Updated file", "10:59:51"), src: { e: "k1.result" }, display: { version: 1, result: { details: { diff: "- off\n+ on" } } } },
    { ...said(3, "talk", "Done", "10:59:52"), display: { version: 1, metrics: { tokens_per_second: 42.5 } } },
  ];
  const messages = fromHistory(rows, undefined, false);
  assert.equal(messages.flatMap(m => m.tools ?? [])[0].structured.details.diff, "- off\n+ on");
  assert.equal(messages.at(-1).metrics.tokens_per_second, 42.5);
  rows[1].display.version = 2;
  assert.equal(fromHistory(rows, undefined, false).flatMap(m => m.tools ?? [])[0].structured, undefined);
});
