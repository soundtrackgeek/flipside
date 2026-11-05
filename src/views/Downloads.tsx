import { useMemo } from "react";
import { FolderOpen, Pause, Play, RotateCcw, Trash2, X, Disc3, Download } from "lucide-react";
import { commands } from "../bindings";
import type { TransferSnapshot, UsenetTransfer } from "../bindings";
import { useApp, type DeemixJob } from "../store";
import { Empty, Section } from "../components/ui";
import { errorMessage, formatBytes, formatSpeed } from "../lib/format";

async function reveal(path: string) {
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}

function Thumb({ src }: { src?: string }) {
  return src ? <img src={src} alt="" /> : <div className="ph"><Disc3 size={22} /></div>;
}

function DeemixRow({ job }: { job: DeemixJob }) {
  const { toast, startDeemix } = useApp();
  const pct = job.total > 0 ? Math.round((job.completed / job.total) * 100) : null;
  return (
    <div className="dl-row">
      <Thumb src={job.cover} />
      <div style={{ minWidth: 0 }}>
        <div className="t"><span className="prov deemix">Deezer</span>{job.artist} — {job.title}</div>
        <div className="m">{job.message}</div>
        {job.status === "running" ? (
          <div className={`progress${pct == null ? " indeterminate" : ""}`}><i style={pct != null ? { width: `${pct}%` } : undefined} /></div>
        ) : (
          <span className={`chip ${job.status === "done" ? "good" : "bad"}`}>{job.status === "done" ? "Completed" : "Failed"}</span>
        )}
      </div>
      <div style={{ display: "flex", gap: 6 }}>
        {job.status !== "running" ? (
          <button className="btn small" title="Download this album again" onClick={() => void startDeemix(job.album, true)}><RotateCcw size={14} /> Re-download</button>
        ) : null}
        {job.destination ? (
          <button className="btn small" onClick={() => reveal(job.destination!).catch((e) => toast("error", errorMessage(e)))}><FolderOpen size={14} /> Show</button>
        ) : null}
      </div>
    </div>
  );
}

function UsenetRow({ t }: { t: UsenetTransfer }) {
  const { toast, setUsenet } = useApp();
  const finished = t.status === "completed" || t.status === "failed";
  return (
    <div className="dl-row">
      <div className="ph"><Download size={22} /></div>
      <div style={{ minWidth: 0 }}>
        <div className="t"><span className="prov usenet">Usenet</span>{t.title}</div>
        <div className="m">{t.error ?? t.message} · {t.indexer}{t.totalBytes ? ` · ${formatBytes(t.downloadedBytes)} / ${formatBytes(t.totalBytes)}` : ""}</div>
        {finished ? (
          <span className={`chip ${t.status === "completed" ? "good" : "bad"}`}>{t.status === "completed" ? "Completed" : "Failed"}</span>
        ) : (
          <div className="progress"><i style={{ width: `${Math.max(2, t.progressPercent)}%` }} /></div>
        )}
      </div>
      <div style={{ display: "flex", gap: 6 }}>
        {finished ? (
          <button className="btn small" title="Download this release again"
            onClick={() => commands.usenetRetryDownload(t.id).then(setUsenet).catch((e) => toast("error", errorMessage(e)))}><RotateCcw size={14} /> Re-download</button>
        ) : null}
        {t.destinationPath ? (
          <button className="btn small" onClick={() => reveal(t.destinationPath!).catch((e) => toast("error", errorMessage(e)))}><FolderOpen size={14} /> Show</button>
        ) : null}
      </div>
    </div>
  );
}

interface Release {
  id: string;
  title: string;
  username: string;
  files: TransferSnapshot[];
}

function SoulseekRow({ release }: { release: Release }) {
  const { setSoulseek, toast } = useApp();
  const stats = useMemo(() => {
    const total = release.files.reduce((n, f) => n + f.sizeBytes, 0);
    const done = release.files.reduce((n, f) => n + f.transferredBytes, 0);
    const completed = release.files.filter((f) => f.status === "completed").length;
    const failed = release.files.filter((f) => f.status === "failed").length;
    const paused = release.files.every((f) => f.status === "paused" || f.status === "completed");
    const speed = release.files.reduce((n, f) => n + (f.status === "downloading" ? f.speedBytesPerSecond : 0), 0);
    const active = release.files.find((f) => f.status === "downloading");
    return { total, done, completed, failed, paused, speed, active };
  }, [release]);
  const finished = stats.completed + stats.failed === release.files.length;
  const pct = stats.total > 0 ? Math.round((stats.done / stats.total) * 100) : 0;
  const act = (p: Promise<unknown>) => p.catch((e) => toast("error", errorMessage(e)));

  return (
    <div className="dl-row">
      <div className="ph"><Disc3 size={22} /></div>
      <div style={{ minWidth: 0 }}>
        <div className="t"><span className="prov soulseek">Soulseek</span>{release.title}</div>
        <div className="m">
          {release.username} · {stats.completed}/{release.files.length} files · {formatBytes(stats.done)} / {formatBytes(stats.total)}
          {stats.speed > 0 ? ` · ${formatSpeed(stats.speed)}` : ""}
          {stats.failed ? ` · ${stats.failed} failed` : ""}
        </div>
        {finished ? (
          <span className={`chip ${stats.failed ? "warn" : "good"}`}>{stats.failed ? "Finished with errors" : "Completed"}</span>
        ) : stats.paused ? (
          <span className="chip">Paused</span>
        ) : (
          <div className="progress"><i style={{ width: `${Math.max(2, pct)}%` }} /></div>
        )}
      </div>
      <div style={{ display: "flex", gap: 6 }}>
        {!finished ? (
          stats.paused ? (
            <button className="icon-btn" title="Resume" onClick={() => act(commands.transferResumeRelease(release.id).then(setSoulseek))}><Play size={15} /></button>
          ) : (
            <button className="icon-btn" title="Pause" onClick={() => act(commands.transferPauseRelease(release.id).then(setSoulseek))}><Pause size={15} /></button>
          )
        ) : null}
        <button className="icon-btn" title="Show in folder" onClick={() => act(commands.transferRevealReleasePath(release.id).then(reveal))}><FolderOpen size={15} /></button>
        {!finished ? (
          <button className="icon-btn" title="Cancel" onClick={() => act(commands.transferCancelRelease(release.id).then(setSoulseek))}><X size={15} /></button>
        ) : null}
      </div>
    </div>
  );
}

export default function Downloads() {
  const { deemixJobs, clearDeemixJobs, usenet, setUsenet, soulseek, setSoulseek, toast } = useApp();

  const releases = useMemo<Release[]>(() => {
    const map = new Map<string, Release>();
    for (const t of soulseek?.transfers ?? []) {
      const id = t.releaseId ?? t.id;
      const r = map.get(id) ?? { id, title: t.releaseTitle ?? t.title, username: t.username, files: [] };
      r.files.push(t);
      map.set(id, r);
    }
    return [...map.values()].reverse();
  }, [soulseek]);

  const nothing = deemixJobs.length === 0 && usenet.transfers.length === 0 && releases.length === 0;
  const clear = (p: Promise<unknown>) => p.catch((e) => toast("error", errorMessage(e)));

  return (
    <>
      <h1 className="page-title">Downloads</h1>
      <p className="page-sub">Everything you've grabbed this session, across Deezer, Usenet and Soulseek.</p>
      {nothing ? (
        <Empty icon={<Download size={38} />}>Nothing downloading. Open an album and hit “Get this album”.</Empty>
      ) : null}

      {deemixJobs.length > 0 ? (
        <Section title="Deezer">
          <div style={{ marginBottom: 10 }}><button className="btn small ghost" onClick={clearDeemixJobs}><Trash2 size={13} /> Clear finished</button></div>
          {deemixJobs.map((j) => <DeemixRow key={j.requestId} job={j} />)}
        </Section>
      ) : null}

      {usenet.transfers.length > 0 ? (
        <Section title="Usenet">
          <div style={{ marginBottom: 10 }}>
            <button className="btn small ghost" onClick={() => clear(commands.usenetClearCompleted().then(setUsenet))}><Trash2 size={13} /> Clear finished</button>
          </div>
          {[...usenet.transfers].reverse().map((t) => <UsenetRow key={t.id} t={t} />)}
        </Section>
      ) : null}

      {releases.length > 0 ? (
        <Section title="Soulseek">
          <div style={{ marginBottom: 10 }}>
            <button className="btn small ghost" onClick={() => clear(commands.transferClearCompleted().then(setSoulseek))}><Trash2 size={13} /> Clear finished</button>
          </div>
          {releases.map((r) => <SoulseekRow key={r.id} release={r} />)}
        </Section>
      ) : null}
    </>
  );
}
