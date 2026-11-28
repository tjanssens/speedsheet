// docs/features.md is the map of every feature: this test keeps it complete and its paths real.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "@playwright/test";

const doc = readFileSync("docs/features.md", "utf8");
const named = new Set([...doc.matchAll(/`([^`]+)`/g)].map((m) => m[1]));

test("every toolbar command is in the feature map", () => {
  const html = readFileSync("index.html", "utf8");
  const cmds = [...new Set([...html.matchAll(/data-cmd="([A-Za-z]+)"/g)].map((m) => m[1]))];
  expect(cmds.length).toBeGreaterThan(10);
  expect(cmds.filter((c) => !named.has(c)), "add these to docs/features.md").toEqual([]);
});

test("every engine command is in the feature map", () => {
  const api = readFileSync("src-tauri/src/api.rs", "utf8");
  const cmds = [...new Set([...api.matchAll(/^ {8}"([a-z_]+)" =>/gm)].map((m) => m[1]))];
  expect(cmds.length).toBeGreaterThan(20);
  expect(cmds.filter((c) => !named.has(c)), "add these to docs/features.md").toEqual([]);
});

test("every file the feature map names exists", () => {
  const dirs = ["", "src", "e2e", "crates/speedsheet-core/src", "crates/speedsheet-core/tests", "src-tauri/src"];
  const files = [...named].filter((n) => /^[\w./-]+\.(rs|ts|md|html)$|\/$/.test(n));
  expect(files.length).toBeGreaterThan(20);
  const missing = files.filter((f) => !dirs.some((d) => existsSync(join(d, f))));
  expect(missing, "fix these paths in docs/features.md").toEqual([]);
});
