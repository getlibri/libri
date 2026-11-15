/** The switch Android draws (Places.kt PaperSwitch): an ink track when on, a
 * lift track ringed in frame when off. Drawing only: the button around it
 * carries role="switch", aria-checked and the tap. */
export function Toggle({ on }: { on: boolean }) {
  return (
    <span aria-hidden className={`relative inline-flex h-6 w-10 shrink-0 items-center rounded-full border-[1.25px] transition-colors duration-150 motion-reduce:transition-none ${on ? "border-ink bg-ink" : "border-frame bg-lift"}`}>
      <span className={`absolute size-4 rounded-full transition-transform duration-150 motion-reduce:transition-none ${on ? "translate-x-[18px] bg-paper" : "translate-x-[3px] bg-soft"}`} />
    </span>
  );
}

/** A switch standing alone at a row's end, its tap target 44 tall. */
export const toggleButton = "-my-2 grid min-h-11 shrink-0 place-items-center disabled:opacity-50";
