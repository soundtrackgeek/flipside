import { useEffect, useState } from "react";
import { commands } from "../bindings";
import type { AlbumCard, SearchResults } from "../bindings";
import { useApp } from "../store";
import { AlbumTile, ArtistTile, Empty, Section, SkeletonGrid } from "../components/ui";
import { errorMessage, formatDuration } from "../lib/format";

export function Search({ q }: { q: string }) {
  const { go } = useApp();
  const [results, setResults] = useState<SearchResults | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setResults(null);
    setError(null);
    commands
      .catalogSearch(q)
      .then((r) => alive && setResults(r))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => { alive = false; };
  }, [q]);

  return (
    <>
      <h1 className="page-title">Results for “{q}”</h1>
      <p className="page-sub">Albums, artists and tracks from the catalog.</p>
      {error ? <Empty>Search failed: {error}</Empty> : null}
      {!results && !error ? <SkeletonGrid count={12} /> : null}
      {results ? (
        <>
          {results.artists.length > 0 ? (
            <Section title="Artists">
              <div className="rail">{results.artists.map((a) => <ArtistTile key={a.id} artist={a} />)}</div>
            </Section>
          ) : null}
          <Section title="Albums" hint={`${results.albums.length} found`}>
            {results.albums.length ? (
              <div className="grid">{results.albums.map((a) => <AlbumTile key={a.id} album={a} />)}</div>
            ) : (
              <Empty>No albums matched.</Empty>
            )}
          </Section>
          {results.tracks.length > 0 ? (
            <Section title="Tracks">
              <div className="tracklist">
                {results.tracks.map((t) => (
                  <div className="track" key={t.id} style={{ cursor: t.albumId ? "pointer" : "default" }}
                    onClick={() => t.albumId && go({ name: "album", id: t.albumId })}>
                    <span className="n">{t.cover ? <img src={t.cover} width={32} height={32} style={{ borderRadius: 4, display: "block" }} alt="" /> : null}</span>
                    <span className="t">
                      {t.title}
                      <div className="a">{t.artist}{t.albumTitle ? ` · ${t.albumTitle}` : ""}</div>
                    </span>
                    <span className="d">{formatDuration(t.durationSeconds)}</span>
                  </div>
                ))}
              </div>
            </Section>
          ) : null}
        </>
      ) : null}
    </>
  );
}

export function Genre({ id, title }: { id: string; title: string }) {
  const [albums, setAlbums] = useState<AlbumCard[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    setAlbums(null);
    commands.catalogGenre(id).then((a) => alive && setAlbums(a)).catch((e) => alive && setError(errorMessage(e)));
    return () => { alive = false; };
  }, [id]);
  return (
    <>
      <h1 className="page-title">{title}</h1>
      <p className="page-sub">Top albums in this genre.</p>
      {error ? <Empty>{error}</Empty> : !albums ? <SkeletonGrid count={12} /> : albums.length ? (
        <div className="grid">{albums.map((a) => <AlbumTile key={a.id} album={a} />)}</div>
      ) : <Empty>Nothing charting here right now.</Empty>}
    </>
  );
}
