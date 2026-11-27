// Canvas-rendered, virtualized grid. Only the visible cells are ever drawn or fetched, in blocks,
// so a sheet with a million rows scrolls as smoothly as one with ten.
import { call, type Block, type CellStyle, type Layout, type Area, type BorderLine } from "./api";

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
const APP_COL_W = 100;
const APP_ROW_H = 22;
const HH = 24;
const BASE_FONT_PX = 13;
const FONT_STACK = 'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';

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
  dark: boolean;
}

export interface CellData {
  text: string;
  kind: number;
  style: number;
  cf: number;
}

type Hit =
  | { area: "corner" }
  | { area: "col"; col: number; resize: number | null; filterBtn: boolean }
  | { area: "row"; row: number; resize: number | null }
  | { area: "cell"; row: number; col: number; filterBtn: boolean }
  | { area: "fill" };

const EMPTY: CellData = { text: "", kind: 0, style: 0, cf: 0 };

// ---------- color helpers ----------

function hex(c: number) {
  return "#" + c.toString(16).padStart(6, "0");
}
function parseColor(s: string): [number, number, number] {
  const m = /^#([0-9a-f]{6})$/i.exec(s.trim());
  if (m) {
    const n = parseInt(m[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  const r = /rgba?\(([^)]+)\)/.exec(s);
  if (r) {
    const p = r[1].split(",").map((x) => parseFloat(x));
    return [p[0], p[1], p[2]];
  }
  return [255, 255, 255];
}
function luminance([r, g, b]: [number, number, number]) {
  const f = (v: number) => {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}
function contrast(a: [number, number, number], b: [number, number, number]) {
  const la = luminance(a);
  const lb = luminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}
const rgbOf = (c: number): [number, number, number] => [(c >> 16) & 255, (c >> 8) & 255, c & 255];

/** Lighten a dark color until it reads on a dark background (keeps the hue). */
function readableOn(c: number, bg: [number, number, number], min = 4): string {
  let [r, g, b] = rgbOf(c);
  for (let i = 0; i < 10 && contrast([r, g, b], bg) < min; i++) {
    r += (255 - r) * 0.25;
    g += (255 - g) * 0.25;
    b += (255 - b) * 0.25;
  }
  return `rgb(${Math.round(r)},${Math.round(g)},${Math.round(b)})`;
}

export class Grid {
  readonly host: HTMLElement;
  readonly scroller: HTMLElement;
  private spacer: HTMLElement;
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private colors!: Colors;
  private font = `${BASE_FONT_PX}px ${FONT_STACK}`;
  private headerFont = `12px ${FONT_STACK}`;

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

  // Layout from the file.
  private defColW = APP_COL_W;
  private defRowH = APP_ROW_H;
  private rowHeights = new Map<number, number>();
  private rowY: Float64Array | null = null;
  private merges: Area[] = [];
  private mergeIndex = new Map<string, Area>();
  frozenRows = 0;
  frozenCols = 0;
  autofilter: Layout["autofilter"] = null;
  private hideGrid = false;

  // Styles of the workbook.
  styles: CellStyle[] = [];
  dxfs: CellStyle[] = [];
  private basePt = 11;
  private fontCache = new Map<string, string>();
  private wrapCache = new Map<string, string[]>();

  active: Pos = { row: 0, col: 0 };
  anchor: Pos = { row: 0, col: 0 };
  /** Optional dashed "marching ants" rectangle around copied cells. */
  copyRect: Rect | null = null;

  private cache = new Map<string, CacheEntry>();
  private gen = 0;
  private frame = 0;
  private drag: null | { kind: "cells" | "cols" | "rows"; start: Pos } = null;
  private resize: null | { kind: "col" | "row"; index: number; start: number; startSize: number } = null;
  /** Dragging the fill handle: the selection being extended and the target so far. */
  private fillDrag: null | { src: Rect; dst: Rect } = null;
  private pointDrag: null | Pos = null;
  private autoScroll = 0;
  private lastMouse = { x: 0, y: 0 };

  onSelectionChange: () => void = () => {};
  onEditRequest: () => void = () => {};
  onFilterButton: (col: number, x: number, y: number) => void = () => {};
  onColResize: (col: number, width: number) => void = () => {};
  onRowResize: (row: number, height: number) => void = () => {};
  onContextMenu: (x: number, y: number) => void = () => {};
  onFill: (src: Rect, dst: Rect) => void = () => {};
  onFillAuto: (src: Rect) => void = () => {};
  /** While a formula is being typed: clicking cells inserts references instead of selecting. */
  pointMode: null | {
    onPick: (r: Rect, dragging: boolean) => void;
  } = null;
  /** References shown in color while a formula is edited. */
  refHighlights: { rect: Rect; color: string }[] = [];
  /** Cursors of other windows on the same file (on any sheet; only this sheet's are drawn). */
  peerCursors: { sheet: number; active: Pos; rect: Rect; color: string; label: string }[] = [];

  constructor(host: HTMLElement) {
    this.host = host;
    this.scroller = host.querySelector("#scroller")!;
    this.spacer = host.querySelector("#spacer")!;
    this.canvas = host.querySelector("#grid-canvas")!;
    this.ctx = this.canvas.getContext("2d", { alpha: false })!;
    this.readColors();
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => this.themeChanged());
    this.scroller.addEventListener("scroll", () => this.render(), { passive: true });
    new ResizeObserver(() => {
      this.updateSpacer();
      this.render();
    }).observe(this.scroller);
    this.scroller.addEventListener("mousedown", (e) => this.onMouseDown(e));
    window.addEventListener("mousemove", (e) => this.onMouseMove(e));
    window.addEventListener("mouseup", () => this.onMouseUp());
    // A release outside the window (or while another window has focus) never sends mouseup:
    // end the drag then too, so the selection doesn't keep following the pointer.
    window.addEventListener("blur", () => this.cancelDrags());
    this.scroller.addEventListener("dblclick", (e) => {
      const h = this.hit(e);
      if (h.area === "fill") {
        this.fillDrag = null;
        this.onFillAuto(this.selection);
        return;
      }
      if (h.area === "cell" && !h.filterBtn) this.onEditRequest();
      if (h.area === "col" && h.resize !== null) this.autoFit(h.resize);
      if (h.area === "row" && h.resize !== null) {
        this.setRowHeight(h.resize, null);
        this.onRowResize(h.resize, this.defRowH);
      }
    });
    this.scroller.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      this.onContextMenu(e.clientX, e.clientY);
    });
  }

  /** Re-read theme colors (after a theme switch). */
  themeChanged() {
    this.readColors();
    this.render();
  }

  private readColors() {
    const cs = getComputedStyle(this.host);
    const v = (n: string) => cs.getPropertyValue(n).trim();
    const bg = v("--bg") || "#ffffff";
    this.colors = {
      bg,
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
      dark: luminance(parseColor(bg)) < 0.3,
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
    this.colWidths = widths.map((w) => (w === null || w === undefined ? -1 : w));
    this.rebuildColX();
    this.rebuildRowY();
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

  /** Workbook styles (once per opened file). */
  setStyles(styles: CellStyle[], dxfs: CellStyle[]) {
    this.styles = styles;
    this.dxfs = dxfs;
    this.basePt = styles[0]?.fontSize ?? 11;
    this.fontCache.clear();
    this.wrapCache.clear();
    this.render();
  }

  /** Sheet layout from the file: row heights, merges, frozen panes, autofilter. */
  setLayout(l: Layout | null) {
    this.defColW = l?.defaultColWidth ?? APP_COL_W;
    this.defRowH = l?.defaultRowHeight ?? APP_ROW_H;
    this.rowHeights = new Map();
    for (const [start, n, h] of l?.rowHeights ?? []) for (let i = 0; i < n; i++) this.rowHeights.set(start + i, h);
    this.merges = l?.merges ?? [];
    this.mergeIndex = new Map();
    for (const m of this.merges) {
      for (let r = m.r0; r <= Math.min(m.r1, m.r0 + 2000); r++)
        for (let c = m.c0; c <= Math.min(m.c1, m.c0 + 200); c++) this.mergeIndex.set(`${r}:${c}`, m);
    }
    this.frozenRows = l?.freeze?.[0] ?? 0;
    this.frozenCols = l?.freeze?.[1] ?? 0;
    this.autofilter = l?.autofilter ?? null;
    this.hideGrid = l?.hideGrid ?? false;
    this.rebuildColX();
    this.rebuildRowY();
    this.updateSpacer();
    this.render();
  }

  mergeAt(row: number, col: number): Area | undefined {
    return this.mergeIndex.get(`${row}:${col}`);
  }

  /** Grow the virtual grid when the user navigates past it. */
  private ensureCapacity(p: Pos) {
    let grew = false;
    if (!this.filtered && p.row >= this.rowCount - 50 && this.rowCount < MAX_ROWS) {
      this.rowCount = Math.min(MAX_ROWS, Math.max(this.rowCount * 2, p.row + 500));
      this.rebuildRowY();
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

  private rebuildRowY() {
    if (this.rowHeights.size === 0) {
      this.rowY = null;
      return;
    }
    const y = new Float64Array(this.rowCount + 1);
    const d = this.defRowH;
    for (let r = 0; r < this.rowCount; r++) y[r + 1] = y[r] + (this.rowHeights.get(r) ?? d);
    this.rowY = y;
  }

  colWidth(c: number) {
    const w = this.colWidths[c];
    return w === undefined || w < 0 ? this.defColW : w;
  }

  rowHeight(r: number) {
    return this.rowHeights.get(r) ?? this.defRowH;
  }

  private rowTop(r: number) {
    if (this.rowY) return r <= this.rowCount ? this.rowY[Math.max(0, r)] : this.rowY[this.rowCount] + (r - this.rowCount) * this.defRowH;
    return r * this.defRowH;
  }

  private rowAtY(y: number): number {
    if (!this.rowY) return Math.max(0, Math.floor(y / this.defRowH));
    const a = this.rowY;
    let lo = 0;
    let hi = this.rowCount - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (a[mid] <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  setColWidth(c: number, w: number) {
    while (this.colWidths.length <= c) this.colWidths.push(-1);
    this.colWidths[c] = Math.max(8, Math.round(w));
    this.rebuildColX();
    this.updateSpacer();
    this.render();
  }

  setRowHeight(r: number, h: number | null) {
    if (h === null) this.rowHeights.delete(r);
    else this.rowHeights.set(r, Math.max(6, Math.round(h)));
    this.rebuildRowY();
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

  cellAt(row: number, col: number): CellData | null {
    const e = this.blockFor(row, col);
    if (!e) return EMPTY;
    const b = e.block;
    if (!b) return null;
    const i = row % BR;
    if (i >= b.rows.length) return EMPTY;
    const k = i * b.cols + (col % BC);
    return { text: b.text[k], kind: b.kinds[k], style: b.styles?.[k] ?? 0, cf: b.cf?.[k] ?? 0 };
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
    return this.rowTop(this.rowCount);
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
    px = Math.max(0, px);
    if (!this.scaled()) {
      this.scroller.scrollTop = px;
      return;
    }
    const view = this.viewH - HH;
    this.scroller.scrollTop = (px * (MAX_SCROLL_PX - view)) / (this.contentH() - view);
  }
  private get frozenH() {
    return this.rowTop(this.frozenRows);
  }
  private get frozenW() {
    return this.colX[Math.min(this.frozenCols, this.colCount)] ?? 0;
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

  /** Canvas x of a column's left edge (frozen columns don't scroll). */
  private xOf(c: number) {
    return this.rowHeaderW + this.colX[Math.min(c, this.colCount)] - (c >= this.frozenCols ? this.scroller.scrollLeft : 0);
  }
  /** Canvas y of a row's top edge (frozen rows don't scroll). */
  private yOf(r: number, top = this.topPx()) {
    return HH + this.rowTop(r) - (r >= this.frozenRows ? top : 0);
  }

  /** Content column under a canvas x. */
  private colAtCanvasX(x: number) {
    const cx = x - this.rowHeaderW;
    if (this.frozenCols > 0 && cx < this.frozenW) return this.colAtX(Math.max(0, cx));
    return this.colAtX(Math.max(0, cx + this.scroller.scrollLeft));
  }
  private rowAtCanvasY(y: number) {
    const cy = y - HH;
    if (this.frozenRows > 0 && cy < this.frozenH) return this.rowAtY(Math.max(0, cy));
    return Math.min(this.rowCount - 1, this.rowAtY(Math.max(0, cy + this.topPx())));
  }

  /** Cell rectangle in canvas coordinates (a merged cell covers its whole range). */
  cellRect(row: number, col: number) {
    const m = this.mergeAt(row, col);
    const r0 = m ? m.r0 : row;
    const c0 = m ? m.c0 : col;
    const r1 = m ? m.r1 : row;
    const c1 = m ? m.c1 : col;
    const x = this.xOf(c0);
    const y = this.yOf(r0);
    return { x, y, w: this.xOf(c1 + 1) - x, h: this.yOf(r1 + 1) - y };
  }

  visibleRows(): number {
    return Math.max(1, Math.floor((this.viewH - HH - this.frozenH) / this.defRowH));
  }

  ensureVisible(p: Pos) {
    this.ensureCapacity(p);
    if (p.row >= this.frozenRows) {
      const top = this.topPx();
      const view = this.viewH - HH;
      const y = this.rowTop(p.row);
      const h = this.rowHeight(p.row);
      if (y - this.frozenH < top) this.setTopPx(y - this.frozenH);
      else if (y + h > top + view) this.setTopPx(Math.min(y - this.frozenH, y + h - view));
    }
    if (p.col >= this.frozenCols) {
      const left = this.scroller.scrollLeft;
      const vw = this.viewW - this.rowHeaderW;
      const x0 = this.colX[p.col] - this.frozenW;
      const x1 = this.colX[p.col + 1];
      if (x0 < left) this.scroller.scrollLeft = x0;
      else if (x1 > left + vw) this.scroller.scrollLeft = Math.min(x0, x1 - vw);
    }
  }

  private hit(e: MouseEvent): Hit {
    const r = this.scroller.getBoundingClientRect();
    return this.hitAt(e.clientX - r.left, e.clientY - r.top);
  }

  private hitAt(x: number, y: number): Hit {
    const col = Math.min(this.colCount - 1, this.colAtCanvasX(x));
    const row = Math.min(this.rowCount - 1, this.rowAtCanvasY(y));
    if (y < HH && x < this.rowHeaderW) return { area: "corner" };
    if (y < HH) {
      // Resize when near a column border; near the left border the previous visible column resizes.
      let resize: number | null = null;
      const right = this.xOf(col + 1);
      const left = this.xOf(col);
      if (Math.abs(x - right) <= 5) resize = col;
      else if (Math.abs(x - left) <= 5 && col > 0) {
        let p = col - 1;
        while (p > 0 && this.colWidth(p) === 0) p--;
        resize = p;
      }
      const filterBtn =
        resize === null && this.showFilterButtons && !this.autofilter && col < Math.max(this.dataCols, 1) && x > right - 20 && x < right - 4;
      return { area: "col", col, resize, filterBtn };
    }
    if (x < this.rowHeaderW) {
      let resize: number | null = null;
      const bottom = this.yOf(row + 1);
      const top = this.yOf(row);
      if (Math.abs(y - bottom) <= 3) resize = row;
      else if (Math.abs(y - top) <= 3 && row > 0) resize = row - 1;
      return { area: "row", row, resize };
    }
    // The fill handle: the small square at the bottom-right of the selection.
    {
      const s = this.selection;
      const hx = this.xOf(s.c1 + 1);
      const hy = this.yOf(s.r1 + 1);
      if (!this.pointMode && Math.abs(x - hx) <= 5 && Math.abs(y - hy) <= 5) return { area: "fill" };
    }
    let filterBtn = false;
    const af = this.autofilter;
    if (af && row === af.row && col >= af.c0 && col <= af.c1) {
      const r = this.cellRect(row, col);
      filterBtn = x > r.x + r.w - 19 && y > r.y + r.h - 19;
    }
    return { area: "cell", row, col, filterBtn };
  }

  private autoFit(col: number) {
    let w = 30;
    const first = this.rowAtCanvasY(HH + this.frozenH);
    for (let r = first; r < first + this.visibleRows() + 2; r++) {
      const c = this.cellAt(r, col);
      if (!c?.text || this.mergeAt(r, col)) continue;
      const st = this.styles[c.style];
      this.ctx.font = this.fontFor(st, c.cf ? this.dxfs[c.cf - 1] : undefined);
      const lines = st?.wrap ? c.text.split("\n") : [c.text];
      for (const l of lines) w = Math.max(w, this.ctx.measureText(l).width + 12 + (st?.indent ?? 0) * 9);
    }
    this.setColWidth(col, Math.min(w, 800));
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
    else if (!extend) {
      // A merged cell is selected as a whole, with the active cell at its top-left.
      const m = this.mergeAt(active.row, active.col);
      if (m) {
        this.active = { row: m.r0, col: m.c0 };
        this.anchor = { row: m.r1, col: m.c1 };
      } else this.anchor = { ...active };
    }
    this.clampSelection();
    this.ensureVisible(this.active);
    this.render();
    this.onSelectionChange();
  }

  move(dr: number, dc: number, extend = false) {
    let { row, col } = this.active;
    // Step out of a merged range in the direction of travel.
    const m = !extend ? this.mergeAt(row, col) : undefined;
    if (m) {
      if (dr > 0) row = m.r1;
      if (dc > 0) col = m.c1;
    }
    row = Math.max(0, row + dr);
    col = Math.max(0, col + dc);
    // Skip hidden rows and columns.
    while (dr && row > 0 && row < this.rowCount - 1 && this.rowHeight(row) === 0) row += Math.sign(dr);
    while (dc && col > 0 && col < this.colCount - 1 && this.colWidth(col) === 0) col += Math.sign(dc);
    this.select({ row, col }, extend);
  }

  /** Move the cursor within the selection (Enter/Tab inside a multi-cell selection), else just move. */
  advance(dr: number, dc: number) {
    const s = this.selection;
    const single = (s.r0 === s.r1 && s.c0 === s.c1) || this.isMergeSelection();
    if (single) {
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

  /** The selection is exactly one merged cell. */
  isMergeSelection() {
    const s = this.selection;
    const m = this.mergeAt(s.r0, s.c0);
    return !!m && m.r0 === s.r0 && m.c0 === s.c0 && m.r1 === s.r1 && m.c1 === s.c1;
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
    if (this.pointMode && h.area === "cell") {
      // Formula point mode: the click picks a reference; the editor keeps focus.
      e.preventDefault();
      const m = this.mergeAt(h.row, h.col);
      const r = m ? { ...m } : { r0: h.row, c0: h.col, r1: h.row, c1: h.col };
      this.pointDrag = { row: h.row, col: h.col };
      this.pointMode.onPick(r, false);
      return;
    }
    this.scroller.focus({ preventScroll: true });
    if (h.area === "fill") {
      const s = this.selection;
      this.fillDrag = { src: s, dst: { ...s } };
      e.preventDefault();
      return;
    }
    if (h.area === "corner") {
      this.selectAll();
      return;
    }
    if (h.area === "col") {
      if (h.resize !== null) {
        this.resize = { kind: "col", index: h.resize, start: e.clientX, startSize: this.colWidth(h.resize) };
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
      if (h.resize !== null) {
        this.resize = { kind: "row", index: h.resize, start: e.clientY, startSize: this.rowHeight(h.resize) };
        e.preventDefault();
        return;
      }
      const last = Math.max(this.colCount - 1, 0);
      const anchorRow = e.shiftKey ? this.anchor.row : h.row;
      this.select({ row: h.row, col: 0 }, false, { row: anchorRow, col: last });
      this.drag = { kind: "rows", start: { row: h.row, col: 0 } };
      e.preventDefault();
      return;
    }
    if (h.filterBtn) {
      const rect = this.cellRect(h.row, h.col);
      const host = this.scroller.getBoundingClientRect();
      this.onFilterButton(h.col, host.left + rect.x + rect.w - 18, host.top + rect.y + rect.h);
      e.preventDefault();
      return;
    }
    this.select({ row: h.row, col: h.col }, e.shiftKey);
    this.drag = { kind: "cells", start: { row: h.row, col: h.col } };
    e.preventDefault();
  }

  /** Stop a drag whose button release never reached us, without applying a fill. */
  private cancelDrags() {
    if (!this.drag && !this.fillDrag && !this.pointDrag && !this.resize) return;
    if (this.fillDrag) {
      this.fillDrag = null;
      this.render();
    }
    this.onMouseUp();
  }

  private onMouseMove(e: MouseEvent) {
    if (e.buttons === 0 && (this.drag || this.fillDrag || this.pointDrag || this.resize)) {
      this.cancelDrags();
      return;
    }
    const r = this.scroller.getBoundingClientRect();
    this.lastMouse = { x: e.clientX - r.left, y: e.clientY - r.top };
    if (this.resize) {
      const z = this.resize;
      if (z.kind === "col") this.setColWidth(z.index, z.startSize + (e.clientX - z.start));
      else this.setRowHeight(z.index, z.startSize + (e.clientY - z.start));
      return;
    }
    if (this.fillDrag || this.pointDrag) {
      const { x, y } = this.lastMouse;
      const hx = Math.max(this.rowHeaderW + 1, Math.min(x, this.viewW - 1));
      const hy = Math.max(HH + 1, Math.min(y, this.viewH - 1));
      const row = this.rowAtCanvasY(hy);
      const col = Math.min(this.colCount - 1, this.colAtCanvasX(hx));
      if (this.pointDrag && this.pointMode) {
        const p = this.pointDrag;
        this.pointMode.onPick({ r0: Math.min(p.row, row), c0: Math.min(p.col, col), r1: Math.max(p.row, row), c1: Math.max(p.col, col) }, true);
      } else if (this.fillDrag) {
        const s = this.fillDrag.src;
        // Extend along the axis where the pointer is farthest outside the selection.
        const down = row > s.r1 ? row - s.r1 : row < s.r0 ? s.r0 - row : 0;
        const across = col > s.c1 ? col - s.c1 : col < s.c0 ? s.c0 - col : 0;
        let dst = { ...s };
        if (down >= across && down > 0) dst = row > s.r1 ? { ...s, r1: row } : { ...s, r0: row };
        else if (across > 0) dst = col > s.c1 ? { ...s, c1: col } : { ...s, c0: col };
        this.fillDrag.dst = dst;
        this.render();
      }
      if ((y < HH + 2 || y > this.viewH - 4 || x > this.viewW - 4) && !this.autoScroll) this.autoScrollLoop();
      return;
    }
    if (!this.drag) {
      // The pointer may be over the scroll spacer (a child of the scroller), so test by position.
      const { x, y } = this.lastMouse;
      if (x >= 0 && y >= 0 && x < this.viewW && y < this.viewH && this.scroller.contains(e.target as Node)) {
        const h = this.hitAt(x, y);
        this.scroller.style.cursor =
          h.area === "col" && h.resize !== null
            ? "col-resize"
            : h.area === "row" && h.resize !== null
              ? "row-resize"
              : (h.area === "col" || h.area === "cell") && h.filterBtn
                ? "pointer"
                : h.area === "fill"
                  ? "crosshair"
                  : "cell";
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
    if (this.drag.kind === "cells") {
      // Dragging over a merged cell extends the selection over all of it.
      let { row, col } = h;
      const m = this.mergeAt(row, col);
      const s = this.drag.start;
      if (m) {
        row = row >= s.row ? m.r1 : m.r0;
        col = col >= s.col ? m.c1 : m.c0;
      }
      this.active = { row, col };
    } else if (this.drag.kind === "cols") this.active = { row: 0, col: h.col };
    else this.active = { row: h.row, col: 0 };
    this.render();
    this.onSelectionChange();
  }

  private autoScrollLoop() {
    const step = () => {
      if (!this.drag && !this.fillDrag && !this.pointDrag) {
        this.autoScroll = 0;
        return;
      }
      const { x, y } = this.lastMouse;
      let moved = false;
      if (y < HH) {
        this.setTopPx(this.topPx() - this.defRowH);
        moved = true;
      } else if (y > this.viewH - 4) {
        this.setTopPx(this.topPx() + this.defRowH);
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
      if (this.drag) this.dragTo();
      else window.dispatchEvent(new MouseEvent("mousemove", { clientX: this.lastMouse.x + this.scroller.getBoundingClientRect().left, clientY: this.lastMouse.y + this.scroller.getBoundingClientRect().top, buttons: 1 }));
      this.autoScroll = requestAnimationFrame(step);
    };
    this.autoScroll = requestAnimationFrame(step);
  }

  private onMouseUp() {
    if (this.fillDrag) {
      const { src, dst } = this.fillDrag;
      this.fillDrag = null;
      this.render();
      if (dst.r0 !== src.r0 || dst.r1 !== src.r1 || dst.c0 !== src.c0 || dst.c1 !== src.c1) this.onFill(src, dst);
    }
    this.pointDrag = null;
    if (this.resize) {
      const z = this.resize;
      if (z.kind === "col") this.onColResize(z.index, this.colWidth(z.index));
      else this.onRowResize(z.index, this.rowHeight(z.index));
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

  private fontFor(st: CellStyle | undefined, dxf?: CellStyle): string {
    if (!st && !dxf) return this.font;
    const bold = dxf?.bold || st?.bold;
    const italic = dxf?.italic || st?.italic;
    const pt = st?.fontSize ?? this.basePt;
    const name = st?.fontName ?? "";
    const key = `${bold ? 1 : 0}${italic ? 1 : 0}|${pt}|${name}`;
    let f = this.fontCache.get(key);
    if (!f) {
      const px = Math.max(6, Math.round((BASE_FONT_PX * pt) / this.basePt * 10) / 10);
      const fam = name ? `"${name.replace(/"/g, "")}", ${FONT_STACK}` : FONT_STACK;
      f = `${italic ? "italic " : ""}${bold ? "bold " : ""}${px}px ${fam}`;
      this.fontCache.set(key, f);
    }
    return f;
  }

  private wrapLines(text: string, width: number, font: string): string[] {
    const key = `${font}|${width}|${text}`;
    let lines = this.wrapCache.get(key);
    if (lines) return lines;
    const ctx = this.ctx;
    lines = [];
    for (const para of text.split("\n")) {
      const words = para.split(/(\s+)/);
      let line = "";
      for (const w of words) {
        const next = line + w;
        if (line && ctx.measureText(next).width > width) {
          lines.push(line.trimEnd());
          line = w.trimStart();
          // Break words longer than the cell.
          while (line && ctx.measureText(line).width > width && line.length > 1) {
            let cut = line.length - 1;
            while (cut > 1 && ctx.measureText(line.slice(0, cut)).width > width) cut--;
            lines.push(line.slice(0, cut));
            line = line.slice(cut);
          }
        } else line = next;
      }
      lines.push(line);
    }
    if (this.wrapCache.size > 4000) this.wrapCache.clear();
    this.wrapCache.set(key, lines);
    return lines;
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
    const RHW = this.rowHeaderW;
    const FR = Math.min(this.frozenRows, this.rowCount);
    const FC = Math.min(this.frozenCols, this.colCount);
    const fH = this.frozenH;
    const fW = this.frozenW;

    // Scrolled ranges.
    const firstRow = Math.max(FR, this.rowAtY(top + fH));
    const lastRow = Math.min(this.rowCount - 1, this.rowAtY(top + H - HH));
    const firstCol = Math.max(FC, this.colAtX(left + fW));
    let lastCol = firstCol;
    while (lastCol < this.colCount - 1 && this.colX[lastCol + 1] < left + W - RHW) lastCol++;

    // Frozen and scrolled parts are drawn separately, unless they are contiguous (not scrolled),
    // which lets text overflow across the freeze line like in Excel.
    const rowRanges: [number, number, number, number][] = []; // r0, r1, clipY, clipH
    if (FR > 0 && firstRow === FR) rowRanges.push([0, lastRow, HH, H - HH]);
    else {
      if (FR > 0) rowRanges.push([0, FR - 1, HH, Math.min(fH, H - HH)]);
      rowRanges.push([firstRow, lastRow, HH + fH, H - HH - fH]);
    }
    const colRanges: [number, number, number, number][] = [];
    if (FC > 0 && left === 0) colRanges.push([0, lastCol, RHW, W - RHW]);
    else {
      if (FC > 0) colRanges.push([0, FC - 1, RHW, Math.min(fW, W - RHW)]);
      colRanges.push([firstCol, lastCol, RHW + fW, W - RHW - fW]);
    }

    for (const [r0, r1, cy, ch] of rowRanges) {
      for (const [c0, c1, cx, cw] of colRanges) {
        if (ch <= 0 || cw <= 0 || r1 < r0 || c1 < c0) continue;
        ctx.save();
        ctx.beginPath();
        ctx.rect(cx, cy, cw, ch);
        ctx.clip();
        this.drawCells(r0, r1, c0, c1, top);
        ctx.restore();
      }
    }

    const sel = this.selection;
    const multi = (sel.r0 !== sel.r1 || sel.c0 !== sel.c1) && !this.isMergeSelection();
    const xOf = (c: number) => this.xOf(c);
    const yOf = (r: number) => this.yOf(r, top);

    ctx.save();
    ctx.beginPath();
    ctx.rect(RHW, HH, W - RHW, H - HH);
    ctx.clip();
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
    // Fill-handle drag target.
    if (this.fillDrag) {
      const q = this.fillDrag.dst;
      ctx.save();
      ctx.setLineDash([3, 3]);
      ctx.strokeStyle = C.muted;
      ctx.lineWidth = 1.5;
      ctx.strokeRect(xOf(q.c0) + 1, yOf(q.r0) + 1, xOf(q.c1 + 1) - xOf(q.c0) - 2, yOf(q.r1 + 1) - yOf(q.r0) - 2);
      ctx.restore();
    }
    // References of the formula being edited, in their colors.
    for (const h of this.refHighlights) {
      const q = h.rect;
      const x0 = xOf(q.c0);
      const y0 = yOf(q.r0);
      const w = xOf(q.c1 + 1) - x0;
      const hgt = yOf(q.r1 + 1) - y0;
      ctx.save();
      ctx.globalAlpha = 0.12;
      ctx.fillStyle = h.color;
      ctx.fillRect(x0, y0, w, hgt);
      ctx.globalAlpha = 1;
      ctx.strokeStyle = h.color;
      ctx.lineWidth = 2;
      ctx.strokeRect(x0 + 1, y0 + 1, w - 2, hgt - 2);
      ctx.restore();
    }
    // Other windows on this file: their selection, active cell and a name tag.
    for (const p of this.peerCursors) {
      if (p.sheet !== this.sheet) continue;
      const q = p.rect;
      const x0 = xOf(q.c0);
      const y0 = yOf(q.r0);
      ctx.save();
      ctx.globalAlpha = 0.1;
      ctx.fillStyle = p.color;
      ctx.fillRect(x0, y0, xOf(q.c1 + 1) - x0, yOf(q.r1 + 1) - y0);
      ctx.globalAlpha = 1;
      const a = this.cellRect(p.active.row, p.active.col);
      ctx.strokeStyle = p.color;
      ctx.lineWidth = 2;
      ctx.strokeRect(a.x + 1, a.y + 1, a.w - 2, a.h - 2);
      ctx.font = `600 10px ${FONT_STACK}`;
      const tw = ctx.measureText(p.label).width + 8;
      const ty = a.y - 14 >= HH ? a.y - 14 : a.y + a.h;
      ctx.fillStyle = p.color;
      ctx.fillRect(a.x, ty, tw, 14);
      ctx.fillStyle = "#fff";
      ctx.textBaseline = "middle";
      ctx.textAlign = "left";
      ctx.fillText(p.label, a.x + 4, ty + 7.5);
      ctx.restore();
    }
    // Selection border and active cell.
    const sx1 = xOf(sel.c1 + 1);
    const sy1 = yOf(sel.r1 + 1);
    if (sx1 > RHW && sy1 > HH) {
      ctx.strokeStyle = C.accent;
      ctx.lineWidth = multi ? 1 : 2;
      ctx.strokeRect(xOf(sel.c0) + 0.5, yOf(sel.r0) + 0.5, sx1 - xOf(sel.c0) - 1, sy1 - yOf(sel.r0) - 1);
      if (multi) {
        const a = this.cellRect(this.active.row, this.active.col);
        ctx.lineWidth = 2;
        ctx.strokeRect(a.x + 1, a.y + 1, a.w - 2, a.h - 2);
      }
      // Fill handle dot.
      ctx.fillStyle = C.accent;
      ctx.fillRect(sx1 - 3, sy1 - 3, 6, 6);
    }
    // Frozen pane dividers.
    ctx.strokeStyle = C.muted;
    ctx.lineWidth = 1;
    ctx.beginPath();
    if (FR > 0) {
      ctx.moveTo(RHW, HH + fH + 0.5);
      ctx.lineTo(W, HH + fH + 0.5);
    }
    if (FC > 0) {
      ctx.moveTo(RHW + fW + 0.5, HH);
      ctx.lineTo(RHW + fW + 0.5, H);
    }
    ctx.stroke();
    ctx.restore();

    this.drawHeaders(rowRanges, colRanges, top);
  }

  /** Draw a rectangle of cells: fills, grid lines, selection tint, text, borders. */
  private drawCells(r0: number, r1: number, c0: number, c1: number, top: number) {
    const ctx = this.ctx;
    const C = this.colors;
    const sel = this.selection;
    const multi = (sel.r0 !== sel.r1 || sel.c0 !== sel.c1) && !this.isMergeSelection();
    const xOf = (c: number) => this.xOf(c);
    const yOf = (r: number) => this.yOf(r, top);
    const bgRgb = parseColor(C.bg);

    // Merged ranges that start outside this region but reach into it are drawn too.
    const merges = this.merges.filter((m) => m.r1 >= r0 && m.r0 <= r1 && m.c1 >= c0 && m.c0 <= c1);
    const coveredBy = (r: number, c: number) => this.mergeAt(r, c);

    // 1. Grid lines.
    if (!this.hideGrid) {
      ctx.strokeStyle = C.grid;
      ctx.lineWidth = 1;
      ctx.beginPath();
      const xa = xOf(c0);
      const xb = xOf(c1 + 1);
      const ya = yOf(r0);
      const yb = yOf(r1 + 1);
      for (let r = r0; r <= r1 + 1; r++) {
        if (r <= r1 && this.rowHeight(r) === 0) continue;
        const y = Math.round(yOf(r)) + 0.5;
        ctx.moveTo(xa, y);
        ctx.lineTo(xb, y);
      }
      for (let c = c0; c <= c1 + 1; c++) {
        if (c <= c1 && this.colWidth(c) === 0) continue;
        const x = Math.round(xOf(c)) + 0.5;
        ctx.moveTo(x, ya);
        ctx.lineTo(x, yb);
      }
      ctx.stroke();
    }

    // 2. Fills (cover grid lines like Excel) and merged areas.
    const fillOf = (cell: CellData | null): number | undefined => {
      if (!cell) return undefined;
      const d = cell.cf ? this.dxfs[cell.cf - 1] : undefined;
      return d?.fill ?? this.styles[cell.style]?.fill;
    };
    for (let r = r0; r <= r1; r++) {
      const h = this.rowHeight(r);
      if (h === 0) continue;
      for (let c = c0; c <= c1; c++) {
        if (coveredBy(r, c)) continue;
        const cell = this.cellAt(r, c);
        const f = fillOf(cell);
        if (f === undefined) continue;
        const x = Math.round(xOf(c));
        const y = Math.round(yOf(r));
        ctx.fillStyle = hex(f);
        ctx.fillRect(x, y, Math.round(xOf(c + 1)) - x + 1, Math.round(yOf(r + 1)) - y + 1);
      }
    }
    for (const m of merges) {
      const x = Math.round(xOf(m.c0));
      const y = Math.round(yOf(m.r0));
      const w = Math.round(xOf(m.c1 + 1)) - x;
      const h = Math.round(yOf(m.r1 + 1)) - y;
      const f = fillOf(this.cellAt(m.r0, m.c0));
      ctx.fillStyle = f === undefined ? C.bg : hex(f);
      ctx.fillRect(x + 1, y + 1, w - 1, h - 1);
      if (f !== undefined) ctx.fillRect(x, y, w + 1, h + 1);
    }

    // 3. Selection tint.
    if (multi) {
      const sx0 = xOf(sel.c0);
      const sy0 = yOf(sel.r0);
      const sx1 = xOf(sel.c1 + 1);
      const sy1 = yOf(sel.r1 + 1);
      if (sx1 > sx0 && sy1 > sy0) {
        ctx.fillStyle = C.accentSoft;
        ctx.fillRect(sx0, sy0, sx1 - sx0, sy1 - sy0);
      }
    }

    // 4. Text.
    ctx.textBaseline = "alphabetic";
    const drawText = (cell: CellData, x: number, y: number, w: number, h: number, r: number, c: number, mergedW: boolean) => {
      const st = this.styles[cell.style];
      const dxf = cell.cf ? this.dxfs[cell.cf - 1] : undefined;
      const font = this.fontFor(st, dxf);
      ctx.font = font;
      const kind = cell.kind & 7;
      const fill = dxf?.fill ?? st?.fill;
      const colorNum = dxf?.color ?? st?.color;
      let color: string;
      if (kind === 4) color = C.error;
      else if (fill !== undefined) {
        const fr = rgbOf(fill);
        color = colorNum !== undefined ? hex(colorNum) : luminance(fr) > 0.35 ? "#000000" : "#ffffff";
      } else if (colorNum !== undefined) color = C.dark ? readableOn(colorNum, bgRgb) : hex(colorNum);
      else color = C.text;
      let align: "left" | "right" | "center" = "left";
      const ha = st?.hAlign ?? "general";
      if (ha === "general") align = kind === 1 ? "right" : kind === 3 || kind === 4 ? "center" : "left";
      else if (ha === "center" || ha === "centeracross" || ha === "distributed") align = "center";
      else if (ha === "right") align = "right";
      const indent = (st?.indent ?? 0) * 9;
      const pxSize = parseFloat(/(\d+(?:\.\d+)?)px/.exec(font)?.[1] ?? "13");
      const lh = Math.round(pxSize * 1.25);
      const wrap = !!st?.wrap || ha === "justify";
      const lines = wrap ? this.wrapLines(cell.text, Math.max(4, w - 6 - indent), font) : cell.text.split("\n").slice(0, 1);
      let clipW = w;
      let clipX = x;
      const tw = !wrap ? ctx.measureText(lines[0]).width : 0;
      if (!wrap && !mergedW && kind !== 1 && tw > w - 6) {
        // Let text overflow into empty neighbours, like Excel.
        const spill = (dir: 1 | -1, need: number) => {
          let cc = c + dir;
          let got = 0;
          while (got < need && cc >= 0 && cc < this.colCount && cc <= c1 + 1 && cc >= c0 - 1) {
            const n = this.cellAt(r, cc);
            if ((n && n.text) || coveredBy(r, cc)) break;
            got += this.colWidth(cc);
            cc += dir;
          }
          return got;
        };
        if (align === "left") clipW += spill(1, tw + 6 - w);
        else if (align === "right") {
          const g = spill(-1, tw + 6 - w);
          clipX -= g;
          clipW += g;
        } else {
          const half = (tw + 6 - w) / 2;
          const gl = spill(-1, half);
          const gr = spill(1, half);
          clipX -= gl;
          clipW += gl + gr;
        }
      }
      ctx.save();
      ctx.beginPath();
      ctx.rect(clipX + 1, y + 1, clipW - 2, h - 1);
      ctx.clip();
      if (clipW > w) {
        // Cover the grid lines the overflow crosses.
        ctx.fillStyle = C.bg;
        if (clipX < x) ctx.fillRect(clipX + 1, y + 1, x - clipX - 1, h - 1);
        if (clipX + clipW > x + w) ctx.fillRect(x + w, y + 1, clipX + clipW - x - w - 1, h - 1);
        if (multi) {
          ctx.fillStyle = C.accentSoft;
          ctx.fillRect(clipX + 1, y + 1, clipW - 2, h - 1);
        }
      }
      ctx.fillStyle = color;
      ctx.textAlign = align;
      const tx = align === "left" ? x + 4 + indent : align === "right" ? x + w - 4 - indent : x + w / 2;
      const total = lines.length * lh;
      const va = st?.vAlign ?? "bottom";
      const asc = Math.round(pxSize * 0.8);
      let ty: number;
      if (va === "top") ty = y + 3 + asc;
      else if (va === "center" || va === "justify" || va === "distributed") ty = y + (h - total) / 2 + asc + (lh - pxSize) / 2;
      else ty = y + h - 4 - total + lh - (lh - pxSize) / 2 - 0;
      // Never start above the cell for bottom/center alignment.
      if (va !== "top" && ty < y + 2 + asc) ty = y + 2 + asc;
      const deco = st?.underline || st?.strike || dxf?.underline || dxf?.strike;
      for (const line of lines) {
        ctx.fillText(line, tx, ty);
        if (deco) {
          const lw = ctx.measureText(line).width;
          const lx = align === "left" ? tx : align === "right" ? tx - lw : tx - lw / 2;
          ctx.fillRect(lx, st?.strike || dxf?.strike ? ty - pxSize * 0.3 : ty + 2, lw, 1);
        }
        ty += lh;
        if (ty - asc > y + h) break;
      }
      ctx.restore();
    };

    for (let r = r0; r <= r1; r++) {
      const h = this.rowHeight(r);
      if (h === 0) continue;
      const y = yOf(r);
      for (let c = c0; c <= c1; c++) {
        if (coveredBy(r, c)) continue;
        const w = this.colWidth(c);
        if (w === 0) continue;
        const cell = this.cellAt(r, c);
        if (!cell || !cell.text) continue;
        drawText(cell, xOf(c), y, w, h, r, c, false);
      }
    }
    for (const m of merges) {
      const cell = this.cellAt(m.r0, m.c0);
      if (!cell || !cell.text) continue;
      const x = xOf(m.c0);
      const y = yOf(m.r0);
      drawText(cell, x, y, xOf(m.c1 + 1) - x, yOf(m.r1 + 1) - y, m.r0, m.c0, true);
    }

    // 5. Borders.
    const lineWidth = (b: BorderLine) =>
      b.style === "medium" || b.style === "mediumDashed" || b.style === "mediumDashDot" || b.style === "mediumDashDotDot" || b.style === "slantDashDot"
        ? 2
        : b.style === "thick"
          ? 3
          : 1;
    const dash = (b: BorderLine): number[] =>
      b.style === "dashed" || b.style === "mediumDashed"
        ? [4, 2]
        : b.style === "dotted" || b.style === "hair"
          ? [1, 2]
          : b.style.startsWith("dashDot") || b.style.startsWith("mediumDashDot")
            ? [5, 2, 1, 2]
            : [];
    const stroke = (b: BorderLine, x0: number, y0: number, x1: number, y1: number, fillBehind: number | undefined) => {
      const lw = lineWidth(b);
      let col = hex(b.color);
      if (C.dark && fillBehind === undefined) col = readableOn(b.color, bgRgb, 1.8);
      ctx.strokeStyle = col;
      ctx.lineWidth = lw;
      ctx.setLineDash(dash(b));
      const off = lw % 2 ? 0.5 : 0;
      ctx.beginPath();
      if (y0 === y1) {
        ctx.moveTo(x0, Math.round(y0) + off);
        ctx.lineTo(x1, Math.round(y1) + off);
      } else {
        ctx.moveTo(Math.round(x0) + off, y0);
        ctx.lineTo(Math.round(x1) + off, y1);
      }
      ctx.stroke();
      if (b.style === "double") {
        ctx.beginPath();
        if (y0 === y1) {
          ctx.moveTo(x0, Math.round(y0) + 2.5);
          ctx.lineTo(x1, Math.round(y1) + 2.5);
        } else {
          ctx.moveTo(Math.round(x0) + 2.5, y0);
          ctx.lineTo(Math.round(x1) + 2.5, y1);
        }
        ctx.stroke();
      }
    };
    for (let r = r0; r <= r1; r++) {
      if (this.rowHeight(r) === 0) continue;
      for (let c = c0; c <= c1; c++) {
        const cell = this.cellAt(r, c);
        if (!cell || !cell.style) continue;
        const st = this.styles[cell.style];
        if (!st || !st.borders || !st.borders.some((b) => b)) continue;
        const m = coveredBy(r, c);
        const [bl, br, bt, bb] = st.borders;
        const x0 = xOf(c);
        const x1 = xOf(c + 1);
        const y0 = yOf(r);
        const y1 = yOf(r + 1);
        const f = st.fill;
        if (bl && (!m || c === m.c0)) stroke(bl, x0, y0, x0, y1, f);
        if (br && (!m || c === m.c1)) stroke(br, x1, y0, x1, y1, f);
        if (bt && (!m || r === m.r0)) stroke(bt, x0, y0, x1, y0, f);
        if (bb && (!m || r === m.r1)) stroke(bb, x0, y1, x1, y1, f);
      }
    }
    ctx.setLineDash([]);

    // 6. Autofilter buttons from the file.
    const af = this.autofilter;
    if (af && af.row >= r0 && af.row <= r1) {
      for (let c = Math.max(af.c0, c0); c <= Math.min(af.c1, c1); c++) {
        if (this.colWidth(c) === 0) continue;
        const rect = this.cellRect(af.row, c);
        this.drawFilterButton(rect.x + rect.w - 18, rect.y + rect.h - 18, this.filterCols.has(c));
      }
    }
  }

  private drawHeaders(rowRanges: [number, number, number, number][], colRanges: [number, number, number, number][], top: number) {
    const ctx = this.ctx;
    const C = this.colors;
    const W = this.viewW;
    const H = this.viewH;
    const RHW = this.rowHeaderW;
    const sel = this.selection;
    const xOf = (c: number) => this.xOf(c);
    const yOf = (r: number) => this.yOf(r, top);
    const allRows = sel.r0 === 0 && sel.r1 >= this.rowCount - 1;
    const allCols = sel.c0 === 0 && sel.c1 >= this.colCount - 1;

    // Column headers.
    ctx.fillStyle = C.headerBg;
    ctx.fillRect(0, 0, W, HH);
    ctx.font = this.headerFont;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    for (const [c0, c1, cx, cw] of colRanges) {
      if (cw <= 0 || c1 < c0) continue;
      ctx.save();
      ctx.beginPath();
      ctx.rect(cx, 0, cw, HH);
      ctx.clip();
      for (let c = c0; c <= c1; c++) {
        const x = xOf(c);
        const w = this.colWidth(c);
        if (w === 0) continue;
        if (c >= sel.c0 && c <= sel.c1) {
          ctx.fillStyle = allRows ? C.accentStrong : C.headerActive;
          ctx.fillRect(x, 0, w, HH);
        }
        ctx.fillStyle = this.filterCols.has(c) ? C.filtered : C.muted;
        ctx.fillText(colName(c), x + w / 2, HH / 2 + 0.5);
        if (this.showFilterButtons && !this.autofilter && c < Math.max(this.dataCols, 1)) this.drawFilterButton(x + w - 18, 4, this.filterCols.has(c));
      }
      ctx.strokeStyle = C.border;
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let c = c0; c <= c1 + 1; c++) {
        const x = Math.round(xOf(c)) + 0.5;
        ctx.moveTo(x, 0);
        ctx.lineTo(x, HH);
      }
      ctx.stroke();
      ctx.restore();
    }
    ctx.strokeStyle = C.border;
    ctx.beginPath();
    ctx.moveTo(0, HH - 0.5);
    ctx.lineTo(W, HH - 0.5);
    ctx.stroke();

    // Row headers.
    ctx.fillStyle = C.headerBg;
    ctx.fillRect(0, HH, RHW, H - HH);
    ctx.textAlign = "right";
    for (const [r0, r1, cy, ch] of rowRanges) {
      if (ch <= 0 || r1 < r0) continue;
      ctx.save();
      ctx.beginPath();
      ctx.rect(0, cy, RHW, ch);
      ctx.clip();
      for (let r = r0; r <= r1; r++) {
        const y = yOf(r);
        const h = this.rowHeight(r);
        if (h === 0) continue;
        if (r >= sel.r0 && r <= sel.r1) {
          ctx.fillStyle = allCols ? C.accentStrong : C.headerActive;
          ctx.fillRect(0, y, RHW, h);
        }
        const dr = this.dataRowOf(r);
        if (dr === null) continue;
        ctx.fillStyle = this.filtered ? C.filtered : C.muted;
        ctx.fillText(String(dr + 1), RHW - 7, y + Math.min(h, 22) / 2 + 0.5);
      }
      ctx.strokeStyle = C.border;
      ctx.beginPath();
      for (let r = r0; r <= r1 + 1; r++) {
        const y = Math.round(yOf(r)) + 0.5;
        ctx.moveTo(0, y);
        ctx.lineTo(RHW, y);
      }
      ctx.stroke();
      ctx.restore();
    }
    ctx.strokeStyle = C.border;
    ctx.beginPath();
    ctx.moveTo(RHW - 0.5, 0);
    ctx.lineTo(RHW - 0.5, H);
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
    ctx.fillStyle = active ? C.filtered : C.headerBg;
    ctx.strokeStyle = active ? C.filtered : C.border;
    ctx.lineWidth = 1;
    ctx.setLineDash([]);
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

  /** Font, alignment and colors for the in-cell editor. */
  editorStyle(p: Pos): { font: string; align: string; wrap: boolean } {
    const c = this.cellAt(p.row, p.col);
    const st = c ? this.styles[c.style] : undefined;
    const ha = st?.hAlign ?? "general";
    return {
      font: this.fontFor(st),
      align: ha === "center" || ha === "right" ? ha : "left",
      wrap: !!st?.wrap,
    };
  }
}
