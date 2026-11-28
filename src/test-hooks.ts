// What the browser tests read to check the UI's invariants (e2e/ui-fuzz.spec.ts): the state
// behind the canvas, which a test cannot see in the DOM. Only in the browser dev build.
import { call, callsInFlight, isTauri } from "./api";
import { app, curSheet, dom, grid } from "./app";

export interface UiState {
  sheet: number;
  sheets: number;
  active: { row: number; col: number };
  anchor: { row: number; col: number };
  selection: { r0: number; c0: number; r1: number; c1: number };
  rowCount: number;
  colCount: number;
  viewRows: number;
  dataCols: number;
  dragging: boolean;
  editing: null | { mode: string; where: string };
  editorVisible: boolean;
  /** The active cell as the grid shows it (null while its block is still loading). */
  shownText: string | null;
  pendingDraw: boolean;
  /** Engine calls not answered yet: the UI is settled when this is 0 and no draw is pending. */
  callsInFlight: number;
  /** id (or tag) of the focused element. */
  focus: string;
}

export function uiState(): UiState {
  const a = grid.active;
  const cell = grid.cellAt(a.row, a.col);
  return {
    sheet: app.sheet,
    sheets: app.book.sheets.length,
    active: { ...a },
    anchor: { ...grid.anchor },
    selection: grid.selection,
    rowCount: grid.rowCount,
    colCount: grid.colCount,
    viewRows: curSheet().rows,
    dataCols: curSheet().cols,
    dragging: grid.mouse.dragging,
    editing: app.editing && { mode: app.editing.mode, where: app.editing.where },
    editorVisible: !dom.editor.hidden,
    shownText: cell ? cell.text : null,
    pendingDraw: grid.pendingDraw,
    callsInFlight: callsInFlight(),
    focus: document.activeElement?.id || document.activeElement?.tagName || "",
  };
}

export function exposeTestHooks() {
  if (isTauri) return;
  // `call` goes to the engine as this window, so a test can compare the grid with the engine.
  const hooks = { state: uiState, call: (cmd: string, args: Record<string, unknown>) => call<unknown>(cmd, args) };
  (window as unknown as { __speedsheet: typeof hooks }).__speedsheet = hooks;
}
