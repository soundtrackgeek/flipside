import { useState, type ReactNode } from "react";
import { Disc3, Download, User } from "lucide-react";
import type { AlbumCard, ArtistCard } from "../bindings";
import { useApp } from "../store";
import { formatCount, kindLabel } from "../lib/format";

export function Cover({ src, alt, round }: { src?: string | null; alt: string; round?: boolean }) {
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  return (
    <div className="art">
      <div className="ph">{round ? <User size={34} /> : <Disc3 size={34} />}</div>
      {src && !failed ? (
        <img
          src={src}
          alt={alt}
          loading="lazy"
          decoding="async"
          className={loaded ? "loaded" : ""}
          onLoad={() => setLoaded(true)}
          onError={() => setFailed(true)}
          draggable={false}
          style={{ position: "absolute", inset: 0 }}
        />
      ) : null}
    </div>
  );
}

export function AlbumTile({ album, showYear = false }: { album: AlbumCard; showYear?: boolean }) {
  const { go } = useApp();
  return (
    <div className="card" role="link" tabIndex={0}
      onClick={() => go({ name: "album", id: album.id })}
      onKeyDown={(e) => e.key === "Enter" && go({ name: "album", id: album.id })}>
      <div style={{ position: "relative" }}>
        <Cover src={album.cover} alt={album.title} />
        <button
          className="get"
          title="Get this album"
          aria-label={`Get ${album.title}`}
          onClick={(e) => { e.stopPropagation(); go({ name: "album", id: album.id, get: true }); }}
        >
          <Download size={18} />
        </button>
      </div>
      <div className="title" title={album.title}>
        {album.title}
        {album.explicit ? <span className="badge-e">E</span> : null}
      </div>
      <div className="sub">
        {showYear ? (
          <>{[album.year, kindLabel(album.kind)].filter(Boolean).join(" · ")}</>
        ) : album.artistId ? (
          <button onClick={(e) => { e.stopPropagation(); go({ name: "artist", id: album.artistId! }); }}>{album.artist}</button>
        ) : (
          album.artist
        )}
      </div>
    </div>
  );
}

export function ArtistTile({ artist }: { artist: ArtistCard }) {
  const { go } = useApp();
  return (
    <div className="card artist" role="link" tabIndex={0}
      onClick={() => go({ name: "artist", id: artist.id })}
      onKeyDown={(e) => e.key === "Enter" && go({ name: "artist", id: artist.id })}>
      <Cover src={artist.picture} alt={artist.name} round />
      <div className="title">{artist.name}</div>
      {artist.fans ? <div className="sub">{formatCount(artist.fans)} fans</div> : null}
    </div>
  );
}

export function Section({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <section className="section">
      <div className="section-head">
        <h2>{title}</h2>
        {hint ? <span>{hint}</span> : null}
      </div>
      {children}
    </section>
  );
}

export function SkeletonGrid({ count = 12, rail }: { count?: number; rail?: boolean }) {
  return (
    <div className={rail ? "rail" : "grid"}>
      {Array.from({ length: count }, (_, i) => (
        <div key={i}>
          <div className="skeleton sk-card" />
          <div className="skeleton sk-line" />
          <div className="skeleton sk-line" style={{ width: "50%" }} />
        </div>
      ))}
    </div>
  );
}

export function Empty({ icon, children }: { icon?: ReactNode; children: ReactNode }) {
  return (
    <div className="empty">
      {icon ? <div className="ico">{icon}</div> : null}
      {children}
    </div>
  );
}
