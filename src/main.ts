import { call, native, isTauri, errorText } from "./api";
import type { AppInfo, BookInfo, Copied, EditResult, Filter, FilterCond, Match, Stats } from "./api";
import { Grid, colName, parseAddress, type Rect } from "./grid";
import { openFilterPopover, closePopover, openMenu } from "./popover";
import { openFeedback } from "./feedback";
import { checkForUpdates } from "./updates";

const $ = <T extends HTMLElement = HTMLElement>(sel: string) => document.querySelector(sel) as T;

const grid = new Grid($("#grid-host"));
const nameBox = $<HTMLInputElement>("#name-box");
const formulaInput = $<HTMLInputElement>("#formula-input");
const editor = $<HTMLTextAreaElement>("#cell-editor");
const statusEl = $("#status");
const tabsEl = $("#tabs");

let book: BookInfo;
let sheet = 0;
/** "enter": typing replaced the cell, arrows commit. "edit": F2/double click, arrows move the caret. */
let editing: null | { mode: "enter" | "edit"; where: "cell" | "bar"; original: string } = null;
let lastCopy: null | { text: string; row: number; col: number; inputs: string[][]; cut: boolean; sheet: number; rect: Rect } = null;
let appInfo: AppInfo | null = null;

// ---------------------------------------------------------------- helpers

const isMac = navigator.platform.toUpperCase().includes("MAC");
const modKey = (e: KeyboardEvent | MouseEvent) => (isMac ? e.metaKey : e.ctrlKey);

function toast(html: string, ms = 3500) {
  const t = $("#toast");
  t.innerHTML = html;
  t.hidden = false;
  clearTimeout((toast as unknown as { timer: number }).timer);
  (toast as unknown as { timer: number }).timer = window.setTimeout(() => (t.hidden = true), ms);
}

function escapeHtml(s: string) {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
}

async function busy<T>(text: string, fn: () => Promise<T>): Promise<T> {
  const el = $("#busy");
  const timer = window.setTimeout(() => {
    $("#busy-text").textContent = text;
    el.hidden = false;
  }, 120);
  try {
    return await fn();
  } finally {
    clearTimeout(timer);
    el.hidden = true;
  }
}

async function showError(e: unknown, title = "Something went wrong") {
  const msg = errorText(e);
  if (isTauri) {
    const { message } = await import("@tauri-apps/plugin-dialog");
    await message(msg, { title, kind: "error" });
  } else {
    alert(`${title}\n\n${msg}`);
  }
}

async function confirmDialog(msg: string, title: string, okLabel = "OK"): Promise<boolean> {
  if (isTauri) {
    const { ask } = await import("@tauri-apps/plugin-dialog");
    return ask(msg, { title, kind: "warning", okLabel, cancelLabel: "Cancel" });
  }
  return confirm(msg);
}

function fmtNum(n: number) {
  return n.toLocaleString(undefined, { maximumFractionDigits: 6 });
}

// ---------------------------------------------------------------- book state

function curSheet() {
  return book.sheets[sheet];
}

function applyInfo(info: BookInfo, focusSheet = sheet) {
  book = info;
  sheet = Math.max(0, Math.min(focusSheet, info.sheets.length - 1));
  const s = curSheet();
  const filterCols = s.filter ? s.filter.conds.map((c) => c.col) : null;
  grid.showFilterButtons = !!s.filter || grid.showFilterButtons;
  grid.setSheet(sheet, s.rows, s.cols, s.colWidths, filterCols);
  renderTabs();
  updateTitle();
  updateButtons();
  updateStatus();
  updateCellUI();
}

async function updateTitle() {
  const title = `${book.name}${book.modified ? " •" : ""} — SpeedSheet`;
  document.title = title;
  if (isTauri) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().setTitle(title).catch(() => {});
  }
}

function updateButtons() {
  const btn = (c: string) => document.querySelector<HTMLButtonElement>(`[data-cmd="${c}"]`)!;
  btn("undo").disabled = !book.canUndo;
  btn("redo").disabled = !book.canRedo;
  btn("clearFilters").disabled = !book.sheets.some((s) => s.filter);
  btn("filter").classList.toggle("bordered", !!curSheet().filter);
}

function renderTabs() {
  tabsEl.innerHTML = "";
  book.sheets.forEach((s, i) => {
    const b = document.createElement("button");
    b.className = "tab" + (i === sheet ? " active" : "");
    b.setAttribute("role", "tab");
    b.setAttribute("aria-selected", String(i === sheet));
    b.textContent = s.name + (s.filter ? " ⏷" : "");
    b.title = s.name;
    b.onclick = () => switchSheet(i);
    b.ondblclick = () => renameTab(i, b);
    b.oncontextmenu = (e) => {
      e.preventDefault();
      switchSheet(i);
      openMenu(e.clientX, e.clientY - 70, [
        { label: "Rename", action: () => renameTab(i, tabsEl.children[i] as HTMLElement) },
        { label: "Delete", action: () => deleteSheet(i), disabled: book.sheets.length < 2 },
      ]);
    };
    tabsEl.appendChild(b);
  });
  (tabsEl.children[sheet] as HTMLElement | undefined)?.scrollIntoView({ block: "nearest", inline: "nearest" });
}

function switchSheet(i: number) {
  if (i === sheet) return;
  commitEdit(0, 0);
  sheet = i;
  grid.showFilterButtons = !!book.sheets[i].filter;
  applyInfo(book, i);
}

function renameTab(i: number, el: HTMLElement) {
  const input = document.createElement("input");
  input.value = book.sheets[i].name;
  el.textContent = "";
  el.appendChild(input);
  input.focus();
  input.select();
  let done = false;
  const finish = async (save: boolean) => {
    if (done) return;
    done = true;
    const name = input.value.trim();
    if (save && name && name !== book.sheets[i].name) {
      try {
        applyInfo(await call<BookInfo>("rename_sheet", { sheet: i, name }));
      } catch (e) {
        showError(e, "Can't rename sheet");
        renderTabs();
      }
    } else renderTabs();
    grid.scroller.focus();
  };
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (e.key === "Enter") finish(true);
    if (e.key === "Escape") finish(false);
  };
  input.onblur = () => finish(true);
}

async function deleteSheet(i: number) {
  if (!(await confirmDialog(`Delete sheet "${book.sheets[i].name}"? You can undo this.`, "Delete sheet", "Delete"))) return;
  try {
    applyInfo(await call<BookInfo>("delete_sheet", { sheet: i }), Math.max(0, i - 1));
  } catch (e) {
    showError(e);
  }
}

// ---------------------------------------------------------------- selection UI

let cellUiTimer = 0;
function updateCellUI() {
  const a = grid.active;
  const sel = grid.selection;
  const dataRow = grid.dataRowOf(a.row);
  const multi = sel.r0 !== sel.r1 || sel.c0 !== sel.c1;
  nameBox.value = multi
    ? `${sel.r1 - sel.r0 + 1}R × ${sel.c1 - sel.c0 + 1}C`
    : `${colName(a.col)}${dataRow !== null ? dataRow + 1 : ""}`;
  if (editing) return;
  clearTimeout(cellUiTimer);
  cellUiTimer = window.setTimeout(async () => {
    const s = sheet;
    const { row, col } = grid.active;
    try {
      const text = await call<string>("get_input", { sheet: s, row, col });
      if (!editing && s === sheet && row === grid.active.row && col === grid.active.col) formulaInput.value = text;
    } catch {
      /* ignore */
    }
    updateStatus();
  }, 16);
}

let statsSeq = 0;
async function updateStatus() {
  const s = curSheet();
  const parts: string[] = [];
  const sel = grid.selection;
  const multi = sel.r0 !== sel.r1 || sel.c0 !== sel.c1;
  if (multi) {
    const seq = ++statsSeq;
    try {
      const r1 = Math.min(sel.r1, Math.max(s.rows - 1, 0));
      const c1 = Math.min(sel.c1, Math.max(s.cols - 1, 0));
      const st = await call<Stats>("range_stats", { sheet, r0: sel.r0, c0: sel.c0, r1, c1 });
      if (seq !== statsSeq) return;
      if (st.numbers > 0) {
        parts.push(`Sum <b>${fmtNum(st.sum)}</b>`);
        parts.push(`Average <b>${fmtNum(st.sum / st.numbers)}</b>`);
        if (st.min !== null && st.max !== null) parts.push(`Min <b>${fmtNum(st.min)}</b>`, `Max <b>${fmtNum(st.max)}</b>`);
      }
      parts.push(`Count <b>${fmtNum(st.count)}</b>`);
    } catch {
      /* ignore */
    }
  }
  const rows = s.filter
    ? `${fmtNum(Math.max(s.rows - (s.header ? 1 : 0), 0))} of ${fmtNum(Math.max(s.dataRows - (s.header ? 1 : 0), 0))} rows`
    : `${fmtNum(s.dataRows)} rows`;
  parts.push(rows);
  statusEl.innerHTML = parts.join(" &nbsp;·&nbsp; ");
}

grid.onSelectionChange = () => {
  if (editing?.where === "cell") commitEdit(0, 0);
  updateCellUI();
};

// ---------------------------------------------------------------- editing

function startEdit(initial: string | null, mode: "enter" | "edit") {
  if (editing) return;
  const p = grid.active;
  grid.ensureVisible(p);
  const r = grid.editorRect(p);
  const original = formulaInput.value;
  editing = { mode, where: "cell", original };
  editor.value = initial ?? original;
  editor.hidden = false;
  editor.style.left = `${r.x}px`;
  editor.style.top = `${r.y}px`;
  editor.style.minWidth = `${r.w + 1}px`;
  editor.style.width = `${r.w + 1}px`;
  editor.style.height = `${r.h + 1}px`;
  sizeEditor();
  formulaInput.value = editor.value;
  editor.focus();
  const end = editor.value.length;
  editor.setSelectionRange(end, end);
}

function sizeEditor() {
  editor.style.width = editor.style.minWidth;
  const needed = Math.min(editor.scrollWidth + 4, grid.host.clientWidth - editor.offsetLeft - 8);
  if (needed > editor.offsetWidth) editor.style.width = `${needed}px`;
  const lines = editor.value.split("\n").length;
  editor.style.height = `${Math.max(23, lines * 18 + 5)}px`;
}

editor.addEventListener("input", () => {
  formulaInput.value = editor.value;
  sizeEditor();
});

formulaInput.addEventListener("focus", () => {
  if (editing) return;
  editing = { mode: "edit", where: "bar", original: formulaInput.value };
});
formulaInput.addEventListener("blur", () => {
  if (editing?.where === "bar") commitEdit(0, 0, false);
});
formulaInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    commitEdit(1, 0);
  } else if (e.key === "Escape") {
    e.preventDefault();
    cancelEdit();
  } else if (e.key === "Tab") {
    e.preventDefault();
    commitEdit(0, e.shiftKey ? -1 : 1);
  }
});

async function commitEdit(dr: number, dc: number, refocus = true) {
  if (!editing) return;
  const where = editing.where;
  const original = editing.original;
  const value = where === "cell" ? editor.value : formulaInput.value;
  editing = null;
  editor.hidden = true;
  const { row, col } = grid.active;
  if (refocus) grid.scroller.focus({ preventScroll: true });
  if (dr || dc) grid.advance(dr, dc);
  if (value === original) {
    updateCellUI();
    return;
  }
  try {
    const res = await call<EditResult>("set_inputs", { sheet, row, col, grid: [[value]] });
    afterEdit(res, false);
  } catch (e) {
    showError(e);
  }
}

function cancelEdit() {
  if (!editing) return;
  formulaInput.value = editing.original;
  editing = null;
  editor.hidden = true;
  grid.scroller.focus({ preventScroll: true });
}

/** Apply an edit result: new dimensions, invalidate visible cells, optionally move the cursor. */
function afterEdit(res: EditResult, moveCursor: boolean) {
  const sheetChanged = res.sheet !== sheet;
  book = res.info;
  if (sheetChanged) sheet = res.sheet;
  applyInfo(res.info, res.sheet);
  if (moveCursor && res.row !== null) grid.select({ row: res.row, col: res.col });
  grid.invalidate();
}

editor.addEventListener("keydown", (e) => {
  if (!editing) return;
  const k = e.key;
  if (k === "Enter" && !e.altKey) {
    e.preventDefault();
    commitEdit(e.shiftKey ? -1 : 1, 0);
  } else if (k === "Enter" && e.altKey) {
    e.preventDefault();
    const s = editor.selectionStart;
    editor.setRangeText("\n", s, editor.selectionEnd, "end");
    editor.dispatchEvent(new Event("input"));
  } else if (k === "Tab") {
    e.preventDefault();
    commitEdit(0, e.shiftKey ? -1 : 1);
  } else if (k === "Escape") {
    e.preventDefault();
    cancelEdit();
  } else if (editing.mode === "enter" && k.startsWith("Arrow")) {
    e.preventDefault();
    const d = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] }[k]!;
    commitEdit(d[0], d[1]);
  } else if (k === "F2") {
    editing.mode = editing.mode === "enter" ? "edit" : "enter";
  }
  e.stopPropagation();
});

grid.onEditRequest = () => startEdit(null, "edit");

nameBox.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    const p = parseAddress(nameBox.value);
    if (p) {
      grid.select(p);
      grid.scroller.focus();
    } else updateCellUI();
  } else if (e.key === "Escape") {
    updateCellUI();
    grid.scroller.focus();
  }
});
nameBox.addEventListener("focus", () => nameBox.select());

// ---------------------------------------------------------------- clipboard

async function clipboardWrite(c: Copied) {
  if (isTauri) {
    const cb = await import("@tauri-apps/plugin-clipboard-manager");
    try {
      await cb.writeHtml(c.html, c.tsv);
    } catch {
      await cb.writeText(c.tsv);
    }
  } else {
    try {
      await navigator.clipboard.write([
        new ClipboardItem({
          "text/plain": new Blob([c.tsv], { type: "text/plain" }),
          "text/html": new Blob([c.html], { type: "text/html" }),
        }),
      ]);
    } catch {
      await navigator.clipboard.writeText(c.tsv);
    }
  }
}

async function clipboardRead(): Promise<string> {
  if (isTauri) {
    const cb = await import("@tauri-apps/plugin-clipboard-manager");
    return (await cb.readText()) ?? "";
  }
  return navigator.clipboard.readText();
}

function selectionClipped(): Rect {
  const s = grid.selection;
  const cs = curSheet();
  return {
    r0: s.r0,
    c0: s.c0,
    r1: Math.max(s.r0, Math.min(s.r1, Math.max(cs.rows - 1, 0))),
    c1: Math.max(s.c0, Math.min(s.c1, Math.max(cs.cols - 1, 0))),
  };
}

async function copy(cut = false) {
  const r = selectionClipped();
  try {
    const c = await call<Copied>("copy_range", { sheet, ...r });
    await clipboardWrite(c);
    lastCopy = { text: c.tsv, row: r.r0, col: r.c0, inputs: c.inputs, cut, sheet, rect: r };
    grid.copyRect = r;
    grid.render();
    const cells = (r.r1 - r.r0 + 1) * (r.c1 - r.c0 + 1);
    toast(`${cut ? "Cut" : "Copied"} ${fmtNum(cells)} cell${cells === 1 ? "" : "s"}`, 1500);
  } catch (e) {
    showError(e, "Copy failed");
  }
}

const norm = (s: string) => s.replace(/\r\n/g, "\n").replace(/\n$/, "");

async function paste() {
  let text: string;
  try {
    text = await clipboardRead();
  } catch (e) {
    showError(e, "Can't read the clipboard");
    return;
  }
  if (!text) return;
  const { row, col } = grid.active;
  const internal = lastCopy && norm(lastCopy.text) === norm(text) ? lastCopy : null;
  try {
    let res: EditResult;
    if (internal?.cut) {
      // Move: clear the source first, then paste the original inputs unchanged.
      await call<EditResult>("clear_range", { sheet: internal.sheet, ...internal.rect });
      res = await call<EditResult>("paste", { sheet, row, col, text, internal: { row, col, inputs: internal.inputs } });
      lastCopy = null;
    } else {
      res = await call<EditResult>("paste", {
        sheet,
        row,
        col,
        text,
        internal: internal ? { row: internal.row, col: internal.col, inputs: internal.inputs } : null,
      });
    }
    grid.copyRect = null;
    afterEdit(res, false);
    const rows = internal ? internal.inputs.length : text.replace(/\r?\n$/, "").split(/\r?\n/).length;
    const cols = internal ? internal.inputs[0]?.length ?? 1 : text.split(/\r?\n/)[0].split("\t").length;
    grid.select({ row, col }, false, { row: row + rows - 1, col: col + cols - 1 });
  } catch (e) {
    showError(e, "Paste failed");
  }
}

async function clearSelection() {
  const r = selectionClipped();
  try {
    afterEdit(await call<EditResult>("clear_range", { sheet, ...r }), false);
  } catch (e) {
    showError(e);
  }
}

// ---------------------------------------------------------------- file commands

const OPEN_EXT = ["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv", "txt"];

async function confirmDiscard(): Promise<boolean> {
  if (!book?.modified) return true;
  return confirmDialog(`"${book.name}" has unsaved changes. Discard them?`, "Unsaved changes", "Discard");
}

async function cmdNew() {
  if (!(await confirmDiscard())) return;
  grid.dropCache();
  grid.showFilterButtons = false;
  applyInfo(await call<BookInfo>("new_book"), 0);
  grid.select({ row: 0, col: 0 });
}

async function openPath(path: string) {
  if (!(await confirmDiscard())) return;
  const t0 = performance.now();
  try {
    const info = await busy(`Opening ${path.split(/[\\/]/).pop()}…`, () => call<BookInfo>("open_file", { path }));
    grid.dropCache();
    grid.showFilterButtons = false;
    sheet = -1;
    applyInfo(info, 0);
    grid.select({ row: 0, col: 0 });
    const rows = info.sheets.reduce((n, s) => n + s.dataRows, 0);
    toast(`Opened ${fmtNum(rows)} rows in ${fmtNum(Math.round(performance.now() - t0))} ms`, 2500);
  } catch (e) {
    showError(e, "Can't open file");
  }
}

async function cmdOpen() {
  let path: string | null;
  if (isTauri) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ multiple: false, directory: false, filters: [{ name: "Spreadsheets", extensions: OPEN_EXT }] });
    path = typeof r === "string" ? r : null;
  } else {
    path = prompt("Path of the file to open (dev bridge):");
  }
  if (path) await openPath(path);
}

async function cmdSave(forceAs = false): Promise<boolean> {
  commitEdit(0, 0);
  let path: string | null = book.path;
  if (forceAs || !path || !book.writable) {
    const base = (book.path ?? "Untitled").replace(/\.(xls|xlsb|ods|xlsm|xlsx|csv|tsv|txt)$/i, "");
    const ext = book.format === "csv" ? "csv" : "xlsx";
    if (isTauri) {
      const { save } = await import("@tauri-apps/plugin-dialog");
      path = await save({
        defaultPath: `${base}.${ext}`,
        filters: [
          { name: "Excel workbook", extensions: ["xlsx"] },
          { name: "CSV (current sheet)", extensions: ["csv"] },
        ],
      });
    } else {
      path = prompt("Save as (path on the dev bridge machine):", `${base}.${ext}`);
    }
    if (!path) return false;
    if (!/\.(xlsx|csv|tsv)$/i.test(path)) path += ".xlsx";
  }
  try {
    const info = await busy("Saving…", () => call<BookInfo>("save_file", { path, sheet }));
    applyInfo(info);
    toast(`Saved ${escapeHtml(info.name)}`, 1800);
    return true;
  } catch (e) {
    showError(e, "Can't save");
    return false;
  }
}

async function cmdUndo(redo = false) {
  commitEdit(0, 0);
  try {
    const res = await busy(redo ? "Redoing…" : "Undoing…", () => call<EditResult | null>(redo ? "redo" : "undo"));
    if (res) afterEdit(res, true);
  } catch (e) {
    showError(e);
  }
}

// ---------------------------------------------------------------- sort & filter

async function cmdSort(ascending: boolean, col = grid.active.col, header = curSheet().header) {
  commitEdit(0, 0);
  try {
    const res = await busy("Sorting…", () => call<EditResult>("sort", { sheet, col, ascending, header }));
    afterEdit(res, false);
  } catch (e) {
    showError(e, "Can't sort");
  }
}

async function setFilter(filter: Filter | null) {
  try {
    const info = await busy("Filtering…", () => call<BookInfo>("set_filter", { sheet, filter }));
    grid.dropCache();
    grid.showFilterButtons = true;
    applyInfo(info);
    grid.select({ row: 0, col: grid.active.col });
  } catch (e) {
    showError(e, "Can't filter");
  }
}

async function openFilterFor(col: number, x: number, y: number) {
  const s = curSheet();
  const header = s.filter?.header ?? s.header;
  let title = colName(col);
  if (header) {
    try {
      const block = await call<{ text: string[] }>("get_cells", { sheet, r0: 0, r1: 0, c0: col, c1: col });
      if (block.text[0]) title = `${colName(col)} · ${block.text[0]}`;
    } catch {
      /* ignore */
    }
  }
  const current = s.filter?.conds.find((c) => c.col === col) ?? null;
  openFilterPopover({
    x,
    y,
    col,
    title,
    header,
    current,
    loadValues: (hdr) => call("column_values", { sheet, col, header: hdr }),
    onSort: (asc, hdr) => cmdSort(asc, col, hdr),
    onApply: (cond: FilterCond | null, hdr: boolean) => {
      const others = (s.filter?.conds ?? []).filter((c) => c.col !== col);
      const conds = cond ? [...others, cond] : others;
      setFilter(conds.length ? { header: hdr, conds } : null);
    },
  });
}

grid.onFilterButton = (col, x, y) => openFilterFor(col, x, y);

function cmdFilter() {
  const col = grid.active.col;
  grid.showFilterButtons = true;
  grid.render();
  const r = grid.cellRect(0, col);
  const host = grid.scroller.getBoundingClientRect();
  openFilterFor(col, host.left + Math.max(r.x, 0) + Math.min(r.w, 200) - 18, host.top + 24);
}

grid.onColResize = (col, width) => {
  call("set_col_width", { sheet, col, width }).catch(() => {});
};

grid.onContextMenu = (x, y) => {
  openMenu(x, y, [
    { label: "Cut", hint: isMac ? "⌘X" : "Ctrl+X", action: () => copy(true) },
    { label: "Copy", hint: isMac ? "⌘C" : "Ctrl+C", action: () => copy() },
    { label: "Paste", hint: isMac ? "⌘V" : "Ctrl+V", action: () => paste() },
    { label: "Clear contents", hint: "Del", action: () => clearSelection() },
    { separator: true },
    { label: "Sort A → Z", action: () => cmdSort(true) },
    { label: "Sort Z → A", action: () => cmdSort(false) },
    { label: "Filter this column…", action: () => cmdFilter() },
  ]);
};

// ---------------------------------------------------------------- find

const findBar = $("#find-bar");
const findInput = $<HTMLInputElement>("#find-input");
const findStatus = $("#find-status");

function openFind() {
  commitEdit(0, 0);
  findBar.hidden = false;
  findInput.focus();
  findInput.select();
}

function closeFind() {
  findBar.hidden = true;
  grid.scroller.focus();
}

async function findNext(backwards = false) {
  const query = findInput.value;
  if (!query) return;
  const all = $<HTMLInputElement>("#find-all").checked;
  const { row, col } = grid.active;
  try {
    const m = await call<Match | null>("find", { query, sheet, row, col, backwards, allSheets: all });
    if (!m) {
      findStatus.textContent = "Not found";
      return;
    }
    findStatus.textContent = m.sheet !== sheet ? book.sheets[m.sheet].name : "";
    if (m.sheet !== sheet) switchSheet(m.sheet);
    grid.select({ row: m.view_row, col: m.col });
  } catch (e) {
    findStatus.textContent = errorText(e);
  }
}

findInput.addEventListener("keydown", (e) => {
  e.stopPropagation();
  if (e.key === "Enter") {
    e.preventDefault();
    findNext(e.shiftKey);
  } else if (e.key === "Escape") closeFind();
});
findInput.addEventListener("input", () => (findStatus.textContent = ""));
$("#find-next").onclick = () => findNext(false);
$("#find-prev").onclick = () => findNext(true);
$("#find-close").onclick = closeFind;

// ---------------------------------------------------------------- print

async function cmdPrint() {
  commitEdit(0, 0);
  const s = curSheet();
  const sel = grid.selection;
  const multi = sel.r0 !== sel.r1 || sel.c0 !== sel.c1;
  const r: Rect = multi ? selectionClipped() : { r0: 0, c0: 0, r1: Math.max(s.rows - 1, 0), c1: Math.max(s.cols - 1, 0) };
  try {
    const doc = await busy("Preparing print…", () =>
      call<{ html: string; rows: number; truncated: boolean }>("print_html", { sheet, ...r }),
    );
    if (doc.truncated && !(await confirmDialog(`Only the first ${fmtNum(doc.rows)} rows will be printed. Continue?`, "Large sheet", "Print"))) return;
    $("#print-root").innerHTML = doc.html;
    if (isTauri) await native("print_page");
    else window.print();
  } catch (e) {
    showError(e, "Can't print");
  }
}

// ---------------------------------------------------------------- keyboard

function handleGridKey(e: KeyboardEvent) {
  if (editing) return;
  const mod = modKey(e);
  const k = e.key;
  const s = curSheet();
  const lastDataRow = Math.max(s.rows - 1, 0);
  const lastDataCol = Math.max(s.cols - 1, 0);
  const dirs: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
  if (k in dirs) {
    e.preventDefault();
    const [dr, dc] = dirs[k];
    if (mod) {
      const a = grid.active;
      const target = {
        row: dr < 0 ? 0 : dr > 0 ? (a.row < lastDataRow ? lastDataRow : a.row) : a.row,
        col: dc < 0 ? 0 : dc > 0 ? (a.col < lastDataCol ? lastDataCol : a.col) : a.col,
      };
      grid.select(target, e.shiftKey);
    } else grid.move(dr, dc, e.shiftKey);
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
      clearSelection();
      return;
    case "Escape":
      if (grid.copyRect) {
        grid.copyRect = null;
        lastCopy = null;
        grid.render();
      }
      return;
  }
  if (mod) return; // shortcuts are handled globally
  if (k.length === 1 && !e.altKey) {
    e.preventDefault();
    startEdit(k, "enter");
  }
}

grid.scroller.addEventListener("keydown", handleGridKey);

document.addEventListener("keydown", (e) => {
  const mod = modKey(e);
  if (!mod) {
    if (e.key === "Escape") closePopover();
    return;
  }
  const k = e.key.toLowerCase();
  const inText = editing !== null || (e.target instanceof HTMLInputElement && e.target !== grid.scroller);
  const run = (fn: () => unknown) => {
    e.preventDefault();
    fn();
  };
  switch (k) {
    case "s":
      return run(() => cmdSave(e.shiftKey));
    case "o":
      return run(cmdOpen);
    case "n":
      return run(cmdNew);
    case "p":
      return run(cmdPrint);
    case "f":
      return run(openFind);
    case "z":
      if (inText) return;
      return run(() => cmdUndo(e.shiftKey));
    case "y":
      if (inText) return;
      return run(() => cmdUndo(true));
    case "c":
      if (inText) return;
      return run(() => copy());
    case "x":
      if (inText) return;
      return run(() => copy(true));
    case "v":
      if (inText) return;
      return run(paste);
    case "a":
      if (inText) return;
      return run(() => grid.selectAll());
  }
});

// ---------------------------------------------------------------- toolbar

const commands: Record<string, () => unknown> = {
  new: cmdNew,
  open: cmdOpen,
  save: () => cmdSave(),
  saveAs: () => cmdSave(true),
  undo: () => cmdUndo(),
  redo: () => cmdUndo(true),
  sortAsc: () => cmdSort(true),
  sortDesc: () => cmdSort(false),
  filter: cmdFilter,
  clearFilters: async () => {
    try {
      let info = book;
      for (let i = 0; i < book.sheets.length; i++) {
        if (book.sheets[i].filter) info = await call<BookInfo>("set_filter", { sheet: i, filter: null });
      }
      grid.dropCache();
      grid.showFilterButtons = false;
      applyInfo(info);
    } catch (e) {
      showError(e);
    }
  },
  find: openFind,
  print: cmdPrint,
  feedback: () => openFeedback(appInfo),
  updates: () => checkForUpdates(appInfo, true),
};

document.querySelectorAll<HTMLButtonElement>("[data-cmd]").forEach((b) => {
  b.addEventListener("mousedown", (e) => e.preventDefault()); // keep focus (and edits) where they are
  b.addEventListener("click", () => commands[b.dataset.cmd!]?.());
});

$("#add-sheet").onclick = async () => {
  try {
    const info = await call<BookInfo>("add_sheet");
    applyInfo(info, info.sheets.length - 1);
  } catch (e) {
    showError(e);
  }
};

// ---------------------------------------------------------------- startup

async function init() {
  try {
    book = await call<BookInfo>("book_info");
  } catch (e) {
    document.body.innerHTML = `<p style="padding:24px">Can't reach the SpeedSheet engine: ${escapeHtml(errorText(e))}.<br>
      In a browser, start the dev bridge: <code>cargo run -p speedsheet --example dev_bridge</code></p>`;
    return;
  }
  applyInfo(book, 0);
  grid.select({ row: 0, col: 0 });
  grid.scroller.focus();
  appInfo = (await native<AppInfo>("app_info").catch(() => undefined)) ?? null;
  window.setTimeout(() => checkForUpdates(appInfo, false), 4000);

  if (isTauri) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const { getCurrentWebview } = await import("@tauri-apps/api/webview");
    const { listen } = await import("@tauri-apps/api/event");
    const win = getCurrentWindow();
    win.onCloseRequested(async (ev) => {
      if (!book.modified) return;
      ev.preventDefault();
      if (await confirmDiscard()) win.destroy();
    });
    getCurrentWebview().onDragDropEvent((ev) => {
      if (ev.payload.type === "drop" && ev.payload.paths.length) openPath(ev.payload.paths[0]);
    });
    listen<string>("open-file", async (ev) => {
      await native("take_pending_open");
      openPath(ev.payload);
    });
    const pending = await native<string | null>("take_pending_open");
    if (pending) openPath(pending);
  }
}

window.addEventListener("beforeunload", (e) => {
  if (!isTauri && book?.modified) e.preventDefault();
});

init();
