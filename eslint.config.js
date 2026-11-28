// Type-aware lint for the UI. Every rule below replaces something a reviewer used to have to
// catch, and its message says how to do it right. Focus: bugs the type checker does not catch.
import js from "@eslint/js";
import tseslint from "typescript-eslint";

const TIMER = "Timers hide ordering problems and pile up. Use Debounced, after() or FrameTask from ./schedule, with a name that says why.";

export default tseslint.config(
  { ignores: ["dist", "src-tauri", "node_modules", "feedback-worker", "e2e", "*.config.*"] },
  js.configs.recommended,
  ...tseslint.configs.recommendedTypeChecked,
  {
    languageOptions: { parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname } },
    rules: {
      // A promise nobody awaits fails silently (a failed save or open would go unnoticed).
      // Await it, or start it with spawn() from ./ui so a failure reaches showError. `void` is
      // not enough: it throws the error away.
      "@typescript-eslint/no-floating-promises": ["error", { ignoreVoid: false }],
      "@typescript-eslint/no-misused-promises": "error",
      eqeqeq: ["error", "smart"],
      "no-restricted-globals": [
        "error",
        { name: "setTimeout", message: TIMER },
        { name: "setInterval", message: TIMER },
        { name: "requestAnimationFrame", message: TIMER },
        { name: "requestIdleCallback", message: TIMER },
      ],
      "no-restricted-properties": [
        "error",
        { object: "window", property: "setTimeout", message: TIMER },
        { object: "window", property: "setInterval", message: TIMER },
        { object: "window", property: "requestAnimationFrame", message: TIMER },
        { object: "window", property: "requestIdleCallback", message: TIMER },
      ],
      "no-restricted-syntax": [
        "error",
        {
          selector: "Program > VariableDeclaration[kind='let'], ExportNamedDeclaration > VariableDeclaration[kind='let']",
          message: "No module-level mutable state: keep it in `app` (app.ts) or in a const object of the module, so what the window knows stays in one visible place.",
        },
      ],
      "no-restricted-imports": [
        "error",
        { name: "@tauri-apps/api/core", message: "Talk to the engine through call() in api.ts: it also works in the browser (dev bridge) and in tests." },
      ],
      // A file this long has more than one job. Split it by what it does (see main.ts for the map).
      "max-lines": ["error", { max: 700, skipBlankLines: true, skipComments: true }],
    },
  },
  {
    files: ["src/schedule.ts"],
    rules: { "no-restricted-globals": "off", "no-restricted-properties": "off" },
  },
  {
    files: ["src/api.ts"],
    rules: { "no-restricted-imports": "off" },
  },
);
