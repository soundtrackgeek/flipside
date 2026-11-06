import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";

// sites.js attaches itself to globalThis, exactly like it does in the browser.
(0, eval)(readFileSync(fileURLToPath(new URL("../sites.js", import.meta.url)), "utf8"));
const { extract } = globalThis.FlipsideSites;

function pageAt(href, html) {
  const { document } = parseHTML(`<!doctype html><html><head></head><body>${html}</body></html>`);
  return extract(document, new URL(href));
}

test("Discogs release: artist link + title, strips the '(2)' disambiguation", () => {
  const found = pageAt("https://www.discogs.com/release/1-x",
    `<h1><a href="/artist/1">Nirvana (2)</a> – Nevermind</h1><a href="/search?year=1991">1991</a>`);
  assert.deepEqual({ ...found }, { artist: "Nirvana", album: "Nevermind", year: 1991, site: "Discogs" });
});

test("Discogs ignores non-release pages", () => {
  assert.equal(pageAt("https://www.discogs.com/artist/1-x", `<h1><a>X</a> – Y</h1>`), null);
});

test("Rate Your Music: title text node + artist", () => {
  const found = pageAt("https://rateyourmusic.com/release/album/radiohead/ok-computer/",
    `<div class="album_title">OK Computer <input class="album_shortcut"></div><div class="album_info"><a class="artist">Radiohead</a></div>`);
  assert.equal(found.artist, "Radiohead");
  assert.equal(found.album, "OK Computer");
});

test("Album Of The Year", () => {
  const found = pageAt("https://www.albumoftheyear.org/album/1-ok-computer.php",
    `<div class="albumHeadline"><h1><span itemprop="byArtist"><a><span itemprop="name">Radiohead</span></a></span> - <span class="albumTitle"><span itemprop="name">OK Computer</span></span></h1></div>`);
  assert.equal(found.artist, "Radiohead");
  assert.equal(found.album, "OK Computer");
});

test("AllMusic", () => {
  const found = pageAt("https://www.allmusic.com/album/ok-computer-mw0000",
    `<h1 class="album-title"> OK Computer </h1><h2 class="album-artist"><a>Radiohead</a></h2>`);
  assert.equal(found.artist, "Radiohead");
  assert.equal(found.album, "OK Computer");
});

test("YouTube: 'Artist - Album (Full Album)' is recognised and cleaned", () => {
  const found = pageAt("https://www.youtube.com/watch?v=abc", `<h1>Pink Floyd - The Wall (Full Album) [HD]</h1>`);
  assert.equal(found.artist, "Pink Floyd");
  assert.equal(found.album, "The Wall");
});

test("YouTube: a normal video gets no badge", () => {
  assert.equal(pageAt("https://www.youtube.com/watch?v=abc", `<h1>Radiohead - Creep (Official Video)</h1>`), null);
});

test("YouTube: auto-generated Topic channel supplies the artist", () => {
  const found = pageAt("https://www.youtube.com/playlist?list=x",
    `<h1>Kid A</h1><ytd-channel-name><a>Radiohead - Topic</a></ytd-channel-name>`);
  assert.equal(found.artist, "Radiohead");
  assert.equal(found.album, "Kid A");
});

test("unknown sites return null", () => {
  assert.equal(pageAt("https://example.com/x", `<h1>A - B</h1>`), null);
});
