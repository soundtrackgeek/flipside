import { useCallback, useEffect, useRef, useState } from "react";
import { Disc3, Loader2, RefreshCw, Search as SearchIcon, Sparkles, Square, X } from "lucide-react";
import { commands, events } from "../bindings";
import type { DiscoveryAlbum, DiscoveryPage, DiscoveryQuery, DiscoveryStatus } from "../bindings";
import { useApp } from "../store";
import { DiscTile, useAlbumOpener, useArtistOpener } from "../components/DiscTile";
import TagInput from "../components/TagInput";
import { Empty } from "../components/ui";
import { errorMessage } from "../lib/format";
import { usePageHas, usePageState } from "../lib/pageState";

const PAGE = 60;
/** The backend returns at most this many albums per query. */
const MAX_QUERY = 200;
type Source = "all" | "missing" | "similar";
/** reset: start over at the top; more: append the next page; refresh: reload, keeping as many albums as are shown. */
type LoadMode = "reset" | "more" | "refresh";

export default function ForYou() {
  const { toast, go } = useApp();
  // Filters and loaded albums are remembered per page, so coming back to For you continues where you left off.
  const restored = usePageHas("items");
  const [source, setSource] = usePageState<Source>("source", "all");
  const [decade, setDecade] = usePageState<number | null>("decade", null);
  const [genre, setGenre] = usePageState<string | null>("genre", null);
  const [yearFrom, setYearFrom] = usePageState("yearFrom", "");
  const [yearTo, setYearTo] = usePageState("yearTo", "");
  const [text, setText] = usePageState("text", "");
  const [debounced, setDebounced] = usePageState("debounced", "");
  const [sort, setSort] = usePageState("sort", "affinity");
  const [exclude, setExclude] = usePageState<string[]>("exclude", []);
  const [excludeReady, setExcludeReady] = usePageState("excludeReady", false);

  const [page, setPage] = usePageState<DiscoveryPage | null>("page", null);
  const [items, setItems] = usePageState<DiscoveryAlbum[]>("items", []);
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = usePageState<DiscoveryStatus | null>("status", null);
  const [error, setError] = useState<string | null>(null);
  const { open, resolving, drawerNode } = useAlbumOpener();
  const openArtist = useArtistOpener();
  const generation = useRef(0);

  useEffect(() => { const t = window.setTimeout(() => setDebounced(text), 250); return () => window.clearTimeout(t); }, [text]);

  const build = useCallback((offset: number): DiscoveryQuery => ({
    source, decade, genre, text: debounced, sort, offset, limit: PAGE, excludeGenres: exclude,
    yearFrom: yearFrom ? Number(yearFrom) : null, yearTo: yearTo ? Number(yearTo) : null,
  }), [source, decade, genre, debounced, sort, yearFrom, yearTo, exclude]);

  const itemsRef = useRef(items);
  itemsRef.current = items;

  const load = useCallback(async (mode: LoadMode) => {
    const mine = ++generation.current;
    setLoading(true);
    try {
      let result: DiscoveryPage;
      if (mode === "refresh") {
        // Reload in chunks until we have as many albums as before, so the list (and scroll position) stays put.
        const want = Math.max(PAGE, itemsRef.current.length);
        const fresh: DiscoveryAlbum[] = [];
        do {
          result = await commands.discoveryQuery({ ...build(fresh.length), limit: Math.min(MAX_QUERY, want - fresh.length) });
          if (mine !== generation.current) return;
          fresh.push(...result.items);
        } while (result.items.length > 0 && fresh.length < want && fresh.length < result.total);
        result = { ...result, items: fresh };
      } else {
        result = await commands.discoveryQuery(build(mode === "reset" ? 0 : itemsRef.current.length));
      }
      if (mine !== generation.current) return;
      setPage(result);
      setItems((prev) => (mode === "more" ? [...prev, ...result.items] : result.items));
      setError(null);
    } catch (e) {
      if (mine === generation.current) setError(errorMessage(e));
    } finally {
      if (mine === generation.current) setLoading(false);
    }
  }, [build, setItems, setPage]);
  // The crawl listener below is registered once; it must always call the *latest* load (current filters).
  const loadRef = useRef(load);
  loadRef.current = load;

  // Reload from the top whenever a filter changes. Coming back to the page refreshes what was shown instead.
  const filterKey = JSON.stringify([source, decade, genre, debounced, sort, yearFrom, yearTo, exclude]);
  const shownFor = useRef(restored ? filterKey : null);
  useEffect(() => {
    if (!excludeReady) return;
    const same = shownFor.current === filterKey;
    shownFor.current = filterKey;
    void load(same ? "refresh" : "reset");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, decade, genre, debounced, sort, yearFrom, yearTo, exclude, excludeReady]);

  // Excluded genres are a standing preference: load them first, save changes.
  useEffect(() => {
    if (excludeReady) return; // already loaded earlier this session; changes are saved as they are made
    void commands.getSettings().then((s) => setExclude(s.excludedGenres ?? [])).catch(() => undefined).finally(() => setExcludeReady(true));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const changeExclude = useCallback((next: string[]) => {
    setExclude(next);
    if (genre && next.some((g) => g.toLowerCase() === genre.toLowerCase())) setGenre(null);
    void commands.getSettings()
      .then((s) => commands.saveSettings({ ...s, excludedGenres: next }))
      .catch((e) => toast("error", errorMessage(e)));
  }, [genre, toast, setExclude, setGenre]);

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
        void loadRef.current("refresh");
      } else if (e.payload.stage === "albums" && Date.now() - lastReload > 20_000) {
        lastReload = Date.now();
        void loadRef.current("refresh");
      }
    }).then((fn) => { if (cancelled) fn(); else off = fn; });
    return () => { cancelled = true; off?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The library loads in the background after launch; keep asking until it is ready, then show the albums.
  const libraryReady = !!status?.library.loaded;
  useEffect(() => {
    if (libraryReady) return;
    const timer = window.setInterval(() => {
      void commands.discoveryStatus().then((next) => {
        setStatus((s) => (s && s.progress.running ? { ...next, progress: s.progress } : next));
        if (next.library.loaded) void loadRef.current("refresh");
      }).catch(() => undefined);
    }, 2000);
    return () => window.clearInterval(timer);
  }, [libraryReady]);

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
          {progress?.continuous ? (
            <div><button className="btn small" onClick={() => void commands.discoveryStop().catch((e) => toast("error", errorMessage(e)))}><Square size={12} /> Stop</button></div>
          ) : null}
        </div>
      ) : null}
      {progress && progress.seedsTotal > 0 ? (
        <div className="status-line" style={{ margin: "0 0 14px", display: "grid", gap: 3 }}>
          <div title="Worked out from your Music Library and MusicBrainz data on this computer; no online lookups needed.">
            Albums missing from artists you own: <b>{(page?.missingTotal ?? 0).toLocaleString()}</b>
          </div>
          <div title="Last.fm is asked which artists sound like each of yours; the answers are the new artists you don't own yet.">
            Finding new artists: <b>{progress.seedsDone.toLocaleString()} / {progress.seedsTotal.toLocaleString()}</b> of your artists asked
          </div>
          {progress.artistsTotal > 0 ? (
            <div title="Albums of the new artists are looked up on MusicBrainz (Discogs as a fallback).">
              Albums found for new artists: <b>{progress.artistsDone.toLocaleString()} / {progress.artistsTotal.toLocaleString()}</b> artists looked up
            </div>
          ) : null}
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
          <button className="btn small" style={{ marginLeft: "auto" }} disabled={running} onClick={() => void commands.discoveryRefresh(true).then((started) => toast("info", started ? "Searching until everything is checked. Press Stop to pause." : "Already running")).catch((e) => toast("error", errorMessage(e)))}>
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
        {items.map((a) => <DiscTile key={a.id} album={a} busy={resolving === a.id} onOpen={(x, auto) => void open(x, auto)} onArtist={(n) => void openArtist(n)} />)}
      </div>

      {page && items.length < page.total ? (
        <div style={{ textAlign: "center", marginTop: 26 }}>
          <button className="btn" disabled={loading} onClick={() => void load("more")}>
            {loading ? <Loader2 size={15} className="spin" /> : null} Show more ({(page.total - items.length).toLocaleString()} left)
          </button>
        </div>
      ) : null}

      {drawerNode}
    </>
  );
}
