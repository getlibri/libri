// Code highlighting, math and diagrams, loaded the first time an answer
// needs them: they are most of the page's weight, and few answers do.
import "katex/dist/katex.min.css";
import { renderAsync } from "pele/lazy";
import type { DiagramPlugin } from "streamdown";
import githubDark from "@shikijs/themes/github-dark";
import githubLight from "@shikijs/themes/github-light";
import { createCodePlugin } from "@streamdown/code";
export { math } from "@streamdown/math";

// github-light's token colours, each darkened in OKLCH with its hue held
// until it reads at 4.5:1 on a code block's light fill (--block, #e5ddd0):
// drawn for white, the comments, strings and keywords fell to 2.2-3.4 there.
// By night only the comments fall short: github-dark's #6a737d reads at
// 3.0:1 on the dark fill (#2e2924), so they are lifted to 5.3.
const RAISED: Record<string, string> = {
  "#6a737d": "#4f5861",
  "#005cc5": "#0353b3",
  "#6f42c1": "#683ab9",
  "#22863a": "#016523",
  "#d73a49": "#b0002a",
  "#e36209": "#933c03",
  "#b31d28": "#ad1423",
  "#586069": "#505760",
};
const raise = (theme: typeof githubLight, name: string, to: Record<string, string>) => ({
  ...theme,
  name,
  tokenColors: theme.tokenColors?.map((rule) => {
    const fg = rule.settings.foreground?.toLowerCase();
    return fg && to[fg] ? { ...rule, settings: { ...rule.settings, foreground: to[fg] } } : rule;
  }),
});
const paperLight = raise(githubLight, "paper-light", RAISED);
const paperDark = raise(githubDark, "paper-dark", { "#6a737d": "#959da7" });
// Streamdown also needs these themes in its shikiTheme prop; its own
// defaults otherwise override the plugin and discard the contrast fixes.
export const code = createCodePlugin({ themes: [paperLight, paperDark] });

// Pele draws ```mermaid blocks in place of mermaid.js: a tenth the size,
// synchronous, and themed by CSS variables (index.css) rather than
// re-rendered per theme. Streamdown's mermaid controls (fullscreen,
// download) work on its SVG as they would on mermaid's.
export const mermaid: DiagramPlugin = {
  name: "mermaid",
  type: "diagram",
  language: "mermaid",
  getMermaid: () => ({
    initialize() {},
    render: async (id, source) => ({
      // An agent writes the diagram, so it carries no links or images.
      svg: (await renderAsync(source, { idPrefix: id, responsive: true, autoDirection: true, links: false, images: false })).svg,
    }),
  }),
};
