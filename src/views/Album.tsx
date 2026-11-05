import { useEffect, useState } from "react";
import { ListChecks, Wand2 } from "lucide-react";
import { commands } from "../bindings";
import type { AlbumDetail, AlbumEnrichment } from "../bindings";
import { useApp } from "../store";
import GetDrawer, { type Tab } from "../components/GetDrawer";
import { Empty } from "../components/ui";
import { errorMessage, formatCount, formatDuration, formatTotalDuration, kindLabel } from "../lib/format";

export default function Album({ id, openGet }: { id: string; openGet?: boolean }) {
  const { go } = useApp();
  const [detail, setDetail] = useState<AlbumDetail | null>(null);
  const [extra, setExtra] = useState<AlbumEnrichment | null>(null);
  const [originalYear, setOriginalYear] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [drawer, setDrawer] = useState<{ tab: Tab; autoStart: boolean } | null>(openGet ? { tab: "auto", autoStart: false } : null);

  useEffect(() => {
    let alive = true;
    setDetail(null);
    setExtra(null);
    setOriginalYear(null);
    setError(null);
    setDrawer(openGet ? { tab: "auto", autoStart: false } : null);
    commands
      .catalogAlbum(id)
      .then((d) => {
        if (!alive) return;
        setDetail(d);
        void commands.catalogAlbumEnrichment(d.album.artist, d.album.title).then((x) => alive && setExtra(x)).catch(() => undefined);
        void commands.catalogOriginalYear(d.album.artist, d.album.title).then((y) => alive && setOriginalYear(y)).catch(() => undefined);
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => { alive = false; };
  }, [id, openGet]);

  if (error) return <Empty>Couldn't load this album: {error}</Empty>;
  if (!detail) return <div className="skeleton" style={{ height: 340, borderRadius: 18 }} />;

  const { album } = detail;
  const discs = [...new Set(detail.tracks.map((t) => t.disc))].sort((a, b) => a - b);
  const tags = [...new Set([...(extra?.styles ?? []), ...detail.genres, ...(extra?.tags ?? [])])].slice(0, 8);

  return (
    <>
      <div className="detail-hero">
        <div className="bg" style={{ backgroundImage: `url(${album.coverLarge})` }} />
        <div className="detail-head">
          <img className="detail-cover" src={album.coverLarge} alt={album.title} draggable={false} />
          <div className="detail-meta">
            <div className="kind">{kindLabel(album.kind)}</div>
            <h1>{album.title}</h1>
            <div className="by">
              {album.artistId ? (
                <button onClick={() => go({ name: "artist", id: album.artistId! })}>{album.artist}</button>
              ) : album.artist}
              {(originalYear ?? album.year) ? ` · ${originalYear ?? album.year}` : ""}
              {detail.tracks.length ? ` · ${detail.tracks.length} tracks` : ""}
              {detail.durationSeconds ? ` · ${formatTotalDuration(detail.durationSeconds)}` : ""}
            </div>
            <div className="facts">
              {(extra?.label ?? detail.label) ? <span className="chip">{extra?.label ?? detail.label}</span> : null}
              {extra?.country ? <span className="chip">{extra.country}</span> : null}
              {extra?.listeners ? <span className="chip">{formatCount(extra.listeners)} listeners</span> : null}
              {tags.map((t) => <span className="chip accent" key={t}>{t}</span>)}
            </div>
            <div className="hero-actions">
              <button className="btn primary" onClick={() => setDrawer({ tab: "auto", autoStart: true })}><Wand2 size={16} /> Auto-get</button>
              <button className="btn" onClick={() => setDrawer({ tab: "deemix", autoStart: false })}><ListChecks size={16} /> Choose source</button>
              {extra?.discogsUrl ? (
                <button className="btn ghost" onClick={() => void import("@tauri-apps/plugin-opener").then((m) => m.openUrl(extra.discogsUrl!))}>Discogs</button>
              ) : null}
            </div>
          </div>
        </div>
      </div>

      {extra?.summary ? <p className="prose" style={{ marginTop: 24 }}>{extra.summary}</p> : null}

      <div className="tracklist">
        {discs.map((disc) => (
          <div key={disc}>
            {discs.length > 1 ? <div className="disc-head">Disc {disc}</div> : null}
            {detail.tracks.filter((t) => t.disc === disc).map((t) => (
              <div className="track" key={t.id}>
                <span className="n">{t.position}</span>
                <span className="t">
                  {t.title}{t.explicit ? <span className="badge-e">E</span> : null}
                  {t.artist && t.artist !== album.artist ? <div className="a">{t.artist}</div> : null}
                </span>
                <span className="d">{formatDuration(t.durationSeconds)}</span>
              </div>
            ))}
          </div>
        ))}
      </div>

      {drawer ? <GetDrawer album={album} initialTab={drawer.tab} autoStart={drawer.autoStart} onClose={() => setDrawer(null)} /> : null}
    </>
  );
}
