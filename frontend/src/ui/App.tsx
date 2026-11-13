/**
 * The page (Milestone 14): the header, the stage, the aside and the footer,
 * laid out as the design draws them (local/m14-design/Main.dc.html for
 * desktop, Mobile.dc.html for mobile). The breakpoint between the two is
 * Tailwind's `md` (768 px).
 *
 * - **Desktop** (`md` and up): a wrapping row under the header, the stage
 *   at `flex: 999 1 560px` and 780 px tall, and the aside at
 *   `flex: 1 1 320px`. The page is at most 1360 px wide, with padding
 *   `28px clamp(16px,3vw,40px) 40px`.
 * - **Mobile**: one column with 16 px gutters, a 640 px stage, the aside's
 *   sections stacked under it, and the stacked footer.
 *
 * The stage holds the hanging badge ({@link Stage}, Task 11; Task 12
 * wires its client, sink and lanyard). The aside is an empty slot for
 * now: FIRMWARE TOOLS and CONSOLE.LOG (Tasks 8 and 12) go in it.
 */

import { Stage } from "./Stage";

/**
 * The page's controller. A placeholder until Task 12's controller store,
 * which the panels will read; the shell reads nothing from it yet.
 */
export type AppController = object;

export interface AppProps {
  controller: AppController;
}

export function App(_props: AppProps) {
  return (
    <div className="box-border flex min-h-screen flex-col bg-background px-4 pt-5 pb-7 font-sans text-foreground md:px-[clamp(16px,3vw,40px)] md:pt-7 md:pb-10">
      <div className="mx-auto flex w-full max-w-[1360px] flex-auto flex-col gap-4 md:gap-6">
        <Header />
        <div className="flex flex-col gap-4 md:flex-row md:flex-wrap md:items-stretch md:gap-6">
          <Stage />
          <aside className="flex min-w-0 flex-col gap-4 md:flex-[1_1_320px]" />
        </div>
        <Footer />
      </div>
    </div>
  );
}

function Header() {
  return (
    <header className="flex flex-col gap-3.5 md:flex-row md:flex-wrap md:items-end md:justify-between md:gap-4 md:border-b md:border-border md:pb-5">
      <div className="flex flex-col gap-1.5 md:gap-2">
        <span className="font-mono text-[10px] leading-none font-semibold tracking-[.14em] text-accent-text md:text-[11px]">
          HACK THE NORTH 2026
        </span>
        <h1 className="m-0 text-[30px] leading-none font-semibold tracking-[-0.02em] md:text-[clamp(30px,4vw,46px)]">
          Badge Emulator<span className="text-accent-strong">.</span>
        </h1>
      </div>
    </header>
  );
}

function Footer() {
  return (
    <footer className="flex flex-col gap-2.5 border-t border-border pt-4 font-mono text-xs leading-[1.6] font-normal text-muted-foreground md:mt-auto md:flex-row md:flex-wrap md:justify-between md:gap-x-6 md:gap-y-2.5 md:pt-5">
      <div className="flex flex-col gap-1">
        <span>
          Copyright © 2026 Mohan Dong ·{" "}
          <a href="https://www.mohandong.com" target="_blank" rel="noopener">
            www.mohandong.com
          </a>
        </span>
        <span>Unofficial project, not affiliated with or endorsed by Hack the North.</span>
      </div>
      <div className="flex flex-wrap items-center gap-x-[18px] gap-y-1.5 md:self-end">
        <a
          href="https://github.com/powersagitar/hack-the-badge"
          target="_blank"
          rel="noopener"
          className="inline-flex items-center gap-1.5"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M9 19c-4.3 1.4-4.3-2.5-6-3m12 5v-3.5c0-1 .1-1.4-.5-2 2.8-.3 5.5-1.4 5.5-6a4.6 4.6 0 0 0-1.3-3.2 4.2 4.2 0 0 0-.1-3.2s-1.1-.3-3.5 1.3a12.3 12.3 0 0 0-6.2 0C6.5 2.8 5.4 3.1 5.4 3.1a4.2 4.2 0 0 0-.1 3.2A4.6 4.6 0 0 0 4 9.5c0 4.6 2.7 5.7 5.5 6-.6.6-.6 1.2-.5 2V21" />
          </svg>
          Source on GitHub
        </a>
        <span>
          Licensed under{" "}
          <a href="https://www.gnu.org/licenses/gpl-3.0.html" target="_blank" rel="noopener">
            GPL-3.0
          </a>
        </span>
      </div>
    </footer>
  );
}
