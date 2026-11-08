import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { commands, events } from "./bindings";
import type {
  AlbumCard,
  ConnectionSnapshot,
  DeemixAlbumDownloadProgress,
  DiscoveryAlbum,
  TransferQueueSnapshot,
  UpdateSnapshot,
  UsenetTransferQueue,
} from "./bindings";
import { errorMessage } from "./lib/format";
import { autoGet, resolveDiscovery, type StepId } from "./lib/autoGet";

/* ------------------------------------------------------------------ routing */

export type Route =
  | { name: "home" }
  | { name: "search"; q: string }
  | { name: "album"; id: string; get?: boolean }
  | { name: "artist"; id: string }
  | { name: "genre"; id: string; title: string }
  | { name: "foryou" }
  | { name: "downloads" }
  | { name: "settings" };

/* ------------------------------------------------------------------- toasts */

export interface Toast {
  id: number;
  kind: "info" | "error" | "success";
  text: string;
}

/* ---------------------------------------------------------- deemix job list */

export interface DeemixJob {
  requestId: string;
  albumId: string;
  title: string;
  artist: string;
  cover: string;
  album: AlbumCard;
  phase: string;
  message: string;
  completed: number;
  total: number;
  status: "running" | "done" | "failed";
  destination?: string;
}

/* ------------------------------------------------- "Download all" batches */

export interface BatchItem {
  title: string;
  state: "pending" | "running" | "done" | "failed";
  /** Where it came from ("deemix", "usenet", "soulseek", "owned"), once done. */
  via: StepId | "owned" | null;
  detail: string;
}

export interface Batch {
  /** What the batch is for, e.g. the artist name; pages use it to show their own batch. */
  key: string;
  items: BatchItem[];
  running: boolean;
  stopping: boolean;
}

interface AppState {
  route: Route;
  canGoBack: boolean;
  go: (route: Route) => void;
  back: () => void;
  toast: (kind: Toast["kind"], text: string) => void;
  toasts: Toast[];
  dismissToast: (id: number) => void;

  deemixJobs: DeemixJob[];
  startDeemix: (album: AlbumCard, allowDuplicate?: boolean) => Promise<boolean>;
  clearDeemixJobs: () => void;
  usenet: UsenetTransferQueue;
  setUsenet: (queue: UsenetTransferQueue) => void;
  soulseek: TransferQueueSnapshot | null;
  setSoulseek: (queue: TransferQueueSnapshot) => void;
  connection: ConnectionSnapshot | null;

  update: UpdateSnapshot;
  installProgress: { phase: string; percent: number | null } | null;
  checkForUpdate: () => Promise<UpdateSnapshot | null>;
  installUpdate: () => Promise<void>;
  checkingUpdate: boolean;

  activeDownloads: number;
  /** One auto-get batch at a time: every album in the list, one after another. */
  batch: Batch | null;
  autoGetAll: (key: string, albums: DiscoveryAlbum[]) => void;
  stopBatch: () => void;
  /** Album the browser extension asked Flipside to get; the shell opens Auto-get for it. */
  extensionAlbum: AlbumCard | null;
  clearExtensionAlbum: () => void;
}

const Ctx = createContext<AppState | null>(null);

export function useApp(): AppState {
  const value = useContext(Ctx);
  if (!value) throw new Error("useApp outside provider");
  return value;
}

let toastSeq = 0;

export function AppProvider({ children }: { children: ReactNode }) {
  const [stack, setStack] = useState<Route[]>([{ name: "home" }]);
  const route = stack[stack.length - 1];
  const go = useCallback((next: Route) => setStack((s) => [...s.slice(-40), next]), []);
  const back = useCallback(() => setStack((s) => (s.length > 1 ? s.slice(0, -1) : s)), []);

  const [extensionAlbum, setExtensionAlbum] = useState<AlbumCard | null>(null);
  const clearExtensionAlbum = useCallback(() => setExtensionAlbum(null), []);

  const [toasts, setToasts] = useState<Toast[]>([]);
  const dismissToast = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);
  const toast = useCallback((kind: Toast["kind"], text: string) => {
    const id = ++toastSeq;
    setToasts((t) => [...t.slice(-3), { id, kind, text }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === "error" ? 9000 : 4500);
  }, []);

  /* --- deemix --- */
  const [deemixJobs, setDeemixJobs] = useState<DeemixJob[]>([]);
  const applyProgress = useCallback((p: DeemixAlbumDownloadProgress) => {
    setDeemixJobs((jobs) =>
      jobs.map((job) =>
        job.requestId === p.requestId
          ? { ...job, phase: p.phase, message: p.message, completed: p.completedTracks, total: p.totalTracks }
          : job,
      ),
    );
  }, []);

  const startDeemix = useCallback(
    async (album: AlbumCard, allowDuplicate = false): Promise<boolean> => {
      const requestId = `fs-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
      const job: DeemixJob = {
        requestId,
        albumId: album.id,
        title: album.title,
        artist: album.artist,
        cover: album.cover,
        album,
        phase: "metadata",
        message: "Starting…",
        completed: 0,
        total: album.trackCount ?? 0,
        status: "running",
      };
      setDeemixJobs((jobs) => [job, ...jobs]);
      try {
        const summary = await commands.downloadDeemixAlbum({
          albumId: album.id,
          requestId,
          expectedArtist: album.artist,
          expectedAlbum: album.title,
          expectedYear: album.year,
          allowDuplicate,
        });
        setDeemixJobs((jobs) =>
          jobs.map((j) =>
            j.requestId === requestId
              ? {
                  ...j,
                  status: "done",
                  phase: "complete",
                  message: `${summary.trackCount} tracks saved`,
                  completed: summary.trackCount,
                  total: summary.trackCount,
                  destination: summary.destinationPath,
                }
              : j,
          ),
        );
        toast("success", `Downloaded ${album.artist} — ${album.title}`);
        return true;
      } catch (error) {
        const message = errorMessage(error);
        setDeemixJobs((jobs) =>
          jobs.map((j) => (j.requestId === requestId ? { ...j, status: "failed", message } : j)),
        );
        toast("error", message);
        return false;
      }
    },
    [toast],
  );
  const clearDeemixJobs = useCallback(() => setDeemixJobs((jobs) => jobs.filter((j) => j.status === "running")), []);

  /* --- queues --- */
  const [usenet, setUsenet] = useState<UsenetTransferQueue>({ transfers: [], activeCount: 0 });
  const [soulseek, setSoulseek] = useState<TransferQueueSnapshot | null>(null);
  const [connection, setConnection] = useState<ConnectionSnapshot | null>(null);

  /* --- download-all batch --- */
  const [batch, setBatch] = useState<Batch | null>(null);
  const batchRunning = useRef(false);
  const stopRequested = useRef(false);
  const connectionRef = useRef(connection);
  connectionRef.current = connection;

  const autoGetAll = useCallback((key: string, albums: DiscoveryAlbum[]) => {
    if (batchRunning.current || albums.length === 0) return;
    batchRunning.current = true;
    stopRequested.current = false;
    setBatch({
      key, running: true, stopping: false,
      items: albums.map((a) => ({ title: a.title, state: "pending", via: null, detail: "Waiting" })),
    });
    const patch = (i: number, item: Partial<BatchItem>) =>
      setBatch((b) => (b ? { ...b, items: b.items.map((x, j) => (j === i ? { ...x, ...item } : x)) } : b));

    void (async () => {
      let got = 0;
      let missed = 0;
      for (let i = 0; i < albums.length && !stopRequested.current; i++) {
        patch(i, { state: "running", detail: "Looking it up..." });
        try {
          const card = await resolveDiscovery(albums[i]);
          const out = await autoGet(
            card,
            { startDeemix, setUsenet, setSoulseek, connection: () => connectionRef.current },
            (_id, state, detail) => state === "running" && patch(i, { detail }),
          );
          patch(i, { state: out.via ? "done" : "failed", via: out.via, detail: out.message });
          if (out.via) got++; else missed++;
        } catch (e) {
          patch(i, { state: "failed", detail: errorMessage(e) });
          missed++;
        }
      }
      const stopped = stopRequested.current;
      batchRunning.current = false;
      setBatch((b) => (b ? { ...b, running: false, stopping: false } : b));
      toast(
        missed && !got ? "error" : "success",
        `${stopped ? "Stopped" : "Done"}: ${got} of ${albums.length} albums downloaded or queued${missed ? `, ${missed} not found` : ""}.`,
      );
    })();
  }, [startDeemix, toast]);

  const stopBatch = useCallback(() => {
    if (!batchRunning.current) return;
    stopRequested.current = true;
    setBatch((b) => (b ? { ...b, stopping: true } : b));
  }, []);

  /* --- updates --- */
  const [update, setUpdate] = useState<UpdateSnapshot>({ checkedAt: null, info: null, error: null });
  const [installProgress, setInstallProgress] = useState<AppState["installProgress"]>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);

  const checkForUpdate = useCallback(async () => {
    setCheckingUpdate(true);
    try {
      const snapshot = await commands.checkAppUpdate();
      setUpdate(snapshot);
      return snapshot;
    } catch (error) {
      toast("error", errorMessage(error));
      return null;
    } finally {
      setCheckingUpdate(false);
    }
  }, [toast]);

  const installUpdate = useCallback(async () => {
    const version = update.info?.version;
    if (!version) return;
    setInstallProgress({ phase: "downloading", percent: 0 });
    try {
      await commands.installAppUpdate(version);
      // On Windows the installer takes over and closes the app; elsewhere restart ourselves.
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (error) {
      setInstallProgress(null);
      toast("error", errorMessage(error));
    }
  }, [update.info?.version, toast]);

  /* --- bootstrap + event wiring --- */
  const mounted = useRef(false);
  useEffect(() => {
    if (mounted.current) return;
    mounted.current = true;
    const unlisten: Array<() => void> = [];
    const keep = (p: Promise<() => void>) => void p.then((fn) => unlisten.push(fn));

    keep(events.flipsideExtensionGet.listen((e) => setExtensionAlbum({ ...e.payload })));
    keep(events.deemixDownloadProgress.listen((e) => applyProgress(e.payload)));
    keep(events.flipsideUsenetTransfers.listen((e) => setUsenet(e.payload)));
    keep(events.flipsideSoulseekTransfers.listen((e) => setSoulseek(e.payload)));
    keep(events.flipsideSoulseekConnection.listen((e) => setConnection(e.payload)));
    keep(events.appUpdateChecked.listen((e) => setUpdate(e.payload)));
    keep(
      events.appUpdateInstallProgress.listen((e) =>
        setInstallProgress({ phase: e.payload.phase, percent: e.payload.percent }),
      ),
    );

    void commands.usenetTransfersSnapshot().then(setUsenet).catch(() => undefined);
    void commands.transfersSnapshot().then(setSoulseek).catch(() => undefined);
    void commands.connectionBootstrap().then((b) => setConnection(b.snapshot)).catch(() => undefined);
    void commands.getAppUpdateStatus().then(setUpdate).catch(() => undefined);
    return () => unlisten.forEach((fn) => fn());
  }, [applyProgress]);

  const activeDownloads = useMemo(() => {
    const soul =
      soulseek?.transfers.filter((t) => t.status !== "completed" && t.status !== "failed" && t.status !== "paused")
        .length ?? 0;
    return deemixJobs.filter((j) => j.status === "running").length + usenet.activeCount + (soul > 0 ? 1 : 0);
  }, [deemixJobs, usenet, soulseek]);

  const value: AppState = {
    route, canGoBack: stack.length > 1, go, back, toast, toasts, dismissToast,
    deemixJobs, startDeemix, clearDeemixJobs, usenet, setUsenet, soulseek, setSoulseek, connection,
    update, installProgress, checkForUpdate, installUpdate, checkingUpdate, activeDownloads,
    batch, autoGetAll, stopBatch,
    extensionAlbum, clearExtensionAlbum,
  };
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
