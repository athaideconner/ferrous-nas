import { useCallback, useEffect, useRef, useState } from "react";

export interface AsyncState<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
  reload: () => void;
}

/**
 * Fetch `fn` on mount and whenever `reload()` is called. When `intervalMs` is
 * given, silently refetches on that interval (without flipping `loading`, so
 * live dashboards don't flicker).
 */
export function useAsync<T>(fn: () => Promise<T>, intervalMs?: number): AsyncState<T> {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const fnRef = useRef(fn);
  fnRef.current = fn;

  const run = useCallback(async (silent: boolean) => {
    if (!silent) setLoading(true);
    try {
      const d = await fnRef.current();
      setData(d);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (!silent) setLoading(false);
    }
  }, []);

  useEffect(() => {
    run(false);
    if (!intervalMs) return;
    const id = setInterval(() => run(true), intervalMs);
    return () => clearInterval(id);
  }, [run, intervalMs]);

  return { data, error, loading, reload: () => run(false) };
}
