import { createContext, useCallback, useContext, useState, type SetStateAction } from "react";
import type { Route } from "../store";

/**
 * Per-page memory. Views unmount when you navigate away, which throws away their React state; values kept
 * here survive that, so going back to a page shows the same filters, loaded results and scroll position.
 * It lives for the app session only (not saved to disk).
 */
const memory = new Map<string, unknown>();
const scrolls = new Map<string, number>();

/** Identity of a page: the same route always maps to the same key. */
export function routeKey(route: Route): string {
  switch (route.name) {
    case "search": return `search:${route.q}`;
    case "album": return `album:${route.id}`;
    case "artist": return `artist:${route.id}`;
    case "genre": return `genre:${route.id}`;
    default: return route.name;
  }
}

export const PageKey = createContext("default");

/** Like `useState`, but the value is remembered for the current page across navigation. */
export function usePageState<T>(name: string, initial: T | (() => T)): [T, (next: SetStateAction<T>) => void] {
  const key = `${useContext(PageKey)}::${name}`;
  const [value, setValue] = useState<T>(() => {
    if (memory.has(key)) return memory.get(key) as T;
    return typeof initial === "function" ? (initial as () => T)() : initial;
  });
  const set = useCallback((next: SetStateAction<T>) => {
    setValue((prev) => {
      const v = typeof next === "function" ? (next as (p: T) => T)(prev) : next;
      memory.set(key, v);
      return v;
    });
  }, [key]);
  return [value, set];
}

/** True when this page has remembered a value under `name` (i.e. we are coming back to it). */
export function usePageHas(name: string): boolean {
  const key = `${useContext(PageKey)}::${name}`;
  const [had] = useState(() => memory.has(key));
  return had;
}

export function saveScroll(page: string, top: number) {
  scrolls.set(page, top);
}

export function savedScroll(page: string): number {
  return scrolls.get(page) ?? 0;
}
