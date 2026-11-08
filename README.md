# Flipside

A beautiful, snappy **visual store for music** — browse charts and discographies like a record shop, then
grab what you want through **Deezer (Deemix)**, **Usenet (Prowlarr)** or **Soulseek**. Built with Rust, Tauri 2
and React.

## Features

- **Discover** — featured album, editor's picks, genres, the album chart and popular artists.
- **For you** — studio albums you don't own: gaps in artists you collect, plus albums by similar artists you don't have
  yet (Last.fm → MusicBrainz/Discogs). Filter by decade, year, genre or text, and exclude genres (`scores` hides
  film/TV/game music, like in Music Library; e.g. `scores, Soundtrack, Synthwave`). Uses your Music Library databases
  (read-only) to hide everything you already own; only studio albums are listed (no live, compilations, EPs, singles).
- **Search** albums, artists and tracks; open rich album and artist pages (track lists, discographies, related
  artists, bios, tags, backdrops).
- **Auto-get** — one click: tries Deezer, then Usenet, then Soulseek, and stops at the first that delivers.
- **Choose source** — or pick a provider by hand in the drawer:
  - **Deezer** — direct, tagged downloads through your Deezer account (MP3 128/320 or FLAC).
  - **Usenet** — Prowlarr finds NZBs; Flipside downloads, repairs (PAR2) and extracts them itself.
  - **Soulseek** — a native protocol client with live results; downloads whole release folders.
- **Browser extension** — `extension/` holds an Edge/Chrome extension that, on Discogs, Rate Your Music, Album Of The
  Year, AllMusic and YouTube, shows whether the album on the page is available via Deezer, Usenet and Soulseek and
  sends it to Auto-get with one click (or automatically, if you enable it). Load it unpacked from `edge://extensions`,
  then paste the token from **Settings → Browser extension**. See `extension/README.md`.
- **Pages remember where you were** — leave a page and come back (sidebar or Back) and its filters, loaded
  results and scroll position are as you left them, for the rest of the session.
- **Shared music** — in Settings, choose folders to share on Soulseek (many users only serve people who share).
- **Downloads** page combining all three, with progress, pause/resume/cancel and *show in folder*.
- **Auto-update** — checks GitHub on launch and every 30 minutes (or on demand in Settings); accept the update
  and it downloads, installs and restarts on its own.

## Install

Download the latest `Flipside_*_x64-setup.exe` from the
[Releases page](https://github.com/soundtrackgeek/flipside/releases/latest) and run it. After that, updates arrive
inside the app.

## First-time setup

Open **Settings** and fill in the providers you use. Secrets are stored in Windows Credential Manager.

| Provider | What you need |
| --- | --- |
| Deezer (Deemix) | Your Deezer `arl` cookie, plus a download folder. |
| Usenet | Prowlarr URL + API key, and your Usenet provider's host, username and password. `unrar` and `par2` should be installed for extraction/repair. |
| Soulseek | Your Soulseek username and password. |
| Last.fm / Fanart.tv / Discogs | Optional API keys for bios, tags, artist backdrops and release details. |

### Seeding keys from `.env`

For development, copy `.env.example` to `.env` (it is gitignored) and fill it in. On startup, any provider keys
found there are copied into Credential Manager if they are not configured yet:

```
DEEMIX_ARL=
PROWLARR_API=
LAST_FM=
LAST_FM_SECRET=
DISCOGS_CONSUMER_KEY=
DISCOGS_CONSUMER_SECRET=
FANART_TV=
```

Default download folders are `Downloads\_Deemix`, `Downloads\_Usenet` and `Downloads\_SoulSeek`.

## Development

Requirements: Rust (stable), Node 22+, and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for
Windows (WebView2, MSVC build tools).

```bash
npm install
npm run tauri dev          # run the app with hot reload
npm run build              # type-check + build the frontend
npm run bindings           # regenerate src/bindings.ts from the Rust commands
cd src-tauri && cargo test --lib -- --test-threads=1
cd src-tauri && cargo test --lib -- --ignored   # live network smoke tests
```

Keyboard: `/` or `Ctrl+K` focuses search, `Alt+←` goes back, `Esc` closes the download drawer.

## Releases

Every push to `master` runs `.github/workflows/release.yml`, which builds the Windows installer, signs the update
artifacts and publishes a GitHub Release `v<version>` (installer, signature and `latest.json`). If that version
already has a release, the run does nothing — bump the version in `package.json`, `src-tauri/Cargo.toml` and
`src-tauri/tauri.conf.json` to ship a change (`npm run check:version` verifies they agree).

The updater needs the `TAURI_SIGNING_PRIVATE_KEY` repository secret; its public key lives in
`src-tauri/tauri.conf.json`.

## Credits

The Deemix, Usenet and Soulseek engines are ported from the author's *Music Library* app.
