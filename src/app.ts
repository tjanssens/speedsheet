// The window's state, in one place. Modules read and change `app`; nothing else keeps state at
// module level (ESLint forbids top-level `let`), so what the window knows is visible here.
import type { AppInfo, BookInfo, Peers } from "./api";
import { Grid, type Rect } from "./grid";
import { $ } from "./ui";

export interface Editing {
  /** "enter": typing replaced the cell, arrows commit. "edit": F2/double click, arrows move the caret. */
  mode: "enter" | "edit";
  where: "cell" | "bar";
  original: string;
}

export interface LastCopy {
  text: string;
  row: number;
  col: number;
  inputs: string[][];
  cut: boolean;
  sheet: number;
  rect: Rect;
}

export const grid = new Grid($("#grid-host"));

export const dom = {
  nameBox: $<HTMLInputElement>("#name-box"),
  formulaInput: $<HTMLInputElement>("#formula-input"),
  editor: $<HTMLTextAreaElement>("#cell-editor"),
  status: $("#status"),
  tabs: $("#tabs"),
};

export const app = {
  /** Set by init() before anything else runs. */
  book: null as unknown as BookInfo,
  sheet: 0,
  editing: null as Editing | null,
  lastCopy: null as LastCopy | null,
  appInfo: null as AppInfo | null,
  /** Other windows on the same file. */
  peers: { number: 1, count: 1, peers: [] } as Peers,
  /** Bumped whenever a different workbook is loaded, so styles and layouts are fetched again. */
  bookGen: 0,
  stylesGen: -1,
  layoutKey: "",
  /** Sequence number of the latest status-bar statistics request (older answers are dropped). */
  statsSeq: 0,
};

export function curSheet() {
  return app.book.sheets[app.sheet];
}

/** The selection, cut to the used range of the sheet. */
export function selectionClipped(): Rect {
  const s = grid.selection;
  const cs = curSheet();
  return {
    r0: s.r0,
    c0: s.c0,
    r1: Math.max(s.r0, Math.min(s.r1, Math.max(cs.rows - 1, 0))),
    c1: Math.max(s.c0, Math.min(s.c1, Math.max(cs.cols - 1, 0))),
  };
}
