import { useCallback, useState } from "react";
import { Disc3, Download, Loader2 } from "lucide-react";
import { commands } from "../bindings";
import type { AlbumCard, DiscoveryAlbum } from "../bindings";
import { useApp } from "../store";
import GetDrawer from "./GetDrawer";
import { limited } from "../lib/limit";
import { errorMessage } from "../lib/format";

/**
 * Cover Art Archive first (loaded directly). If that fails, retry once (it sometimes times out),
 * then ask the backend, which tries Deezer and Discogs and remembers the answer.
 */
export function DiscCover({ album }: { album: DiscoveryAlbum }) {
  const [src, setSrc] = useState<string | null>(album.cover);
  const [loaded, setLoaded] = useState(false);
  const [attempt, setAttempt] = useState(0);

  function onError() {
    if (attempt === 0) {
      window.setTimeout(() => setAttempt(1), 1500);
      return;
    }
    if (attempt === 1) {
      setAttempt(2);
      void limited(() => commands.catalogCover(album.id, album.artist, album.title))
        .then((url) => setSrc(url && url !== src ? url : null))
        .catch(() => setSrc(null));
      return;
    }
    setSrc(null);
  }

  return (
    <div className="art">
      <div className="ph"><Disc3 size={34} /></div>
      {src ? (
        <img
          key={`${src}#${attempt === 1 ? "retry" : ""}`}
          src={src} alt={album.title} loading="lazy" decoding="async" draggable={false}
          className={loaded ? "loaded" : ""} style={{ position: "absolute", inset: 0 }}
          onLoad={() => setLoaded(true)} onError={onError}
        />
      ) : null}
    </div>
  );
}

/** Opens a MusicBrainz-sourced album: on Deezer when it is there, else the Usenet/Soulseek drawer. */
export function useAlbumOpener() {
  const { go, toast } = useApp();
  const [resolving, setResolving] = useState<string | null>(null);
  const [drawer, setDrawer] = useState<AlbumCard | null>(null);

  const open = useCallback(async (album: DiscoveryAlbum) => {
    setResolving(album.id);
    try {
      const found = await commands.catalogResolveAlbum(album.artist, album.title);
      if (found) {
        go({ name: "album", id: found.id });
      } else {
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
  }, [go, toast]);

  const drawerNode = drawer ? <GetDrawer album={drawer} onClose={() => setDrawer(null)} /> : null;
  return { open, resolving, drawerNode };
}

/** Opens the artist's page (their Deezer page when found, a search otherwise). */
export function useArtistOpener() {
  const { go } = useApp();
  return useCallback(async (name: string) => {
    try {
      const artist = await commands.catalogResolveArtist(name);
      go(artist ? { name: "artist", id: artist.id } : { name: "search", q: name });
    } catch {
      go({ name: "search", q: name });
    }
  }, [go]);
}

export function DiscTile({
  album, busy, onOpen, onArtist, showArtist = true,
}: {
  album: DiscoveryAlbum; busy: boolean; onOpen: (a: DiscoveryAlbum) => void; onArtist?: (name: string) => void; showArtist?: boolean;
}) {
  return (
    <div className="card" role="link" tabIndex={0} onClick={() => onOpen(album)} onKeyDown={(e) => e.key === "Enter" && onOpen(album)}>
      <div style={{ position: "relative" }}>
        <DiscCover album={album} />
        <button className="get" title="Get this album" aria-label={`Get ${album.title}`} onClick={(e) => { e.stopPropagation(); onOpen(album); }}>
          {busy ? <Loader2 size={18} className="spin" /> : <Download size={18} />}
        </button>
      </div>
      <div className="title" title={album.title}>{album.title}</div>
      <div className="sub">
        {showArtist ? (
          <button onClick={(e) => { e.stopPropagation(); onArtist?.(album.artist); }}>{album.artist}</button>
        ) : null}
        {showArtist && album.year ? " · " : ""}{album.year ?? ""}
      </div>
      <div className="sub" title={album.similar ? `Because you own ${album.because.join(", ")}` : undefined} style={{ color: "var(--faint)" }}>
        {album.similar
          ? (album.because.length ? `Like ${album.because[0]}` : "Similar to your artists")
          : album.genres[0] ?? (showArtist ? "Missing from your library" : " ")}
      </div>
    </div>
  );
}
