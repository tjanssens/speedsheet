// Unit tests for the grid's pure selection rules (cells.ts). Run: npm test
import { describe, expect, it } from "vitest";
import { advanceWithin, colName, contains, fillTarget, MAX_COLS, MAX_ROWS, parseAddress, rectOf, type Rect } from "./cells";

describe("addresses", () => {
  it("names columns like Excel", () => {
    expect([0, 25, 26, 51, 52, 701, 702, MAX_COLS - 1].map(colName)).toEqual(["A", "Z", "AA", "AZ", "BA", "ZZ", "AAA", "XFD"]);
  });

  it("parses addresses, with or without $, and rejects what is not one", () => {
    expect(parseAddress("B12")).toEqual({ row: 11, col: 1 });
    expect(parseAddress(" $xfd$1048576 ")).toEqual({ row: MAX_ROWS - 1, col: MAX_COLS - 1 });
    for (const bad of ["", "A0", "12", "B", "XFE1", "A1048577", "A1:B2", "Sheet1!A1"]) expect(parseAddress(bad), bad).toBeNull();
  });

  it("colName and parseAddress are inverses", () => {
    for (const col of [0, 7, 26, 300, 16383]) for (const row of [0, 9, 1048575]) {
      expect(parseAddress(`${colName(col)}${row + 1}`)).toEqual({ row, col });
    }
  });
});

describe("selection", () => {
  it("spans anchor and active cell in any direction", () => {
    expect(rectOf({ row: 5, col: 1 }, { row: 2, col: 4 })).toEqual({ r0: 2, c0: 1, r1: 5, c1: 4 });
  });

  const sel: Rect = { r0: 1, c0: 1, r1: 2, c1: 3 }; // B2:D3

  it("Tab walks along the row and wraps to the next row, then back to the start", () => {
    const seen = [];
    let p = { row: 1, col: 1 };
    for (let i = 0; i < 6; i++) {
      p = advanceWithin(sel, p, 0, 1);
      seen.push(`${colName(p.col)}${p.row + 1}`);
    }
    expect(seen).toEqual(["C2", "D2", "B3", "C3", "D3", "B2"]);
  });

  it("Enter walks down the column and wraps to the next column; Shift goes back", () => {
    expect(advanceWithin(sel, { row: 2, col: 1 }, 1, 0)).toEqual({ row: 1, col: 2 });
    expect(advanceWithin(sel, { row: 1, col: 1 }, -1, 0)).toEqual({ row: 2, col: 3 });
    expect(advanceWithin(sel, { row: 1, col: 1 }, 0, -1)).toEqual({ row: 2, col: 3 });
  });

  it("never leaves the selection, whatever the steps", () => {
    let p = { row: 1, col: 1 };
    const steps: [number, number][] = [[1, 0], [-1, 0], [0, 1], [0, -1]];
    for (let i = 0; i < 500; i++) {
      const [dr, dc] = steps[(i * 7 + (i >> 3)) % 4];
      p = advanceWithin(sel, p, dr, dc);
      expect(contains(sel, p)).toBe(true);
    }
  });
});

describe("fill handle", () => {
  const src: Rect = { r0: 2, c0: 2, r1: 3, c1: 3 };

  it("grows down, up, right or left, never diagonally", () => {
    expect(fillTarget(src, 9, 4)).toEqual({ ...src, r1: 9 });
    expect(fillTarget(src, 0, 3)).toEqual({ ...src, r0: 0 });
    expect(fillTarget(src, 4, 8)).toEqual({ ...src, c1: 8 });
    expect(fillTarget(src, 3, 0)).toEqual({ ...src, c0: 0 });
  });

  it("inside the source it fills nothing", () => {
    expect(fillTarget(src, 3, 2)).toEqual(src);
  });

  it("always contains the source", () => {
    for (let row = 0; row < 12; row++)
      for (let col = 0; col < 12; col++) {
        const d = fillTarget(src, row, col);
        expect(d.r0 <= src.r0 && d.c0 <= src.c0 && d.r1 >= src.r1 && d.c1 >= src.c1).toBe(true);
        expect(d.r0 === src.r0 && d.r1 === src.r1 || d.c0 === src.c0 && d.c1 === src.c1).toBe(true);
      }
  });
});
