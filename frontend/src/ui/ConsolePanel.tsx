/**
 * The CONSOLE.LOG panel (Milestone 14), as the design draws it
 * (local/m14-design/Main.dc.html and Mobile.dc.html): a bordered section
 * with the mono "CONSOLE.LOG" header and a scrolling mono body, at most
 * 360 px tall on desktop and 180 px tall on mobile.
 *
 * - It reads a `ConsoleLog` through `useSyncExternalStore`. The model
 *   notifies once per append call and hands out one frozen snapshot per
 *   change, and React batches the notifications of one task into one
 *   render. Rows are memoised on their entry, which never changes once
 *   made, so a render only creates the new rows; the model's line cap
 *   bounds the rest.
 * - Newest at the bottom (the human partner accepted this over the
 *   design's newest-first mock). The body follows the tail unless the
 *   visitor has scrolled up more than `TAIL_SLACK_PX`.
 * - `page` and `event` entries are muted, with a `[page]` / `[event]`
 *   prefix.
 * - `footer` is the seam for a later console input row; it renders under
 *   the body and is empty today.
 */
import { memo, useLayoutEffect, useRef, useSyncExternalStore, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import type { ConsoleEntry, ConsoleLog } from "./console-log";

/** How far above the bottom the visitor may be and still follow the tail. */
export const TAIL_SLACK_PX = 24;

/** Whether a scroll position is within `TAIL_SLACK_PX` of the bottom. */
export function nearTail(scrollHeight: number, scrollTop: number, clientHeight: number): boolean {
  return scrollHeight - scrollTop - clientHeight <= TAIL_SLACK_PX;
}

export interface ConsolePanelProps {
  log: ConsoleLog;
  /** A row under the log, for a future console input. */
  footer?: ReactNode;
}

export function ConsolePanel({ log, footer }: ConsolePanelProps) {
  const entries = useSyncExternalStore(log.subscribe, log.getSnapshot, log.getSnapshot);
  const body = useRef<HTMLDivElement>(null);
  const following = useRef(true);

  useLayoutEffect(() => {
    const el = body.current;
    if (el && following.current) el.scrollTop = el.scrollHeight;
  }, [entries]);

  return (
    <section className="flex flex-col overflow-hidden rounded-panel-mobile border border-border bg-card md:min-h-[220px] md:flex-auto md:rounded-panel">
      <div className="border-b border-border px-4 py-3 font-mono text-[10px] leading-none font-semibold tracking-[.14em] text-accent-text md:px-[18px] md:text-[11px]">
        CONSOLE.LOG
      </div>
      <div
        ref={body}
        role="log"
        aria-label="Badge console"
        onScroll={(e) => {
          const el = e.currentTarget;
          following.current = nearTail(el.scrollHeight, el.scrollTop, el.clientHeight);
        }}
        className="flex h-[180px] flex-col gap-1 overflow-y-auto px-4 py-2.5 font-mono text-xs leading-[1.5] text-secondary-foreground md:h-auto md:max-h-[360px] md:flex-auto md:px-[18px] md:py-3 md:text-[12.5px]"
      >
        {entries.map((entry) => (
          <ConsoleRow key={entry.id} entry={entry} />
        ))}
      </div>
      {footer}
    </section>
  );
}

const ConsoleRow = memo(function ConsoleRow({ entry }: { entry: ConsoleEntry }) {
  const muted = entry.source !== "serial";
  return (
    <div
      data-source={entry.source}
      className={cn("min-h-[1.5em] shrink-0 break-words whitespace-pre-wrap", muted && "text-muted-foreground")}
    >
      {muted ? `[${entry.source}] ${entry.text}` : entry.text}
    </div>
  );
});
