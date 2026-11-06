import { useEffect, useRef, useState } from "react";
import { ArrowLeft, Compass, Download, Gem, Loader2, Search as SearchIcon, Settings as SettingsIcon, Sparkles, X } from "lucide-react";
import { AppProvider, useApp, type Route } from "./store";
import Home from "./views/Home";
import { Genre, Search } from "./views/Browse";
import Album from "./views/Album";
import Artist from "./views/Artist";
import Downloads from "./views/Downloads";
import ForYou from "./views/ForYou";
import Settings from "./views/Settings";
import GetDrawer from "./components/GetDrawer";

function NavButton({ route, label, icon, active, badge }: { route: Route; label: string; icon: React.ReactNode; active: boolean; badge?: number }) {
  const { go } = useApp();
  return (
    <button className={`nav-btn${active ? " active" : ""}`} onClick={() => go(route)} aria-label={label}>
      {icon}
      <span>{label}</span>
      {badge ? <span className="nav-badge">{badge}</span> : null}
    </button>
  );
}

function UpdateBanner() {
  const { update, installUpdate, installProgress } = useApp();
  const [dismissed, setDismissed] = useState<string | null>(null);
  if (!update.info || dismissed === update.info.version) return null;
  const busy = !!installProgress;
  return (
    <div className="update-banner" role="status">
      <Sparkles size={18} />
      <span>
        <strong>Flipside {update.info.version}</strong> is ready — you're on {update.info.currentVersion}.
      </span>
      {busy ? (
        <>
          <div className="meter"><i style={{ width: `${installProgress?.percent ?? 5}%` }} /></div>
          <span className="status-line"><Loader2 size={14} className="spin" /> {installProgress?.phase === "installing" ? "Installing…" : "Downloading…"}</span>
        </>
      ) : (
        <>
          <span style={{ flex: 1 }} />
          <button className="btn small primary" onClick={() => void installUpdate()}>Install & restart</button>
          <button className="icon-btn" style={{ width: 28, height: 28 }} aria-label="Dismiss" onClick={() => setDismissed(update.info!.version)}><X size={14} /></button>
        </>
      )}
    </div>
  );
}

function Shell() {
  const { route, go, back, canGoBack, toasts, dismissToast, activeDownloads, connection, extensionAlbum, clearExtensionAlbum } = useApp();
  const [query, setQuery] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const scroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement)?.tagName === "INPUT" || (e.target as HTMLElement)?.tagName === "SELECT";
      if ((e.key === "/" && !typing) || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k")) {
        e.preventDefault();
        input.current?.focus();
        input.current?.select();
      }
      if (e.altKey && e.key === "ArrowLeft") back();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [back]);

  useEffect(() => { scroller.current?.scrollTo({ top: 0, behavior: "instant" as ScrollBehavior }); }, [route]);

  const conn = connection?.state;
  const connClass = conn === "online" ? "online" : conn === "error" ? "error" : conn === "connecting" || conn === "authenticating" || conn === "reconnecting" ? "busy" : "";

  return (
    <div className="shell">
      <nav className="sidebar">
        <div className="brand" title="Flipside">F</div>
        <NavButton route={{ name: "home" }} label="Discover" icon={<Compass size={20} />} active={route.name === "home" || route.name === "genre"} />
        <NavButton route={{ name: "foryou" }} label="For you" icon={<Gem size={20} />} active={route.name === "foryou"} />
        <NavButton route={{ name: "downloads" }} label="Downloads" icon={<Download size={20} />} active={route.name === "downloads"} badge={activeDownloads} />
        <div className="spacer" />
        <div title={`Soulseek: ${conn ?? "offline"}`} style={{ padding: 10 }}><span className={`conn-dot ${connClass}`} /></div>
        <NavButton route={{ name: "settings" }} label="Settings" icon={<SettingsIcon size={20} />} active={route.name === "settings"} />
      </nav>

      <div className="main">
        <header className="topbar">
          <button className="icon-btn" onClick={back} disabled={!canGoBack} aria-label="Back" title="Back (Alt+←)"><ArrowLeft size={18} /></button>
          <form
            className="searchbox"
            onSubmit={(e) => { e.preventDefault(); if (query.trim()) go({ name: "search", q: query.trim() }); }}
          >
            <SearchIcon size={16} />
            <input
              ref={input}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search albums, artists, tracks…"
              spellCheck={false}
              aria-label="Search"
            />
            <kbd>/</kbd>
          </form>
        </header>
        <UpdateBanner />
        <div className="scroller" ref={scroller}>
          {route.name === "home" && <Home />}
          {route.name === "search" && <Search q={route.q} />}
          {route.name === "genre" && <Genre id={route.id} title={route.title} />}
          {route.name === "album" && <Album id={route.id} openGet={route.get} />}
          {route.name === "artist" && <Artist id={route.id} />}
          {route.name === "foryou" && <ForYou />}
          {route.name === "downloads" && <Downloads />}
          {route.name === "settings" && <Settings />}
        </div>
      </div>

      {extensionAlbum ? (
        <GetDrawer key={`${extensionAlbum.artist}|${extensionAlbum.title}`} album={extensionAlbum} autoStart onClose={clearExtensionAlbum} />
      ) : null}

      <div className="toasts" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`} onClick={() => dismissToast(t.id)}>{t.text}</div>
        ))}
      </div>
    </div>
  );
}

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}
