/**
 * Entry point (Milestone 14): renders the React page, {@link App}, into
 * `index.html`'s `#root`, themed by `ui/theme.css`.
 *
 * Until Task 12 the page is the empty shell, and its controller is a
 * placeholder: today's `createBadgeController` paints into the old shell's
 * DOM (`ui/shell.ts`), which this page no longer has. Task 12 replaces
 * both with the controller store the panels read, and deletes `main.ts`,
 * `ui/shell.ts` and `ui/shell.css`.
 */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App, type AppController } from "./ui/App";
import "./ui/theme.css";

const root = document.getElementById("root");
if (!root) throw new Error("index.html has no #root");

const controller: AppController = {};

createRoot(root).render(
  <StrictMode>
    <App controller={controller} />
  </StrictMode>,
);
