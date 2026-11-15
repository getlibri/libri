// The return page a connection's sign-in comes back through: it sends the
// browser on only to the origin Libri carried in the state, at the one
// path, and otherwise shows its address to paste.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const html = readFileSync(new URL("../public/connect/back/index.html", import.meta.url), "utf8");
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];
assert.ok(!/<script[^>]+src=|<link|@import|<img|<iframe/i.test(html) && !/\burl\(/.test(html), "the page loads nothing");

/** Runs the page at `href`: where it sent the browser, and what it showed. */
function open(href) {
  const shown = {};
  const node = (id) => (shown[id] ??= { hidden: id === "stuck", value: "" });
  const page = new URL(href);
  const location = { search: page.search, href, replaced: null, replace(to) { this.replaced = to; } };
  const context = { URL, URLSearchParams, atob, location, document: { getElementById: node } };
  vm.runInNewContext(script, context);
  return { to: location.replaced, stuck: !node("stuck").hidden, address: node("address").value, forward: context.forward };
}

// As Libri writes it: NONCE.base64url(origin), unpadded.
const carry = (nonce, origin) => `${nonce}.${Buffer.from(origin).toString("base64url")}`;
const page = "https://pages.example.com/connect/back";

test("a carried origin is sent on to its /api/connections/back, query intact", () => {
  const state = carry("n0nce-_x", "http://box.tail.example.net:6780");
  const query = `?code=c1&state=${encodeURIComponent(state)}&iss=https%3A%2F%2Fauth.example.com`;
  const r = open(page + query);
  assert.equal(r.to, `http://box.tail.example.net:6780/api/connections/back${query}`);
  assert.equal(r.stuck, false);
});

test("a server's refusal is sent on too, for Libri to show", () => {
  const query = `?error=access_denied&state=${carry("n", "https://libri.example.com")}`;
  assert.equal(open(page + query).to, `https://libri.example.com/api/connections/back${query}`);
});

test("the page works at any address of its own", () => {
  const query = `?code=c&state=${carry("n", "http://192.168.1.20:6780")}`;
  for (const at of ["https://other.example.org/x/y/connect/back", "http://localhost:1/back"]) {
    assert.equal(open(at + query).to, `http://192.168.1.20:6780/api/connections/back${query}`);
  }
});

test("anything but a bare http or https origin is refused, with the address shown", () => {
  for (const origin of [
    "javascript:alert(1)",
    "data:text/html,hi",
    "file:///etc/passwd",
    "https://ada:pw@libri.example.com",
    "https://libri.example.com/elsewhere",
    "https://libri.example.com/?next=https://x.example",
    "https://libri.example.com#x",
    "//evil.example.com",
    "libri.example.com",
  ]) {
    const href = `${page}?code=c&state=${encodeURIComponent(carry("n", origin))}`;
    const r = open(href);
    assert.equal(r.to, null, origin);
    assert.equal(r.stuck, true, origin);
    assert.equal(r.address, href);
  }
});

test("a state with no origin, a broken one, or no code is not sent anywhere", () => {
  for (const query of ["?code=c&state=plain", "?code=c&state=n.%%%", "?state=" + carry("n", "https://libri.example.com"), ""]) {
    assert.equal(open(page + query).to, null, query);
  }
});
