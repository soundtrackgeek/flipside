## Development Workflow

**IMPORTANT**: After any code change, bug fix, or feature addition/removal, you MUST complete all of these steps:

1. **Update README.md** if the change affects:
   - Usage examples or commands
   - Installation instructions
   - Configuration options
   - Available features

2. **Update CHANGELOG.md**:
   - Add new version number following semantic versioning (MAJOR.MINOR.PATCH)
   - Add entry under appropriate category (Added, Changed, Fixed, Removed)
   - Include date in format YYYY-MM-DD

3. **Bump the version everywhere it lives** (they must agree; `npm run check:version` enforces it):
   - `package.json`
   - `src-tauri/Cargo.toml`
   - `src-tauri/tauri.conf.json`
   - `src-tauri/Cargo.lock` is updated by building (`cargo check`) after the bump.

4. **Commit and push changes**:
   - Use `git add` to stage all modified files (README.md, CHANGELOG.md, and code files)
   - Create descriptive commit message following existing style
   - Push to remote repository with `git push`
   - **Gotcha**: a plain `git push` is rejected with `fatal error in commit_refs` (the default git credential
     lacks the `workflow` scope and the repo contains workflow files). Always push with the GitHub CLI's
     credential instead:
     `git -c credential.helper= -c "credential.helper=!gh auth git-credential" push`

5. **After every push, make sure the release workflow produced downloadable `.exe` files**:
   - `.github/workflows/release.yml` runs on every push to `master`. It builds the Windows installer
     (NSIS `.exe`), signs the updater artifacts and publishes a GitHub Release named `v<version>` with
     the installer, its `.sig` and `latest.json` attached. The in-app updater reads `latest.json`.
   - A push that does not bump the version builds nothing (the release for that version already exists),
     so always bump the version (step 3).
   - Check the run with `gh run list --limit 3` / `gh run watch`, and confirm the release has an `.exe`
     with `gh release view v<version>`. If the workflow fails, fix it and push again.

6. **Run `cargo clean` after each push** (from `src-tauri/`) to reclaim disk space; the `target/` directory
   gets very large. The next build will be a full rebuild, which is expected.

## Project notes

- **Stack**: Tauri 2 (Rust backend in `src-tauri/`) + React/TypeScript/Vite frontend in `src/`.
- **Node comes from fnm** on this machine; load it in the same shell command, e.g. in PowerShell:
  `fnm env --use-on-cd --shell powershell | Out-String | Invoke-Expression; npm run build`.
- **Backend layout** (`src-tauri/src/`): `catalog.rs` (Deezer browse/search + Last.fm/Fanart/Discogs
  enrichment), `deemix*.rs` (Deezer downloads), `usenet.rs` (Prowlarr + NNTP downloader), `soulseek/`
  (native Soulseek client), `keys.rs` (metadata API keys + `.env` seeding), `db.rs` (settings JSON +
  Deemix receipts SQLite), `updater.rs` (update check/install), `events.rs` + `bindings.rs` (typed IPC).
- **Provider code is ported from Music Library** (`C:\_code\music_backup_v5`). Prefer syncing fixes from there
  over rewriting; keep the module APIs unchanged.
- **Typed IPC**: commands are listed once in `bindings.rs`. `src/bindings.ts` is generated (debug builds
  refresh it; `npm run bindings` regenerates it). Event names derive from their wire names, e.g.
  `flipside://usenet-transfers` becomes `events.flipsideUsenetTransfers`.
- **Secrets**: never commit `.env` (it is gitignored). API keys live in Windows Credential Manager under the
  `com.soundtrackgeek.flipside.*` services; `.env` only seeds them on first run.
- **Updater signing**: the private key is in `%USERPROFILE%\.flipside-signing\flipside.key` and in the GitHub
  secret `TAURI_SIGNING_PRIVATE_KEY`. The matching public key is in `tauri.conf.json`. Losing the private key
  breaks updates for installed copies.
- **Windows is case-insensitive**: do not create files that differ only by case (e.g. `app.tsx` and `App.tsx`).
- **Tests**: `cd src-tauri && cargo test --lib -- --test-threads=1`. Live-network smoke tests are `#[ignore]`d;
  run them with `-- --ignored`.
