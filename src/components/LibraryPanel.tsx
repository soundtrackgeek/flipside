import { useCallback, useEffect, useState } from "react";
import { open as pickFile } from "@tauri-apps/plugin-dialog";
import { Check, Database, FolderOpen, Loader2, RefreshCw } from "lucide-react";
import { commands } from "../bindings";
import type { AppSettings, LibraryStatus } from "../bindings";
import { useApp } from "../store";
import { errorMessage } from "../lib/format";

type Key = "libraryDbPath" | "musicbrainzCachePath" | "musicbrainzOverlayPath";

/** Points Flipside at the Music Library databases that say what you own. */
export default function LibraryPanel({ settings, save }: { settings: AppSettings; save: (s: AppSettings) => Promise<void> }) {
  const { toast } = useApp();
  const [draft, setDraft] = useState<AppSettings>(settings);
  const [status, setStatus] = useState<LibraryStatus | null>(null);
  useEffect(() => setDraft(settings), [settings]);

  const refresh = useCallback(() => commands.libraryStatus().then(setStatus).catch(() => undefined), []);
  useEffect(() => { void refresh(); }, [refresh]);
  // While the library loads (a few seconds) keep the numbers fresh.
  useEffect(() => {
    if (!status?.loading) return;
    const t = window.setInterval(() => void refresh(), 1200);
    return () => window.clearInterval(t);
  }, [status?.loading, refresh]);

  async function browse(key: Key) {
    const picked = await pickFile({ multiple: false, directory: false, filters: [{ name: "SQLite database", extensions: ["sqlite3", "sqlite", "db"] }] });
    if (typeof picked === "string") setDraft((d) => ({ ...d, [key]: picked }));
  }

  async function apply(next: AppSettings) {
    await save(next);
    await commands.libraryReload().then(setStatus).catch((e) => toast("error", errorMessage(e)));
    window.setTimeout(() => void refresh(), 1500);
  }

  const field = (label: string, key: Key, detected: string | null | undefined) => (
    <div className="field wide">
      <label>{label}</label>
      <div className="row">
        <input
          value={draft[key] ?? ""} onChange={(e) => setDraft({ ...draft, [key]: e.target.value })} spellCheck={false}
          placeholder={detected ? `Auto: ${detected}` : "Auto-detect"}
        />
        <button className="btn" type="button" onClick={() => void browse(key)}><FolderOpen size={15} /> Browse</button>
      </div>
    </div>
  );

  return (
    <div className="panel">
      <h3><Database size={18} /> Your library</h3>
      <p className="desc">
        Flipside reads your Music Library databases (read-only) to hide everything you already own and to find studio albums you're
        missing. Leave a path empty to use the one Music Library already knows.
      </p>
      <div className="status-line" style={{ marginBottom: 14, flexWrap: "wrap" }}>
        {status?.loading ? <span className="chip"><Loader2 size={11} className="spin" /> reading…</span> : null}
        {status?.loaded ? (
          <>
            <span className="chip good"><Check size={11} /> {status.ownedAlbums.toLocaleString()} albums · {status.ownedArtists.toLocaleString()} artists</span>
            <span className="chip accent">{status.missingAlbums.toLocaleString()} studio albums missing</span>
          </>
        ) : null}
        {!status?.loaded && !status?.loading ? <span className="chip warn">not loaded</span> : null}
      </div>
      {status?.error ? <div className="note err">{status.error}</div> : null}
      <div className="form">
        {field("Music Library database", "libraryDbPath", status?.libraryPath)}
        {field("MusicBrainz cache", "musicbrainzCachePath", status?.musicbrainzCachePath)}
        {field("MusicBrainz overlay (newer data)", "musicbrainzOverlayPath", status?.musicbrainzOverlayPath)}
        <div className="field wide">
          <label className="switch" style={{ fontWeight: 500, fontSize: 13, color: "var(--text)" }}>
            <input type="checkbox" checked={draft.hideOwned ?? true} onChange={(e) => { const next = { ...draft, hideOwned: e.target.checked }; setDraft(next); void apply(next); }} />
            Hide albums I already own everywhere in Flipside
          </label>
        </div>
      </div>
      <div className="form-actions">
        <button className="btn primary" onClick={() => void apply(draft)}>Save &amp; reload</button>
        <button className="btn ghost" onClick={() => void commands.libraryReload().then(setStatus).catch((e) => toast("error", errorMessage(e)))}>
          <RefreshCw size={14} className={status?.loading ? "spin" : ""} /> Reload now
        </button>
      </div>
    </div>
  );
}
