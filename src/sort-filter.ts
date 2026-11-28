// Sorting and filtering: the toolbar buttons, Ctrl+Shift+L and the Excel-style filter popover.
import { call, type BookInfo, type EditResult, type Filter, type FilterCond } from "./api";
import { app, curSheet, grid } from "./app";
import { afterEdit, applyInfo, loadLayout } from "./book";
import { commitEdit } from "./editing";
import { colName } from "./grid";
import { openFilterPopover } from "./popover";
import { busy, spawn, toast } from "./ui";

export async function cmdSort(ascending: boolean, col = grid.active.col, header = curSheet().header) {
  await commitEdit(0, 0);
  const res = await busy("Sorting…", () => call<EditResult>("sort", { sheet: app.sheet, col, ascending, header }));
  afterEdit(res, false);
}

async function setFilter(filter: Filter | null) {
  const info = await busy("Filtering…", () => call<BookInfo>("set_filter", { sheet: app.sheet, filter }));
  grid.dropCache();
  grid.showFilterButtons = true;
  applyInfo(info);
  grid.select({ row: 0, col: grid.active.col });
}

export async function openFilterFor(col: number, x: number, y: number) {
  const s = curSheet();
  const sheet = app.sheet;
  const header = s.filter?.header ?? s.header;
  let title = colName(col);
  const headerRow = grid.autofilter ? grid.autofilter.row : 0;
  if (header || grid.autofilter) {
    try {
      const block = await call<{ text: string[] }>("get_cells", { sheet, r0: headerRow, r1: headerRow, c0: col, c1: col });
      if (block.text[0]) title = `${colName(col)} · ${block.text[0]}`;
    } catch {
      /* the title stays the column letter */
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
    onSort: (asc, hdr) => spawn(cmdSort(asc, col, hdr), "Can't sort"),
    onApply: (cond: FilterCond | null, hdr: boolean) => {
      const others = (s.filter?.conds ?? []).filter((c) => c.col !== col);
      const conds = cond ? [...others, cond] : others;
      spawn(setFilter(conds.length ? { header: hdr, conds } : null), "Can't filter");
    },
  });
}

/** "Filter this column…": the popover under the column's filter button. */
export function cmdFilter() {
  const col = grid.active.col;
  const host = grid.scroller.getBoundingClientRect();
  const af = grid.autofilter;
  if (af && col >= af.c0 && col <= af.c1) {
    // The file has its own filter row: open the popover under that cell.
    grid.ensureVisible({ row: af.row, col });
    const r = grid.cellRect(af.row, col);
    spawn(openFilterFor(col, host.left + Math.max(r.x, 0) + Math.min(r.w, 200) - 18, host.top + r.y + r.h));
    return;
  }
  grid.showFilterButtons = true;
  grid.render();
  const r = grid.cellRect(0, col);
  spawn(openFilterFor(col, host.left + Math.max(r.x, 0) + Math.min(r.w, 200) - 18, host.top + 24));
}

/** Excel's Data > Filter (Ctrl+Shift+L): put filter buttons on the header row of the list
 * around the active cell (or of the selection), or take them off again. */
export async function cmdToggleFilter() {
  const sel = grid.selection;
  const had = !!grid.autofilter;
  const info = await call<BookInfo>("toggle_autofilter", { sheet: app.sheet, r0: sel.r0, c0: sel.c0, r1: sel.r1, c1: sel.c1 });
  grid.dropCache();
  grid.showFilterButtons = false;
  applyInfo(info);
  await loadLayout(true);
  toast(had ? "Filter removed" : "Filter buttons added to the header row", 1800);
}

export async function clearFilters() {
  let info = app.book;
  for (let i = 0; i < app.book.sheets.length; i++) {
    if (app.book.sheets[i].filter) info = await call<BookInfo>("set_filter", { sheet: i, filter: null });
  }
  grid.dropCache();
  grid.showFilterButtons = false;
  applyInfo(info);
}
