import { useCallback, useEffect, useRef, useState } from "react";

import {
  api,
  errorMessage,
  type SimPreset,
  type SimResult,
  type SimSpec,
  type SpecEdit,
  type WhatIfResult,
} from "../api";
import { applyEdits } from "./spec";

const STORAGE_KEY = "flowlab-scenario";
/** Wait this long after the last edit before simulating. */
const DEBOUNCE_MS = 350;
const MAX_UNDO = 50;

/** A result kept to compare later edits with. */
export interface Baseline {
  spec: SimSpec;
  result: SimResult;
}

export interface Scenario {
  presets: SimPreset[];
  spec: SimSpec | null;
  /** Result of the last spec that simulated; may be older than `spec` while `running`. */
  result: SimResult | null;
  /** The spec `result` is the result of. */
  resultSpec: SimSpec | null;
  running: boolean;
  /** Why the current spec could not be simulated. */
  error: string | null;
  whatIf: WhatIfResult | null;
  whatIfRunning: boolean;
  whatIfError: string | null;
  baseline: Baseline | null;
  canUndo: boolean;
  /** Replace the spec (an edit). */
  setSpec: (spec: SimSpec) => void;
  /** Apply a what-if change. */
  apply: (edits: readonly SpecEdit[]) => void;
  /** Start from a preset. */
  load: (preset: SimPreset) => void;
  undo: () => void;
  pinBaseline: () => void;
  clearBaseline: () => void;
}

function readStored(): SimSpec | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as SimSpec) : null;
  } catch {
    return null;
  }
}

function store(spec: SimSpec): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(spec));
  } catch {
    // Storage unavailable: the scenario lives for this page only.
  }
}

/**
 * The scenario being designed: the spec, its simulation (rerun shortly after every edit) and
 * the one-change variants. Lives in the app shell so it survives switching views.
 */
export function useScenario(): Scenario {
  const [presets, setPresets] = useState<SimPreset[]>([]);
  const [spec, setSpecState] = useState<SimSpec | null>(null);
  const [history, setHistory] = useState<SimSpec[]>([]);
  const [result, setResult] = useState<SimResult | null>(null);
  const [resultSpec, setResultSpec] = useState<SimSpec | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [whatIf, setWhatIf] = useState<WhatIfResult | null>(null);
  const [whatIfRunning, setWhatIfRunning] = useState(false);
  const [whatIfError, setWhatIfError] = useState<string | null>(null);
  const [baseline, setBaseline] = useState<Baseline | null>(null);
  const specRef = useRef<SimSpec | null>(null);
  const historyRef = useRef<SimSpec[]>([]);
  useEffect(() => {
    specRef.current = spec;
    historyRef.current = history;
  }, [spec, history]);

  useEffect(() => {
    api.simPresets().then(
      (list) => {
        setPresets(list);
        setSpecState((s) => s ?? readStored() ?? list[0]?.spec ?? null);
      },
      (e: unknown) => setError(errorMessage(e)),
    );
  }, []);

  // Simulate shortly after the last edit; a newer edit cancels the older run.
  useEffect(() => {
    if (!spec) return;
    store(spec);
    const ctl = new AbortController();
    const timer = setTimeout(() => {
      setRunning(true);
      setWhatIfRunning(true);
      api.simRun(spec, ctl.signal).then(
        (r) => {
          setResult(r);
          setResultSpec(spec);
          setError(null);
          setRunning(false);
        },
        (e: unknown) => {
          if (ctl.signal.aborted) return;
          setError(errorMessage(e));
          setRunning(false);
        },
      );
      api.whatIf(spec, ctl.signal).then(
        (w) => {
          setWhatIf(w);
          setWhatIfError(null);
          setWhatIfRunning(false);
        },
        (e: unknown) => {
          if (ctl.signal.aborted) return;
          setWhatIf(null);
          setWhatIfError(errorMessage(e));
          setWhatIfRunning(false);
        },
      );
    }, DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
      ctl.abort();
    };
  }, [spec]);

  const setSpec = useCallback((next: SimSpec) => {
    const prev = specRef.current;
    if (prev) setHistory((h) => [...h.slice(-(MAX_UNDO - 1)), prev]);
    setSpecState(next);
  }, []);

  const apply = useCallback(
    (edits: readonly SpecEdit[]) => {
      const prev = specRef.current;
      if (prev) setSpec(applyEdits(prev, edits));
    },
    [setSpec],
  );

  const load = useCallback(
    (preset: SimPreset) => {
      setSpec(structuredClone(preset.spec));
      setBaseline(null);
    },
    [setSpec],
  );

  const undo = useCallback(() => {
    const h = historyRef.current;
    const prev = h.at(-1);
    if (!prev) return;
    setHistory(h.slice(0, -1));
    setSpecState(prev);
  }, []);

  const pinBaseline = useCallback(() => {
    if (result && resultSpec) setBaseline({ spec: resultSpec, result });
  }, [result, resultSpec]);

  return {
    presets,
    spec,
    result,
    resultSpec,
    running,
    error,
    whatIf,
    whatIfRunning,
    whatIfError,
    baseline,
    canUndo: history.length > 0,
    setSpec,
    apply,
    load,
    undo,
    pinBaseline,
    clearBaseline: () => setBaseline(null),
  };
}
