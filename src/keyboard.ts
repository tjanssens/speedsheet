// Keys on the grid (moving, editing, deleting) and the app-wide shortcuts (Ctrl+S, Ctrl+Z, ...).
import { call, isTauri, native } from "./api";
import { app, curSheet, grid } from "./app";
import { clearSelection, copy, dropCopy, paste } from "./clipboard";
import { openFind, cmdPrint } from "./commands";
import { startEdit } from "./editing";
import { cmdNew, cmdOpen, cmdSave, cmdUndo } from "./files";
import { closePopover } from "./popover";
import { cmdToggleFilter } from "./sort-filter";
import { modKey, spawn } from "./ui";

const ARROWS: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };

/** Ctrl+Arrow: jump to the edge of the data, like Excel (the engine knows where the data is). */
async function jump(dr: number, dc: number, extend: boolean) {
  const sheet = app.sheet;
  const moves = grid.moveCount;
  const from = grid.active;
  const to = await call<{ row: number; col: number }>("jump", { sheet, row: from.row, col: from.col, dr, dc });
  // The user may have moved on while the engine answered.
  if (sheet !== app.sheet || grid.moveCount !== moves) return;
  grid.select(to, extend);
}

function handleGridKey(e: KeyboardEvent) {
  if (app.editing) return;
  const mod = modKey(e);
  const k = e.key;
  const s = curSheet();
  const lastDataRow = Math.max(s.rows - 1, 0);
  const lastDataCol = Math.max(s.cols - 1, 0);
  if (k in ARROWS) {
    e.preventDefault();
    const [dr, dc] = ARROWS[k];
    if (mod) spawn(jump(dr, dc, e.shiftKey));
    else grid.move(dr, dc, e.shiftKey);
    return;
  }
  switch (k) {
    case "PageDown":
    case "PageUp": {
      e.preventDefault();
      const n = grid.pageRows() * (k === "PageDown" ? 1 : -1);
      grid.move(n, 0, e.shiftKey);
      return;
    }
    case "Home":
      e.preventDefault();
      grid.select(mod ? { row: 0, col: 0 } : { row: grid.active.row, col: 0 }, e.shiftKey);
      return;
    case "End":
      e.preventDefault();
      grid.select(mod ? { row: lastDataRow, col: lastDataCol } : { row: grid.active.row, col: lastDataCol }, e.shiftKey);
      return;
    case "Enter":
      e.preventDefault();
      grid.advance(e.shiftKey ? -1 : 1, 0);
      return;
    case "Tab":
      e.preventDefault();
      grid.advance(0, e.shiftKey ? -1 : 1);
      return;
    case "F2":
      e.preventDefault();
      startEdit(null, "edit");
      return;
    case "Delete":
    case "Backspace":
      e.preventDefault();
      spawn(clearSelection());
      return;
    case "Escape":
      dropCopy();
      return;
  }
  if (mod) return; // shortcuts are handled globally
  if (k.length === 1 && !e.altKey) {
    e.preventDefault();
    startEdit(k, "enter");
  }
}

function handleShortcut(e: KeyboardEvent) {
  const mod = modKey(e);
  if (!mod) {
    if (e.key === "Escape") closePopover();
    return;
  }
  const k = e.key.toLowerCase();
  const inText = app.editing !== null || (e.target instanceof HTMLInputElement && e.target !== grid.scroller);
  const run = (work: () => Promise<unknown> | void, title?: string) => {
    e.preventDefault();
    const p = work();
    if (p) spawn(p, title);
  };
  switch (k) {
    case "s":
      return run(() => cmdSave(e.shiftKey));
    case "o":
      return run(cmdOpen);
    case "n":
      if (e.shiftKey && isTauri) return run(() => native("new_window"));
      return run(cmdNew);
    case "p":
      return run(cmdPrint, "Can't print");
    case "f":
      return run(openFind);
    case "l":
      if (inText || !e.shiftKey) return;
      return run(cmdToggleFilter, "Can't set the filter");
    case "z":
      if (inText) return;
      return run(() => cmdUndo(e.shiftKey));
    case "y":
      if (inText) return;
      return run(() => cmdUndo(true));
    case "c":
      if (inText) return;
      return run(() => copy(), "Copy failed");
    case "x":
      if (inText) return;
      return run(() => copy(true), "Cut failed");
    case "v":
      if (inText) return;
      return run(paste, "Paste failed");
    case "a":
      if (inText) return;
      return run(() => grid.selectAll());
  }
}

export function setupKeyboard() {
  grid.scroller.addEventListener("keydown", handleGridKey);
  document.addEventListener("keydown", handleShortcut);
}
