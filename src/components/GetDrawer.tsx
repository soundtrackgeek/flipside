import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CircleDashed, CircleSlash, CloudDownload, Loader2, Newspaper, RefreshCw, Share2, Wand2, X, XCircle, Zap, CheckCircle2 } from "lucide-react";
import { commands, events } from "../bindings";
import type {
  AlbumCard,
  DeemixAlbumDownloadPreflight,
  SearchResult,
  UsenetSearchResponse,
  UsenetSearchResult,
} from "../bindings";
import { useApp } from "../store";
import { errorMessage, formatAge, formatBytes, formatSpeed, remoteTitle, simplifyTitle } from "../lib/format";
import { autoGet, group, type AutoOutcome, type Candidate, type StepId, type StepState } from "../lib/autoGet";

export type Tab = "auto" | "deemix" | "usenet" | "soulseek";

export default function GetDrawer({
  album, onClose, initialTab = "auto", autoStart = false,
}: { album: AlbumCard; onClose: () => void; initialTab?: Tab; autoStart?: boolean }) {
  const [tab, setTab] = useState<Tab>(initialTab);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <>
      <div className="scrim" onClick={onClose} />
      <aside className="drawer" role="dialog" aria-label={`Get ${album.title}`}>
        <div className="drawer-head">
          <img src={album.cover} alt="" />
          <div className="grow">
            <h3>{album.title}</h3>
            <p>{album.artist}{album.year ? ` · ${album.year}` : ""}</p>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="Close"><X size={18} /></button>
        </div>
        <div className="tabs">
          <button className={`tab${tab === "auto" ? " active" : ""}`} onClick={() => setTab("auto")}><Wand2 size={15} /> Auto</button>
          <button className={`tab${tab === "deemix" ? " active" : ""}`} onClick={() => setTab("deemix")}><Zap size={15} /> Deezer</button>
          <button className={`tab${tab === "usenet" ? " active" : ""}`} onClick={() => setTab("usenet")}><Newspaper size={15} /> Usenet</button>
          <button className={`tab${tab === "soulseek" ? " active" : ""}`} onClick={() => setTab("soulseek")}><Share2 size={15} /> Soulseek</button>
        </div>
        <div className="drawer-body">
          {tab === "auto" ? <AutoTab album={album} autoStart={autoStart} /> : null}
          {tab === "deemix" ? <DeemixTab album={album} /> : null}
          {tab === "usenet" ? <UsenetTab album={album} /> : null}
          {tab === "soulseek" ? <SoulseekTab album={album} /> : null}
        </div>
      </aside>
    </>
  );
}

/* ------------------------------------------------------------------ Deemix */

function DeemixTab({ album }: { album: AlbumCard }) {
  if (!album.id) {
    return <div className="empty">This album wasn't found on Deezer. Try Usenet or Soulseek.</div>;
  }
  return <DeemixTabInner album={album} />;
}

function DeemixTabInner({ album }: { album: AlbumCard }) {
  const { deemixJobs, startDeemix, go } = useApp();
  const [pre, setPre] = useState<DeemixAlbumDownloadPreflight | null>(null);
  const [error, setError] = useState<string | null>(null);
  const job = deemixJobs.find((j) => j.albumId === album.id && j.status === "running") ?? deemixJobs.find((j) => j.albumId === album.id);

  useEffect(() => {
    let alive = true;
    commands
      .preflightDeemixAlbumDownload({ albumId: album.id, artist: album.artist, album: album.title, year: album.year })
      .then((p) => alive && setPre(p))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => { alive = false; };
  }, [album.id, album.artist, album.title, album.year, job?.status]);

  const running = job?.status === "running";
  const pct = job && job.total > 0 ? Math.round((job.completed / job.total) * 100) : null;

  return (
    <>
      <p className="page-sub" style={{ marginTop: 0 }}>
        Downloads straight from Deezer through your account, tags the files and drops them in your Deezer folder.
      </p>
      {error ? <div className="note err">{error}</div> : null}
      {pre?.alreadyDownloaded && !running ? (
        <div className="note ok"><CheckCircle2 size={14} style={{ verticalAlign: -2 }} /> {pre.message}</div>
      ) : null}
      {job ? (
        <div className="result" style={{ gridTemplateColumns: "1fr" }}>
          <div className="name">{job.status === "failed" ? "Failed" : job.status === "done" ? "Done" : job.message}</div>
          {running ? (
            <div className={`progress${pct == null ? " indeterminate" : ""}`}><i style={pct != null ? { width: `${pct}%` } : undefined} /></div>
          ) : (
            <div className="meta"><span className={`chip ${job.status === "done" ? "good" : "bad"}`}>{job.message}</span></div>
          )}
        </div>
      ) : null}
      <div className="form-actions">
        <button className="btn primary" disabled={running} onClick={() => void startDeemix(album, !!pre?.alreadyDownloaded)}>
          {running ? <Loader2 size={16} className="spin" /> : <CloudDownload size={16} />}
          {pre?.alreadyDownloaded ? "Download another copy" : "Download from Deezer"}
        </button>
        <button className="btn ghost" onClick={() => go({ name: "downloads" })}>Open downloads</button>
      </div>
    </>
  );
}

/* ------------------------------------------------------------------ Usenet */

function UsenetTab({ album }: { album: AlbumCard }) {
  const { setUsenet, toast, go } = useApp();
  const [response, setResponse] = useState<UsenetSearchResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [queued, setQueued] = useState<Set<string>>(new Set());

  const search = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      setResponse(await commands.usenetSearch({ title: simplifyTitle(album.title), artist: album.artist, year: album.year, limit: 40 }));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }, [album]);

  useEffect(() => { void search(); }, [search]);

  async function grab(r: UsenetSearchResult) {
    try {
      const queue = await commands.usenetEnqueueDownload({
        guid: r.guid, title: r.title, indexer: r.indexer, downloadUrl: r.downloadUrl, sizeBytes: r.sizeBytes,
        expectedArtist: album.artist, expectedAlbum: album.title, expectedYear: album.year, releaseGroupId: null,
      });
      setUsenet(queue);
      setQueued((s) => new Set(s).add(r.guid));
      toast("success", "Queued on Usenet");
    } catch (e) {
      toast("error", errorMessage(e));
    }
  }

  return (
    <>
      <div className="form-actions" style={{ marginTop: 0, marginBottom: 14 }}>
        <button className="btn small" onClick={() => void search()} disabled={busy}>
          <RefreshCw size={14} className={busy ? "spin" : ""} /> Search again
        </button>
        {queued.size > 0 ? <button className="btn small ghost" onClick={() => go({ name: "downloads" })}>View queue</button> : null}
      </div>
      {error ? <div className="note err">{error} — check Settings → Providers.</div> : null}
      {busy && !response ? <div className="progress indeterminate"><i /></div> : null}
      {response && response.results.length === 0 ? <div className="empty">No NZBs found for this album.</div> : null}
      {response?.results.map((r) => (
        <div className="result" key={r.guid}>
          <div className="name">{r.title}</div>
          <button className="btn small primary go" disabled={queued.has(r.guid)} onClick={() => void grab(r)}>
            {queued.has(r.guid) ? "Queued" : "Grab"}
          </button>
          <div className="meta">
            <span className="chip">{r.indexer}</span>
            <span className="chip">{formatBytes(r.sizeBytes)}</span>
            <span className="chip">{formatAge(r.ageDays)}</span>
            {r.grabs != null ? <span className="chip">{r.grabs} grabs</span> : null}
            {r.matchScore >= 70 ? <span className="chip good">strong match</span> : null}
          </div>
        </div>
      ))}
    </>
  );
}

/* --------------------------------------------------------------- Soulseek */

function SoulseekTab({ album }: { album: AlbumCard }) {
  const { connection, setSoulseek, toast, go } = useApp();
  const [results, setResults] = useState<Map<string, SearchResult>>(new Map());
  const [state, setState] = useState<"idle" | "searching" | "done">("idle");
  const [error, setError] = useState<string | null>(null);
  const [queued, setQueued] = useState<Set<string>>(new Set());
  const clientId = useRef<string>("");
  const online = connection?.state === "online";

  const stop = useCallback(() => {
    if (clientId.current) {
      void commands.searchStop(clientId.current).catch(() => undefined);
      void commands.searchClose(clientId.current).catch(() => undefined);
      clientId.current = "";
    }
  }, []);

  const search = useCallback(async () => {
    stop();
    const id = `flip-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
    clientId.current = id;
    setResults(new Map());
    setError(null);
    setState("searching");
    const query = `${album.artist} ${simplifyTitle(album.title)}`.trim();
    try {
      await commands.searchStart(id, query);
    } catch (e) {
      setError(errorMessage(e));
      setState("done");
    }
  }, [album, stop]);

  useEffect(() => {
    let off: (() => void) | undefined;
    let cancelled = false;
    void events.flipsideSoulseekSearch.listen((e) => {
      const p = e.payload;
      if (p.snapshot.clientId !== clientId.current) return;
      if (p.results.length) {
        setResults((prev) => { const next = new Map(prev); p.results.forEach((r) => next.set(r.id, r)); return next; });
      }
      if (p.event === "completed" || p.event === "stopped") setState("done");
      if (p.event === "error") { setError(p.snapshot.message); setState("done"); }
    }).then((fn) => { if (cancelled) fn(); else off = fn; });
    return () => { cancelled = true; off?.(); stop(); };
  }, [stop]);

  useEffect(() => {
    if (online) void search();
  }, [online, search]);

  const candidates = useMemo(() => group(results.values()), [results]);

  async function download(c: Candidate) {
    try {
      const snapshot = await commands.transferEnqueueRelease({
        title: `${album.artist} - ${album.title}${album.year ? ` (${album.year})` : ""}`,
        username: c.username,
        remoteFolder: c.folder,
        files: c.files.map((f) => ({ title: remoteTitle(f.filename), remoteFilename: f.filename, sizeBytes: f.sizeBytes })),
        expectedTrackCount: c.files.length,
        releaseGroupId: null,
        alternatives: [],
      });
      setSoulseek(snapshot);
      setQueued((s) => new Set(s).add(c.id));
      toast("success", `${c.files.length} files queued from ${c.username}`);
    } catch (e) {
      toast("error", errorMessage(e));
    }
  }

  async function connect() {
    try { await commands.connectionConnect(); } catch (e) { toast("error", errorMessage(e)); }
  }

  if (!online) {
    return (
      <div className="empty">
        <div className="ico"><Share2 size={34} /></div>
        {connection?.state === "unconfigured"
          ? "Soulseek isn't set up yet. Add your username and password in Settings → Providers."
          : <>Soulseek is {connection?.state ?? "offline"}. {connection?.message}</>}
        <div className="form-actions" style={{ justifyContent: "center" }}>
          {connection?.state === "offline" || connection?.state === "error" ? <button className="btn primary" onClick={() => void connect()}>Connect</button> : null}
          <button className="btn ghost" onClick={() => go({ name: "settings" })}>Open settings</button>
        </div>
      </div>
    );
  }

  return (
    <>
      <div className="form-actions" style={{ marginTop: 0, marginBottom: 14 }}>
        <button className="btn small" onClick={() => void search()} disabled={state === "searching"}>
          <RefreshCw size={14} className={state === "searching" ? "spin" : ""} /> Search again
        </button>
        <span className="status-line">
          {state === "searching" ? "Listening to the network…" : `${candidates.length} sources`}
        </span>
        {queued.size > 0 ? <button className="btn small ghost" onClick={() => go({ name: "downloads" })}>View queue</button> : null}
      </div>
      {error ? <div className="note err">{error}</div> : null}
      {state === "searching" && candidates.length === 0 ? <div className="progress indeterminate"><i /></div> : null}
      {state === "done" && candidates.length === 0 && !error ? <div className="empty">Nobody is sharing this album right now.</div> : null}
      {candidates.map((c) => (
        <div className="result" key={c.id}>
          <div className="name" title={c.folder}>{c.folder.split("\\").slice(-2).join(" / ")}</div>
          <button className="btn small primary go" disabled={queued.has(c.id)} onClick={() => void download(c)}>
            {queued.has(c.id) ? "Queued" : "Download"}
          </button>
          <div className="meta">
            <span className="chip accent">{[...c.formats].join(" / ")}</span>
            <span className="chip">{c.files.length} files</span>
            <span className="chip">{formatBytes(c.totalSize)}</span>
            <span className="chip">{c.username}</span>
            <span className={`chip ${c.slotFree ? "good" : "warn"}`}>{c.slotFree ? "free slot" : `queue ${c.queue}`}</span>
            <span className="chip">{formatSpeed(c.speed)}</span>
          </div>
        </div>
      ))}
    </>
  );
}

/* -------------------------------------------------------------------- Auto */

interface Step { id: StepId; label: string; state: StepState; detail: string }

const INITIAL_STEPS: Step[] = [
  { id: "deemix", label: "Deezer", state: "pending", detail: "Fastest and tagged, tried first" },
  { id: "usenet", label: "Usenet", state: "pending", detail: "Best Prowlarr match" },
  { id: "soulseek", label: "Soulseek", state: "pending", detail: "Best complete folder on the network" },
];

function AutoTab({ album, autoStart }: { album: AlbumCard; autoStart: boolean }) {
  const { startDeemix, setUsenet, setSoulseek, connection, go } = useApp();
  const [steps, setSteps] = useState<Step[]>(INITIAL_STEPS);
  const [running, setRunning] = useState(false);
  const [outcome, setOutcome] = useState<AutoOutcome | null>(null);
  const started = useRef(false);
  const connectionRef = useRef(connection);
  connectionRef.current = connection;

  const run = useCallback(async () => {
    setRunning(true);
    setOutcome(null);
    setSteps(INITIAL_STEPS);
    const result = await autoGet(
      album,
      { startDeemix, setUsenet, setSoulseek, connection: () => connectionRef.current },
      (id, state, detail) => setSteps((s) => s.map((x) => (x.id === id ? { ...x, state, detail } : x))),
    );
    setOutcome(result);
    setRunning(false);
  }, [album, startDeemix, setUsenet, setSoulseek]);

  useEffect(() => {
    if (autoStart && !started.current) {
      started.current = true;
      void run();
    }
  }, [autoStart, run]);

  const icon = (s: StepState) =>
    s === "running" ? <Loader2 size={18} className="spin" /> :
    s === "done" ? <CheckCircle2 size={18} style={{ color: "var(--good)" }} /> :
    s === "failed" ? <XCircle size={18} style={{ color: "var(--bad)" }} /> :
    s === "skipped" ? <CircleSlash size={18} style={{ color: "var(--faint)" }} /> :
    <CircleDashed size={18} style={{ color: "var(--faint)" }} />;

  return (
    <>
      <p className="page-sub" style={{ marginTop: 0 }}>
        Flipside tries each provider in order and stops at the first one that delivers: Deezer, then Usenet, then Soulseek.
      </p>
      {steps.map((s) => (
        <div className="result" key={s.id} style={{ gridTemplateColumns: "28px 1fr", alignItems: "center", opacity: s.state === "pending" || s.state === "skipped" ? 0.75 : 1 }}>
          <span>{icon(s.state)}</span>
          <div>
            <div className="name">{s.label}</div>
            <div className="status-line">{s.detail}</div>
          </div>
        </div>
      ))}
      {outcome ? <div className={`note${outcome.via ? " ok" : " err"}`}>{outcome.message}</div> : null}
      <div className="form-actions">
        <button className="btn primary" disabled={running} onClick={() => void run()}>
          {running ? <Loader2 size={16} className="spin" /> : <Wand2 size={16} />}
          {running ? "Working..." : outcome ? "Run again" : "Get it automatically"}
        </button>
        <button className="btn ghost" onClick={() => go({ name: "downloads" })}>Open downloads</button>
      </div>
    </>
  );
}
