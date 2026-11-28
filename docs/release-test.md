# Release test: use the app like a user

Instructions for the agent that tests each new release (the scheduled task "SpeedSheet
release testen"). The goal is to find bugs before Tom does: in features without tests first,
then everywhere. Every bug found becomes a GitHub issue labelled `feedback` and
`found-by-agent`; the feedback task then reproduces it with a test and fixes it.

## Set up

1. Check out the release tag (`git checkout v<version>`), `npm ci`,
   `cargo build --release -p speedsheet --example dev_bridge`.
2. Make test files: `testdata/styled-members.xlsx`, the corpus in `testdata/corpus/` (if
   present), and a big one from `cargo run --release -p speedsheet-core --example bench`.
3. Start the bridge on a copy of a file (`target/release/examples/dev_bridge <copy>`) and the UI
   (`npx vite --port 1420`), then drive http://localhost:1420 with Playwright. Restart the
   bridge for each file: it hands the file to the first tab only.

## What to do

Work through `docs/features.md`, section by section, starting with the rows marked "none" in
the Tests column. For each feature:

- Do what a user does: mouse and keyboard, in the order a person would, also the unusual
  orders (edit while a filter is on, undo after a sort, paste over a merged cell, very long
  text, `İ`, emoji, numbers like `1e308`, dates around midnight and new year).
- Compare with Excel's behaviour (or LibreOffice: `soffice --headless --convert-to pdf`) for
  the same file and steps. Different from Excel without a reason is a bug.
- Watch the browser console: any error is a bug.
- Check the result survives save and open again.
- Time it on the big file: open, sort, filter, find. Anything that feels slow (over 200 ms for
  an action on 500k rows) is a bug for SpeedSheet, whose product is speed.

Also run `UI_FUZZ_STEPS=3000 npx playwright test ui-fuzz` three times.

## Report

One issue per bug, title in plain words ("Paste over a merged cell loses the value"), body:
steps to reproduce from a fresh start, expected (with what Excel does), actual, the file used
(attach or describe it), screenshot if visual, the release version. Search open issues first;
add to an existing one instead of filing twice. No bug found in a feature: say so in a summary
comment on the release, listing what was tested.
