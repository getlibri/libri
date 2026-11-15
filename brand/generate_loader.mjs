// Draws the Working loader, a book lying flat seen end-on whose pages flip
// left to right one at a time until it is shut on the other side, then back,
// and writes its keyframes for the page and the Android app. Run
// `node brand/generate_loader.mjs` from anywhere; it rewrites the two files
// below in place.
//
// Each page is one quadratic stroke "M root Q control end", kept 56 long as
// it bends. Closed, the five pages lie flat one gap apart and read as a bar.
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// The loop's length, the gap between closed pages, frames per loop.
const DURATION_MS = 2600;
const GAP = 1.5;
const FRAMES = 40;
// Pages, page length, spine root, droop, flex.
const N = 5, L = 56, B0 = [60, 92], D = 7, F = 1.3;
// Each page's move and the lag between pages, in half-loops.
const MOVE = 0.55, LAG = 0.06;
export const VIEWBOX = "-2 12 124 88";
export const STROKE = 7;

const ease = (p) => (p <= 0 ? 0 : p >= 1 ? 1 : (1 - Math.cos(Math.PI * p)) / 2);
// T in [0, 2): over, then back.
const prog = (k, T) => (T < 1 ? ease((T - (N - 1 - k) * LAG) / MOVE) : 1 - ease((T - 1 - k * LAG) / MOVE));
const angle = (k, T) => 180 - 180 * prog(k, T);
const level = (k, T) => k + (N - 1 - 2 * k) * prog(k, T);
const rad = (a) => (a * Math.PI) / 180;

function page(k, T) {
  const B = [B0[0], B0[1] - GAP * level(k, T)];
  const a = angle(k, T);
  const w = (angle(k, T + 1e-3) - angle(k, T - 1e-3)) / 2e-3;
  const h = D * Math.sin(rad(2 * a)) + (F * rad(w)) / 3;
  const quad = (c) => {
    const dx = Math.cos(rad(a)), dy = -Math.sin(rad(a)), px = -dy, py = dx;
    return [[B[0] + (dx * c) / 2 - px * h, B[1] + (dy * c) / 2 - py * h], [B[0] + dx * c, B[1] + dy * c]];
  };
  const arc = (c) => {
    const [m, e] = quad(c);
    let s = 0, o = B;
    for (let i = 1; i <= 40; i++) {
      const t = i / 40, u = 1 - t;
      const p = [u * u * B[0] + 2 * u * t * m[0] + t * t * e[0], u * u * B[1] + 2 * u * t * m[1] + t * t * e[1]];
      s += Math.hypot(p[0] - o[0], p[1] - o[1]);
      o = p;
    }
    return s;
  };
  let c = L;
  for (let i = 0; i < 4; i++) c *= L / arc(c);
  const [m, e] = quad(c);
  const f = (v) => Number(v.toFixed(1));
  // The root rounds down, as the drawing it was picked from did.
  return [B[0], Math.floor(B[1] * 10 + 1e-9) / 10, f(m[0]), f(m[1]), f(e[0]), f(e[1])];
}

// frames[page][frame] = [x0, y0, cx, cy, x1, y1], evenly spaced in time.
const frames = [...Array(N).keys()].map((k) => [...Array(FRAMES + 1).keys()].map((i) => page(k, (2 * i) / FRAMES)));
const d = ([x0, y0, cx, cy, x1, y1]) => `M${x0} ${y0}Q${cx} ${cy} ${x1} ${y1}`;

const header = "Written by brand/generate_loader.mjs; change that and rerun it.";
writeFileSync(
  join(root, "libri/site/src/bookLoader.ts"),
  `// ${header}
// The Working loader's keyframes: per page, its SVG path at evenly spaced
// moments of one loop. The first is the shut book, a flat bar.
export const BOOK_VIEWBOX = ${JSON.stringify(VIEWBOX)};
export const BOOK_STROKE = ${STROKE};
export const BOOK_DURATION_MS = ${DURATION_MS};
export const BOOK_PAGES: string[][] = ${JSON.stringify(frames.map((p) => p.map(d)), null, 0).replaceAll('],[', '],\n  [')};
`,
);
const vb = VIEWBOX.split(" ").map(Number);
writeFileSync(
  join(root, "android/app/src/main/java/su/libri/BookLoader.kt"),
  `// ${header}
package su.libri

/** The Working loader's keyframes, as the page draws them: per page,
 * x0 y0 cx cy x1 y1 of its quadratic stroke at each of ${FRAMES + 1} evenly spaced
 * moments of one loop, in a box of ${vb[2]} by ${vb[3]} from (${vb[0]}, ${vb[1]}). The first
 * is the shut book, a flat bar. */
object BookLoader {
    const val DURATION_MS = ${DURATION_MS}
    const val STROKE = ${STROKE}f
    const val LEFT = ${vb[0]}f
    const val TOP = ${vb[1]}f
    const val WIDTH = ${vb[2]}f
    const val HEIGHT = ${vb[3]}f
    const val FRAMES = ${FRAMES + 1}
    val PAGES: List<FloatArray> = listOf(
${frames.map((p) => `        floatArrayOf(${p.flat().map((v) => `${v}f`).join(", ")}),`).join("\n")}
    )
}
`,
);
