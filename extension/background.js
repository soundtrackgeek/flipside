// Service worker: the only place that talks to Flipside. Its requests carry a chrome-extension:// Origin,
// which Flipside requires; content scripts would send the web page's Origin and be refused.

const BASE = "http://127.0.0.1:47615";

async function settings() {
  const { token = "", autoGet = false } = await chrome.storage.local.get(["token", "autoGet"]);
  return { token: token.trim(), autoGet };
}

async function call(path, method, body, token) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 60000);
  try {
    const response = await fetch(BASE + path, {
      method,
      headers: { "Content-Type": "application/json", ...(token ? { Authorization: `Bearer ${token}` } : {}) },
      body: body ? JSON.stringify(body) : undefined,
      signal: controller.signal,
    });
    const data = await response.json().catch(() => ({}));
    if (response.status === 401) return { ok: false, code: "unpaired", error: "Not paired. Open the extension options and paste the token from Flipside → Settings." };
    if (!response.ok) return { ok: false, code: "error", error: data.error || `Flipside answered ${response.status}` };
    return { ok: true, data };
  } catch (error) {
    if (error && error.name === "AbortError") return { ok: false, code: "timeout", error: "Flipside took too long to answer." };
    return { ok: false, code: "offline", error: "Can't reach Flipside. Is it running?" };
  } finally {
    clearTimeout(timer);
  }
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  (async () => {
    const { token, autoGet } = await settings();
    switch (message.type) {
      case "settings":
        return { ok: true, data: { autoGet, hasToken: !!token } };
      case "ping":
        return call("/ping", "GET", null, token);
      case "check":
        return call("/check", "POST", { artist: message.artist, album: message.album, year: message.year ?? null }, token);
      case "get":
        return call("/get", "POST", { artist: message.artist, album: message.album, year: message.year ?? null }, token);
      default:
        return { ok: false, code: "error", error: "Unknown message" };
    }
  })().then(sendResponse);
  return true; // keep the channel open for the async response
});

chrome.action.onClicked.addListener(() => chrome.runtime.openOptionsPage());
