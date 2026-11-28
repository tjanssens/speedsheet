// Cell addresses and the selection rules of the grid, as plain functions without DOM or canvas,
// so they can be unit-tested (cells.test.ts) and reused.

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

export const MAX_ROWS = 1_048_576;
export const MAX_COLS = 16_384;

/** Column letters: 0 -> A, 25 -> Z, 26 -> AA. */
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

/** "B12" or "$B$12" to a position; null for anything else or outside Excel's sheet. */
export function parseAddress(s: string): Pos | null {
  const m = /^\s*\$?([A-Za-z]{1,3})\$?(\d{1,7})\s*$/.exec(s);
  if (!m) return null;
  let col = 0;
  for (const ch of m[1].toUpperCase()) col = col * 26 + (ch.charCodeAt(0) - 64);
  const row = parseInt(m[2], 10);
  if (row < 1 || row > MAX_ROWS || col > MAX_COLS) return null;
  return { row: row - 1, col: col - 1 };
}

/** The rectangle spanned by the anchor and the active cell. */
export function rectOf(anchor: Pos, active: Pos): Rect {
  return {
    r0: Math.min(anchor.row, active.row),
    r1: Math.max(anchor.row, active.row),
    c0: Math.min(anchor.col, active.col),
    c1: Math.max(anchor.col, active.col),
  };
}

export function contains(r: Rect, p: Pos) {
  return p.row >= r.r0 && p.row <= r.r1 && p.col >= r.c0 && p.col <= r.c1;
}

/**
 * Enter/Tab inside a multi-cell selection, like Excel: the active cell walks through the
 * selection (Tab along the row, Enter down the column) and wraps around at its edges.
 */
export function advanceWithin(sel: Rect, active: Pos, dr: number, dc: number): Pos {
  let { row, col } = active;
  if (dc) {
    col += dc;
    if (col > sel.c1) {
      col = sel.c0;
      row = row + 1 > sel.r1 ? sel.r0 : row + 1;
    } else if (col < sel.c0) {
      col = sel.c1;
      row = row - 1 < sel.r0 ? sel.r1 : row - 1;
    }
  } else {
    row += dr;
    if (row > sel.r1) {
      row = sel.r0;
      col = col + 1 > sel.c1 ? sel.c0 : col + 1;
    } else if (row < sel.r0) {
      row = sel.r1;
      col = col - 1 < sel.c0 ? sel.c1 : col - 1;
    }
  }
  return { row, col };
}

/**
 * Dragging the fill handle of `src` to (row, col): the selection grows along the axis where the
 * pointer is farthest outside it, never diagonally.
 */
export function fillTarget(src: Rect, row: number, col: number): Rect {
  const down = row > src.r1 ? row - src.r1 : row < src.r0 ? src.r0 - row : 0;
  const across = col > src.c1 ? col - src.c1 : col < src.c0 ? src.c0 - col : 0;
  if (down >= across && down > 0) return row > src.r1 ? { ...src, r1: row } : { ...src, r0: row };
  if (across > 0) return col > src.c1 ? { ...src, c1: col } : { ...src, c0: col };
  return { ...src };
}
