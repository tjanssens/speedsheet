# Feedback proxy

The **Feedback** button in SpeedSheet sends the user's message (plus app version and OS) to this
small Cloudflare Worker, which creates a GitHub issue in `tjanssens/speedsheet`. The GitHub token
lives only in Cloudflare, never in the app.

If the proxy is not configured or not reachable, the app opens GitHub's "new issue" page with the
feedback already filled in instead (that requires a GitHub account and, for a private repo, access).

## One-time setup (about 10 minutes)

1. **Token.** On GitHub: Settings → Developer settings → Fine-grained tokens → Generate new token.
   Repository access: *Only select repositories* → `speedsheet`. Permissions: *Issues: Read and write*.
2. **Deploy the worker** (needs a free Cloudflare account):
   ```sh
   cd feedback-worker
   npx wrangler login
   npx wrangler secret put GITHUB_TOKEN   # paste the token
   npx wrangler deploy                    # prints the URL, e.g. https://speedsheet-feedback.<you>.workers.dev
   ```
3. **Tell the build about it.** In the GitHub repo: Settings → Secrets and variables → Actions →
   *Variables* → New variable `FEEDBACK_URL` = the URL from step 2. The release workflow bakes it
   into the app. For a local build: `SPEEDSHEET_FEEDBACK_URL=https://… npm run tauri build`.
4. Optional: create a `feedback` label in the repo so reports are easy to find.

Test it:
```sh
curl -X POST https://speedsheet-feedback.<you>.workers.dev \
  -H 'content-type: application/json' \
  -d '{"message":"Test from curl","version":"0.1.0","os":"macos","osVersion":"15","arch":"aarch64"}'
```

Abuse protection: 5 reports per minute per IP (Cloudflare rate limiting), message size capped at 10k characters.
Anyone who finds the URL could still send reports, so keep an eye on the label and rotate the URL if needed.
