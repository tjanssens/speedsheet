// Type-aware lint for the UI. Focus: bugs the type checker does not catch, such as promises
// that are never awaited (a failed save or open would go unnoticed) and unsafe `any` flows.
import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist", "src-tauri", "node_modules", "feedback-worker", "e2e", "*.config.*"] },
  js.configs.recommended,
  ...tseslint.configs.recommendedTypeChecked,
  {
    languageOptions: { parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname } },
    rules: {
      "@typescript-eslint/no-floating-promises": "error",
      "@typescript-eslint/no-misused-promises": "error",
      eqeqeq: ["error", "smart"],
    },
  },
);
