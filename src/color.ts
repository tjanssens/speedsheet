// Color math for drawing cells: hex strings, contrast, and keeping dark text readable on a dark theme.

export function hex(c: number) {
  return "#" + c.toString(16).padStart(6, "0");
}
export function parseColor(s: string): [number, number, number] {
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
export function luminance([r, g, b]: [number, number, number]) {
  const f = (v: number) => {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}
export function contrast(a: [number, number, number], b: [number, number, number]) {
  const la = luminance(a);
  const lb = luminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}
export const rgbOf = (c: number): [number, number, number] => [(c >> 16) & 255, (c >> 8) & 255, c & 255];

/** Lighten a dark color until it reads on a dark background (keeps the hue). */
export function readableOn(c: number, bg: [number, number, number], min = 4): string {
  let [r, g, b] = rgbOf(c);
  for (let i = 0; i < 10 && contrast([r, g, b], bg) < min; i++) {
    r += (255 - r) * 0.25;
    g += (255 - g) * 0.25;
    b += (255 - b) * 0.25;
  }
  return `rgb(${Math.round(r)},${Math.round(g)},${Math.round(b)})`;
}