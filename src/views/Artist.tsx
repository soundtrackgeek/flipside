import { useEffect, useState } from "react";
import { CircleStop, ExternalLink, Wand2 } from "lucide-react";
import { commands } from "../bindings";
import type { ArtistDetail, ArtistEnrichment, DiscoveryAlbum } from "../bindings";
import { DiscTile, useAlbumOpener, useArtistOpener } from "../components/DiscTile";
import { AlbumTile, Empty, Section, SkeletonGrid } from "../components/ui";
import { limited } from "../lib/limit";
import { errorMessage, formatCount } from "../lib/format";
import { usePageState } from "../lib/pageState";
import { useApp, type Batch } from "../store";

const SIMILAR_ARTISTS = 8;
const ALBUMS_PER_SIMILAR = 12;

/** One "If you like ..." row: a similar artist and their own studio albums. */
function SimilarRow({
  name, busyId, onOpen, onArtist,
}: { name: string; busyId: string | null; onOpen: (a: DiscoveryAlbum, auto?: boolean) => void; onArtist: (name: string) => void }) {
  const [albums, setAlbums] = usePageState<DiscoveryAlbum[] | null>(`similar:${name}`, null);

  useEffect(() => {
    if (albums) return; // remembered from an earlier visit
    let alive = true;
    void limited(() => commands.catalogArtistAlbums(name))
      .then((a) => alive && setAlbums(a))
      .catch(() => alive && setAlbums([]));
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
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

/** "Download all": auto-get every studio album in the list, after a second click to confirm. */
function DownloadAllButton({ batchKey, albums }: { batchKey: string; albums: DiscoveryAlbum[] }) {
  const { batch, autoGetAll } = useApp();
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    if (!confirming) return;
    const t = window.setTimeout(() => setConfirming(false), 5000);
    return () => window.clearTimeout(t);
  }, [confirming]);

  if (batch?.running) {
    return batch.key === batchKey ? null : (
      <button className="btn small" disabled title="Another Download all is still running">
        <Wand2 size={14} /> Download all
      </button>
    );
  }
  if (confirming) {
    return (
      <>
        <button className="btn small primary" onClick={() => { setConfirming(false); autoGetAll(batchKey, albums); }}>
          <Wand2 size={14} /> Get all {albums.length} albums
        </button>
        <button className="btn small ghost" onClick={() => setConfirming(false)}>Cancel</button>
      </>
    );
  }
  return (
    <button className="btn small" title="Auto-get every studio album: Deezer, then Usenet, then Soulseek" onClick={() => setConfirming(true)}>
      <Wand2 size={14} /> Download all
    </button>
  );
}

/** Progress of this page's "Download all" batch, and a summary once it is finished. */
function BatchLine({ batch }: { batch: Batch }) {
  const { stopBatch, go } = useApp();
  const total = batch.items.length;
  const finished = batch.items.filter((x) => x.state === "done" || x.state === "failed").length;
  const current = batch.items.find((x) => x.state === "running");
  const count = (via: string) => batch.items.filter((x) => x.via === via).length;
  const missed = batch.items.filter((x) => x.state === "failed");

  if (batch.running) {
    return (
      <div className="batch-line">
        <div className="progress"><i style={{ width: `${Math.round((finished / total) * 100)}%` }} /></div>
        <span>
          {batch.stopping ? "Stopping after this album... " : ""}
          {finished + 1} of {total}{current ? `: ${current.title} (${current.detail})` : ""}
        </span>
        {!batch.stopping ? <button className="btn small ghost" onClick={stopBatch}><CircleStop size={14} /> Stop</button> : null}
      </div>
    );
  }

  const parts = [
    count("deemix") ? `${count("deemix")} from Deezer` : "",
    count("usenet") ? `${count("usenet")} queued on Usenet` : "",
    count("soulseek") ? `${count("soulseek")} queued on Soulseek` : "",
    count("owned") ? `${count("owned")} already downloaded` : "",
    missed.length ? `${missed.length} not found (${missed.map((x) => x.title).join(", ")})` : "",
  ].filter(Boolean);
  return (
    <div className="batch-line">
      <span>{finished < total ? "Stopped" : "Finished"}: {parts.join(", ") || "nothing done"}.</span>
      <button className="btn small ghost" onClick={() => go({ name: "downloads" })}>Open downloads</button>
    </div>
  );
}

export default function Artist({ id }: { id: string }) {
  const [detail, setDetail] = usePageState<ArtistDetail | null>("detail", null);
  const [extra, setExtra] = usePageState<ArtistEnrichment | null>("extra", null);
  const [extraDone, setExtraDone] = usePageState("extraDone", false);
  const [albums, setAlbums] = usePageState<DiscoveryAlbum[] | null>("albums", null);
  const [error, setError] = useState<string | null>(null);
  const { open, resolving, drawerNode } = useAlbumOpener();
  const openArtist = useArtistOpener();
  const { batch } = useApp();
  const batchKey = `artist:${id}`;

  useEffect(() => {
    if (detail && extraDone && albums) return; // remembered from an earlier visit
    let alive = true;
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
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

      <Section
        title={`Albums by ${artist.name}`}
        hint={albums && albums.length ? `${albums.length} studio albums` : undefined}
        action={albums && albums.length ? <DownloadAllButton batchKey={batchKey} albums={albums} /> : undefined}
      >
        {batch?.key === batchKey ? <BatchLine batch={batch} /> : null}
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
