// Feedback dialog: sends text + app version + OS straight to a GitHub issue through the
// feedback proxy. Without a proxy (or when it is unreachable) it opens a prefilled issue page.
import { native, isTauri, errorText, type AppInfo } from "./api";
import { toast } from "./ui";

const backdrop = document.getElementById("modal-backdrop")!;
const modal = document.getElementById("modal")!;

function close() {
  backdrop.hidden = true;
  modal.innerHTML = "";
}

backdrop.addEventListener("mousedown", (e) => {
  if (e.target === backdrop) close();
});

async function openUrl(url: string) {
  if (isTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else window.open(url, "_blank", "noopener");
}

export function issueUrl(repo: string, message: string, meta: string) {
  const firstLine = message.trim().split("\n")[0].slice(0, 80) || "Feedback";
  const body = `${message.trim()}\n\n---\n${meta}\n\n<!-- speedsheet-feedback -->`;
  const q = new URLSearchParams({ title: `Feedback: ${firstLine}`, body, labels: "feedback" });
  return `https://github.com/${repo}/issues/new?${q.toString()}`;
}

export function openFeedback(info: AppInfo | null) {
  const repo = info?.repo ?? "tjanssens/speedsheet";
  const meta = info ? `SpeedSheet ${info.version} · ${info.os} ${info.osVersion} · ${info.arch}` : "SpeedSheet (browser dev mode)";
  modal.innerHTML = "";
  const h = document.createElement("h2");
  h.textContent = "Send feedback";
  const p = document.createElement("p");
  p.textContent = "What works, what doesn't, what you're missing? Your message becomes an issue for the developer.";
  const text = document.createElement("textarea");
  text.placeholder = "Describe the problem or idea…";
  text.setAttribute("aria-label", "Feedback");
  const contact = document.createElement("input");
  contact.className = "modal-field";
  contact.placeholder = "Your e-mail (optional, shown in the report)";
  contact.setAttribute("aria-label", "Contact (optional)");
  const m = document.createElement("div");
  m.className = "meta";
  m.textContent = `Included automatically: ${meta}`;
  const actions = document.createElement("div");
  actions.className = "actions";
  const cancel = document.createElement("button");
  cancel.className = "bordered";
  cancel.textContent = "Cancel";
  cancel.onclick = close;
  const send = document.createElement("button");
  send.className = "primary";
  send.textContent = "Send";
  send.onclick = async () => {
    const message = text.value.trim();
    if (!message) {
      text.focus();
      return;
    }
    send.disabled = true;
    send.textContent = "Sending…";
    try {
      if (!info?.feedbackConfigured) throw new Error("no feedback service configured");
      const res = await native<{ issueUrl: string; number: number }>("send_feedback", {
        message,
        contact: contact.value.trim() || null,
      });
      close();
      toast(res?.number ? `Thanks! Your feedback was filed as issue #${res.number}.` : "Thanks! Your feedback was sent.", 5000);
    } catch (e) {
      // Fall back to GitHub's own form, prefilled, so nothing is lost.
      const extra = contact.value.trim() ? `\nContact: ${contact.value.trim()}` : "";
      await openUrl(issueUrl(repo, message, meta + extra)).catch(() => {});
      close();
      toast(`Couldn't send directly (${errorText(e)}). GitHub opened with your feedback filled in; press "Create" there.`, 7000);
    }
  };
  actions.append(cancel, send);
  modal.append(h, p, text, contact, m, actions);
  backdrop.hidden = false;
  text.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Escape") close();
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) send.click();
  });
  contact.addEventListener("keydown", (e) => e.stopPropagation());
  text.focus();
}
