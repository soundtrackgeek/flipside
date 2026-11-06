const $ = (id) => document.getElementById(id);
const status = (text, cls) => { $("status").textContent = text; $("status").className = cls || ""; };

chrome.storage.local.get(["token", "autoGet"]).then(({ token = "", autoGet = false }) => {
  $("token").value = token;
  $("auto").checked = autoGet;
});

async function save() {
  await chrome.storage.local.set({ token: $("token").value.trim(), autoGet: $("auto").checked });
}

$("save").addEventListener("click", async () => { await save(); status("Saved.", "good"); });

$("test").addEventListener("click", async () => {
  await save();
  status("Testing...");
  const result = await chrome.runtime.sendMessage({ type: "ping" });
  if (!result.ok) return status(result.error, "bad");
  if (!result.data.paired) return status(`Flipside ${result.data.version} is running, but the token is wrong or missing.`, "bad");
  status(`Connected to Flipside ${result.data.version}.`, "good");
});
