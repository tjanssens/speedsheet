// UI smoke test: the real UI in Chromium against the dev bridge (the same Rust API as the app).
// Run: cargo build -p speedsheet --example dev_bridge && npx playwright test
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  timeout: 30_000,
  retries: 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: { baseURL: "http://localhost:1420", trace: "off", screenshot: "only-on-failure" },
  webServer: [
    {
      // A copy, so the test can save without touching the committed file.
      command: "mkdir -p target/e2e && cp testdata/styled-members.xlsx target/e2e/members.xlsx && target/debug/examples/dev_bridge target/e2e/members.xlsx",
      // _events does not hand out the file, so the first browser tab still gets it.
      url: "http://127.0.0.1:1421/_events?view=probe",
      reuseExistingServer: !process.env.CI,
    },
    { command: "npx vite --port 1420 --strictPort", url: "http://localhost:1420", reuseExistingServer: !process.env.CI },
  ],
});
