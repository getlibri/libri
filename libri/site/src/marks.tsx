// The makers' marks for models: the composer's model picker draws them, and
// so do the circles of the agents running over the composer (agents.tsx).

/** The maker's mark (LobeHub's icons, MIT, in logos/), by the first
 * word of the model's own id; a model whose maker has none shows its
 * provider's, as OpenCode's own free models. */
const MAKERS: [RegExp, string][] = [
  [/^(claude|opus|sonnet|haiku|fable)/, "claude"],
  [/^(gpt|o\d|codex)/, "openai"],
  [/^deepseek/, "deepseek"],
  [/^(glm|zhipu)/, "zhipu"],
  [/^kimi/, "moonshot"],
  [/^grok/, "xai"],
  [/^qwen/, "qwen"],
  [/^gemini/, "gemini"],
  [/^(mistral|devstral|codestral)/, "mistral"],
  [/^minimax/, "minimax"],
  [/^(muse|llama)/, "meta"],
  [/^mimo/, "xiaomimimo"],
  [/^nemotron/, "nvidia"],
  [/^hy-|^hunyuan/, "hunyuan"],
  [/^step/, "stepfun"],
  [/^longcat/, "longcat"],
];

/** The mark for a model, by its id, then its provider, then its harness. A
 * worker's pick names no provider; an empty model is its harness's default. */
export function logoOf(o: { harness: string; model: string; provider?: string }) {
  const id = o.model.split("/").pop()!.toLowerCase();
  const maker = MAKERS.find(([re]) => re.test(id))?.[1];
  if (maker) return maker;
  if (o.provider?.startsWith("opencode") || o.harness === "opencode") return "opencode";
  return o.harness === "claude" ? "claude" : o.harness === "codex" ? "openai" : o.harness === "muse" ? "meta" : o.harness === "grok" ? "xai" : null;
}

// Bundled, so each mark gets a hashed name under /_site as the other assets.
const LOGOS = Object.fromEntries(
  Object.entries(import.meta.glob<string>("./logos/*.svg", { eager: true, query: "?url", import: "default" })).map(([path, url]) => [path.slice(8, -4), url]),
);

/** Each maker's brand colour, as light-dark() pairs: the brand's own where
 * it stands 3:1 against --hover (a circle's fill), else the nearest shade
 * that does. Makers whose mark is black or white (OpenAI, xAI, Moonshot,
 * OpenCode, StepFun, LongCat) have none and take the text's colour. Android
 * keeps the same table (Glyphs.kt BRANDS). */
const BRANDS: Record<string, string> = {
  claude: "light-dark(#c6532d, #d97757)",
  deepseek: "light-dark(#4867fe, #5773fe)",
  gemini: "light-dark(#086eff, #3186ff)",
  hunyuan: "light-dark(#0055e9, #2776ff)",
  meta: "light-dark(#0074e1, #0081fb)",
  minimax: "light-dark(#e91034, #f23f5d)",
  mistral: "light-dark(#d74004, #fa520f)",
  nvidia: "light-dark(#528100, #76b900)",
  qwen: "light-dark(#615ced, #736eef)",
  zhipu: "light-dark(#3859ff, #5773ff)",
  xiaomimimo: "light-dark(#c75200, #ff6900)",
};

/** A mark in its maker's colour, or the text's own where the brand is
 * black or white; `size` is its Tailwind size class. */
export function Logo({ name, size = "size-4" }: { name: string | null; size?: string }) {
  if (!name || !LOGOS[name]) return <span className={`${size} shrink-0`} aria-hidden />;
  // Quoted: Vite inlines small marks as data URLs, which hold quotes.
  const url = `url("${LOGOS[name]}")`;
  return (
    <span
      aria-hidden
      className={`${size} shrink-0 bg-current`}
      style={{ maskImage: url, WebkitMaskImage: url, maskSize: "contain", maskRepeat: "no-repeat", maskPosition: "center", ...(BRANDS[name] && { backgroundColor: BRANDS[name] }) }}
    />
  );
}
