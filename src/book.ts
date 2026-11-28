// Showing the workbook: sheet tabs, title, toolbar state, the name box, formula bar and status
// bar. `applyInfo` brings all of it in line with a BookInfo from the engine.
import { call, isTauri, type BookInfo, type EditResult, type Layout, type Stats, type Styles } from "./api";
import { app, curSheet, dom, grid } from "./app";
import { commitEdit } from "./editing";
import { colName } from "./grid";
import { sendPresence } from "./peers";
import { openMenu } from "./popover";
import { Debounced } from "./schedule";
import { confirmDialog, fmtNum, showError, spawn } from "./ui";

export async function loadStyles() {
  if (app.stylesGen === app.bookGen) return;
  const gen = app.bookGen;
  try {
    const st = await call<Styles>("get_styles");
    if (gen !== app.bookGen) return;
    app.stylesGen = gen;
    grid.setStyles(st.styles, st.dxfs);
  } catch {
    /* the grid keeps the default look */
  }
}

export async function loadLayout(force = false) {
  const s = curSheet();
  const key = `${app.bookGen}:${app.sheet}:${s.rows}:${JSON.stringify(s.filter)}`;
  if (!force && key === app.layoutKey) return;
  app.layoutKey = key;
  try {
    const l = await call<Layout>("get_layout", { sheet: app.sheet });
    if (key !== app.layoutKey) return;
    grid.setLayout(l);
  } catch {
    grid.setLayout(null);
  }
}

export function applyInfo(info: BookInfo, focusSheet = app.sheet) {
  app.book = info;
  app.sheet = Math.max(0, Math.min(focusSheet, info.sheets.length - 1));
  const s = curSheet();
  const filterCols = s.filter ? s.filter.conds.map((c) => c.col) : null;
  grid.showFilterButtons = !!s.filter || grid.showFilterButtons;
  // Another sheet or file: drop the old layout right away instead of showing it on the new sheet.
  if (!app.layoutKey.startsWith(`${app.bookGen}:${app.sheet}:`)) grid.setLayout(null);
  grid.setSheet(app.sheet, s.rows, s.cols, s.colWidths, filterCols);
  spawn(loadStyles());
  spawn(loadLayout());
  renderTabs();
  spawn(updateTitle());
  updateButtons();
  spawn(updateStatus());
  updateCellUI();
}

/** Apply an edit result: new dimensions, invalidate visible cells, optionally move the cursor. */
export function afterEdit(res: EditResult, moveCursor: boolean) {
  applyInfo(res.info, res.sheet);
  if (moveCursor && res.row !== null) grid.select({ row: res.row, col: res.col });
  grid.invalidate();
}

export async function updateTitle() {
  // Like Excel: "Book.xlsx:2" when the file is open in more than one window.
  const book = app.book;
  const name = app.peers.count > 1 ? `${book.name}:${app.peers.number}` : book.name;
  const title = `${name}${book.modified ? " •" : ""} — SpeedSheet`;
  document.title = title;
  if (isTauri) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow()
      .setTitle(title)
      .catch(() => {});
  }
}

/** The user changed something the engine does not report back (a column width): mark it modified. */
export function markModified() {
  if (app.book.modified) return;
  app.book.modified = true;
  spawn(updateTitle());
}

function updateButtons() {
  const btn = (c: string) => document.querySelector<HTMLButtonElement>(`[data-cmd="${c}"]`)!;
  btn("undo").disabled = !app.book.canUndo;
  btn("redo").disabled = !app.book.canRedo;
  btn("clearFilters").disabled = !app.book.sheets.some((s) => s.filter);
  btn("filter").classList.toggle("bordered", !!curSheet().filter);
}

export function renderTabs() {
  const tabs = dom.tabs;
  tabs.innerHTML = "";
  app.book.sheets.forEach((s, i) => {
    const b = document.createElement("button");
    b.className = "tab" + (i === app.sheet ? " active" : "");
    b.setAttribute("role", "tab");
    b.setAttribute("aria-selected", String(i === app.sheet));
    b.textContent = s.name + (s.filter ? " ⏷" : "");
    b.title = s.name;
    b.onclick = () => switchSheet(i);
    b.ondblclick = () => renameTab(i, b);
    b.oncontextmenu = (e) => {
      e.preventDefault();
      switchSheet(i);
      openMenu(e.clientX, e.clientY - 70, [
        { label: "Rename", action: () => renameTab(i, tabs.children[i] as HTMLElement) },
        { label: "Delete", action: () => spawn(deleteSheet(i)), disabled: app.book.sheets.length < 2 },
      ]);
    };
    tabs.appendChild(b);
  });
  (tabs.children[app.sheet] as HTMLElement | undefined)?.scrollIntoView({ block: "nearest", inline: "nearest" });
}

export function switchSheet(i: number) {
  if (i === app.sheet) return;
  spawn(commitEdit(0, 0));
  app.sheet = i;
  grid.showFilterButtons = !!app.book.sheets[i].filter;
  applyInfo(app.book, i);
  sendPresence();
}

function renameTab(i: number, el: HTMLElement) {
  const input = document.createElement("input");
  input.value = app.book.sheets[i].name;
  el.textContent = "";
  el.appendChild(input);
  input.focus();
  input.select();
  const state = { done: false };
  const finish = async (save: boolean) => {
    if (state.done) return;
    state.done = true;
    const name = input.value.trim();
    if (save && name && name !== app.book.sheets[i].name) {
      try {
        applyInfo(await call<BookInfo>("rename_sheet", { sheet: i, name }));
      } catch (e) {
        renderTabs();
        await showError(e, "Can't rename sheet");
      }
    } else renderTabs();
    grid.scroller.focus();
  };
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (e.key === "Enter") spawn(finish(true));
    if (e.key === "Escape") spawn(finish(false));
  };
  input.onblur = () => spawn(finish(true));
}

async function deleteSheet(i: number) {
  if (!(await confirmDialog(`Delete sheet "${app.book.sheets[i].name}"? You can undo this.`, "Delete sheet", "Delete"))) return;
  applyInfo(await call<BookInfo>("delete_sheet", { sheet: i }), Math.max(0, i - 1));
}

// ---------------------------------------------------------------- name box, formula bar, status

/** Fetch the active cell's input for the formula bar once the cursor rests (not on every step). */
const refreshFormulaBar = new Debounced(16, () => spawn(showActiveInput()));

async function showActiveInput() {
  const s = app.sheet;
  const { row, col } = grid.active;
  try {
    const text = await call<string>("get_input", { sheet: s, row, col });
    if (!app.editing && s === app.sheet && row === grid.active.row && col === grid.active.col) dom.formulaInput.value = text;
  } catch {
    /* the bar keeps the previous text */
  }
  await updateStatus();
}

/** `resetNameBox`: also overwrite the name box while the user is typing an address in it. */
export function updateCellUI(resetNameBox = false) {
  const a = grid.active;
  const sel = grid.selection;
  const dataRow = grid.dataRowOf(a.row);
  const multi = sel.r0 !== sel.r1 || sel.c0 !== sel.c1;
  // An edit that finishes in the background must not overwrite an address being typed.
  if (resetNameBox || document.activeElement !== dom.nameBox)
    dom.nameBox.value = multi ? `${sel.r1 - sel.r0 + 1}R × ${sel.c1 - sel.c0 + 1}C` : `${colName(a.col)}${dataRow !== null ? dataRow + 1 : ""}`;
  if (app.editing) return;
  refreshFormulaBar.run();
}

export async function updateStatus() {
  const s = curSheet();
  const parts: string[] = [];
  const sel = grid.selection;
  const multi = sel.r0 !== sel.r1 || sel.c0 !== sel.c1;
  if (multi) {
    const seq = ++app.statsSeq;
    try {
      const r1 = Math.min(sel.r1, Math.max(s.rows - 1, 0));
      const c1 = Math.min(sel.c1, Math.max(s.cols - 1, 0));
      const st = await call<Stats>("range_stats", { sheet: app.sheet, r0: sel.r0, c0: sel.c0, r1, c1 });
      if (seq !== app.statsSeq) return;
      if (st.numbers > 0) {
        parts.push(`Sum <b>${fmtNum(st.sum)}</b>`);
        parts.push(`Average <b>${fmtNum(st.sum / st.numbers)}</b>`);
        if (st.min !== null && st.max !== null) parts.push(`Min <b>${fmtNum(st.min)}</b>`, `Max <b>${fmtNum(st.max)}</b>`);
      }
      parts.push(`Count <b>${fmtNum(st.count)}</b>`);
    } catch {
      /* no statistics, rows only */
    }
  }
  const rows = s.filter
    ? `${fmtNum(Math.max(s.rows - (s.header ? 1 : 0), 0))} of ${fmtNum(Math.max(s.dataRows - (s.header ? 1 : 0), 0))} rows`
    : `${fmtNum(s.dataRows)} rows`;
  parts.push(rows);
  dom.status.innerHTML = parts.join(" &nbsp;·&nbsp; ");
}
