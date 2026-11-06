import { useEffect, useState } from "react";
import { ExternalLink } from "lucide-react";
import { commands } from "../bindings";
import type { ArtistDetail, ArtistEnrichment, DiscoveryAlbum } from "../bindings";
import { DiscTile, useAlbumOpener, useArtistOpener } from "../components/DiscTile";
import { AlbumTile, Empty, Section, SkeletonGrid } from "../components/ui";
import { limited } from "../lib/limit";
import { errorMessage, formatCount } from "../lib/format";

const SIMILAR_ARTISTS = 8;
const ALBUMS_PER_SIMILAR = 12;

/** One "If you like ..." row: a similar artist and their own studio albums. */
function SimilarRow({
  name, busyId, onOpen, onArtist,
}: { name: string; busyId: string | null; onOpen: (a: DiscoveryAlbum, auto?: boolean) => void; onArtist: (name: string) => void }) {
  const [albums, setAlbums] = useState<DiscoveryAlbum[] | null>(null);

  useEffect(() => {
    let alive = true;
    void limited(() => commands.catalogArtistAlbums(name))
      .then((a) => alive && setAlbums(a))
      .catch(() => alive && setAlbums([]));
    return () => { alive = false; };
  }, [name]);

  // Nothing worth showing (unknown artist, or you already own all of their albums).
  if (albums && albums.length === 0) return null;

  return (
    <div style={{ marginBottom: 26 }}>
      <div className="section-head" style={{ marginBottom: 10 }}>
        <h2 style={{ fontSize: 19 }}>
          <button onClick={() => onArtist(name)} style={{ font: "inherit" }} className="artist-link">{name}</button>
        </h2>
        {albums ? <span>{albums.length} studio {albums.length === 1 ? "album" : "albums"}</span> : null}
      </div>
      {albums ? (
        <div className="rail">
          {albums.slice(0, ALBUMS_PER_SIMILAR).map((a) => (
            <DiscTile key={a.id} album={a} showArtist={false} busy={busyId === a.id} onOpen={onOpen} />
          ))}
        </div>
      ) : (
        <SkeletonGrid rail count={6} />
      )}
    </div>
  );
}

export default function Artist({ id }: { id: string }) {
  const [detail, setDetail] = useState<ArtistDetail | null>(null);
  const [extra, setExtra] = useState<ArtistEnrichment | null>(null);
  const [extraDone, setExtraDone] = useState(false);
  const [albums, setAlbums] = useState<DiscoveryAlbum[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { open, resolving, drawerNode } = useAlbumOpener();
  const openArtist = useArtistOpener();

  useEffect(() => {
    let alive = true;
    setDetail(null); setExtra(null); setExtraDone(false); setAlbums(null); setError(null);
    commands
      .catalogArtist(id)
      .then((d) => {
        if (!alive) return;
        setDetail(d);
        const name = d.artist.name;
        void commands.catalogArtistEnrichment(name).then((x) => alive && setExtra(x)).catch(() => undefined).finally(() => alive && setExtraDone(true));
        void commands.catalogArtistAlbums(name).then((a) => alive && setAlbums(a)).catch(() => alive && setAlbums([]));
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => { alive = false; };
  }, [id]);

  if (error) return <Empty>Couldn't load this artist: {error}</Empty>;
  if (!detail) return <div className="skeleton" style={{ height: 320, borderRadius: 18 }} />;

  const { artist } = detail;
  // Deezer fallback: only full albums, never singles or EPs.
  const deezerAlbums = detail.albums.filter((a) => a.kind.toLowerCase() === "album");
  const similarNames = !extraDone
    ? []
    : (extra?.similar.length ? extra.similar : detail.related.map((r) => r.name))
        .filter((n) => n.toLowerCase() !== artist.name.toLowerCase())
        .slice(0, SIMILAR_ARTISTS);

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

      {extra?.bio ? (
        <div style={{ marginTop: 22 }}>
          <p className="prose" style={{ margin: 0 }}>{extra.bio}</p>
          {extra.bioUrl ? (
            <button
              className="chip pick" style={{ marginTop: 10 }}
              onClick={() => void import("@tauri-apps/plugin-opener").then((m) => m.openUrl(extra.bioUrl!))}
            >
              Read more on {extra.bioSource} <ExternalLink size={11} />
            </button>
          ) : null}
        </div>
      ) : null}

      <Section title={`Albums by ${artist.name}`} hint={albums && albums.length ? `${albums.length} studio albums` : undefined}>
        {albums === null ? (
          <SkeletonGrid count={6} />
        ) : albums.length > 0 ? (
          <div className="grid">
            {albums.map((a) => <DiscTile key={a.id} album={a} showArtist={false} busy={resolving === a.id} onOpen={(x, auto) => void open(x, auto)} />)}
          </div>
        ) : deezerAlbums.length > 0 ? (
          <>
            <p className="page-sub" style={{ marginTop: -6 }}>No studio albums found in MusicBrainz, so these are the full albums Deezer lists (singles and EPs are left out).</p>
            <div className="grid">{deezerAlbums.map((a) => <AlbumTile key={a.id} album={a} showYear />)}</div>
          </>
        ) : (
          <Empty>Nothing to show here. You may already own every album we know of by {artist.name}.</Empty>
        )}
      </Section>

      {similarNames.length > 0 ? (
        <Section title={`If you like ${artist.name}…`} hint="Similar artists and their albums">
          {similarNames.map((name) => (
            <SimilarRow key={name} name={name} busyId={resolving} onOpen={(x, auto) => void open(x, auto)} onArtist={(n) => void openArtist(n)} />
          ))}
        </Section>
      ) : null}

      {drawerNode}
    </>
  );
}
