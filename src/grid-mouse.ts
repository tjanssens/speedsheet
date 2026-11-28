// Mouse on the grid: selecting by clicking and dragging, whole rows and columns, resizing them,
// the fill handle, picking references while typing a formula, and scrolling while dragging.
import { fillTarget, type Pos, type Rect } from "./cells";
import type { Grid } from "./grid";
import { HH } from "./grid-paint";
import { FrameTask } from "./schedule";

export class GridMouse {
  drag: null | { kind: "cells" | "cols" | "rows"; start: Pos } = null;
  resize: null | { kind: "col" | "row"; index: number; start: number; startSize: number } = null;
  /** Dragging the fill handle: the selection being extended and the target so far. */
  fillDrag: null | { src: Rect; dst: Rect } = null;
  pointDrag: null | Pos = null;
  private lastMouse = { x: 0, y: 0 };
  private autoScroll = new FrameTask(() => this.autoScrollStep());

  constructor(private readonly g: Grid) {}

  /** A button is held down for one of the grid's drags. */
  get dragging() {
    return !!(this.drag || this.fillDrag || this.pointDrag || this.resize);
  }

  onMouseDown(e: MouseEvent) {
    if (e.button !== 0) {
      // Right click inside the selection keeps it; elsewhere it selects the cell first.
      const h = this.g.hit(e);
      if (h.area === "cell") {
        const s = this.g.selection;
        if (h.row < s.r0 || h.row > s.r1 || h.col < s.c0 || h.col > s.c1) this.g.select({ row: h.row, col: h.col });
      }
      return;
    }
    const h = this.g.hit(e);
    if (this.g.pointMode && h.area === "cell") {
      // Formula point mode: the click picks a reference; the editor keeps focus.
      e.preventDefault();
      const m = this.g.mergeAt(h.row, h.col);
      const r = m ? { ...m } : { r0: h.row, c0: h.col, r1: h.row, c1: h.col };
      this.pointDrag = { row: h.row, col: h.col };
      this.g.pointMode.onPick(r, false);
      return;
    }
    this.g.scroller.focus({ preventScroll: true });
    if (h.area === "fill") {
      const s = this.g.selection;
      this.fillDrag = { src: s, dst: { ...s } };
      e.preventDefault();
      return;
    }
    if (h.area === "corner") {
      this.g.selectAll();
      return;
    }
    if (h.area === "col") {
      if (h.resize !== null) {
        this.resize = { kind: "col", index: h.resize, start: e.clientX, startSize: this.g.colWidth(h.resize) };
        e.preventDefault();
        return;
      }
      if (h.filterBtn) {
        const rect = this.g.cellRect(0, h.col);
        const host = this.g.scroller.getBoundingClientRect();
        this.g.onFilterButton(h.col, host.left + rect.x + rect.w - 18, host.top + HH);
        e.preventDefault();
        return;
      }
      // Whole column: the active cell stays at the top, the anchor at the bottom.
      const last = Math.max(this.g.rowCount - 1, 0);
      const anchorCol = e.shiftKey ? this.g.anchor.col : h.col;
      this.g.select({ row: 0, col: h.col }, false, { row: last, col: anchorCol });
      this.drag = { kind: "cols", start: { row: 0, col: h.col } };
      e.preventDefault();
      return;
    }
    if (h.area === "row") {
      if (h.resize !== null) {
        this.resize = { kind: "row", index: h.resize, start: e.clientY, startSize: this.g.rowHeight(h.resize) };
        e.preventDefault();
        return;
      }
      const last = Math.max(this.g.colCount - 1, 0);
      const anchorRow = e.shiftKey ? this.g.anchor.row : h.row;
      this.g.select({ row: h.row, col: 0 }, false, { row: anchorRow, col: last });
      this.drag = { kind: "rows", start: { row: h.row, col: 0 } };
      e.preventDefault();
      return;
    }
    if (h.filterBtn) {
      const rect = this.g.cellRect(h.row, h.col);
      const host = this.g.scroller.getBoundingClientRect();
      this.g.onFilterButton(h.col, host.left + rect.x + rect.w - 18, host.top + rect.y + rect.h);
      e.preventDefault();
      return;
    }
    this.g.select({ row: h.row, col: h.col }, e.shiftKey);
    this.drag = { kind: "cells", start: { row: h.row, col: h.col } };
    e.preventDefault();
  }

  /** Stop a drag whose button release never reached us, without applying a fill. */
  cancelDrags() {
    if (!this.drag && !this.fillDrag && !this.pointDrag && !this.resize) return;
    if (this.fillDrag) {
      this.fillDrag = null;
      this.g.render();
    }
    this.onMouseUp();
  }

  onMouseMove(e: MouseEvent) {
    if (e.buttons === 0 && (this.drag || this.fillDrag || this.pointDrag || this.resize)) {
      this.cancelDrags();
      return;
    }
    const r = this.g.scroller.getBoundingClientRect();
    this.lastMouse = { x: e.clientX - r.left, y: e.clientY - r.top };
    if (this.resize) {
      const z = this.resize;
      if (z.kind === "col") this.g.setColWidth(z.index, z.startSize + (e.clientX - z.start));
      else this.g.setRowHeight(z.index, z.startSize + (e.clientY - z.start));
      return;
    }
    if (this.fillDrag || this.pointDrag) {
      const { x, y } = this.lastMouse;
      const hx = Math.max(this.g.rowHeaderW + 1, Math.min(x, this.g.viewW - 1));
      const hy = Math.max(HH + 1, Math.min(y, this.g.viewH - 1));
      const row = this.g.rowAtCanvasY(hy);
      const col = Math.min(this.g.colCount - 1, this.g.colAtCanvasX(hx));
      if (this.pointDrag && this.g.pointMode) {
        const p = this.pointDrag;
        this.g.pointMode.onPick({ r0: Math.min(p.row, row), c0: Math.min(p.col, col), r1: Math.max(p.row, row), c1: Math.max(p.col, col) }, true);
      } else if (this.fillDrag) {
        this.fillDrag.dst = fillTarget(this.fillDrag.src, row, col);
        this.g.render();
      }
      if ((y < HH + 2 || y > this.g.viewH - 4 || x > this.g.viewW - 4) ) this.autoScroll.request();
      return;
    }
    if (!this.drag) {
      // The pointer may be over the scroll spacer (a child of the scroller), so test by position.
      const { x, y } = this.lastMouse;
      if (x >= 0 && y >= 0 && x < this.g.viewW && y < this.g.viewH && this.g.scroller.contains(e.target as Node)) {
        const h = this.g.hitAt(x, y);
        this.g.scroller.style.cursor =
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
    const outside = y < HH || y > this.g.viewH - 4 || x < this.g.rowHeaderW || x > this.g.viewW - 4;
    if (outside) this.autoScroll.request();
  }

  private dragTo() {
    if (!this.drag) return;
    const { x, y } = this.lastMouse;
    const h = this.g.hitAt(Math.max(this.g.rowHeaderW + 1, Math.min(x, this.g.viewW - 1)), Math.max(HH + 1, Math.min(y, this.g.viewH - 1)));
    if (h.area !== "cell") return;
    if (this.drag.kind === "cells") {
      // Dragging over a merged cell extends the selection over all of it.
      let { row, col } = h;
      const m = this.g.mergeAt(row, col);
      const s = this.drag.start;
      if (m) {
        row = row >= s.row ? m.r1 : m.r0;
        col = col >= s.col ? m.c1 : m.c0;
      }
      this.g.active = { row, col };
    } else if (this.drag.kind === "cols") this.g.active = { row: 0, col: h.col };
    else this.g.active = { row: h.row, col: 0 };
    this.g.render();
    this.g.selectionChanged();
  }

  /** One frame of scrolling while a drag holds the pointer beyond the grid's edge. */
  private autoScrollStep() {
    if (!this.drag && !this.fillDrag && !this.pointDrag) return;
    const { x, y } = this.lastMouse;
    let moved = false;
    if (y < HH) {
      this.g.setTopPx(this.g.topPx() - this.g.defRowH);
      moved = true;
    } else if (y > this.g.viewH - 4) {
      this.g.setTopPx(this.g.topPx() + this.g.defRowH);
      moved = true;
    }
    if (x < this.g.rowHeaderW) {
      this.g.scroller.scrollLeft -= 30;
      moved = true;
    } else if (x > this.g.viewW - 4) {
      this.g.scroller.scrollLeft += 30;
      moved = true;
    }
    if (!moved) return;
    if (this.drag) this.dragTo();
    else {
      const host = this.g.scroller.getBoundingClientRect();
      window.dispatchEvent(new MouseEvent("mousemove", { clientX: x + host.left, clientY: y + host.top, buttons: 1 }));
    }
    this.autoScroll.request();
  }

  onMouseUp() {
    if (this.fillDrag) {
      const { src, dst } = this.fillDrag;
      this.fillDrag = null;
      this.g.render();
      if (dst.r0 !== src.r0 || dst.r1 !== src.r1 || dst.c0 !== src.c0 || dst.c1 !== src.c1) this.g.onFill(src, dst);
    }
    this.pointDrag = null;
    if (this.resize) {
      const z = this.resize;
      if (z.kind === "col") this.g.onColResize(z.index, this.g.colWidth(z.index));
      else this.g.onRowResize(z.index, this.g.rowHeight(z.index));
      this.resize = null;
    }
    this.drag = null;
  }}
