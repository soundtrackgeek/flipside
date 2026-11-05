import { useEffect, useMemo, useState } from "react";
import { commands } from "../bindings";
import type { ArtistDetail, ArtistEnrichment } from "../bindings";
import { useApp } from "../store";
import { AlbumTile, ArtistTile, Empty, Section } from "../components/ui";
import { errorMessage, formatCount, formatDuration, kindLabel } from "../lib/format";

export default function Artist({ id }: { id: string }) {
  const { go } = useApp();
  const [detail, setDetail] = useState<ArtistDetail | null>(null);
  const [extra, setExtra] = useState<ArtistEnrichment | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<string>("all");

  useEffect(() => {
    let alive = true;
    setDetail(null);
    setExtra(null);
    setError(null);
    setFilter("all");
    commands
      .catalogArtist(id)
      .then((d) => {
        if (!alive) return;
        setDetail(d);
        void commands.catalogArtistEnrichment(d.artist.name).then((x) => alive && setExtra(x)).catch(() => undefined);
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => { alive = false; };
  }, [id]);

  const kinds = useMemo(() => {
    const counts = new Map<string, number>();
    detail?.albums.forEach((a) => counts.set(a.kind || "album", (counts.get(a.kind || "album") ?? 0) + 1));
    return [...counts.entries()];
  }, [detail]);

  if (error) return <Empty>Couldn't load this artist: {error}</Empty>;
  if (!detail) return <div className="skeleton" style={{ height: 320, borderRadius: 18 }} />;

  const { artist } = detail;
  const albums = detail.albums.filter((a) => filter === "all" || (a.kind || "album") === filter);

  return (
    <>
      <div className={`detail-hero${extra?.background ? " has-fanart" : ""}`}>
        <div className="bg" style={{ backgroundImage: `url(${extra?.background ?? artist.picture})` }} />
        <div className="detail-head">
          <img className="detail-cover round" src={artist.picture} alt={artist.name} draggable={false} />
          <div className="detail-meta">
            <div className="kind">Artist</div>
            <h1>{artist.name}</h1>
            <div className="facts">
              {artist.fans ? <span className="chip">{formatCount(artist.fans)} fans</span> : null}
              {extra?.listeners ? <span className="chip">{formatCount(extra.listeners)} listeners</span> : null}
              {extra?.tags.map((t) => <span className="chip accent" key={t}>{t}</span>)}
            </div>
          </div>
        </div>
      </div>

      {extra?.bio ? <p className="prose" style={{ marginTop: 22 }}>{extra.bio}</p> : null}

      {detail.topTracks.length > 0 ? (
        <Section title="Popular tracks">
          <div className="tracklist" style={{ marginTop: 0 }}>
            {detail.topTracks.map((t, i) => (
              <div className="track" key={t.id} style={{ cursor: t.albumId ? "pointer" : "default" }}
                onClick={() => t.albumId && go({ name: "album", id: t.albumId })}>
                <span className="n">{i + 1}</span>
                <span className="t">{t.title}<div className="a">{t.albumTitle}</div></span>
                <span className="d">{formatDuration(t.durationSeconds)}</span>
              </div>
            ))}
          </div>
        </Section>
      ) : null}

      <Section title="Discography" hint={`${detail.albums.length} releases`}>
        {kinds.length > 1 ? (
          <div className="facts" style={{ marginTop: 0 }}>
            <button className={`chip${filter === "all" ? " accent" : ""}`} onClick={() => setFilter("all")}>All</button>
            {kinds.map(([kind, n]) => (
              <button key={kind} className={`chip${filter === kind ? " accent" : ""}`} onClick={() => setFilter(kind)}>
                {kindLabel(kind)} · {n}
              </button>
            ))}
          </div>
        ) : null}
        <div className="grid">{albums.map((a) => <AlbumTile key={a.id} album={a} showYear />)}</div>
      </Section>

      {detail.related.length > 0 ? (
        <Section title="Fans also like">
          <div className="rail">{detail.related.map((a) => <ArtistTile key={a.id} artist={a} />)}</div>
        </Section>
      ) : null}
    </>
  );
}
