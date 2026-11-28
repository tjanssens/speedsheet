// The toolbar, the context menu, find, print and the theme menu, plus what the grid reports
// back (resized columns, the fill handle).
import { call, errorText, isTauri, native, type BookInfo, type EditResult, type Match } from "./api";
import { app, curSheet, grid, selectionClipped } from "./app";
import { afterEdit, applyInfo, markModified, switchSheet } from "./book";
import { clearSelection, copy, paste } from "./clipboard";
import { commitEdit } from "./editing";
import { openFeedback } from "./feedback";
import { cmdNew, cmdOpen, cmdSave, cmdUndo } from "./files";
import type { Rect } from "./grid";
import { openMenu } from "./popover";
import { clearFilters, cmdFilter, cmdSort, cmdToggleFilter, openFilterFor } from "./sort-filter";
import { checkForUpdates } from "./updates";
import { $, busy, confirmDialog, fmtNum, isMac, spawn } from "./ui";

// ---------------------------------------------------------------- find

const find = {
  bar: $("#find-bar"),
  input: $<HTMLInputElement>("#find-input"),
  status: $("#find-status"),
};

export function openFind() {
  spawn(commitEdit(0, 0));
  find.bar.hidden = false;
  find.input.focus();
  find.input.select();
}

function closeFind() {
  find.bar.hidden = true;
  grid.scroller.focus();
}

async function findNext(backwards = false) {
  const query = find.input.value;
  if (!query) return;
  const all = $<HTMLInputElement>("#find-all").checked;
  const { row, col } = grid.active;
  try {
    const m = await call<Match | null>("find", { query, sheet: app.sheet, row, col, backwards, allSheets: all });
    if (!m) {
      find.status.textContent = "Not found";
      return;
    }
    find.status.textContent = m.sheet !== app.sheet ? app.book.sheets[m.sheet].name : "";
    if (m.sheet !== app.sheet) switchSheet(m.sheet);
    grid.select({ row: m.view_row, col: m.col });
  } catch (e) {
    find.status.textContent = errorText(e);
  }
}

// ---------------------------------------------------------------- print

export async function cmdPrint() {
  await commitEdit(0, 0);
  const s = curSheet();
  const sel = grid.selection;
  const multi = sel.r0 !== sel.r1 || sel.c0 !== sel.c1;
  const r: Rect = multi ? selectionClipped() : { r0: 0, c0: 0, r1: Math.max(s.rows - 1, 0), c1: Math.max(s.cols - 1, 0) };
  const doc = await busy("Preparing print…", () =>
    call<{ html: string; rows: number; truncated: boolean }>("print_html", { sheet: app.sheet, ...r }),
  );
  if (doc.truncated && !(await confirmDialog(`Only the first ${fmtNum(doc.rows)} rows will be printed. Continue?`, "Large sheet", "Print"))) return;
  $("#print-root").innerHTML = doc.html;
  if (isTauri) await native("print_page");
  else window.print();
}

// ---------------------------------------------------------------- appearance

type ThemeChoice = "system" | "light" | "dark";
const prefs = {
  get theme(): ThemeChoice {
    try {
      return (localStorage.getItem("theme") as ThemeChoice) || "system";
    } catch {
      return "system";
    }
  },
  get lightSheet(): boolean {
    try {
      return localStorage.getItem("lightSheet") === "1";
    } catch {
      return false;
    }
  },
  save(key: string, value: string) {
    try {
      localStorage.setItem(key, value);
    } catch {
      /* the choice only lasts this session */
    }
  },
};

export function applyTheme() {
  const t = prefs.theme;
  if (t === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = t;
  grid.host.classList.toggle("light-sheet", prefs.lightSheet);
  grid.themeChanged();
}

function cmdTheme() {
  const btn = document.querySelector<HTMLElement>('[data-cmd="theme"]')!.getBoundingClientRect();
  const mark = (on: boolean) => (on ? "✓ " : "  ");
  const t = prefs.theme;
  const set = (v: ThemeChoice) => {
    prefs.save("theme", v);
    applyTheme();
  };
  openMenu(btn.right - 220, btn.bottom + 4, [
    { label: `${mark(t === "system")}Follow system`, action: () => set("system") },
    { label: `${mark(t === "light")}Light`, action: () => set("light") },
    { label: `${mark(t === "dark")}Dark`, action: () => set("dark") },
    { separator: true },
    {
      label: `${mark(prefs.lightSheet)}Light sheet`,
      hint: "cells stay white",
      action: () => {
        prefs.save("lightSheet", prefs.lightSheet ? "0" : "1");
        applyTheme();
      },
    },
  ]);
}

// ---------------------------------------------------------------- toolbar and grid hooks

/** Every toolbar button (`data-cmd` in index.html). features.md lists them; a test checks that. */
export const commands: Record<string, () => void> = {
  new: () => spawn(cmdNew()),
  open: () => spawn(cmdOpen()),
  save: () => spawn(cmdSave()),
  saveAs: () => spawn(cmdSave(true)),
  undo: () => spawn(cmdUndo()),
  redo: () => spawn(cmdUndo(true)),
  sortAsc: () => spawn(cmdSort(true), "Can't sort"),
  sortDesc: () => spawn(cmdSort(false), "Can't sort"),
  filter: () => spawn(cmdToggleFilter(), "Can't set the filter"),
  clearFilters: () => spawn(clearFilters()),
  find: openFind,
  print: () => spawn(cmdPrint(), "Can't print"),
  feedback: () => openFeedback(app.appInfo),
  updates: () => spawn(checkForUpdates(app.appInfo, true)),
  theme: cmdTheme,
};

async function addSheet() {
  const info = await call<BookInfo>("add_sheet");
  applyInfo(info, info.sheets.length - 1);
}

export function setupCommands() {
  document.querySelectorAll<HTMLButtonElement>("[data-cmd]").forEach((b) => {
    b.addEventListener("mousedown", (e) => e.preventDefault()); // keep focus (and edits) where they are
    b.addEventListener("click", () => commands[b.dataset.cmd!]?.());
  });
  $("#add-sheet").onclick = () => spawn(addSheet());

  find.input.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      spawn(findNext(e.shiftKey));
    } else if (e.key === "Escape") closeFind();
  });
  find.input.addEventListener("input", () => (find.status.textContent = ""));
  $("#find-next").onclick = () => spawn(findNext(false));
  $("#find-prev").onclick = () => spawn(findNext(true));
  $("#find-close").onclick = closeFind;

  grid.onFilterButton = (col, x, y) => spawn(openFilterFor(col, x, y));
  grid.onColResize = (col, width) => {
    // Keep the local copy in step, so the next refresh doesn't snap the column back.
    const widths = curSheet().colWidths;
    while (widths.length <= col) widths.push(null);
    widths[col] = width;
    call("set_col_width", { sheet: app.sheet, col, width }).catch(() => {});
    markModified();
  };
  grid.onRowResize = (row, height) => {
    call("set_row_height", { sheet: app.sheet, row, height }).catch(() => {});
    markModified();
  };
  grid.onFill = (src, dst) => spawn(fill(src, dst), "Can't fill");
  grid.onFillAuto = (src) => spawn(fillAuto(src), "Can't fill");
  grid.onContextMenu = (x, y) => {
    openMenu(x, y, [
      { label: "Cut", hint: isMac ? "⌘X" : "Ctrl+X", action: () => spawn(copy(true), "Cut failed") },
      { label: "Copy", hint: isMac ? "⌘C" : "Ctrl+C", action: () => spawn(copy(), "Copy failed") },
      { label: "Paste", hint: isMac ? "⌘V" : "Ctrl+V", action: () => spawn(paste(), "Paste failed") },
      { label: "Clear contents", hint: "Del", action: () => spawn(clearSelection()) },
      { separator: true },
      { label: "Sort A → Z", action: () => spawn(cmdSort(true), "Can't sort") },
      { label: "Sort Z → A", action: () => spawn(cmdSort(false), "Can't sort") },
      { label: "Filter this column…", action: () => cmdFilter() },
    ]);
  };
}

async function fill(src: Rect, dst: Rect) {
  await commitEdit(0, 0);
  const res = await call<EditResult>("fill", { sheet: app.sheet, src, dst });
  afterEdit(res, false);
  grid.select({ row: dst.r0, col: dst.c0 }, false, { row: dst.r1, col: dst.c1 });
}

async function fillAuto(src: Rect) {
  await commitEdit(0, 0);
  const res = await call<{ end: number; result: EditResult } | null>("fill_auto", { sheet: app.sheet, ...src });
  if (!res) return;
  afterEdit(res.result, false);
  grid.select({ row: src.r0, col: src.c0 }, false, { row: res.end, col: src.c1 });
}
