import { describe, expect, it } from "vitest";
import { contrast, hex, parseColor, readableOn } from "./color";

describe("colors", () => {
  it("formats and parses colors", () => {
    expect(hex(0x0a0b0c)).toBe("#0a0b0c");
    expect(parseColor("#ff8000")).toEqual([255, 128, 0]);
    expect(parseColor("rgb(1, 2, 3)")).toEqual([1, 2, 3]);
  });

  it("lightens dark text until it reads on a dark background", () => {
    const bg: [number, number, number] = [30, 30, 30];
    const out = parseColor(readableOn(0x000080, bg));
    expect(contrast(out, bg)).toBeGreaterThanOrEqual(4);
  });
});
