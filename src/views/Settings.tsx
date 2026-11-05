import { useCallback, useEffect, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { Check, FolderOpen, KeyRound, Loader2, Newspaper, RefreshCw, Share2, Sparkles, Zap } from "lucide-react";
import { commands } from "../bindings";
import type {
  AppSettings, ConnectionBootstrap, ConnectionProfile, DeemixCredentialStatus, KeyStatus, UsenetBootstrap, UsenetProfile,
} from "../bindings";
import { useApp } from "../store";
import SharingPanel from "../components/SharingPanel";
import { errorMessage } from "../lib/format";

function Panel({ icon, title, desc, children }: { icon: ReactNode; title: string; desc: string; children: ReactNode }) {
  return (
    <div className="panel">
      <h3>{icon} {title}</h3>
      <p className="desc">{desc}</p>
      {children}
    </div>
  );
}

function Field({ label, wide, hint, children }: { label: string; wide?: boolean; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className={`field${wide ? " wide" : ""}`}>
      <label>{label}{hint ? <span style={{ fontWeight: 400 }}>{hint}</span> : null}</label>
      {children}
    </div>
  );
}

function FolderField({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  async function browse() {
    const picked = await pickFolder({ directory: true, multiple: false, defaultPath: value || undefined });
    if (typeof picked === "string") onChange(picked);
  }
  return (
    <Field label={label} wide>
      <div className="row">
        <input value={value} onChange={(e) => onChange(e.target.value)} spellCheck={false} />
        <button className="btn" type="button" onClick={() => void browse()}><FolderOpen size={15} /> Browse</button>
      </div>
    </Field>
  );
}

/* ------------------------------------------------------------------ updates */

function UpdatesPanel({ settings, save }: { settings: AppSettings; save: (s: AppSettings) => Promise<void> }) {
  const { update, checkForUpdate, checkingUpdate, installUpdate, installProgress } = useApp();
  const [version, setVersion] = useState("");
  useEffect(() => { void getVersion().then(setVersion).catch(() => undefined); }, []);
  return (
    <Panel icon={<Sparkles size={18} />} title="App updates" desc="Flipside checks GitHub for a new version on launch and every 30 minutes. You just accept — it downloads and installs itself.">
      <div className="status-line" style={{ marginBottom: 14 }}>
        <span>Installed: <strong style={{ color: "var(--text)" }}>v{version}</strong></span>
        {update.info ? <span className="chip accent">v{update.info.version} available</span> : update.checkedAt ? <span className="chip good">Up to date</span> : null}
        {update.checkedAt ? <span>· checked {new Date(update.checkedAt).toLocaleTimeString()}</span> : null}
      </div>
      {update.error ? <div className="note err">Update check failed: {update.error}</div> : null}
      {update.info?.notes ? <p className="prose" style={{ whiteSpace: "pre-wrap", fontSize: 13 }}>{update.info.notes}</p> : null}
      <div className="form-actions" style={{ marginTop: 0 }}>
        <button className="btn" onClick={() => void checkForUpdate()} disabled={checkingUpdate}>
          <RefreshCw size={15} className={checkingUpdate ? "spin" : ""} /> Check for updates
        </button>
        {update.info ? (
          <button className="btn primary" onClick={() => void installUpdate()} disabled={!!installProgress}>
            {installProgress ? <Loader2 size={15} className="spin" /> : null}
            {installProgress ? `${installProgress.phase}… ${installProgress.percent != null ? Math.round(installProgress.percent) + "%" : ""}` : "Install & restart"}
          </button>
        ) : null}
        <label className="switch" style={{ marginLeft: "auto" }}>
          <input type="checkbox" checked={settings.autoUpdateCheck ?? true} onChange={(e) => void save({ ...settings, autoUpdateCheck: e.target.checked })} />
          Check automatically
        </label>
      </div>
    </Panel>
  );
}

/* ------------------------------------------------------------------- deemix */

function DeemixPanel({ settings, save }: { settings: AppSettings; save: (s: AppSettings) => Promise<void> }) {
  const { toast } = useApp();
  const [draft, setDraft] = useState(settings);
  const [status, setStatus] = useState<DeemixCredentialStatus | null>(null);
  const [arl, setArl] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => setDraft(settings), [settings]);
  useEffect(() => { void commands.getDeemixCredentialStatus().then(setStatus).catch(() => undefined); }, []);

  async function saveArl() {
    setBusy(true);
    try {
      const test = await commands.saveDeemixArl(arl);
      setArl("");
      setStatus(await commands.getDeemixCredentialStatus());
      toast("success", `Deezer connected as ${test.accountName}`);
    } catch (e) { toast("error", errorMessage(e)); } finally { setBusy(false); }
  }
  async function test() {
    setBusy(true);
    try {
      const t = await commands.testDeemixConnection();
      toast("success", `${t.accountName} (${t.country ?? "?"}) — ${t.canStreamLossless ? "lossless" : t.canStreamHq ? "HQ" : "standard"} streaming`);
    } catch (e) { toast("error", errorMessage(e)); } finally { setBusy(false); }
  }

  return (
    <Panel icon={<Zap size={18} />} title="Deezer (Deemix)" desc="Your Deezer ARL cookie lets Flipside download straight from your account.">
      <div className="form">
        <Field label="ARL" wide hint={status?.configured ? <span className="chip good"><Check size={11} /> stored securely</span> : <span className="chip warn">not set</span>}>
          <div className="row">
            <input type="password" placeholder={status?.configured ? "•••••••• (enter a new one to replace)" : "Paste your ARL"} value={arl} onChange={(e) => setArl(e.target.value)} spellCheck={false} />
            <button className="btn" disabled={!arl || busy} onClick={() => void saveArl()}>Save</button>
            <button className="btn ghost" disabled={!status?.configured || busy} onClick={() => void test()}>Test</button>
          </div>
        </Field>
        <FolderField label="Download folder" value={draft.deemixDownloadPath ?? ""} onChange={(v) => setDraft({ ...draft, deemixDownloadPath: v })} />
        <Field label="Quality">
          <select value={draft.deemixDownloadQuality} onChange={(e) => setDraft({ ...draft, deemixDownloadQuality: e.target.value })}>
            <option value="mp3_128">MP3 128</option>
            <option value="mp3_320">MP3 320</option>
            <option value="flac">FLAC</option>
          </select>
        </Field>
        <Field label="Folder layout">
          <select value={draft.deemixDownloadOrganization} onChange={(e) => setDraft({ ...draft, deemixDownloadOrganization: e.target.value })}>
            <option value="flat_artist_album_year">Artist - Album (Year)</option>
            <option value="artist_album_year_folders">Artist / Album (Year)</option>
          </select>
        </Field>
        <Field label="If the quality isn't available">
          <label className="switch" style={{ height: 38 }}>
            <input type="checkbox" checked={draft.deemixDownloadFallback} onChange={(e) => setDraft({ ...draft, deemixDownloadFallback: e.target.checked })} />
            Fall back to a lower quality
          </label>
        </Field>
      </div>
      <div className="form-actions">
        <button className="btn primary" onClick={() => void save(draft).then(() => toast("success", "Saved"))}>Save Deezer settings</button>
      </div>
    </Panel>
  );
}

/* ------------------------------------------------------------------- usenet */

function UsenetPanel() {
  const { toast } = useApp();
  const [boot, setBoot] = useState<UsenetBootstrap | null>(null);
  const [profile, setProfile] = useState<UsenetProfile | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [testMsg, setTestMsg] = useState<string | null>(null);

  const load = useCallback(async () => {
    const b = await commands.usenetBootstrap();
    setBoot(b);
    setProfile(b.profile);
  }, []);
  useEffect(() => { void load().catch((e) => toast("error", errorMessage(e))); }, [load, toast]);

  if (!boot || !profile) return <Panel icon={<Newspaper size={18} />} title="Usenet" desc="Loading…"><span /></Panel>;
  const set = (patch: Partial<UsenetProfile>) => setProfile({ ...profile, ...patch });

  async function save() {
    setBusy(true);
    try {
      const b = await commands.usenetSaveProfile({ profile: profile!, prowlarrApiKey: apiKey || null, newsPassword: password || null });
      setBoot(b); setProfile(b.profile); setApiKey(""); setPassword("");
      toast("success", "Usenet settings saved");
    } catch (e) { toast("error", errorMessage(e)); } finally { setBusy(false); }
  }
  async function test() {
    setBusy(true); setTestMsg(null);
    try { setTestMsg((await commands.usenetTestConnections()).message); } catch (e) { setTestMsg(errorMessage(e)); } finally { setBusy(false); }
  }

  return (
    <Panel icon={<Newspaper size={18} />} title="Usenet" desc="Prowlarr finds the NZBs; Flipside downloads, repairs and extracts them itself.">
      <div className="form">
        <Field label="Prowlarr URL"><input value={profile.prowlarrUrl} onChange={(e) => set({ prowlarrUrl: e.target.value })} spellCheck={false} /></Field>
        <Field label="Prowlarr API key" hint={boot.hasProwlarrApiKey ? <span className="chip good"><Check size={11} /> stored</span> : <span className="chip warn">not set</span>}>
          <input type="password" placeholder={boot.hasProwlarrApiKey ? "•••••••• (replace)" : "API key"} value={apiKey} onChange={(e) => setApiKey(e.target.value)} />
        </Field>
        <Field label="Usenet server"><input value={profile.newsHost} onChange={(e) => set({ newsHost: e.target.value })} spellCheck={false} /></Field>
        <Field label="Port / TLS">
          <div className="row">
            <input type="number" value={profile.newsPort} onChange={(e) => set({ newsPort: Number(e.target.value) })} />
            <label className="switch"><input type="checkbox" checked={profile.useTls} onChange={(e) => set({ useTls: e.target.checked })} />TLS</label>
          </div>
        </Field>
        <Field label="Username"><input value={profile.username} onChange={(e) => set({ username: e.target.value })} spellCheck={false} autoComplete="off" /></Field>
        <Field label="Password" hint={boot.hasNewsPassword ? <span className="chip good"><Check size={11} /> stored</span> : <span className="chip warn">not set</span>}>
          <input type="password" placeholder={boot.hasNewsPassword ? "•••••••• (replace)" : "Usenet password"} value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" />
        </Field>
        <FolderField label="Download folder" value={profile.downloadDirectory} onChange={(v) => set({ downloadDirectory: v })} />
        <Field label="Connections"><input type="number" min={1} max={50} value={profile.connections} onChange={(e) => set({ connections: Number(e.target.value) })} /></Field>
        <Field label="Tools">
          <div className="status-line">
            <span className={`chip ${boot.extractorPath ? "good" : "warn"}`}>unrar {boot.extractorPath ? "found" : "missing"}</span>
            <span className={`chip ${boot.par2Path ? "good" : "warn"}`}>par2 {boot.par2Path ? "found" : "missing"}</span>
          </div>
        </Field>
      </div>
      {testMsg ? <div className="note" style={{ marginTop: 14 }}>{testMsg}</div> : null}
      <div className="form-actions">
        <button className="btn primary" disabled={busy} onClick={() => void save()}>Save Usenet settings</button>
        <button className="btn ghost" disabled={busy} onClick={() => void test()}>Test connections</button>
      </div>
    </Panel>
  );
}

/* ----------------------------------------------------------------- soulseek */

function SoulseekPanel() {
  const { toast, connection } = useApp();
  const [boot, setBoot] = useState<ConnectionBootstrap | null>(null);
  const [profile, setProfile] = useState<ConnectionProfile | null>(null);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void commands.connectionBootstrap().then((b) => { setBoot(b); setProfile(b.profile ?? b.suggestedProfile); }).catch((e) => toast("error", errorMessage(e)));
  }, [toast]);

  if (!boot || !profile) return <Panel icon={<Share2 size={18} />} title="Soulseek" desc="Loading…"><span /></Panel>;
  const set = (patch: Partial<ConnectionProfile>) => setProfile({ ...profile, ...patch });
  const state = connection?.state ?? boot.snapshot.state;

  async function save(connect: boolean) {
    setBusy(true);
    try {
      const b = await commands.connectionSaveProfile({ profile: profile!, password: password || null });
      setBoot(b); setProfile(b.profile ?? b.suggestedProfile); setPassword("");
      if (connect) await commands.connectionConnect();
      toast("success", connect ? "Saved — connecting to Soulseek" : "Soulseek settings saved");
    } catch (e) { toast("error", errorMessage(e)); } finally { setBusy(false); }
  }

  return (
    <Panel icon={<Share2 size={18} />} title="Soulseek" desc="Flipside talks to the Soulseek network directly — no slskd needed.">
      <div className="status-line" style={{ marginBottom: 14 }}>
        <span className={`conn-dot ${state === "online" ? "online" : state === "error" ? "error" : state === "offline" || state === "unconfigured" ? "" : "busy"}`} />
        <span style={{ textTransform: "capitalize" }}>{state}</span>
        {connection?.message ? <span>· {connection.message}</span> : null}
      </div>
      <div className="form">
        <Field label="Username"><input value={profile.username} onChange={(e) => set({ username: e.target.value })} spellCheck={false} autoComplete="off" /></Field>
        <Field label="Password" hint={boot.hasPassword ? <span className="chip good"><Check size={11} /> stored</span> : <span className="chip warn">not set</span>}>
          <input type="password" placeholder={boot.hasPassword ? "•••••••• (replace)" : "Soulseek password"} value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" />
        </Field>
        <Field label="Server"><input value={profile.serverHost} onChange={(e) => set({ serverHost: e.target.value })} spellCheck={false} /></Field>
        <Field label="Port"><input type="number" value={profile.serverPort} onChange={(e) => set({ serverPort: Number(e.target.value) })} /></Field>
        <FolderField label="Download folder" value={profile.downloadDirectory} onChange={(v) => set({ downloadDirectory: v })} />
        <Field label="Options">
          <div style={{ display: "flex", gap: 20, height: 38, alignItems: "center" }}>
            <label className="switch"><input type="checkbox" checked={profile.rememberPassword} onChange={(e) => set({ rememberPassword: e.target.checked })} />Remember password</label>
            <label className="switch"><input type="checkbox" checked={profile.autoConnect} onChange={(e) => set({ autoConnect: e.target.checked })} />Connect on launch</label>
          </div>
        </Field>
      </div>
      <div className="form-actions">
        <button className="btn primary" disabled={busy} onClick={() => void save(true)}>Save & connect</button>
        <button className="btn" disabled={busy} onClick={() => void save(false)}>Save</button>
        {state === "online" || state === "connecting" || state === "reconnecting" ? (
          <button className="btn ghost" onClick={() => void commands.connectionDisconnect().catch((e) => toast("error", errorMessage(e)))}>Disconnect</button>
        ) : null}
      </div>
    </Panel>
  );
}

/* --------------------------------------------------------------- metadata */

function MetadataPanel() {
  const { toast } = useApp();
  const [keys, setKeys] = useState<KeyStatus[]>([]);
  const [values, setValues] = useState<Record<string, string>>({});
  useEffect(() => { void commands.keysStatus().then(setKeys).catch(() => undefined); }, []);

  async function save(id: string) {
    try { setKeys(await commands.keysSave(id, values[id] ?? "")); setValues((v) => ({ ...v, [id]: "" })); toast("success", "Key saved"); }
    catch (e) { toast("error", errorMessage(e)); }
  }
  async function remove(id: string) {
    try { setKeys(await commands.keysDelete(id)); } catch (e) { toast("error", errorMessage(e)); }
  }

  return (
    <Panel icon={<KeyRound size={18} />} title="Metadata providers" desc="Last.fm adds bios, tags and listener counts. Fanart.tv adds artist backdrops. Discogs adds label and style details. All optional.">
      <div className="form">
        {keys.map((k) => (
          <Field key={k.id} label={k.label} hint={k.configured ? <span className="chip good"><Check size={11} /> stored</span> : <span className="chip warn">not set</span>}>
            <div className="row">
              <input type="password" placeholder={k.configured ? "•••••••• (replace)" : "Paste key"} value={values[k.id] ?? ""} onChange={(e) => setValues((v) => ({ ...v, [k.id]: e.target.value }))} spellCheck={false} />
              <button className="btn" disabled={!values[k.id]} onClick={() => void save(k.id)}>Save</button>
              {k.configured ? <button className="btn ghost" onClick={() => void remove(k.id)}>Remove</button> : null}
            </div>
          </Field>
        ))}
      </div>
    </Panel>
  );
}

/* --------------------------------------------------------------------- page */

export default function Settings() {
  const { toast } = useApp();
  const [settings, setSettings] = useState<AppSettings | null>(null);
  useEffect(() => { void commands.getSettings().then(setSettings).catch((e) => toast("error", errorMessage(e))); }, [toast]);

  const save = useCallback(async (next: AppSettings) => {
    try { setSettings(await commands.saveSettings(next)); } catch (e) { toast("error", errorMessage(e)); }
  }, [toast]);

  return (
    <div className="settings">
      <h1 className="page-title">Settings</h1>
      <p className="page-sub">Providers, folders and updates. Secrets are kept in Windows Credential Manager.</p>
      {settings ? <UpdatesPanel settings={settings} save={save} /> : null}
      {settings ? <DeemixPanel settings={settings} save={save} /> : null}
      <UsenetPanel />
      <SoulseekPanel />
      <SharingPanel />
      <MetadataPanel />
    </div>
  );
}
