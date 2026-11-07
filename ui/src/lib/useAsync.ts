import { useEffect, useState } from "react";

/** The state of a value loaded asynchronously. */
export type Loaded<T> =
  | { status: "loading"; data: T | null }
  | { status: "ready"; data: T }
  | { status: "error"; data: T | null; error: string };

/**
 * Run `load` whenever `deps` change. A newer run supersedes an older one; the previous data
 * stays visible while the next arrives.
 */
export function useAsync<T>(load: () => Promise<T>, deps: readonly unknown[]): Loaded<T> {
  const [state, setState] = useState<Loaded<T>>({ status: "loading", data: null });
  // A generic loader: the caller's `deps` list what `load` reads, which the rule cannot see.
  // oxlint-disable react-hooks/exhaustive-deps
  useEffect(() => {
    let live = true;
    setState((s) => ({ status: "loading", data: s.data }));
    load().then(
      (data) => live && setState({ status: "ready", data }),
      (error: unknown) =>
        live &&
        setState((s) => ({
          status: "error",
          data: s.data,
          error: error instanceof Error ? error.message : String(error),
        })),
    );
    return () => {
      live = false;
    };
  }, deps);
  // oxlint-enable react-hooks/exhaustive-deps
  return state;
}
