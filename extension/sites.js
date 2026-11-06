// Per-site album extraction. Each extractor takes (doc, url) and returns {artist, album, year?} or null.
// Pure functions of the DOM so they can be tested without a browser. Selectors are best effort: sites
// change markup, which is why the on-page panel lets you edit what was detected before sending it.
(function (root) {
  const clean = (s) => (s || "").replace(/\s+/g, " ").trim();
  const text = (doc, sel) => clean(doc.querySelector(sel)?.textContent);
  const meta = (doc, name) => clean(doc.querySelector(`meta[property="${name}"], meta[name="${name}"]`)?.getAttribute("content"));
  const SEPARATOR = /\s+[-–—]\s+/;

  function splitArtistAlbum(value) {
    const parts = clean(value).split(SEPARATOR);
    if (parts.length < 2) return null;
    return { artist: parts[0], album: parts.slice(1).join(" - ") };
  }

  // Discogs disambiguates same-named artists as "Name (2)"; the number is not part of the name.
  const stripDiscogsSuffix = (name) => name.replace(/\s*\(\d+\)$/, "").trim();

  function discogs(doc, url) {
    if (!/\/(release|master)\//.test(url.pathname)) return null;
    let pair = null;
    const h1 = doc.querySelector("h1");
    if (h1) {
      const artist = clean(h1.querySelector("a")?.textContent);
      const whole = clean(h1.textContent);
      if (artist && whole.startsWith(artist)) {
        const album = whole.slice(artist.length).replace(/^\s*[-–—]\s*/, "").trim();
        if (album) pair = { artist, album };
      }
      pair = pair || splitArtistAlbum(whole);
    }
    pair = pair || splitArtistAlbum((meta(doc, "og:title") || doc.title).replace(/\s*\|.*$/, "").replace(/\s*\((?:[^)]*(?:CD|LP|Vinyl|File|Album|Cassette)[^)]*)\)\s*$/i, ""));
    if (!pair) return null;
    pair.artist = stripDiscogsSuffix(pair.artist);
    const year = text(doc, "a[href*='year=']").match(/\d{4}/)?.[0];
    return { ...pair, year: year ? Number(year) : null };
  }

  function rym(doc, url) {
    if (!/\/release\//.test(url.pathname)) return null;
    const titleNode = doc.querySelector(".album_title");
    const album = clean([...(titleNode?.childNodes || [])].find((n) => n.nodeType === 3 && clean(n.textContent))?.textContent);
    const artist = text(doc, ".album_info .artist");
    if (artist && album) return { artist, album, year: null };
    // "OK Computer by Radiohead (album, ...)": cut the descriptor, then split on the last " by ".
    const heading = clean((meta(doc, "og:title") || doc.title).replace(/\s*[-|(].*$/, ""));
    const at = heading.lastIndexOf(" by ");
    return at > 0 ? { album: heading.slice(0, at), artist: heading.slice(at + 4), year: null } : null;
  }

  function aoty(doc, url) {
    if (!/\/album\//.test(url.pathname)) return null;
    const artist = text(doc, ".albumHeadline [itemprop='byArtist'] [itemprop='name']") || text(doc, ".artistTitle a");
    const album = text(doc, ".albumHeadline .albumTitle [itemprop='name']") || text(doc, ".albumTitle");
    if (artist && album) return { artist, album, year: null };
    const pair = splitArtistAlbum((meta(doc, "og:title") || doc.title).replace(/\s*\|.*$/, ""));
    return pair ? { ...pair, year: null } : null;
  }

  function allmusic(doc, url) {
    if (!/^\/album\//.test(url.pathname)) return null;
    const album = text(doc, "h1.album-title");
    const artist = text(doc, "h2.album-artist");
    if (album && artist) return { artist, album, year: null };
    const pair = splitArtistAlbum(doc.title.replace(/\s*\|.*$/, ""));
    return pair ? { ...pair, year: null } : null;
  }

  // YouTube has no structured album data. Only offer the badge when the title says it is a whole album
  // (or the uploader is an auto-generated "Artist - Topic" channel), otherwise every video would get one.
  const ALBUM_MARKER = /full\s+album|album\s+stream|complete\s+album|\(album\)|\[album\]|\bfull\s+lp\b|\bfull\s+ep\b/i;
  const NOISE = /\s*[\[(](?:full\s+album(?:\s+stream)?|album\s+stream|complete\s+album|official\s+(?:audio|video|album)[^\])]*|hd|hq|remastered?[^\])]*|\d{4})[\])]\s*/gi;

  function youtube(doc, url) {
    if (url.pathname !== "/watch" && url.pathname !== "/playlist") return null;
    const title = clean(text(doc, "h1.ytd-watch-metadata yt-formatted-string") || text(doc, "h1") || doc.title.replace(/\s*-\s*YouTube$/, ""));
    const channel = text(doc, "ytd-channel-name a, #owner #channel-name a");
    const topic = /\s-\s*Topic$/.test(channel);
    if (!ALBUM_MARKER.test(title) && !topic) return null;
    const tidy = (s) => clean(s.replace(NOISE, " ").replace(/\s*\|.*$/, "").replace(ALBUM_MARKER, "").replace(/[-–—]\s*$/, ""));
    const pair = splitArtistAlbum(tidy(title));
    if (pair && pair.artist && pair.album) return { ...pair, year: null };
    if (topic) return { artist: channel.replace(/\s-\s*Topic$/, ""), album: tidy(title), year: null };
    return null;
  }

  const SITES = [
    { host: /(^|\.)discogs\.com$/, name: "Discogs", extract: discogs },
    { host: /(^|\.)rateyourmusic\.com$/, name: "Rate Your Music", extract: rym },
    { host: /(^|\.)albumoftheyear\.org$/, name: "Album Of The Year", extract: aoty },
    { host: /(^|\.)allmusic\.com$/, name: "AllMusic", extract: allmusic },
    { host: /(^|\.)youtube\.com$/, name: "YouTube", extract: youtube },
  ];

  function extract(doc, url) {
    const site = SITES.find((s) => s.host.test(url.hostname));
    if (!site) return null;
    let found = null;
    try { found = site.extract(doc, url); } catch (_) { found = null; }
    if (!found || !found.artist || !found.album) return null;
    return { artist: clean(found.artist), album: clean(found.album), year: found.year || null, site: site.name };
  }

  root.FlipsideSites = { extract, splitArtistAlbum };
})(typeof globalThis !== "undefined" ? globalThis : this);
