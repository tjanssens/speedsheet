// Canvas-rendered, virtualized grid. Only the visible cells are ever drawn or fetched, in blocks,
// so a sheet with a million rows scrolls as smoothly as one with ten.
import { call, type Block, type CellStyle, type Layout, type Area } from "./api";
import { advanceWithin, MAX_COLS, MAX_ROWS, rectOf, type Pos, type Rect } from "./cells";
import { luminance, parseColor } from "./color";
import { GridMouse } from "./grid-mouse";
import { GridPainter, HH } from "./grid-paint";
import { FrameTask } from "./schedule";

export { colName, parseAddress, MAX_COLS, MAX_ROWS, type Pos, type Rect } from "./cells";

const BR = 128; // rows per fetched block
const BC = 32; // cols per fetched block
const MAX_SCROLL_PX = 8_000_000; // stay well below browser limits for element sizes
const APP_COL_W = 100;
const APP_ROW_H = 22;

interface CacheEntry {
  gen: number;
  block?: Block;
  pending?: boolean;
}

export interface Colors {
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

export type Hit =
  | { area: "corner" }
  | { area: "col"; col: number; resize: number | null; filterBtn: boolean }
  | { area: "row"; row: number; resize: number | null }
  | { area: "cell"; row: number; col: number; filterBtn: boolean }
  | { area: "fill" };

const EMPTY: CellData = { text: "", kind: 0, style: 0, cf: 0 };


export class Grid {
  readonly host: HTMLElement;
  readonly scroller: HTMLElement;
  private spacer: HTMLElement;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  colors!: Colors;

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
  colX: number[] = [0];
  rowHeaderW = 50;

  // Layout from the file.
  private defColW = APP_COL_W;
  defRowH = APP_ROW_H;
  private rowHeights = new Map<number, number>();
  private rowY: Float64Array | null = null;
  merges: Area[] = [];
  private mergeIndex = new Map<string, Area>();
  frozenRows = 0;
  frozenCols = 0;
  autofilter: Layout["autofilter"] = null;
  hideGrid = false;

  // Styles of the workbook.
  styles: CellStyle[] = [];
  dxfs: CellStyle[] = [];
  basePt = 11;

  active: Pos = { row: 0, col: 0 };
  anchor: Pos = { row: 0, col: 0 };
  /** Optional dashed "marching ants" rectangle around copied cells. */
  copyRect: Rect | null = null;

  private cache = new Map<string, CacheEntry>();
  private gen = 0;
  readonly painter: GridPainter = new GridPainter(this);
  readonly mouse: GridMouse = new GridMouse(this);
  private frame = new FrameTask(() => this.painter.draw());

  onSelectionChange: () => void = () => {};
  private moves = 0;

  /** Counts every change of the selection. Work that finishes later (undo, Ctrl+Arrow) moves the
   * cursor only when this did not change meanwhile: the user's own click wins. */
  get moveCount() {
    return this.moves;
  }

  selectionChanged() {
    this.moves++;
    this.onSelectionChange();
  }
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
    this.scroller.addEventListener("mousedown", (e) => this.mouse.onMouseDown(e));
    window.addEventListener("mousemove", (e) => this.mouse.onMouseMove(e));
    window.addEventListener("mouseup", () => this.mouse.onMouseUp());
    // A release outside the window (or while another window has focus) never sends mouseup:
    // end the drag then too, so the selection doesn't keep following the pointer.
    window.addEventListener("blur", () => this.mouse.cancelDrags());
    this.scroller.addEventListener("dblclick", (e) => {
      const h = this.hit(e);
      if (h.area === "fill") {
        this.mouse.fillDrag = null;
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
    this.painter.clearCaches();
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

  rowAtY(y: number): number {
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

  get viewW() {
    return this.scroller.clientWidth;
  }
  get viewH() {
    return this.scroller.clientHeight;
  }
  private contentH() {
    return this.rowTop(this.rowCount);
  }
  private scaled() {
    return this.contentH() > MAX_SCROLL_PX;
  }
  topPx(): number {
    const st = this.scroller.scrollTop;
    if (!this.scaled()) return st;
    const view = this.viewH - HH;
    return (st * (this.contentH() - view)) / (MAX_SCROLL_PX - view);
  }
  setTopPx(px: number) {
    px = Math.max(0, px);
    if (!this.scaled()) {
      this.scroller.scrollTop = px;
      return;
    }
    const view = this.viewH - HH;
    this.scroller.scrollTop = (px * (MAX_SCROLL_PX - view)) / (this.contentH() - view);
  }
  get frozenH() {
    return this.rowTop(this.frozenRows);
  }
  get frozenW() {
    return this.colX[Math.min(this.frozenCols, this.colCount)] ?? 0;
  }

  private updateSpacer() {
    const digits = String(Math.min(this.rowCount, MAX_ROWS)).length;
    this.rowHeaderW = Math.max(42, 14 + digits * 8);
    this.spacer.style.height = `${HH + Math.min(this.contentH(), MAX_SCROLL_PX)}px`;
    this.spacer.style.width = `${this.rowHeaderW + this.colX[this.colCount]}px`;
  }

  colAtX(px: number): number {
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
  xOf(c: number) {
    return this.rowHeaderW + this.colX[Math.min(c, this.colCount)] - (c >= this.frozenCols ? this.scroller.scrollLeft : 0);
  }
  /** Canvas y of a row's top edge (frozen rows don't scroll). */
  yOf(r: number, top = this.topPx()) {
    return HH + this.rowTop(r) - (r >= this.frozenRows ? top : 0);
  }

  /** Content column under a canvas x. */
  colAtCanvasX(x: number) {
    const cx = x - this.rowHeaderW;
    if (this.frozenCols > 0 && cx < this.frozenW) return this.colAtX(Math.max(0, cx));
    return this.colAtX(Math.max(0, cx + this.scroller.scrollLeft));
  }
  rowAtCanvasY(y: number) {
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

  hit(e: MouseEvent): Hit {
    const r = this.scroller.getBoundingClientRect();
    return this.hitAt(e.clientX - r.left, e.clientY - r.top);
  }

  hitAt(x: number, y: number): Hit {
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
      this.ctx.font = this.painter.fontFor(st, c.cf ? this.dxfs[c.cf - 1] : undefined);
      const lines = st?.wrap ? c.text.split("\n") : [c.text];
      for (const l of lines) w = Math.max(w, this.ctx.measureText(l).width + 12 + (st?.indent ?? 0) * 9);
    }
    this.setColWidth(col, Math.min(w, 800));
    this.onColResize(col, this.colWidth(col));
  }

  // ---------- selection ----------

  get selection(): Rect {
    return rectOf(this.anchor, this.active);
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
    this.selectionChanged();
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
    this.active = advanceWithin(s, this.active, dr, dc);
    this.ensureVisible(this.active);
    this.render();
    this.selectionChanged();
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
    this.selectionChanged();
  }

  pageRows() {
    return Math.max(1, this.visibleRows() - 1);
  }

  // ---------- mouse ----------

  // ---------- drawing ----------

  /** Draw on the next frame (any number of calls in one frame draw once). */
  render() {
    this.frame.request();
  }

  /** A draw is scheduled and has not happened yet. */
  get pendingDraw() {
    return this.frame.pending;
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
      font: this.painter.fontFor(st),
      align: ha === "center" || ha === "right" ? ha : "left",
      wrap: !!st?.wrap,
    };
  }
}
