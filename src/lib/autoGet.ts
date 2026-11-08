import { commands, events } from "../bindings";
import type {
  AlbumCard, ConnectionSnapshot, DiscoveryAlbum, SearchResult, TransferQueueSnapshot, UsenetTransferQueue,
} from "../bindings";
import { errorMessage, formatBytes, remoteFolder, remoteTitle, simplifyTitle } from "./format";

/* ------------------------------------------------------- Soulseek grouping */

const AUDIO = new Set(["flac", "mp3", "m4a", "aac", "ogg", "opus", "wav", "aiff", "ape", "wv"]);

export interface Candidate {
  id: string;
  username: string;
  folder: string;
  files: SearchResult[];
  formats: Set<string>;
  totalSize: number;
  slotFree: boolean;
  speed: number;
  queue: number;
}

/** Groups Soulseek search hits into one candidate per user folder, best first. */
export function group(results: Iterable<SearchResult>): Candidate[] {
  const map = new Map<string, Candidate>();
  for (const r of results) {
    const ext = r.extension.toLowerCase().replace(/^\./, "");
    if (r.isPrivate || !AUDIO.has(ext)) continue;
    const folder = remoteFolder(r.filename);
    const key = `${r.username.toLowerCase()}\u001f${folder.toLowerCase()}`;
    const c = map.get(key);
    if (c) {
      if (!c.files.some((f) => f.filename === r.filename)) { c.files.push(r); c.totalSize += r.sizeBytes; }
      c.slotFree ||= r.slotFree;
      c.speed = Math.max(c.speed, r.averageSpeed);
      c.queue = Math.min(c.queue, r.queueLength);
      c.formats.add(ext.toUpperCase());
    } else {
      map.set(key, { id: key, username: r.username, folder, files: [r], formats: new Set([ext.toUpperCase()]), totalSize: r.sizeBytes, slotFree: r.slotFree, speed: r.averageSpeed, queue: r.queueLength });
    }
  }
  return [...map.values()]
    .map((c) => ({ ...c, files: [...c.files].sort((a, b) => a.filename.localeCompare(b.filename, undefined, { numeric: true, sensitivity: "base" })) }))
    .sort((a, b) => Number(b.slotFree) - Number(a.slotFree) || b.files.length - a.files.length || b.speed - a.speed || a.queue - b.queue)
    .slice(0, 30);
}

/** Searches Soulseek until it finishes (or times out) and returns the best-looking complete release. */
async function pickSoulseek(album: AlbumCard): Promise<Candidate | null> {
  const id = `flip-auto-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
  const found = new Map<string, SearchResult>();
  let finished = false;
  const off = await events.flipsideSoulseekSearch.listen((e) => {
    const p = e.payload;
    if (p.snapshot.clientId !== id) return;
    p.results.forEach((r) => found.set(r.id, r));
    if (p.event === "completed" || p.event === "stopped" || p.event === "error") finished = true;
  });
  const wanted = album.trackCount ?? 0;
  const good = () =>
    group(found.values()).filter((c) =>
      wanted ? c.files.length >= Math.max(3, Math.round(wanted * 0.8)) && c.files.length <= wanted * 1.6 : c.files.length >= 3,
    );
  try {
    await commands.searchStart(id, `${album.artist} ${simplifyTitle(album.title)}`.trim());
    const started = Date.now();
    while (!finished && Date.now() - started < 22_000) {
      await new Promise((r) => setTimeout(r, 1000));
      // Good enough to stop early: a complete copy with a free slot, after giving peers a few seconds to answer.
      if (Date.now() - started > 8_000 && good().some((c) => c.slotFree)) break;
    }
  } finally {
    off();
    void commands.searchStop(id).catch(() => undefined);
    void commands.searchClose(id).catch(() => undefined);
  }
  const lossless = (c: Candidate) => (c.formats.has("FLAC") ? 1 : 0);
  return good().sort((a, b) => Number(b.slotFree) - Number(a.slotFree) || lossless(b) - lossless(a) || b.speed - a.speed)[0] ?? null;
}

/* ---------------------------------------------------------------- Pipeline */

export type StepId = "deemix" | "usenet" | "soulseek";
export type StepState = "pending" | "running" | "done" | "skipped" | "failed";

export interface AutoDeps {
  startDeemix: (album: AlbumCard, allowDuplicate?: boolean) => Promise<boolean>;
  setUsenet: (queue: UsenetTransferQueue) => void;
  setSoulseek: (queue: TransferQueueSnapshot) => void;
  /** Read when the Soulseek step starts, so a long batch sees the current connection. */
  connection: () => ConnectionSnapshot | null;
}

export interface AutoOutcome {
  /** Where the album came from; "owned" when it was already downloaded, null when nothing worked. */
  via: StepId | "owned" | null;
  message: string;
}

/** A card for an album Deezer doesn't have, so the Usenet and Soulseek steps can still look for it. */
export function discoveryCard(album: DiscoveryAlbum): AlbumCard {
  return {
    id: "", title: album.title, artist: album.artist, artistId: null, cover: album.cover, coverLarge: album.cover,
    year: album.year, releaseDate: null, kind: "album", trackCount: null, explicit: false,
  };
}

/** Finds the Deezer copy of a MusicBrainz album, or falls back to a Deezer-less card. */
export async function resolveDiscovery(album: DiscoveryAlbum): Promise<AlbumCard> {
  return (await commands.catalogResolveAlbum(album.artist, album.title)) ?? discoveryCard(album);
}

/**
 * Tries each provider in order and stops at the first one that delivers: Deezer, then Usenet, then Soulseek.
 * `onStep` reports progress for each provider.
 */
export async function autoGet(
  album: AlbumCard,
  deps: AutoDeps,
  onStep: (id: StepId, state: StepState, detail: string) => void = () => undefined,
): Promise<AutoOutcome> {
  // 1. Deezer
  onStep("deemix", "running", "Checking your Deezer account...");
  try {
    const cred = album.id ? await commands.getDeemixCredentialStatus() : null;
    if (!album.id) {
      onStep("deemix", "skipped", "Not available on Deezer");
    } else if (!cred?.configured) {
      onStep("deemix", "skipped", "No Deezer ARL set");
    } else {
      const pre = await commands.preflightDeemixAlbumDownload({ albumId: album.id, artist: album.artist, album: album.title, year: album.year });
      if (pre.alreadyDownloaded) {
        onStep("deemix", "done", "Already in your Deezer folder");
        return { via: "owned", message: "You already have this album, so nothing to do. Use the Deezer tab to download another copy." };
      }
      onStep("deemix", "running", "Downloading from Deezer...");
      if (await deps.startDeemix(album)) {
        onStep("deemix", "done", "Downloaded and tagged");
        return { via: "deemix", message: "Got it from Deezer." };
      }
      onStep("deemix", "failed", "Deezer download failed");
    }
  } catch (e) {
    onStep("deemix", "failed", errorMessage(e));
  }

  // 2. Usenet
  onStep("usenet", "running", "Searching Usenet...");
  try {
    const res = await commands.usenetSearch({ title: simplifyTitle(album.title), artist: album.artist, year: album.year, limit: 40 });
    const best = [...res.results].sort((a, b) => b.matchScore - a.matchScore || (b.grabs ?? 0) - (a.grabs ?? 0))[0];
    if (!best || best.matchScore < 60) {
      onStep("usenet", "skipped", res.results.length ? "No confident match" : "No NZBs found");
    } else {
      deps.setUsenet(
        await commands.usenetEnqueueDownload({
          guid: best.guid, title: best.title, indexer: best.indexer, downloadUrl: best.downloadUrl, sizeBytes: best.sizeBytes,
          expectedArtist: album.artist, expectedAlbum: album.title, expectedYear: album.year, releaseGroupId: null,
        }),
      );
      onStep("usenet", "done", `Queued "${best.title}" (${formatBytes(best.sizeBytes)})`);
      return { via: "usenet", message: "Queued on Usenet. Watch it in Downloads." };
    }
  } catch (e) {
    onStep("usenet", "failed", errorMessage(e));
  }

  // 3. Soulseek
  const connection = deps.connection();
  if (connection?.state !== "online") {
    onStep("soulseek", "skipped", connection?.state === "unconfigured" ? "Soulseek is not set up" : `Soulseek is ${connection?.state ?? "offline"}`);
  } else {
    onStep("soulseek", "running", "Listening to the network (up to 20s)...");
    try {
      const c = await pickSoulseek(album);
      if (!c) {
        onStep("soulseek", "skipped", "No complete copy found");
      } else {
        deps.setSoulseek(
          await commands.transferEnqueueRelease({
            title: `${album.artist} - ${album.title}${album.year ? ` (${album.year})` : ""}`,
            username: c.username,
            remoteFolder: c.folder,
            files: c.files.map((f) => ({ title: remoteTitle(f.filename), remoteFilename: f.filename, sizeBytes: f.sizeBytes })),
            expectedTrackCount: c.files.length,
            releaseGroupId: null,
            alternatives: [],
          }),
        );
        onStep("soulseek", "done", `${c.files.length} ${[...c.formats].join("/")} files from ${c.username}`);
        return { via: "soulseek", message: "Queued on Soulseek. Watch it in Downloads." };
      }
    } catch (e) {
      onStep("soulseek", "failed", errorMessage(e));
    }
  }
  return { via: null, message: "None of the providers could get this album. Try the individual tabs to pick a source by hand." };
}
