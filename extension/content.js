// On-page badge. Collapsed it is a small pill; click it for the Deezer / Usenet / Soulseek breakdown,
// an editable artist + album, and a button that sends the album to Flipside.
(function () {
  if (window.top !== window) return;
  const HOST_ID = "flipside-connector-host";
  const DAY = 24 * 3600 * 1000;
  let lastKey = "";
  let runId = 0;
  let timer = null;

  const send = (message) =>
    chrome.runtime.sendMessage(message).catch(() => ({ ok: false, code: "offline", error: "Extension was reloaded. Refresh this page." }));

  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

  const STYLE = `
    :host { all: initial; }
    * { box-sizing: border-box; font-family: "Segoe UI", system-ui, sans-serif; }
    .wrap { position: fixed; right: 16px; bottom: 16px; z-index: 2147483647; color: #ececf3; display: flex; flex-direction: column; align-items: flex-end; }
    .pill { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-radius: 999px; border: 1px solid #34344a; background: #14141d; box-shadow: 0 6px 24px #0008; cursor: pointer; font-size: 13px; }
    .pill b { color: #a28bff; }
    .panel { width: 320px; margin-bottom: 10px; padding: 14px; border-radius: 14px; border: 1px solid #34344a; background: #14141d; box-shadow: 0 10px 36px #000a; font-size: 13px; }
    .panel h4 { margin: 0 0 8px; font-size: 14px; display: flex; justify-content: space-between; }
    .panel h4 button { all: unset; cursor: pointer; color: #9a9ab0; }
    input { width: 100%; margin: 3px 0 8px; padding: 7px 9px; border-radius: 7px; border: 1px solid #2c2c3a; background: #0d0d14; color: inherit; font-size: 13px; }
    label { color: #9a9ab0; font-size: 11px; text-transform: uppercase; letter-spacing: .05em; }
    .row { display: flex; gap: 8px; align-items: baseline; padding: 4px 0; }
    .row .name { width: 62px; font-weight: 600; }
    .row .detail { color: #9a9ab0; flex: 1; font-size: 12px; }
    .ok { color: #5fd38d; } .warn { color: #f0b429; } .no { color: #7a7a90; } .err { color: #ff7a7a; }
    .actions { display: flex; gap: 8px; margin-top: 10px; }
    button.btn { flex: 1; padding: 8px 10px; border-radius: 8px; border: 0; background: #7c5cff; color: #fff; font-size: 13px; cursor: pointer; }
    button.btn.ghost { background: #26263a; }
    button.btn:disabled { opacity: .5; cursor: default; }
    .note { margin-top: 8px; color: #9a9ab0; font-size: 12px; }
  `;

  const MARK = { found: ["✓", "ok"], online: ["✓", "ok"], "needs-login": ["!", "warn"], missing: ["✗", "no"], unavailable: ["–", "no"], error: ["!", "err"] };
  const PROVIDERS = [["deezer", "Deezer"], ["usenet", "Usenet"], ["soulseek", "Soulseek"]];

  function mount() {
    let host = document.getElementById(HOST_ID);
    if (!host) {
      host = document.createElement("div");
      host.id = HOST_ID;
      document.documentElement.appendChild(host);
      host.attachShadow({ mode: "open" }).innerHTML = `<style>${STYLE}</style><div class="wrap"></div>`;
    }
    return host.shadowRoot;
  }

  const unmount = () => document.getElementById(HOST_ID)?.remove();

  async function wasRequested(key) {
    const { requested = {} } = await chrome.storage.local.get("requested");
    return Date.now() - (requested[key] || 0) < DAY;
  }

  async function markRequested(key) {
    const { requested = {} } = await chrome.storage.local.get("requested");
    const now = Date.now();
    for (const k of Object.keys(requested)) if (now - requested[k] > 7 * DAY) delete requested[k];
    requested[key] = now;
    await chrome.storage.local.set({ requested });
  }

  function render(state) {
    const wrap = mount().querySelector(".wrap");
    const { found, result, open, busy, message, note } = state;
    const mark = (p) => (p ? MARK[p.state] || ["?", "no"] : ["…", "no"]);
    const summary = result
      ? PROVIDERS.map(([k, label]) => `<span class="${mark(result[k])[1]}">${label} ${mark(result[k])[0]}</span>`).join(" · ")
      : message ? `<span class="err">${esc(message)}</span>` : busy ? "checking…" : "";
    const rows = result
      ? PROVIDERS.map(([k, label]) => {
          const [m, cls] = mark(result[k]);
          return `<div class="row"><span class="name">${label}</span><span class="${cls}">${m}</span><span class="detail">${esc(result[k]?.detail)}</span></div>`;
        }).join("")
      : "";
    wrap.innerHTML = `
      ${open ? `<div class="panel">
        <h4><span>Flipside</span><button data-act="close" aria-label="Close">✕</button></h4>
        <label>Artist</label><input data-f="artist" value="${esc(found.artist)}" />
        <label>Album</label><input data-f="album" value="${esc(found.album)}" />
        ${rows}
        ${message && !result ? `<div class="note err">${esc(message)}</div>` : ""}
        ${note ? `<div class="note ok">${esc(note)}</div>` : ""}
        <div class="actions">
          <button class="btn ghost" data-act="check" ${busy ? "disabled" : ""}>${busy ? "Working…" : "Check again"}</button>
          <button class="btn" data-act="get" ${busy ? "disabled" : ""}>Get it</button>
        </div>
        <div class="note">Soulseek is searched when you press Get it (about 20 seconds).</div>
      </div>` : ""}
      <div class="pill" data-act="toggle"><b>Flipside</b> ${summary}</div>`;
  }

  async function run(found) {
    const id = ++runId;
    const state = { found: { ...found }, result: null, open: false, busy: true, message: "", note: "" };
    const key = () => `${state.found.artist}|${state.found.album}`.toLowerCase();
    const paint = () => { if (id === runId) render(state); };

    const check = async () => {
      state.busy = true; state.message = ""; state.note = ""; paint();
      const reply = await send({ type: "check", ...state.found });
      state.busy = false;
      if (reply.ok) { state.result = reply.data; }
      else {
        state.result = null;
        state.message = reply.error;
        if (reply.code === "unpaired" || reply.code === "offline") state.open = true;
      }
      paint();
      return reply;
    };

    const get = async () => {
      state.busy = true; state.message = ""; paint();
      const reply = await send({ type: "get", ...state.found });
      state.busy = false;
      if (reply.ok) { await markRequested(key()); state.note = "Sent to Flipside."; }
      else state.message = reply.error;
      paint();
    };

    paint();
    mount().querySelector(".wrap").onclick = async (event) => {
      const act = event.target.closest("[data-act]")?.dataset.act;
      if (!act) return;
      mount().querySelectorAll("input[data-f]").forEach((input) => { state.found[input.dataset.f] = input.value.trim(); });
      if (act === "toggle") { state.open = !state.open; paint(); }
      else if (act === "close") { state.open = false; paint(); }
      else if (act === "check") await check();
      else if (act === "get") await get();
    };

    const reply = await check();
    if (!reply.ok || id !== runId) return;
    const settings = (await send({ type: "settings" })).data;
    const usable = ["deezer", "usenet"].some((k) => reply.data[k]?.state === "found");
    if (settings?.autoGet && usable && !(await wasRequested(key()))) await get();
  }

  function scan() {
    const found = window.FlipsideSites.extract(document, new URL(location.href));
    const key = found ? `${found.artist}|${found.album}|${location.href}` : "";
    if (key === lastKey) return;
    lastKey = key;
    runId++; // abandon any run still in flight for the previous page
    if (!found) return unmount();
    void run(found);
  }

  // Sites like YouTube change pages without reloading, so look again whenever the URL or title moves.
  let seen = "";
  setInterval(() => {
    const now = `${location.href}|${document.title}`;
    if (now === seen) return;
    seen = now;
    clearTimeout(timer);
    timer = setTimeout(scan, 900);
  }, 700);
})();
