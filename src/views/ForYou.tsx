import { useCallback, useEffect, useRef, useState } from "react";
import { Disc3, Download, Loader2, RefreshCw, Search as SearchIcon, Sparkles, X } from "lucide-react";
import { commands, events } from "../bindings";
import type { AlbumCard, DiscoveryAlbum, DiscoveryPage, DiscoveryQuery, DiscoveryStatus } from "../bindings";
import { useApp } from "../store";
import GetDrawer from "../components/GetDrawer";
import TagInput from "../components/TagInput";
import { Empty } from "../components/ui";
import { errorMessage } from "../lib/format";

const PAGE = 60;
type Source = "all" | "missing" | "similar";

function DiscCover({ src, alt }: { src: string; alt: string }) {
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  return (
    <div className="art">
      <div className="ph"><Disc3 size={34} /></div>
      {!failed ? (
        <img
          src={src} alt={alt} loading="lazy" decoding="async" draggable={false}
          className={loaded ? "loaded" : ""} style={{ position: "absolute", inset: 0 }}
          onLoad={() => setLoaded(true)} onError={() => setFailed(true)}
        />
      ) : null}
    </div>
  );
}

function Tile({ album, busy, onOpen }: { album: DiscoveryAlbum; busy: boolean; onOpen: (a: DiscoveryAlbum) => void }) {
  const { go } = useApp();
  return (
    <div className="card" role="link" tabIndex={0} onClick={() => onOpen(album)} onKeyDown={(e) => e.key === "Enter" && onOpen(album)}>
      <div style={{ position: "relative" }}>
        <DiscCover src={album.cover} alt={album.title} />
        <button className="get" title="Get this album" aria-label={`Get ${album.title}`} onClick={(e) => { e.stopPropagation(); onOpen(album); }}>
          {busy ? <Loader2 size={18} className="spin" /> : <Download size={18} />}
        </button>
      </div>
      <div className="title" title={album.title}>{album.title}</div>
      <div className="sub">
        <button onClick={(e) => { e.stopPropagation(); go({ name: "search", q: album.artist }); }}>{album.artist}</button>
        {album.year ? ` · ${album.year}` : ""}
      </div>
      <div className="sub" title={album.similar ? `Because you own ${album.because.join(", ")}` : "You own other albums by this artist"} style={{ color: "var(--faint)" }}>
        {album.similar ? (album.because.length ? `Like ${album.because[0]}` : "Similar to your artists") : album.genres[0] ?? "Missing from your library"}
      </div>
    </div>
  );
}

export default function ForYou() {
  const { toast, go } = useApp();
  const [source, setSource] = useState<Source>("all");
  const [decade, setDecade] = useState<number | null>(null);
  const [genre, setGenre] = useState<string | null>(null);
  const [yearFrom, setYearFrom] = useState("");
  const [yearTo, setYearTo] = useState("");
  const [text, setText] = useState("");
  const [debounced, setDebounced] = useState("");
  const [sort, setSort] = useState("affinity");
  const [exclude, setExclude] = useState<string[]>([]);
  const [excludeReady, setExcludeReady] = useState(false);

  const [page, setPage] = useState<DiscoveryPage | null>(null);
  const [items, setItems] = useState<DiscoveryAlbum[]>([]);
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState<DiscoveryStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [resolving, setResolving] = useState<string | null>(null);
  const [drawer, setDrawer] = useState<AlbumCard | null>(null);
  const generation = useRef(0);

  useEffect(() => { const t = window.setTimeout(() => setDebounced(text), 250); return () => window.clearTimeout(t); }, [text]);

  const build = useCallback((offset: number): DiscoveryQuery => ({
    source, decade, genre, text: debounced, sort, offset, limit: PAGE, excludeGenres: exclude,
    yearFrom: yearFrom ? Number(yearFrom) : null, yearTo: yearTo ? Number(yearTo) : null,
  }), [source, decade, genre, debounced, sort, yearFrom, yearTo, exclude]);

  const load = useCallback(async (reset: boolean) => {
    const mine = ++generation.current;
    setLoading(true);
    try {
      const result = await commands.discoveryQuery(build(reset ? 0 : items.length));
      if (mine !== generation.current) return;
      setPage(result);
      setItems((prev) => (reset ? result.items : [...prev, ...result.items]));
      setError(null);
    } catch (e) {
      if (mine === generation.current) setError(errorMessage(e));
    } finally {
      if (mine === generation.current) setLoading(false);
    }
  }, [build, items.length]);
  // The crawl listener below is registered once; it must always call the *latest* load (current filters).
  const loadRef = useRef(load);
  loadRef.current = load;

  // Reload from the top whenever a filter changes.
  useEffect(() => {
    if (excludeReady) void load(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, decade, genre, debounced, sort, yearFrom, yearTo, exclude, excludeReady]);

  // Excluded genres are a standing preference: load them first, save changes.
  useEffect(() => {
    void commands.getSettings().then((s) => setExclude(s.excludedGenres ?? [])).catch(() => undefined).finally(() => setExcludeReady(true));
  }, []);
  const changeExclude = useCallback((next: string[]) => {
    setExclude(next);
    if (genre && next.some((g) => g.toLowerCase() === genre.toLowerCase())) setGenre(null);
    void commands.getSettings()
      .then((s) => commands.saveSettings({ ...s, excludedGenres: next }))
      .catch((e) => toast("error", errorMessage(e)));
  }, [genre, toast]);

  // Crawl status + live progress; refresh the grid as the crawl finds more albums.
  useEffect(() => {
    let off: (() => void) | undefined;
    let cancelled = false;
    let lastReload = 0;
    void commands.discoveryStatus().then(setStatus).catch(() => undefined);
    void events.flipsideDiscoveryProgress.listen((e) => {
      setStatus((s) => (s ? { ...s, progress: e.payload } : s));
      if (!e.payload.running) {
        void commands.discoveryStatus().then(setStatus).catch(() => undefined);
        void loadRef.current(true);
      } else if (e.payload.stage === "albums" && Date.now() - lastReload > 20_000) {
        lastReload = Date.now();
        void loadRef.current(true);
      }
    }).then((fn) => { if (cancelled) fn(); else off = fn; });
    return () => { cancelled = true; off?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function open(album: DiscoveryAlbum) {
    setResolving(album.id);
    try {
      const found = await commands.catalogResolveAlbum(album.artist, album.title);
      if (found) {
        go({ name: "album", id: found.id });
      } else {
        // Not on Deezer: still downloadable from Usenet or Soulseek.
        setDrawer({
          id: "", title: album.title, artist: album.artist, artistId: null, cover: album.cover, coverLarge: album.cover,
          year: album.year, releaseDate: null, kind: "album", trackCount: null, explicit: false,
        });
      }
    } catch (e) {
      toast("error", errorMessage(e));
    } finally {
      setResolving(null);
    }
  }

  const progress = status?.progress;
  const running = !!progress?.running;
  const pct = progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : null;
  const lib = status?.library;
  const filtered = decade !== null || genre !== null || !!yearFrom || !!yearTo || !!debounced;
  const suggestions = ["scores", "Soundtrack", ...(page?.genres.map((g) => g.label) ?? [])];

  return (
    <>
      <h1 className="page-title">For you</h1>
      <p className="page-sub">
        Studio albums you don't own yet: gaps in artists you collect, plus albums by artists similar to them. Live albums,
        compilations, EPs and singles are left out.
      </p>

      {lib && !lib.loaded ? (
        <div className={`note${lib.error ? " err" : ""}`}>
          {lib.error ? <>Couldn't read your library: {lib.error} Set the file locations in Settings.</> : <><Loader2 size={13} className="spin" style={{ verticalAlign: -2 }} /> Reading your library…</>}
        </div>
      ) : null}

      {running ? (
        <div className="result" style={{ gridTemplateColumns: "1fr" }}>
          <div className="name"><Sparkles size={14} style={{ verticalAlign: -2 }} /> {progress?.message}</div>
          <div className={`progress${pct == null ? " indeterminate" : ""}`}><i style={pct != null ? { width: `${pct}%` } : undefined} /></div>
          <div className="status-line">Finding similar artists and their albums in the background; new albums appear here as they are found.</div>
        </div>
      ) : null}

      <div className="filters">
        <div className="seg" role="tablist" aria-label="Source">
          {([
            ["all", "All", (page?.missingTotal ?? 0) + (page?.similarTotal ?? 0)],
            ["missing", "From artists you own", page?.missingTotal ?? 0],
            ["similar", "From similar artists", page?.similarTotal ?? 0],
          ] as Array<[Source, string, number]>).map(([key, label, count]) => (
            <button key={key} role="tab" aria-selected={source === key} className={source === key ? "active" : ""} onClick={() => setSource(key)}>
              {label} <span>{count.toLocaleString()}</span>
            </button>
          ))}
        </div>
        <div className="filter-row">
          <div className="searchbox small">
            <SearchIcon size={14} />
            <input value={text} onChange={(e) => setText(e.target.value)} placeholder="Filter by artist or title" spellCheck={false} aria-label="Filter" />
          </div>
          <label className="mini">From <input inputMode="numeric" maxLength={4} value={yearFrom} onChange={(e) => { setYearFrom(e.target.value.replace(/\D/g, "")); setDecade(null); }} placeholder="year" /></label>
          <label className="mini">To <input inputMode="numeric" maxLength={4} value={yearTo} onChange={(e) => { setYearTo(e.target.value.replace(/\D/g, "")); setDecade(null); }} placeholder="year" /></label>
          <label className="mini">Sort
            <select value={sort} onChange={(e) => setSort(e.target.value)}>
              <option value="affinity">Best match</option>
              <option value="newest">Newest</option>
              <option value="oldest">Oldest</option>
              <option value="artist">Artist A–Z</option>
            </select>
          </label>
          {filtered ? (
            <button className="btn small ghost" onClick={() => { setDecade(null); setGenre(null); setYearFrom(""); setYearTo(""); setText(""); }}><X size={13} /> Clear</button>
          ) : null}
          <button className="btn small" style={{ marginLeft: "auto" }} disabled={running} onClick={() => void commands.discoveryRefresh().then((started) => toast("info", started ? "Looking for new similar artists…" : "Already running")).catch((e) => toast("error", errorMessage(e)))}>
            <RefreshCw size={13} className={running ? "spin" : ""} /> Find more
          </button>
        </div>
        <TagInput
          label="Exclude genres" tags={exclude} onChange={changeExclude} suggestions={suggestions}
          placeholder='e.g. scores, Soundtrack, Synthwave (scores = film, TV and game music)'
        />
        {page && page.decades.length > 0 ? (
          <div className="chips-row" aria-label="Decade">
            {page.decades.map((d) => (
              <button key={d.key} className={`chip pick${decade === Number(d.key) ? " accent" : ""}`} onClick={() => { setDecade(decade === Number(d.key) ? null : Number(d.key)); setYearFrom(""); setYearTo(""); }}>
                {d.label} · {d.count.toLocaleString()}
              </button>
            ))}
          </div>
        ) : null}
        {page && page.genres.length > 0 ? (
          <div className="chips-row" aria-label="Genre">
            {page.genres.slice(0, 24).map((g) => (
              <button key={g.key} className={`chip pick${genre?.toLowerCase() === g.key ? " accent" : ""}`} onClick={() => setGenre(genre?.toLowerCase() === g.key ? null : g.label)}>
                {g.label} · {g.count.toLocaleString()}
              </button>
            ))}
          </div>
        ) : null}
      </div>

      {error ? <Empty>{error}</Empty> : null}
      {!error && page && items.length === 0 && !loading ? (
        <Empty icon={<Disc3 size={38} />}>
          {filtered ? "Nothing matches those filters." : source === "similar" && !running ? "No similar artists yet. They are found in the background once Last.fm is set up in Settings." : "Nothing to show yet."}
        </Empty>
      ) : null}

      <div className="grid" style={{ marginTop: 18 }}>
        {items.map((a) => <Tile key={a.id} album={a} busy={resolving === a.id} onOpen={(x) => void open(x)} />)}
      </div>

      {page && items.length < page.total ? (
        <div style={{ textAlign: "center", marginTop: 26 }}>
          <button className="btn" disabled={loading} onClick={() => void load(false)}>
            {loading ? <Loader2 size={15} className="spin" /> : null} Show more ({(page.total - items.length).toLocaleString()} left)
          </button>
        </div>
      ) : null}

      {drawer ? <GetDrawer album={drawer} onClose={() => setDrawer(null)} /> : null}
    </>
  );
}
