// Drawing the grid on its canvas: cells (fills, grid lines, text with Excel's overflow, borders),
// the selection, copy and fill marquees, formula references, other windows' cursors, headers.
// It only reads the grid's state; everything it draws comes from Grid.
import type { BorderLine, CellStyle } from "./api";
import { colName } from "./cells";
import { hex, luminance, parseColor, readableOn, rgbOf } from "./color";
import type { CellData, Grid } from "./grid";

/** Height of the column header row. */
export const HH = 24;
export const BASE_FONT_PX = 13;
export const FONT_STACK = 'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';

export class GridPainter {
  private font = `${BASE_FONT_PX}px ${FONT_STACK}`;
  private headerFont = `12px ${FONT_STACK}`;
  private fontCache = new Map<string, string>();
  private wrapCache = new Map<string, string[]>();

  constructor(private readonly g: Grid) {}

  /** The workbook's styles changed: fonts and wrapped lines are measured again. */
  clearCaches() {
    this.fontCache.clear();
    this.wrapCache.clear();
  }

  fontFor(st: CellStyle | undefined, dxf?: CellStyle): string {
    if (!st && !dxf) return this.font;
    const bold = dxf?.bold || st?.bold;
    const italic = dxf?.italic || st?.italic;
    const pt = st?.fontSize ?? this.g.basePt;
    const name = st?.fontName ?? "";
    const key = `${bold ? 1 : 0}${italic ? 1 : 0}|${pt}|${name}`;
    let f = this.fontCache.get(key);
    if (!f) {
      const px = Math.max(6, Math.round((BASE_FONT_PX * pt) / this.g.basePt * 10) / 10);
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
    const ctx = this.g.ctx;
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

  draw() {
    const dpr = window.devicePixelRatio || 1;
    const W = this.g.viewW;
    const H = this.g.viewH;
    if (this.g.canvas.width !== Math.round(W * dpr) || this.g.canvas.height !== Math.round(H * dpr)) {
      this.g.canvas.width = Math.round(W * dpr);
      this.g.canvas.height = Math.round(H * dpr);
      this.g.canvas.style.width = `${W}px`;
      this.g.canvas.style.height = `${H}px`;
    }
    const ctx = this.g.ctx;
    const C = this.g.colors;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = C.bg;
    ctx.fillRect(0, 0, W, H);

    const top = this.g.topPx();
    const left = this.g.scroller.scrollLeft;
    const RHW = this.g.rowHeaderW;
    const FR = Math.min(this.g.frozenRows, this.g.rowCount);
    const FC = Math.min(this.g.frozenCols, this.g.colCount);
    const fH = this.g.frozenH;
    const fW = this.g.frozenW;

    // Scrolled ranges.
    const firstRow = Math.max(FR, this.g.rowAtY(top + fH));
    const lastRow = Math.min(this.g.rowCount - 1, this.g.rowAtY(top + H - HH));
    const firstCol = Math.max(FC, this.g.colAtX(left + fW));
    let lastCol = firstCol;
    while (lastCol < this.g.colCount - 1 && this.g.colX[lastCol + 1] < left + W - RHW) lastCol++;

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

    const sel = this.g.selection;
    const multi = (sel.r0 !== sel.r1 || sel.c0 !== sel.c1) && !this.g.isMergeSelection();
    const xOf = (c: number) => this.g.xOf(c);
    const yOf = (r: number) => this.g.yOf(r, top);

    ctx.save();
    ctx.beginPath();
    ctx.rect(RHW, HH, W - RHW, H - HH);
    ctx.clip();
    // Copy marquee.
    if (this.g.copyRect) {
      const q = this.g.copyRect;
      ctx.save();
      ctx.setLineDash([5, 3]);
      ctx.strokeStyle = C.accent;
      ctx.lineWidth = 1.5;
      ctx.strokeRect(xOf(q.c0) + 1, yOf(q.r0) + 1, xOf(q.c1 + 1) - xOf(q.c0) - 2, yOf(q.r1 + 1) - yOf(q.r0) - 2);
      ctx.restore();
    }
    // Fill-handle drag target.
    if (this.g.mouse.fillDrag) {
      const q = this.g.mouse.fillDrag.dst;
      ctx.save();
      ctx.setLineDash([3, 3]);
      ctx.strokeStyle = C.muted;
      ctx.lineWidth = 1.5;
      ctx.strokeRect(xOf(q.c0) + 1, yOf(q.r0) + 1, xOf(q.c1 + 1) - xOf(q.c0) - 2, yOf(q.r1 + 1) - yOf(q.r0) - 2);
      ctx.restore();
    }
    // References of the formula being edited, in their colors.
    for (const h of this.g.refHighlights) {
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
    for (const p of this.g.peerCursors) {
      if (p.sheet !== this.g.sheet) continue;
      const q = p.rect;
      const x0 = xOf(q.c0);
      const y0 = yOf(q.r0);
      ctx.save();
      ctx.globalAlpha = 0.1;
      ctx.fillStyle = p.color;
      ctx.fillRect(x0, y0, xOf(q.c1 + 1) - x0, yOf(q.r1 + 1) - y0);
      ctx.globalAlpha = 1;
      const a = this.g.cellRect(p.active.row, p.active.col);
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
        const a = this.g.cellRect(this.g.active.row, this.g.active.col);
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
    const ctx = this.g.ctx;
    const C = this.g.colors;
    const sel = this.g.selection;
    const multi = (sel.r0 !== sel.r1 || sel.c0 !== sel.c1) && !this.g.isMergeSelection();
    const xOf = (c: number) => this.g.xOf(c);
    const yOf = (r: number) => this.g.yOf(r, top);
    const bgRgb = parseColor(C.bg);

    // Merged ranges that start outside this region but reach into it are drawn too.
    const merges = this.g.merges.filter((m) => m.r1 >= r0 && m.r0 <= r1 && m.c1 >= c0 && m.c0 <= c1);
    const coveredBy = (r: number, c: number) => this.g.mergeAt(r, c);

    // 1. Grid lines.
    if (!this.g.hideGrid) {
      ctx.strokeStyle = C.grid;
      ctx.lineWidth = 1;
      ctx.beginPath();
      const xa = xOf(c0);
      const xb = xOf(c1 + 1);
      const ya = yOf(r0);
      const yb = yOf(r1 + 1);
      for (let r = r0; r <= r1 + 1; r++) {
        if (r <= r1 && this.g.rowHeight(r) === 0) continue;
        const y = Math.round(yOf(r)) + 0.5;
        ctx.moveTo(xa, y);
        ctx.lineTo(xb, y);
      }
      for (let c = c0; c <= c1 + 1; c++) {
        if (c <= c1 && this.g.colWidth(c) === 0) continue;
        const x = Math.round(xOf(c)) + 0.5;
        ctx.moveTo(x, ya);
        ctx.lineTo(x, yb);
      }
      ctx.stroke();
    }

    // 2. Fills (cover grid lines like Excel) and merged areas.
    const fillOf = (cell: CellData | null): number | undefined => {
      if (!cell) return undefined;
      const d = cell.cf ? this.g.dxfs[cell.cf - 1] : undefined;
      return d?.fill ?? this.g.styles[cell.style]?.fill;
    };
    for (let r = r0; r <= r1; r++) {
      const h = this.g.rowHeight(r);
      if (h === 0) continue;
      for (let c = c0; c <= c1; c++) {
        if (coveredBy(r, c)) continue;
        const cell = this.g.cellAt(r, c);
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
      const f = fillOf(this.g.cellAt(m.r0, m.c0));
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
      const st = this.g.styles[cell.style];
      const dxf = cell.cf ? this.g.dxfs[cell.cf - 1] : undefined;
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
          while (got < need && cc >= 0 && cc < this.g.colCount && cc <= c1 + 1 && cc >= c0 - 1) {
            const n = this.g.cellAt(r, cc);
            if ((n && n.text) || coveredBy(r, cc)) break;
            got += this.g.colWidth(cc);
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
      const h = this.g.rowHeight(r);
      if (h === 0) continue;
      const y = yOf(r);
      for (let c = c0; c <= c1; c++) {
        if (coveredBy(r, c)) continue;
        const w = this.g.colWidth(c);
        if (w === 0) continue;
        const cell = this.g.cellAt(r, c);
        if (!cell || !cell.text) continue;
        drawText(cell, xOf(c), y, w, h, r, c, false);
      }
    }
    for (const m of merges) {
      const cell = this.g.cellAt(m.r0, m.c0);
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
      if (this.g.rowHeight(r) === 0) continue;
      for (let c = c0; c <= c1; c++) {
        const cell = this.g.cellAt(r, c);
        if (!cell || !cell.style) continue;
        const st = this.g.styles[cell.style];
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
    const af = this.g.autofilter;
    if (af && af.row >= r0 && af.row <= r1) {
      for (let c = Math.max(af.c0, c0); c <= Math.min(af.c1, c1); c++) {
        if (this.g.colWidth(c) === 0) continue;
        const rect = this.g.cellRect(af.row, c);
        this.drawFilterButton(rect.x + rect.w - 18, rect.y + rect.h - 18, this.g.filterCols.has(c));
      }
    }
  }

  private drawHeaders(rowRanges: [number, number, number, number][], colRanges: [number, number, number, number][], top: number) {
    const ctx = this.g.ctx;
    const C = this.g.colors;
    const W = this.g.viewW;
    const H = this.g.viewH;
    const RHW = this.g.rowHeaderW;
    const sel = this.g.selection;
    const xOf = (c: number) => this.g.xOf(c);
    const yOf = (r: number) => this.g.yOf(r, top);
    const allRows = sel.r0 === 0 && sel.r1 >= this.g.rowCount - 1;
    const allCols = sel.c0 === 0 && sel.c1 >= this.g.colCount - 1;

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
        const w = this.g.colWidth(c);
        if (w === 0) continue;
        if (c >= sel.c0 && c <= sel.c1) {
          ctx.fillStyle = allRows ? C.accentStrong : C.headerActive;
          ctx.fillRect(x, 0, w, HH);
        }
        ctx.fillStyle = this.g.filterCols.has(c) ? C.filtered : C.muted;
        ctx.fillText(colName(c), x + w / 2, HH / 2 + 0.5);
        if (this.g.showFilterButtons && !this.g.autofilter && c < Math.max(this.g.dataCols, 1)) this.drawFilterButton(x + w - 18, 4, this.g.filterCols.has(c));
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
        const h = this.g.rowHeight(r);
        if (h === 0) continue;
        if (r >= sel.r0 && r <= sel.r1) {
          ctx.fillStyle = allCols ? C.accentStrong : C.headerActive;
          ctx.fillRect(0, y, RHW, h);
        }
        const dr = this.g.dataRowOf(r);
        if (dr === null) continue;
        ctx.fillStyle = this.g.filtered ? C.filtered : C.muted;
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
    const ctx = this.g.ctx;
    const C = this.g.colors;
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
  }}
