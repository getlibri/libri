// Draws Libri's mark from its master, brand/spine.svg, and writes every
// icon made from it. Run `node brand/generate_icons.mjs` from anywhere; it
// rewrites the files below in place, so a change to the mark is a change to
// spine.svg and a rerun.
//
// The mark is drawn as the master draws it: five plain strokes, one colour,
// round caps. Each icon only places it, scaling the strokes and their width
// together, so every size is the same shape.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// The five strokes and their width, read from the master: each path is
// "M spine Q control end".
const master = readFileSync(join(root, "brand/spine.svg"), "utf8");
const curves = [...master.matchAll(/d="M([^"]+)"/g)].map((m) => m[1].split(/[\sQ,]+/).filter(Boolean).map(Number));
if (curves.length !== 5 || curves.some((c) => c.length !== 6)) throw new Error("spine.svg: want five 'M x y Q cx cy ex ey' paths");
const width = Number(master.match(/stroke-width="([\d.]+)"/)?.[1]);
if (!width) throw new Error("spine.svg: want a stroke-width");

// The mark's extent as drawn: points along each curve, widened by half the
// stroke (the round caps and the stroke's edge reach that far).
const q = ([ax, ay, cx, cy, bx, by], t) => {
  const u = 1 - t;
  return [u * u * ax + 2 * u * t * cx + t * t * bx, u * u * ay + 2 * u * t * cy + t * t * by];
};
const samples = curves.flatMap((c) => Array.from({ length: 65 }, (_, i) => q(c, i / 64)));
const xs = samples.map((p) => p[0]);
const ys = samples.map((p) => p[1]);
const [x0, y0, x1, y1] = [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
const mx = (x0 + x1) / 2;
const my = (y0 + y1) / 2;
// Scale k so the drawn mark is `side` wide at its longer side.
const fitSquare = (side) => side / (Math.max(x1 - x0, y1 - y0) + width);
// Scale k so the drawn mark stays within radius r of its centre: what an
// adaptive or maskable icon's safe circle asks for.
const fitCircle = (r) => r / (Math.max(...samples.map(([x, y]) => Math.hypot(x - mx, y - my))) + width / 2);

// Placing the mark: scale k about its own centre, then move that centre to
// (cx, cy). Path data is written in the placed coordinates.
const f = (v) => String(Math.round(v * 100) / 100);
const pathData = (k, cx, cy, sep = " ") =>
  curves
    .map((c) => {
      const p = (i) => f(cx + (c[i] - mx) * k) + sep + f(cy + (c[i + 1] - my) * k);
      return `M${p(0)}Q${p(2)} ${p(4)}`;
    })
    .join("");

const out = (file, text) => writeFileSync(join(root, file), text);
const generated = "Written by brand/generate_icons.mjs from brand/spine.svg; change the mark there, not here.";
const svgStroke = (stroke, w, d) => `<path fill="none" stroke="${stroke}" stroke-width="${f(w)}" stroke-linecap="round" d="${d}"/>`;

// Colours: the page's ink in each theme (libri/site/src/index.css --ink),
// and its dark paper (--paper) as the icons' plain ground.
const inkLight = "#2a2019";
const inkDark = "#ebe3d5";
const ground = "#171411";

// The app icon for the web (manifest, home screens; the build renders its
// PNGs): full bleed on the plain ground, the mark inside the maskable safe
// circle (radius 205 of 512) so launchers crop it to their shape.
{
  const k = fitCircle(186);
  out(
    "libri/site/public/icon/app.svg",
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">\n` +
      `  <!-- ${generated} -->\n` +
      `  <rect width="512" height="512" fill="${ground}"/>\n` +
      `  ${svgStroke(inkDark, width * k, pathData(k, 256, 256))}\n</svg>\n`,
  );
}

// The favicon: filling the square edge to edge, in the ink of the browser's
// own scheme, so it reads on a light or a dark tab strip.
{
  const k = fitSquare(120);
  out(
    "libri/site/public/icon/favicon.svg",
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 120">\n` +
      `  <!-- ${generated} -->\n` +
      `  <style>path{stroke:${inkLight}}@media(prefers-color-scheme:dark){path{stroke:${inkDark}}}</style>\n` +
      `  ${svgStroke(inkLight, width * k, pathData(k, 60, 60))}\n</svg>\n`,
  );
}

// The mark in the page, at the side panel's head: square, in plain black,
// for the page to draw in its own ink through a CSS mask (App.tsx Panel).
{
  const k = fitSquare(24);
  out(
    "libri/site/src/mark.svg",
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">\n` +
      `  <!-- ${generated} -->\n` +
      `  ${svgStroke("#000", width * k, pathData(k, 12, 12))}\n</svg>\n`,
  );
}

// Android's adaptive icon: 108dp layers, of which every launcher shape keeps
// the middle 66dp circle whole, so the mark stays within radius 30.
const vector = (comment, size, view, body) =>
  `<?xml version="1.0" encoding="utf-8"?>\n<!-- ${comment}\n     ${generated} -->\n` +
  `<vector xmlns:android="http://schemas.android.com/apk/res/android"\n` +
  `    android:width="${size}dp"\n    android:height="${size}dp"\n` +
  `    android:viewportWidth="${view}"\n    android:viewportHeight="${view}">\n${body}</vector>\n`;
const vfill = (fill, d) => `    <path\n        android:fillColor="${fill}"\n        android:pathData="${d}" />\n`;
const vstroke = (stroke, w, d) =>
  `    <path\n        android:strokeColor="${stroke}"\n        android:strokeWidth="${f(w)}"\n` +
  `        android:strokeLineCap="round"\n        android:pathData="${d}" />\n`;
{
  const k = fitCircle(30);
  const d = pathData(k, 54, 54, ",");
  const res = "android/app/src/main/res/drawable";
  out(`${res}/ic_launcher_background.xml`, vector("The icon's plain ground, the page's dark paper.", 108, 108, vfill(ground, "M0,0h108v108h-108z")));
  out(`${res}/ic_launcher_foreground.xml`, vector("The mark, light on the ground.", 108, 108, vstroke(inkDark, width * k, d)));
  out(`${res}/ic_launcher_monochrome.xml`, vector("The mark alone, for a launcher to tint as a themed icon.", 108, 108, vstroke("#FFFFFFFF", width * k, d)));
}
// The status bar's icon: white (the system tints it), in the 24dp icon
// grid's 20dp live area.
{
  const k = fitSquare(20);
  out(
    "android/app/src/main/res/drawable/ic_notify.xml",
    vector("The mark in one colour for the status bar.", 24, 24, vstroke("#FFFFFFFF", width * k, pathData(k, 12, 12, ","))),
  );
}
