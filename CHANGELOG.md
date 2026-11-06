# Changelog

All notable changes to Flipside are documented here. Versions follow [Semantic Versioning](https://semver.org/).

## [0.10.1] - 2026-11-06

### Fixed
- For you kept saying "Reading your library…" with an empty list when opened right after launch, even after the library had finished loading (it asked once and never again). It now checks every two seconds until the library is ready and then fills in the albums.

## [0.10.0] - 2026-11-06

### Added
- For you: **progress counters**: "Your artists checked for similar artists: 23,200 / 42,000" and "Similar artists explored: X / Y", shown at all times and updated live.
- **Find more now keeps going**: it works through your artists round after round (Last.fm's rate limit is respected) until everything is checked, with a **Stop** button. Progress is saved, so stopping and starting again continues where it left off.

### Fixed
- Artists Last.fm had no similar artists for were asked about again on every pass; they are now marked as checked. Artists that fail are skipped for the rest of the session instead of being retried forever.

## [0.9.0] - 2026-11-05

### Added
- **Auto-get button on every album tile** (the wand, bottom-left on hover, next to the download button). One click starts Deezer → Usenet → Soulseek straight away, without opening the album first.

## [0.8.0] - 2026-11-05

### Added
- **Re-download** button on finished Deezer and Usenet rows in Downloads, for when you deleted an album and want it again. Usenet transfers now remember their original request (downloads made before this version cannot be repeated). Deezer re-runs skip the duplicate check.

## [0.7.0] - 2026-11-05

### Added
- **New artist page** in two sections:
  - **Albums by <artist>**: only that artist's own studio albums (from MusicBrainz, local data first), each with its original release year and cover. Falls back to Deezer's list only when MusicBrainz knows nothing about the artist.
  - **If you like <artist>…**: up to eight similar artists (Last.fm, or Deezer's related artists), each with a row of their own studio albums. Albums you already own stay hidden.
- **Wikipedia biography** on artist pages (checked to be about a musician), with a "Read more on Wikipedia" link; Last.fm's blurb is the fallback.
- **Original release year** on every album tile and on album pages. Deezer only knows the date of the release it serves (often a re-release, e.g. 2008 for a 1972 album) and gives search results no date at all, so the year now comes from MusicBrainz, looked up as tiles scroll into view and cached.

### Changed
- Clicking an artist name on a For you tile now opens that artist's page instead of running a search.
- "Best of", "Greatest Hits", anthology and similar titles are dropped from the studio-album lists, since MusicBrainz often leaves such records untagged.

## [0.6.0] - 2026-11-05

### Added
- Many more album covers on the For you page. When the Cover Art Archive has no picture (or times out), Flipside retries once and then looks the album up on **Deezer** and then **Discogs**. Answers, including "nothing found", are cached in the local database so each album is looked up once. In a 30-album sample the Cover Art Archive alone covered 19, Deezer and Discogs each found about half, and in the app 55 of the first 60 tiles now show art.

## [0.5.1] - 2026-11-05

### Fixed
- Excluded genres (and every other For you filter) were forgotten whenever the background crawl refreshed the page, so excluded albums reappeared a few seconds later. The refresh now always uses your current filters.

## [0.5.0] - 2026-11-05

### Added
- **Exclude genres** on the For you page: a tag field (type a genre and press Enter or comma) that hides any album with one of those genres, e.g. `scores`, `Soundtrack`, `Synthwave`. `scores` is Music Library's umbrella and expands to Action, Animation, Anime, Comedy, Documentary, Drama, Fantasy, Horror, Sci-Fi, Thriller, TV, Video Game and Western. Matching ignores case and hyphens, suggestions come from the genres on screen, and the list is remembered between sessions. Excluded genres also disappear from the genre chips and counts.

## [0.4.0] - 2026-11-05

### Added
- **For you** page: studio albums you don't own yet, with decade, year range, genre, text and sort filters.
  - *From artists you own*: gaps in your collection, computed from your Music Library database plus the MusicBrainz cache and the newer overlay (no network needed).
  - *From similar artists*: artists Last.fm says are similar to yours but that you don't have, with their albums found through MusicBrainz (Discogs as a fallback) by a background crawl that runs at launch when the data is a week old, or on "Find more". New albums appear as they are found.
  - **Studio albums only**: MusicBrainz primary type Album with no secondary types and at least one official release. Live albums, compilations, soundtracks, remixes, EPs, singles and bootlegs are excluded. Anything you own is excluded, as are releases marked "not in scope" in Music Library.
  - "Best match" sorting spreads results across artists instead of listing one prolific artist first.
  - Opening an album finds it on Deezer; if it isn't there, the Get drawer still offers Usenet and Soulseek.
- Settings → **Your library**: locations of the Music Library, MusicBrainz cache and overlay databases (auto-detected from Music Library by default), load status, and reload. All databases are opened read-only.
- **Hide albums I already own** (on by default): albums in your library no longer appear anywhere in Flipside (Discover, search, artist pages, genres).

### Fixed
- Discogs and MusicBrainz requests now identify Flipside with a proper User-Agent (Discogs refuses anonymous clients).
- Settings switch labels are left-aligned.

## [0.3.0] - 2026-11-05

### Added
- **Shared music** section in Settings (under Soulseek): add, enable/disable, remove and rescan shared folders, set upload slots, and see live upload activity. Many Soulseek users only serve people who share something back.

## [0.2.0] - 2026-11-05

### Added
- **Auto-get**: a one-click way to get an album. Flipside tries Deezer first, then Usenet (best Prowlarr match, at least a confident score), then Soulseek (best complete folder, preferring free slots and FLAC), and stops at the first provider that delivers. Each step shows its status and why it was skipped or failed.
- The album page now has "Auto-get" and "Choose source" buttons; the Get drawer opens on a new Auto tab, and the manual provider tabs are still there.

## [0.1.1] - 2026-11-05

### Fixed
- Pages could not be scrolled: the content area now scrolls, so the bottom of Discover (genres, charts) and Settings are reachable.

## [0.1.0] - 2026-11-05

### Added
- First release of Flipside, a Tauri 2 + Rust + React desktop app: a visual, snappy music store.
- Discover page with a featured album, Deezer editor's picks, genre browsing, the current album chart and popular artists.
- Search across albums, artists and tracks, plus album pages (track list, label, tags) and artist pages (discography filter, popular tracks, related artists, Fanart.tv backdrops, Last.fm bios).
- "Get this album" drawer with three download providers, ported from Music Library's Wish List:
  - **Deezer (Deemix)** — direct download with tagging, quality and folder-layout settings, and duplicate detection.
  - **Usenet** — Prowlarr search plus a built-in NZB downloader (NNTP, PAR2 repair, extraction).
  - **Soulseek** — native protocol client with live, streaming search results and release-folder downloads.
- Downloads page that merges all three providers with progress, pause/resume/cancel and "show in folder".
- Settings → Providers for Deezer ARL, Prowlarr/Usenet, Soulseek and the Last.fm, Fanart.tv and Discogs keys. Secrets are stored in Windows Credential Manager; keys in a local `.env` seed it on first run.
- Auto-update: checks GitHub releases on start and every 30 minutes, with a manual "Check for updates" button; accepting an update downloads and installs it automatically.
- GitHub Actions release workflow that builds the Windows installer (`.exe`) and publishes it, with the updater manifest, to Releases.
