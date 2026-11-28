// Update check: quietly on startup, or on demand from the toolbar. Updates come from the
// latest GitHub release and are verified with the updater signature before installing.
import { isTauri, errorText, type AppInfo } from "./api";
import { spawn } from "./ui";
import type { Update } from "@tauri-apps/plugin-updater";

const banner = () => document.getElementById("update-banner")!;

function show(html: string, actions: { label: string; primary?: boolean; run: () => void }[] = []) {
  const b = banner();
  b.innerHTML = `<span>${html}</span>`;
  for (const a of actions) {
    const btn = document.createElement("button");
    btn.textContent = a.label;
    btn.className = a.primary ? "primary" : "bordered";
    btn.onclick = a.run;
    b.append(btn);
  }
  b.hidden = false;
}

function hide() {
  banner().hidden = true;
}

export async function checkForUpdates(info: AppInfo | null, manual: boolean) {
  if (!isTauri || !info) {
    if (manual) show("Update checks are only available in the installed app.", [{ label: "OK", run: hide }]);
    return;
  }
  if (!info.updatesConfigured) {
    if (manual) show(`This build (${info.version}) has no update channel. Download new versions from GitHub.`, [{ label: "OK", run: hide }]);
    return;
  }
  if (manual) show("Checking for updates…");
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) {
      if (manual) show(`You're on the latest version (${info.version}).`, [{ label: "OK", run: hide }]);
      return;
    }
    show(`SpeedSheet <b>${update.version}</b> is available (you have ${info.version}).`, [
      { label: "Later", run: hide },
      {
        label: "Install and restart",
        primary: true,
        run: () => spawn(install(update)),
      },
    ]);
  } catch (e) {
    if (manual) show(`Couldn't check for updates: ${errorText(e)}`, [{ label: "OK", run: hide }]);
  }
}

async function install(update: Update) {
  const progress = { total: 0, done: 0 };
  show("Downloading update…");
  try {
    await update.downloadAndInstall((ev) => {
      if (ev.event === "Started") progress.total = ev.data.contentLength ?? 0;
      if (ev.event === "Progress") {
        progress.done += ev.data.chunkLength;
        if (progress.total) show(`Downloading update… ${Math.round((progress.done / progress.total) * 100)}%`);
      }
      if (ev.event === "Finished") show("Installing…");
    });
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  } catch (e) {
    show(`Update failed: ${errorText(e)}`, [{ label: "OK", run: hide }]);
  }
}
