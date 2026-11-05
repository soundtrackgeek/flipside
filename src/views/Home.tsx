import { useEffect, useState } from "react";
import { Download, Play } from "lucide-react";
import { commands } from "../bindings";
import type { HomeFeed } from "../bindings";
import { useApp } from "../store";
import { AlbumTile, ArtistTile, Section, SkeletonGrid, Empty } from "../components/ui";
import { errorMessage, kindLabel } from "../lib/format";

let cached: HomeFeed | null = null;

export default function Home() {
  const { go } = useApp();
  const [feed, setFeed] = useState<HomeFeed | null>(cached);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    commands
      .catalogHome()
      .then((f) => { cached = f; if (alive) setFeed(f); })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => { alive = false; };
  }, []);

  if (error && !feed) return <Empty>Couldn't load the store: {error}</Empty>;
  if (!feed) {
    return (
      <>
        <div className="skeleton" style={{ height: 300, borderRadius: 22, marginTop: 4 }} />
        <Section title="Editor's picks"><SkeletonGrid rail count={8} /></Section>
      </>
    );
  }

  const pool = feed.picks.length ? feed.picks : feed.newReleases.length ? feed.newReleases : feed.chartAlbums;
  const featured = pool[0];
  const picks = feed.picks.slice(feed.picks[0] === featured ? 1 : 0);

  return (
    <>
      {featured ? (
        <div className="hero">
          <div className="hero-bg" style={{ backgroundImage: `url(${featured.coverLarge})` }} />
          <div className="hero-body">
            <img className="hero-cover" src={featured.coverLarge} alt="" draggable={false} />
            <div className="hero-text">
              <div className="eyebrow">Featured {kindLabel(featured.kind)}</div>
              <h1>{featured.title}</h1>
              <p>{featured.artist}{featured.year ? ` · ${featured.year}` : ""}</p>
              <div className="hero-actions">
                <button className="btn primary" onClick={() => go({ name: "album", id: featured.id, get: true })}>
                  <Download size={16} /> Get album
                </button>
                <button className="btn" onClick={() => go({ name: "album", id: featured.id })}>
                  <Play size={15} /> Open
                </button>
              </div>
            </div>
          </div>
        </div>
      ) : null}

      {picks.length > 0 ? (
        <Section title="Editor's picks" hint="Hand-picked by Deezer">
          <div className="rail">{picks.map((a) => <AlbumTile key={a.id} album={a} />)}</div>
        </Section>
      ) : null}

      {feed.newReleases.length > 0 ? (
        <Section title="New releases" hint="Fresh from the shelves">
          <div className="rail">{feed.newReleases.map((a) => <AlbumTile key={a.id} album={a} />)}</div>
        </Section>
      ) : null}

      <Section title="Browse by genre">
        <div className="genre-pills">
          {feed.genres.map((g) => (
            <button
              key={g.id}
              className="genre-pill"
              style={{ ["--img" as string]: `url(${g.picture})` }}
              onClick={() => go({ name: "genre", id: g.id, title: g.name })}
            >
              <span>{g.name}</span>
            </button>
          ))}
        </div>
      </Section>

      <Section title="Top albums right now" hint="Deezer chart">
        <div className="grid">{feed.chartAlbums.slice(0, 18).map((a) => <AlbumTile key={a.id} album={a} />)}</div>
      </Section>

      <Section title="Popular artists">
        <div className="rail">{feed.topArtists.map((a) => <ArtistTile key={a.id} artist={a} />)}</div>
      </Section>
    </>
  );
}
