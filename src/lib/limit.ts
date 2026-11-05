// A tiny concurrency limiter so a page full of lookups cannot hammer Deezer, Discogs or MusicBrainz.
let active = 0;
const waiting: Array<() => void> = [];
const MAX_ACTIVE = 3;

export function limited<T>(task: () => Promise<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const run = () => {
      active++;
      task().then(resolve, reject).finally(() => { active--; waiting.shift()?.(); });
    };
    if (active < MAX_ACTIVE) run(); else waiting.push(run);
  });
}
