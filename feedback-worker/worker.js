// SpeedSheet feedback proxy (Cloudflare Worker).
//
// The desktop app POSTs { message, contact, version, os, osVersion, arch } here and this worker
// files a GitHub issue with a token that never leaves Cloudflare. Configure with:
//   GITHUB_TOKEN  (secret)  fine-grained token, repository access: speedsheet only, permission Issues: read & write
//   GITHUB_REPO   (var)     "tjanssens/speedsheet"
//   LIMITER       (binding) optional rate limiter, see wrangler.toml

const MAX_MESSAGE = 10_000;

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "access-control-allow-origin": "*" },
  });
}

function clean(s, max) {
  return String(s ?? "").replace(/\u0000/g, "").trim().slice(0, max);
}

export default {
  async fetch(request, env) {
    if (request.method === "OPTIONS") {
      return new Response(null, {
        headers: {
          "access-control-allow-origin": "*",
          "access-control-allow-methods": "POST",
          "access-control-allow-headers": "content-type",
        },
      });
    }
    if (request.method !== "POST") return json({ error: "POST feedback as JSON" }, 405);

    if (env.LIMITER) {
      const ip = request.headers.get("cf-connecting-ip") ?? "unknown";
      const { success } = await env.LIMITER.limit({ key: ip });
      if (!success) return json({ error: "Too many reports, try again in a minute." }, 429);
    }

    let data;
    try {
      data = await request.json();
    } catch {
      return json({ error: "invalid JSON" }, 400);
    }
    const message = clean(data.message, MAX_MESSAGE);
    if (!message) return json({ error: "message is empty" }, 400);
    const contact = clean(data.contact, 200);
    const version = clean(data.version, 40);
    const os = clean(`${clean(data.os, 40)} ${clean(data.osVersion, 60)}`, 100);
    const arch = clean(data.arch, 20);

    const firstLine = message.split("\n")[0].slice(0, 80);
    const body = [
      message,
      "",
      "---",
      `**App version:** ${version || "unknown"}`,
      `**OS:** ${os || "unknown"} (${arch || "?"})`,
      contact ? `**Contact:** ${contact}` : null,
      "",
      "_Sent from the in-app feedback button._",
      "<!-- speedsheet-feedback -->",
    ]
      .filter((l) => l !== null)
      .join("\n");

    const create = (labels) =>
      fetch(`https://api.github.com/repos/${env.GITHUB_REPO}/issues`, {
        method: "POST",
        headers: {
          authorization: `Bearer ${env.GITHUB_TOKEN}`,
          accept: "application/vnd.github+json",
          "user-agent": "speedsheet-feedback-worker",
          "x-github-api-version": "2022-11-28",
        },
        body: JSON.stringify({ title: `Feedback: ${firstLine}`, body, ...(labels ? { labels } : {}) }),
      });

    let res = await create(["feedback"]);
    if (res.status === 422 || res.status === 403) res = await create(null); // label missing or not allowed
    if (!res.ok) return json({ error: `GitHub answered ${res.status}` }, 502);
    const issue = await res.json();
    return json({ url: issue.html_url, number: issue.number });
  },
};
