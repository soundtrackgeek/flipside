import { useEffect, useState } from "react";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { FolderPlus, HardDrive, Loader2, RefreshCw, Trash2 } from "lucide-react";
import { commands, events } from "../bindings";
import type { LocalSharesSnapshot, UploadQueueSnapshot } from "../bindings";
import { useApp } from "../store";
import { errorMessage, formatBytes } from "../lib/format";

/** Soulseek is a trading network: many peers refuse to serve people who share nothing. */
export default function SharingPanel() {
  const { toast } = useApp();
  const [shares, setShares] = useState<LocalSharesSnapshot | null>(null);
  const [uploads, setUploads] = useState<UploadQueueSnapshot | null>(null);

  useEffect(() => {
    let alive = true;
    const off: Array<() => void> = [];
    void commands.localSharesSnapshot().then((s) => alive && setShares(s)).catch((e) => toast("error", errorMessage(e)));
    void commands.uploadsSnapshot().then((u) => alive && setUploads(u)).catch(() => undefined);
    void events.flipsideSoulseekLocalShares.listen((e) => setShares(e.payload)).then((f) => off.push(f));
    void events.flipsideSoulseekUploads.listen((e) => setUploads(e.payload)).then((f) => off.push(f));
    return () => { alive = false; off.forEach((f) => f()); };
  }, [toast]);

  const run = (p: Promise<LocalSharesSnapshot>) => p.then(setShares).catch((e) => toast("error", errorMessage(e)));

  async function add() {
    const picked = await pickFolder({ directory: true, multiple: false });
    if (typeof picked === "string") await run(commands.localSharesAdd(picked));
  }

  if (!shares) return null;
  const enabledRoots = shares.roots.filter((r) => r.enabled);

  return (
    <div className="panel">
      <h3><HardDrive size={18} /> Shared music</h3>
      <p className="desc">
        Some Soulseek users only let you download if you share something back. Add the folders you want to offer;
        only enabled folders are indexed, advertised and available for upload.
      </p>
      <div className="status-line" style={{ marginBottom: 14, flexWrap: "wrap" }}>
        <span className={`chip ${enabledRoots.length ? "good" : "warn"}`}>
          {enabledRoots.length ? `Sharing ${shares.totalFileCount.toLocaleString()} files · ${formatBytes(shares.totalSizeBytes)}` : "Not sharing anything yet"}
        </span>
        {shares.scanning ? <span className="chip"><Loader2 size={11} className="spin" /> scanning…</span> : null}
        {uploads ? (
          <span>{uploads.activeCount} active uploads · {uploads.sessionUploadedBytes > 0 ? `${formatBytes(uploads.sessionUploadedBytes)} uploaded this session` : "nothing uploaded this session"}</span>
        ) : null}
      </div>

      {shares.roots.map((r) => (
        <div className="result" key={r.id} style={{ gridTemplateColumns: "28px 1fr auto auto", alignItems: "center" }}>
          <input type="checkbox" checked={r.enabled} aria-label={`Share ${r.alias}`} onChange={(e) => void run(commands.localSharesSetEnabled(r.id, e.target.checked))} />
          <div style={{ minWidth: 0 }}>
            <div className="name">{r.alias}</div>
            <div className="status-line" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={r.path}>
              {r.error ? <span style={{ color: "var(--bad)" }}>{r.error}</span> : r.path.replace(/^\\\\\?\\/, "")}
            </div>
          </div>
          <span className="chip">{r.fileCount.toLocaleString()} files · {formatBytes(r.totalSizeBytes)}</span>
          <button className="icon-btn" aria-label={`Stop sharing ${r.alias}`} title="Remove" onClick={() => void run(commands.localSharesRemove(r.id))}><Trash2 size={15} /></button>
        </div>
      ))}

      <div className="form-actions" style={{ marginTop: shares.roots.length ? 6 : 0 }}>
        <button className="btn primary" onClick={() => void add()}><FolderPlus size={15} /> Add folder</button>
        <button className="btn" disabled={shares.scanning || shares.roots.length === 0} onClick={() => void run(commands.localSharesRescan())}>
          <RefreshCw size={15} className={shares.scanning ? "spin" : ""} /> Rescan
        </button>
        <label className="field" style={{ flexDirection: "row", alignItems: "center", gap: 10, marginLeft: "auto" }}>
          <label style={{ margin: 0 }}>Upload slots</label>
          <input
            type="number" min={1} max={3} style={{ width: 70 }} value={shares.uploadSlots}
            onChange={(e) => void run(commands.localSharesSetUploadSlots(Math.min(3, Math.max(1, Number(e.target.value) || 1))))}
          />
        </label>
      </div>
    </div>
  );
}
