// Canvas-rendered, virtualized grid. Only the visible cells are ever drawn or fetched, in blocks,
// so a sheet with a million rows scrolls as smoothly as one with ten.
import { call, type Block } from "./api";

export interface Pos {
  row: number;
  col: number;
}
export interface Rect {
  r0: number;
  c0: number;
  r1: number;
  c1: number;
}

const BR = 128; // rows per fetched block
const BC = 32; // cols per fetched block
const MAX_SCROLL_PX = 8_000_000; // stay well below browser limits for element sizes
export const MAX_ROWS = 1_048_576;
export const MAX_COLS = 16_384;
const DEFAULT_COL_W = 100;
const RH = 22;
const HH = 24;

export function colName(c: number): string {
  let s = "";
  c += 1;
  while (c > 0) {
    const m = (c - 1) % 26;
    s = String.fromCharCode(65 + m) + s;
    c = Math.floor((c - 1) / 26);
  }
  return s;
}

export function parseAddress(s: string): Pos | null {
  const m = /^\s*\$?([A-Za-z]{1,3})\$?(\d{1,7})\s*$/.exec(s);
  if (!m) return null;
  let col = 0;
  for (const ch of m[1].toUpperCase()) col = col * 26 + (ch.charCodeAt(0) - 64);
  const row = parseInt(m[2], 10);
  if (row < 1 || row > MAX_ROWS || col > MAX_COLS) return null;
  return { row: row - 1, col: col - 1 };
}

interface CacheEntry {
  gen: number;
  block?: Block;
  pending?: boolean;
}

interface Colors {
  bg: string;
  grid: string;
  text: string;
  muted: string;
  accent: string;
  accentSoft: string;
  accentStrong: string;
  headerBg: string;
  headerActive: string;
  error: string;
  filtered: string;
  border: string;
}

type Hit =
  | { area: "corner" }
  | { area: "col"; col: number; resize: boolean; filterBtn: boolean }
  | { area: "row"; row: number }
  | { area: "cell"; row: number; col: number };

export class Grid {
  readonly host: HTMLElement;
  readonly scroller: HTMLElement;
  private spacer: HTMLElement;
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private colors!: Colors;
  private font = '13px system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';
  private headerFont = '12px system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';

  sheet = 0;
  /** Rows in the (filtered) view and used columns, from the backend. */
  viewRows = 0;
  dataCols = 0;
  filtered = false;
  filterCols = new Set<number>();
  showFilterButtons = false;
  rowCount = 1000;
  colCount = 30;
  private colWidths: number[] = [];
  private colX: number[] = [0];
  private rowHeaderW = 50;

  active: Pos = { row: 0, col: 0 };
  anchor: Pos = { row: 0, col: 0 };
  /** Optional dashed "marching ants" rectangle around copied cells. */
  copyRect: Rect | null = null;

  private cache = new Map<string, CacheEntry>();
  private gen = 0;
  private frame = 0;
  private drag: null | { kind: "cells" | "cols" | "rows"; start: Pos } = null;
  private resize: null | { col: number; startX: number; startW: number } = null;
  private autoScroll = 0;
  private lastMouse = { x: 0, y: 0 };

  onSelectionChange: () => void = () => {};
  onEditRequest: () => void = () => {};
  onFilterButton: (col: number, x: number, y: number) => void = () => {};
  onColResize: (col: number, width: number) => void = () => {};
  onContextMenu: (x: number, y: number) => void = () => {};

  constructor(host: HTMLElement) {
    this.host = host;
    this.scroller = host.querySelector("#scroller")!;
    this.spacer = host.querySelector("#spacer")!;
    this.canvas = host.querySelector("#grid-canvas")!;
    this.ctx = this.canvas.getContext("2d", { alpha: false })!;
    this.readColors();
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      this.readColors();
      this.render();
    });
    this.scroller.addEventListener("scroll", () => this.render(), { passive: true });
    new ResizeObserver(() => {
      this.updateSpacer();
      this.render();
    }).observe(this.scroller);
    this.scroller.addEventListener("mousedown", (e) => this.onMouseDown(e));
    window.addEventListener("mousemove", (e) => this.onMouseMove(e));
    window.addEventListener("mouseup", () => this.onMouseUp());
    this.scroller.addEventListener("dblclick", (e) => {
      const h = this.hit(e);
      if (h.area === "cell") this.onEditRequest();
      if (h.area === "col" && h.resize) this.autoFit(this.resizeTarget(e) ?? h.col);
    });
    this.scroller.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      this.onContextMenu(e.clientX, e.clientY);
    });
  }

  private readColors() {
    const cs = getComputedStyle(document.documentElement);
    const v = (n: string) => cs.getPropertyValue(n).trim();
    this.colors = {
      bg: v("--bg"),
      grid: v("--grid"),
      text: v("--text"),
      muted: v("--muted"),
      accent: v("--accent"),
      accentSoft: v("--accent-soft"),
      accentStrong: v("--accent-strong"),
      headerBg: v("--header-bg"),
      headerActive: v("--header-active"),
      error: v("--error"),
      filtered: v("--filtered"),
      border: v("--border"),
    };
  }

  // ---------- model ----------

  /** Point the grid at a sheet (or refresh its dimensions). */
  setSheet(sheet: number, viewRows: number, dataCols: number, widths: (number | null)[], filteredCols: number[] | null) {
    const changedSheet = sheet !== this.sheet;
    this.sheet = sheet;
    this.viewRows = viewRows;
    this.dataCols = dataCols;
    this.filtered = filteredCols !== null;
    this.filterCols = new Set(filteredCols ?? []);
    this.rowCount = this.filtered ? Math.max(viewRows, 1) : Math.min(MAX_ROWS, Math.max(viewRows + 500, 1000));
    this.colCount = Math.min(MAX_COLS, Math.max(dataCols + 10, 30));
    this.colWidths = widths.map((w) => w ?? DEFAULT_COL_W);
    this.rebuildColX();
    if (changedSheet) {
      this.active = { row: 0, col: 0 };
      this.anchor = { row: 0, col: 0 };
      this.scroller.scrollTop = 0;
      this.scroller.scrollLeft = 0;
      this.copyRect = null;
    }
    this.clampSelection();
    this.invalidate();
    this.updateSpacer();
  }

  /** Grow the virtual grid when the user navigates past it. */
  private ensureCapacity(p: Pos) {
    let grew = false;
    if (!this.filtered && p.row >= this.rowCount - 50 && this.rowCount < MAX_ROWS) {
      this.rowCount = Math.min(MAX_ROWS, Math.max(this.rowCount * 2, p.row + 500));
      grew = true;
    }
    if (p.col >= this.colCount - 5 && this.colCount < MAX_COLS) {
      this.colCount = Math.min(MAX_COLS, Math.max(this.colCount + 30, p.col + 10));
      this.rebuildColX();
      grew = true;
    }
    if (grew) this.updateSpacer();
  }

  private rebuildColX() {
    const x = new Array<number>(this.colCount + 1);
    x[0] = 0;
    for (let c = 0; c < this.colCount; c++) x[c + 1] = x[c] + this.colWidth(c);
    this.colX = x;
  }

  colWidth(c: number) {
    return this.colWidths[c] ?? DEFAULT_COL_W;
  }

  setColWidth(c: number, w: number) {
    while (this.colWidths.length <= c) this.colWidths.push(DEFAULT_COL_W);
    this.colWidths[c] = Math.max(24, Math.round(w));
    this.rebuildColX();
    this.updateSpacer();
    this.render();
  }

  /** Drop cached cell data; visible blocks are fetched again (stale data stays on screen meanwhile). */
  invalidate() {
    this.gen++;
    this.render();
  }

  dropCache() {
    this.cache.clear();
    this.gen++;
    this.render();
  }

  private blockFor(row: number, col: number): CacheEntry | undefined {
    const br = Math.floor(row / BR);
    const bc = Math.floor(col / BC);
    const key = `${this.sheet}:${br}:${bc}`;
    let e = this.cache.get(key);
    // Nothing to fetch outside the used range of an unfiltered sheet.
    const outside = (!this.filtered && br * BR >= this.viewRows) || bc * BC >= this.dataCols;
    if (outside) return undefined;
    if (!e) {
      e = { gen: -1 };
      this.cache.set(key, e);
    }
    if (e.gen !== this.gen && !e.pending) {
      e.pending = true;
      const gen = this.gen;
      const sheet = this.sheet;
      const entry = e;
      call<Block>("get_cells", { sheet, r0: br * BR, r1: br * BR + BR - 1, c0: bc * BC, c1: bc * BC + BC - 1 })
        .then((b) => {
          if (sheet !== this.sheet) return;
          entry.block = b;
          entry.gen = gen;
          this.render();
        })
        .catch(() => {})
        .finally(() => {
          entry.pending = false;
          if (entry.gen !== this.gen && sheet === this.sheet) this.render();
        });
      if (this.cache.size > 400) this.evict();
    }
    return e;
  }

  private evict() {
    // Keep the most recently inserted half; Map preserves insertion order.
    const keep = Array.from(this.cache.entries()).slice(-200);
    this.cache = new Map(keep);
  }

  cellAt(row: number, col: number): { text: string; kind: number } | null {
    const e = this.blockFor(row, col);
    if (!e) return { text: "", kind: 0 };
    const b = e.block;
    if (!b) return null;
    const i = row % BR;
    if (i >= b.rows.length) return { text: "", kind: 0 };
    const k = i * b.cols + (col % BC);
    return { text: b.text[k], kind: b.kinds[k] };
  }

  /** Data row number (0-based) shown in the row header for a view row. */
  dataRowOf(row: number): number | null {
    if (!this.filtered) return row;
    const e = this.blockFor(row, 0);
    const b = e?.block;
    if (!b) return null;
    return b.rows[row % BR] ?? null;
  }

  // ---------- geometry ----------

  private get viewW() {
    return this.scroller.clientWidth;
  }
  private get viewH() {
    return this.scroller.clientHeight;
  }
  private contentH() {
    return this.rowCount * RH;
  }
  private scaled() {
    return this.contentH() > MAX_SCROLL_PX;
  }
  private topPx(): number {
    const st = this.scroller.scrollTop;
    if (!this.scaled()) return st;
    const view = this.viewH - HH;
    return (st * (this.contentH() - view)) / (MAX_SCROLL_PX - view);
  }
  private setTopPx(px: number) {
    if (!this.scaled()) {
      this.scroller.scrollTop = px;
      return;
    }
    const view = this.viewH - HH;
    this.scroller.scrollTop = (px * (MAX_SCROLL_PX - view)) / (this.contentH() - view);
  }

  private updateSpacer() {
    const digits = String(Math.min(this.rowCount, MAX_ROWS)).length;
    this.rowHeaderW = Math.max(42, 14 + digits * 8);
    this.spacer.style.height = `${HH + Math.min(this.contentH(), MAX_SCROLL_PX)}px`;
    this.spacer.style.width = `${this.rowHeaderW + this.colX[this.colCount]}px`;
  }

  private colAtX(px: number): number {
    // Binary search in prefix sums.
    let lo = 0;
    let hi = this.colCount - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.colX[mid] <= px) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  /** Cell rectangle in canvas coordinates. */
  cellRect(row: number, col: number) {
    const x = this.rowHeaderW + this.colX[col] - this.scroller.scrollLeft;
    const y = HH + row * RH - this.topPx();
    return { x, y, w: this.colWidth(col), h: RH };
  }

  visibleRows(): number {
    return Math.max(1, Math.floor((this.viewH - HH) / RH));
  }

  ensureVisible(p: Pos) {
    this.ensureCapacity(p);
    const top = this.topPx();
    const view = this.viewH - HH;
    const y = p.row * RH;
    if (y < top) this.setTopPx(y);
    else if (y + RH > top + view) this.setTopPx(y + RH - view);
    const left = this.scroller.scrollLeft;
    const vw = this.viewW - this.rowHeaderW;
    const x0 = this.colX[p.col];
    const x1 = this.colX[p.col + 1];
    if (x0 < left) this.scroller.scrollLeft = x0;
    else if (x1 > left + vw) this.scroller.scrollLeft = Math.min(x0, x1 - vw);
  }

  private hit(e: MouseEvent): Hit {
    const r = this.scroller.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    return this.hitAt(x, y);
  }

  private hitAt(x: number, y: number): Hit {
    const cx = Math.max(0, x - this.rowHeaderW + this.scroller.scrollLeft);
    const col = Math.min(this.colCount - 1, this.colAtX(cx));
    const row = Math.min(this.rowCount - 1, Math.max(0, Math.floor((y - HH + this.topPx()) / RH)));
    if (y < HH && x < this.rowHeaderW) return { area: "corner" };
    if (y < HH) {
      const right = this.rowHeaderW + this.colX[col + 1] - this.scroller.scrollLeft;
      const left = this.rowHeaderW + this.colX[col] - this.scroller.scrollLeft;
      const resize = Math.abs(x - right) <= 4 || (Math.abs(x - left) <= 4 && col > 0);
      const filterBtn = this.showFilterButtons && col < Math.max(this.dataCols, 1) && x > right - 20 && x < right - 4;
      return { area: "col", col, resize, filterBtn };
    }
    if (x < this.rowHeaderW) return { area: "row", row };
    return { area: "cell", row, col };
  }

  private resizeTarget(e: MouseEvent): number | null {
    const r = this.scroller.getBoundingClientRect();
    const x = e.clientX - r.left;
    const cx = x - this.rowHeaderW + this.scroller.scrollLeft;
    const col = this.colAtX(cx);
    const left = this.colX[col];
    // Closer to the left border means the previous column is resized.
    if (cx - left <= 4 && col > 0) return col - 1;
    return col;
  }

  private autoFit(col: number) {
    this.ctx.font = this.font;
    let w = 40;
    const first = Math.floor(this.topPx() / RH);
    for (let r = first; r < first + this.visibleRows() + 1; r++) {
      const c = this.cellAt(r, col);
      if (c?.text) w = Math.max(w, this.ctx.measureText(c.text).width + 14);
    }
    this.setColWidth(col, Math.min(w, 600));
    this.onColResize(col, this.colWidth(col));
  }

  // ---------- selection ----------

  get selection(): Rect {
    return {
      r0: Math.min(this.anchor.row, this.active.row),
      r1: Math.max(this.anchor.row, this.active.row),
      c0: Math.min(this.anchor.col, this.active.col),
      c1: Math.max(this.anchor.col, this.active.col),
    };
  }

  private clampSelection() {
    const clamp = (p: Pos) => ({
      row: Math.max(0, Math.min(p.row, this.rowCount - 1)),
      col: Math.max(0, Math.min(p.col, this.colCount - 1)),
    });
    this.active = clamp(this.active);
    this.anchor = clamp(this.anchor);
  }

  select(active: Pos, extend = false, anchor?: Pos) {
    this.ensureCapacity(active);
    this.active = { ...active };
    if (anchor) this.anchor = { ...anchor };
    else if (!extend) this.anchor = { ...active };
    this.clampSelection();
    this.ensureVisible(this.active);
    this.render();
    this.onSelectionChange();
  }

  move(dr: number, dc: number, extend = false) {
    const p = { row: Math.max(0, this.active.row + dr), col: Math.max(0, this.active.col + dc) };
    this.select(p, extend);
  }

  /** Move the cursor within the selection (Enter/Tab inside a multi-cell selection), else just move. */
  advance(dr: number, dc: number) {
    const s = this.selection;
    if (s.r0 === s.r1 && s.c0 === s.c1) {
      this.move(dr, dc);
      return;
    }
    let { row, col } = this.active;
    if (dc) {
      col += dc;
      if (col > s.c1) {
        col = s.c0;
        row = row + 1 > s.r1 ? s.r0 : row + 1;
      } else if (col < s.c0) {
        col = s.c1;
        row = row - 1 < s.r0 ? s.r1 : row - 1;
      }
    } else {
      row += dr;
      if (row > s.r1) {
        row = s.r0;
        col = col + 1 > s.c1 ? s.c0 : col + 1;
      } else if (row < s.r0) {
        row = s.r1;
        col = col - 1 < s.c0 ? s.c1 : col - 1;
      }
    }
    this.active = { row, col };
    this.ensureVisible(this.active);
    this.render();
    this.onSelectionChange();
  }

  selectAll() {
    const rows = Math.max(this.viewRows, 1);
    const cols = Math.max(this.dataCols, 1);
    this.active = { row: 0, col: 0 };
    this.anchor = { row: rows - 1, col: cols - 1 };
    this.render();
    this.onSelectionChange();
  }

  pageRows() {
    return Math.max(1, this.visibleRows() - 1);
  }

  // ---------- mouse ----------

  private onMouseDown(e: MouseEvent) {
    if (e.button !== 0) {
      // Right click inside the selection keeps it; elsewhere it selects the cell first.
      const h = this.hit(e);
      if (h.area === "cell") {
        const s = this.selection;
        if (h.row < s.r0 || h.row > s.r1 || h.col < s.c0 || h.col > s.c1) this.select({ row: h.row, col: h.col });
      }
      return;
    }
    const h = this.hit(e);
    this.scroller.focus({ preventScroll: true });
    if (h.area === "corner") {
      this.selectAll();
      return;
    }
    if (h.area === "col") {
      if (h.resize) {
        const col = this.resizeTarget(e) ?? h.col;
        this.resize = { col, startX: e.clientX, startW: this.colWidth(col) };
        e.preventDefault();
        return;
      }
      if (h.filterBtn) {
        const rect = this.cellRect(0, h.col);
        const host = this.scroller.getBoundingClientRect();
        this.onFilterButton(h.col, host.left + rect.x + rect.w - 18, host.top + HH);
        e.preventDefault();
        return;
      }
      // Whole column: the active cell stays at the top, the anchor at the bottom.
      const last = Math.max(this.rowCount - 1, 0);
      const anchorCol = e.shiftKey ? this.anchor.col : h.col;
      this.select({ row: 0, col: h.col }, false, { row: last, col: anchorCol });
      this.drag = { kind: "cols", start: { row: 0, col: h.col } };
      e.preventDefault();
      return;
    }
    if (h.area === "row") {
      const last = Math.max(this.colCount - 1, 0);
      const anchorRow = e.shiftKey ? this.anchor.row : h.row;
      this.select({ row: h.row, col: 0 }, false, { row: anchorRow, col: last });
      this.drag = { kind: "rows", start: { row: h.row, col: 0 } };
      e.preventDefault();
      return;
    }
    this.select({ row: h.row, col: h.col }, e.shiftKey);
    this.drag = { kind: "cells", start: { row: h.row, col: h.col } };
    e.preventDefault();
  }

  private onMouseMove(e: MouseEvent) {
    const r = this.scroller.getBoundingClientRect();
    this.lastMouse = { x: e.clientX - r.left, y: e.clientY - r.top };
    if (this.resize) {
      this.setColWidth(this.resize.col, this.resize.startW + (e.clientX - this.resize.startX));
      return;
    }
    if (!this.drag) {
      if (e.target === this.scroller) {
        const h = this.hitAt(this.lastMouse.x, this.lastMouse.y);
        this.scroller.style.cursor = h.area === "col" && h.resize ? "col-resize" : h.area === "col" && h.filterBtn ? "pointer" : "cell";
      }
      return;
    }
    this.dragTo();
    const { x, y } = this.lastMouse;
    const outside = y < HH || y > this.viewH - 4 || x < this.rowHeaderW || x > this.viewW - 4;
    if (outside && !this.autoScroll) this.autoScrollLoop();
  }

  private dragTo() {
    if (!this.drag) return;
    const { x, y } = this.lastMouse;
    const h = this.hitAt(Math.max(this.rowHeaderW + 1, Math.min(x, this.viewW - 1)), Math.max(HH + 1, Math.min(y, this.viewH - 1)));
    if (h.area !== "cell") return;
    if (this.drag.kind === "cells") this.active = { row: h.row, col: h.col };
    else if (this.drag.kind === "cols") this.active = { row: 0, col: h.col };
    else this.active = { row: h.row, col: 0 };
    this.render();
    this.onSelectionChange();
  }

  private autoScrollLoop() {
    const step = () => {
      if (!this.drag) {
        this.autoScroll = 0;
        return;
      }
      const { x, y } = this.lastMouse;
      let moved = false;
      if (y < HH) {
        this.setTopPx(Math.max(0, this.topPx() - RH));
        moved = true;
      } else if (y > this.viewH - 4) {
        this.setTopPx(this.topPx() + RH);
        moved = true;
      }
      if (x < this.rowHeaderW) {
        this.scroller.scrollLeft -= 30;
        moved = true;
      } else if (x > this.viewW - 4) {
        this.scroller.scrollLeft += 30;
        moved = true;
      }
      if (!moved) {
        this.autoScroll = 0;
        return;
      }
      this.dragTo();
      this.autoScroll = requestAnimationFrame(step);
    };
    this.autoScroll = requestAnimationFrame(step);
  }

  private onMouseUp() {
    if (this.resize) {
      this.onColResize(this.resize.col, this.colWidth(this.resize.col));
      this.resize = null;
    }
    this.drag = null;
  }

  // ---------- drawing ----------

  render() {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.draw();
    });
  }

  private draw() {
    const dpr = window.devicePixelRatio || 1;
    const W = this.viewW;
    const H = this.viewH;
    if (this.canvas.width !== Math.round(W * dpr) || this.canvas.height !== Math.round(H * dpr)) {
      this.canvas.width = Math.round(W * dpr);
      this.canvas.height = Math.round(H * dpr);
      this.canvas.style.width = `${W}px`;
      this.canvas.style.height = `${H}px`;
    }
    const ctx = this.ctx;
    const C = this.colors;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = C.bg;
    ctx.fillRect(0, 0, W, H);

    const top = this.topPx();
    const left = this.scroller.scrollLeft;
    const firstRow = Math.floor(top / RH);
    const lastRow = Math.min(this.rowCount - 1, Math.floor((top + H - HH) / RH));
    const firstCol = this.colAtX(left);
    let lastCol = firstCol;
    while (lastCol < this.colCount - 1 && this.colX[lastCol + 1] < left + W - this.rowHeaderW) lastCol++;
    const RHW = this.rowHeaderW;
    const xOf = (c: number) => RHW + this.colX[c] - left;
    const yOf = (r: number) => HH + r * RH - top;
    const sel = this.selection;

    // Selection fill.
    const sx0 = Math.max(xOf(sel.c0), RHW);
    const sy0 = Math.max(yOf(sel.r0), HH);
    const sx1 = xOf(sel.c1 + 1);
    const sy1 = yOf(sel.r1 + 1);
    const multi = sel.r0 !== sel.r1 || sel.c0 !== sel.c1;
    if (multi && sx1 > sx0 && sy1 > sy0) {
      ctx.fillStyle = C.accentSoft;
      ctx.fillRect(sx0, sy0, sx1 - sx0, sy1 - sy0);
    }

    // Grid lines.
    ctx.strokeStyle = C.grid;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let r = firstRow; r <= lastRow + 1; r++) {
      const y = Math.round(yOf(r)) + 0.5;
      ctx.moveTo(RHW, y);
      ctx.lineTo(W, y);
    }
    for (let c = firstCol; c <= lastCol + 1; c++) {
      const x = Math.round(xOf(c)) + 0.5;
      ctx.moveTo(x, HH);
      ctx.lineTo(x, H);
    }
    ctx.stroke();

    // Cell text.
    ctx.font = this.font;
    ctx.textBaseline = "middle";
    for (let r = firstRow; r <= lastRow; r++) {
      const y = yOf(r);
      for (let c = firstCol; c <= lastCol; c++) {
        const cell = this.cellAt(r, c);
        if (!cell || !cell.text) continue;
        const x = xOf(c);
        const w = this.colWidth(c);
        const kind = cell.kind & 7;
        let clipW = w;
        let align: CanvasTextAlign = "left";
        if (kind === 1) align = "right";
        else if (kind === 3 || kind === 4) align = "center";
        const tw = ctx.measureText(cell.text).width;
        if (align === "left" && tw > w - 8) {
          // Let text overflow into empty neighbours, like Excel.
          let cc = c + 1;
          while (clipW < tw + 8 && cc < this.colCount && cc <= lastCol + 1) {
            const n = this.cellAt(r, cc);
            if (n && n.text) break;
            clipW += this.colWidth(cc);
            cc++;
          }
        }
        ctx.save();
        ctx.beginPath();
        ctx.rect(x + 1, y + 1, clipW - 2, RH - 2);
        ctx.clip();
        if (clipW > w) {
          // Cover the grid lines the overflow crosses.
          ctx.fillStyle = C.bg;
          ctx.fillRect(x + 1, y + 1, clipW - 2, RH - 2);
          if (multi && r >= sel.r0 && r <= sel.r1 && c >= sel.c0 && c <= sel.c1) {
            ctx.fillStyle = C.accentSoft;
            ctx.fillRect(x + 1, y + 1, clipW - 2, RH - 2);
          }
        }
        ctx.fillStyle = kind === 4 ? C.error : C.text;
        ctx.textAlign = align;
        const tx = align === "left" ? x + 4 : align === "right" ? x + w - 4 : x + w / 2;
        ctx.fillText(cell.text, tx, y + RH / 2 + 0.5);
        ctx.restore();
      }
    }

    // Copy marquee.
    if (this.copyRect) {
      const q = this.copyRect;
      ctx.save();
      ctx.setLineDash([5, 3]);
      ctx.strokeStyle = C.accent;
      ctx.lineWidth = 1.5;
      ctx.strokeRect(xOf(q.c0) + 1, yOf(q.r0) + 1, xOf(q.c1 + 1) - xOf(q.c0) - 2, yOf(q.r1 + 1) - yOf(q.r0) - 2);
      ctx.restore();
    }

    // Selection border and active cell.
    if (sx1 > RHW && sy1 > HH) {
      ctx.strokeStyle = C.accent;
      ctx.lineWidth = multi ? 1 : 2;
      ctx.strokeRect(xOf(sel.c0) + 0.5, yOf(sel.r0) + 0.5, xOf(sel.c1 + 1) - xOf(sel.c0) - 1, yOf(sel.r1 + 1) - yOf(sel.r0) - 1);
      if (multi) {
        const a = this.cellRect(this.active.row, this.active.col);
        ctx.lineWidth = 2;
        ctx.strokeRect(a.x + 1, a.y + 1, a.w - 2, a.h - 2);
      }
      // Fill handle dot.
      ctx.fillStyle = C.accent;
      ctx.fillRect(xOf(sel.c1 + 1) - 3, yOf(sel.r1 + 1) - 3, 6, 6);
    }

    // Column headers.
    ctx.fillStyle = C.headerBg;
    ctx.fillRect(0, 0, W, HH);
    ctx.font = this.headerFont;
    ctx.textAlign = "center";
    const allRows = sel.r0 === 0 && sel.r1 >= this.rowCount - 1;
    const allCols = sel.c0 === 0 && sel.c1 >= this.colCount - 1;
    for (let c = firstCol; c <= lastCol; c++) {
      const x = xOf(c);
      const w = this.colWidth(c);
      if (c >= sel.c0 && c <= sel.c1) {
        ctx.fillStyle = allRows ? C.accentStrong : C.headerActive;
        ctx.fillRect(x, 0, w, HH);
      }
      ctx.fillStyle = this.filterCols.has(c) ? C.filtered : C.muted;
      ctx.fillText(colName(c), x + w / 2, HH / 2 + 0.5);
      if (this.showFilterButtons && c < Math.max(this.dataCols, 1)) this.drawFilterButton(x + w - 18, 4, this.filterCols.has(c));
    }
    ctx.strokeStyle = C.border;
    ctx.beginPath();
    ctx.moveTo(0, HH - 0.5);
    ctx.lineTo(W, HH - 0.5);
    for (let c = firstCol; c <= lastCol + 1; c++) {
      const x = Math.round(xOf(c)) + 0.5;
      ctx.moveTo(x, 0);
      ctx.lineTo(x, HH);
    }
    ctx.stroke();

    // Row headers.
    ctx.fillStyle = C.headerBg;
    ctx.fillRect(0, HH, RHW, H - HH);
    ctx.textAlign = "right";
    for (let r = firstRow; r <= lastRow; r++) {
      const y = yOf(r);
      if (r >= sel.r0 && r <= sel.r1) {
        ctx.fillStyle = allCols ? C.accentStrong : C.headerActive;
        ctx.fillRect(0, y, RHW, RH);
      }
      const dr = this.dataRowOf(r);
      if (dr === null) continue;
      ctx.fillStyle = this.filtered ? C.filtered : C.muted;
      ctx.fillText(String(dr + 1), RHW - 7, y + RH / 2 + 0.5);
    }
    ctx.strokeStyle = C.border;
    ctx.beginPath();
    ctx.moveTo(RHW - 0.5, 0);
    ctx.lineTo(RHW - 0.5, H);
    for (let r = firstRow; r <= lastRow + 1; r++) {
      const y = Math.round(yOf(r)) + 0.5;
      if (y < HH) continue;
      ctx.moveTo(0, y);
      ctx.lineTo(RHW, y);
    }
    ctx.stroke();

    // Corner.
    ctx.fillStyle = C.headerBg;
    ctx.fillRect(0, 0, RHW, HH);
    ctx.strokeStyle = C.border;
    ctx.strokeRect(0.5, 0.5, RHW - 1, HH - 1);
    ctx.fillStyle = C.muted;
    ctx.beginPath();
    ctx.moveTo(RHW - 5, HH - 5);
    ctx.lineTo(RHW - 5, HH - 13);
    ctx.lineTo(RHW - 13, HH - 5);
    ctx.fill();
  }

  private drawFilterButton(x: number, y: number, active: boolean) {
    const ctx = this.ctx;
    const C = this.colors;
    ctx.fillStyle = active ? C.filtered : C.bg;
    ctx.strokeStyle = active ? C.filtered : C.border;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.roundRect(x + 0.5, y + 0.5, 15, 15, 3);
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = active ? C.bg : C.muted;
    ctx.beginPath();
    if (active) {
      ctx.moveTo(x + 4, y + 5);
      ctx.lineTo(x + 12, y + 5);
      ctx.lineTo(x + 9, y + 9);
      ctx.lineTo(x + 9, y + 12);
      ctx.lineTo(x + 7, y + 12);
      ctx.lineTo(x + 7, y + 9);
    } else {
      ctx.moveTo(x + 4.5, y + 6);
      ctx.lineTo(x + 11.5, y + 6);
      ctx.lineTo(x + 8, y + 10.5);
    }
    ctx.closePath();
    ctx.fill();
  }

  /** Where to place the in-cell editor (host coordinates). */
  editorRect(p: Pos) {
    const r = this.cellRect(p.row, p.col);
    return { x: Math.max(r.x, this.rowHeaderW), y: Math.max(r.y, HH), w: r.w, h: r.h };
  }
}
