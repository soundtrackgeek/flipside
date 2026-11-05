# Changelog

All notable changes to Flipside are documented here. Versions follow [Semantic Versioning](https://semver.org/).

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
