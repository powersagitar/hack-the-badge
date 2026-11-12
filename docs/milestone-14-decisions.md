# Milestone 14 decisions

## Dependencies added

All pinned exactly in `frontend/package.json`.

| package | exact version | why |
|---|---|---|
| `react` | 19.3.1 | The page's UI (Milestone 14 replaces `ui/shell.ts`'s DOM building with components). |
| `react-dom` | 19.3.1 | Renders the React page into `#root`. |
| `matter-js` | 0.20.0 | The lanyard and badge physics (the design's swing). |
| `radix-ui` | 1.8.1 | Primitives under shadcn's Button (`Slot`), Select and ScrollArea; added by the shadcn CLI. |
| `class-variance-authority` | 0.7.1 | shadcn's Button variants (`cva`). |
| `clsx` | 2.1.1 | shadcn's `cn` (`src/lib/utils.ts`). |
| `tailwind-merge` | 3.7.0 | shadcn's `cn` (`src/lib/utils.ts`). |
| `lucide-react` | 1.72.0 | The Select's chevron and check icons. |
| `tailwindcss` (dev) | 4.3.5 | The styling and theme tokens. |
| `@tailwindcss/vite` (dev) | 4.3.5 | Tailwind v4's Vite plugin. |
| `@vitejs/plugin-react` (dev) | 6.3.0 | JSX/React in Vite 8. |
| `@types/react` (dev) | 19.3.0 | React's types. |
| `@types/react-dom` (dev) | 19.3.0 | react-dom's types. |
| `@types/matter-js` (dev) | 0.20.2 | matter.js's types. |
| `@testing-library/react` (dev) | 16.3.3 | Component tests over happy-dom. |
| `@testing-library/dom` (dev) | 10.4.2 | `@testing-library/react`'s required peer. |

The shadcn components came from the CLI, `bunx --bun shadcn@4.23.0 add
button select input scroll-area` (not a dependency). Its current
`new-york-v4` registry imports `cn` from shadcn's new npm package `cn`
(0.5.2, published 2026-11-02) and lists it as a dependency. We kept the
classic `src/lib/utils.ts` (`clsx` + `tailwind-merge`) instead, as the
plan asks, rewrote the four components' import to `@/lib/utils`, and did
not add `cn`. The CLI also resolves the `@` alias only with a tsconfig
`baseUrl`, which TypeScript 7 removed (TS5102), so `baseUrl` was present
only while the CLI ran; `paths` alone serves `tsc`, Bun and Vite (whose
`@` alias is in `vite.config.ts`).

## Build size (Task 7)

`bun run build`, `frontend/dist/`:

- Before (main.ts page): JS 27.40 kB (gzip 10.31 kB) + bridge 6.24 kB +
  worker 10.81 kB, CSS 2.71 kB, wasm 143.64 kB; `dist/` 3380 KiB.
- After (the empty React shell): JS 222.71 kB (gzip 69.81 kB), CSS
  22.03 kB (gzip 5.12 kB); `dist/` 3428 KiB.

Until Task 12 wires the controller, the page does not reference the
firmware Worker, so the worker, bridge and wasm chunks are absent from
the build; they return then.
