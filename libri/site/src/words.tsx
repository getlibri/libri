import { useEffect, useState } from "react";
import { Streamdown } from "streamdown";

type Plugins = Pick<typeof import("./plugins"), "code" | "math" | "mermaid">;
let plugins: Plugins | undefined;
let loading: Promise<Plugins> | undefined;
// A code fence, or what may open math.
const NEEDS = /```|\$|\\[([]/;

/** The code, math and diagram plugins once loaded, which the first text needing them starts. */
export function usePlugins(text: string) {
  const [, loaded] = useState<Plugins>();
  const needed = !plugins && NEEDS.test(text);
  useEffect(() => {
    if (!needed) return;
    loading ??= import("./plugins").then(({ code, math, mermaid }) => (plugins = { code, math, mermaid }));
    loading.then(loaded, () => (loading = undefined));
  }, [needed]);
  return plugins;
}

// Words someone wrote, a report, a message, a worker's turn, as the chat
// draws them, code, math and diagrams included: inside an element of class
// `md`, whose table cells break between words only (index.css).
function Words({ text }: { text: string }) {
  const plugins = usePlugins(text);
  return (
    <Streamdown plugins={plugins} shikiTheme={plugins?.code.getThemes()} mode="static" lineNumbers={false} controls={{ code: true, table: false }} linkSafety={{ enabled: false }}>
      {text}
    </Streamdown>
  );
}

export const md = (text: string) => <Words text={text} />;
