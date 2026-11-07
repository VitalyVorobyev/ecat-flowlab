import {
  Button,
  DensityProvider,
  Disclosure,
  ErrorBox,
  Select,
  SkeletonRows,
} from "@vitavision/ui";
import { Download, Pin, PinOff, Rows3, Undo2, Upload } from "lucide-react";
import { useState } from "react";

import { api, type CaptureInfo, errorMessage, type SimSpec } from "../api";
import { FilePickButton } from "../components/FilePickButton";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { downloadJson } from "../lib/download";
import { ns } from "../lib/format";
import { ScenarioResult } from "../scenario/ScenarioResult";
import { SetupEditor } from "../scenario/SetupEditor";
import { changeText, diffSpecs, transferLine } from "../scenario/spec";
import type { Scenario as ScenarioState } from "../scenario/useScenario";
import { WhatIfPanel } from "../scenario/WhatIfPanel";

const MAX_LISTED_CHANGES = 6;

function BaselineBar({ scenario }: { scenario: ScenarioState }) {
  const { baseline, spec } = scenario;
  if (!baseline || !spec) return null;
  const changes = diffSpecs(baseline.spec, spec).filter((c) => c.path !== "/description");
  const done = baseline.result.transfers.every((t) => t.verified)
    ? Math.max(...baseline.result.transfers.map((t) => t.end_ns ?? 0))
    : null;
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-line bg-surface px-3 py-2 text-sm">
      <Pin className="size-3.5 text-fg-muted" />
      <span className="text-fg-muted">
        Baseline: all done after{" "}
        <span className="font-mono text-fg">{done === null ? "–" : ns(done)}</span>
      </span>
      <span className="min-w-0 flex-1 text-xs text-fg-muted">
        {changes.length === 0
          ? "no changes since"
          : `changed: ${changes
              .slice(0, MAX_LISTED_CHANGES)
              .map((c) => changeText(c, spec))
              .join("; ")}${
              changes.length > MAX_LISTED_CHANGES
                ? `; and ${changes.length - MAX_LISTED_CHANGES} more`
                : ""
            }`}
      </span>
      <Button
        size="sm"
        variant="ghost"
        disabled={changes.length === 0}
        onClick={() => scenario.setSpec(baseline.spec)}
      >
        Back to baseline
      </Button>
      <Button
        size="sm"
        variant="ghost"
        icon={<PinOff className="size-3.5" />}
        onClick={scenario.clearBaseline}
      >
        Unpin
      </Button>
    </div>
  );
}

/**
 * Design a segment and its transfers, see what the model predicts after every edit, and which
 * single change would help most.
 */
export function Scenario({
  scenario,
  onCreated,
}: {
  scenario: ScenarioState;
  onCreated: (info: CaptureInfo) => void;
}) {
  const [selected, setSelected] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);
  const { spec, result } = scenario;

  if (!spec) {
    return scenario.error ? (
      <ErrorBox className="m-6">{scenario.error}</ErrorBox>
    ) : (
      <SkeletonRows rows={12} className="m-6" />
    );
  }

  const transfer = selected !== null && selected < spec.transfers.length ? selected : null;
  const transferSpec = transfer === null ? undefined : spec.transfers[transfer];

  const openFrames = async () => {
    setBusy(true);
    try {
      onCreated(await api.simulate({ ...spec, capture: { enabled: true, max_frames: 200_000 } }));
    } catch (e) {
      setFileError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const loadFile = (file: File) => {
    file.text().then(
      (text) => {
        try {
          scenario.setSpec(JSON.parse(text) as SimSpec);
          setFileError(null);
        } catch (e) {
          setFileError(`${file.name}: ${errorMessage(e)}`);
        }
      },
      (e: unknown) => setFileError(errorMessage(e)),
    );
  };

  const presetValue =
    scenario.presets.find((p) => JSON.stringify(p.spec) === JSON.stringify(spec))?.id ?? "";

  return (
    <div className="flex min-h-full flex-col lg:h-full lg:flex-row">
      <aside className="flex shrink-0 flex-col border-line bg-surface lg:w-[25rem] lg:overflow-y-auto lg:border-r xl:w-[27rem]">
        <div className="sticky top-0 z-10 flex flex-col gap-2 border-b border-line bg-surface px-4 py-3">
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-semibold text-fg">Setup</h2>
            <div className="ml-auto flex items-center gap-1">
              <Button
                size="sm"
                variant="ghost"
                icon={<Undo2 className="size-3.5" />}
                disabled={!scenario.canUndo}
                onClick={scenario.undo}
              >
                Undo
              </Button>
              <FilePickButton
                accept=".json,application/json"
                variant="ghost"
                size="sm"
                icon={<Upload className="size-3.5" />}
                onFile={loadFile}
              >
                Open
              </FilePickButton>
              <Button
                size="sm"
                variant="ghost"
                icon={<Download className="size-3.5" />}
                onClick={() => downloadJson("scenario.json", spec)}
              >
                Save
              </Button>
            </div>
          </div>
          <Select
            aria-label="Start from a preset"
            value={presetValue}
            placeholder="Start from a preset…"
            options={scenario.presets.map((p) => ({ value: p.id, label: p.title }))}
            onValueChange={(v) => {
              const p = scenario.presets.find((x) => x.id === v);
              if (p) scenario.load(p);
            }}
          />
        </div>
        <DensityProvider value="compact">
          <SetupEditor spec={spec} onChange={scenario.setSpec} />
        </DensityProvider>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col gap-4 p-4 lg:overflow-y-auto xl:p-5">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-base font-semibold text-fg">Result</h2>
          <ProvenanceBadge provenance="simulated" />
          <span className="text-xs text-fg-muted">
            {scenario.running ? "simulating…" : "a model prediction, not a measurement"}
          </span>
          <div className="ml-auto flex items-center gap-1">
            <Button
              size="sm"
              variant="ghost"
              icon={<Pin className="size-3.5" />}
              disabled={!result || scenario.running}
              onClick={scenario.pinBaseline}
              title="Keep this result to compare the next edits with"
            >
              Pin as baseline
            </Button>
            <Button
              size="sm"
              variant="ghost"
              icon={<Rows3 className="size-3.5" />}
              loading={busy}
              disabled={!result}
              onClick={() => void openFrames()}
              title="Simulate again with a capture and open its frames in the analysis views"
            >
              Open frames
            </Button>
            <Button
              size="sm"
              variant="ghost"
              icon={<Download className="size-3.5" />}
              disabled={!result}
              onClick={() => result && downloadJson("scenario.result.json", result)}
            >
              Result
            </Button>
          </div>
        </div>
        {scenario.error ? (
          <ErrorBox>
            {scenario.error}
            {result ? " — the result below is of the last setup that ran." : ""}
          </ErrorBox>
        ) : null}
        {fileError ? <ErrorBox>{fileError}</ErrorBox> : null}
        <BaselineBar scenario={scenario} />
        {result ? (
          <div className={scenario.running || scenario.error ? "opacity-60" : undefined}>
            <ScenarioResult
              result={result}
              baseline={scenario.baseline?.result ?? null}
              selected={transfer}
              onSelect={setSelected}
            >
              <WhatIfPanel
                result={scenario.error ? null : scenario.whatIf}
                running={scenario.whatIfRunning}
                error={scenario.whatIfError}
                transfer={transfer}
                transferLabel={
                  transferSpec ? `transfer ${transfer}: ${transferLine(transferSpec, spec)}` : null
                }
                onApply={scenario.apply}
              />
            </ScenarioResult>
          </div>
        ) : scenario.error ? null : (
          <SkeletonRows rows={8} />
        )}
        {result ? (
          <Disclosure summary="Model assumptions" count={result.meta.assumptions.length}>
            <ul className="ml-4 list-disc space-y-1 text-sm text-fg-muted">
              {result.meta.assumptions.map((a) => (
                <li key={a}>{a}</li>
              ))}
            </ul>
          </Disclosure>
        ) : null}
        <p className="text-xs text-fg-muted">
          Every number follows from the setup on the left: link, cycle, mailbox policy, ESC memory,
          PDI rate and overhead, application time. Calibrate the PDI and the policy with lab
          measurements (Study, and the book's lab plan) before trusting absolute times.
        </p>
      </main>
    </div>
  );
}
