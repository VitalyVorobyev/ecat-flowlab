import { SchemaValueForm, type JsonSchema } from "@vitavision/forms";
import { Button, Callout, Dialog, ErrorBox, SkeletonRows } from "@vitavision/ui";
import { useEffect, useState } from "react";

import { api, type CaptureInfo, errorMessage } from "../api";

/** Configure and generate a synthetic capture with known ground truth. */
export function SynthDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (info: CaptureInfo) => void;
}) {
  const [schema, setSchema] = useState<JsonSchema | null>(null);
  const [value, setValue] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open || schema) return;
    let live = true;
    Promise.all([api.synthSchema(), api.synthExample()]).then(
      ([s, example]) => {
        if (!live) return;
        setSchema(s);
        setValue(example);
      },
      (e: unknown) => live && setError(errorMessage(e)),
    );
    return () => {
      live = false;
    };
  }, [open, schema]);

  const generate = async () => {
    setBusy(true);
    setError(null);
    try {
      const info = await api.synth(value);
      onCreated(info);
      onOpenChange(false);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const reset = () => {
    void api.synthExample().then(setValue, (e: unknown) => setError(errorMessage(e)));
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title="Generate a synthetic capture"
      className="w-[min(44rem,calc(100vw-2rem))]"
      description="A master and a line of slaves, written as a capture at the master's NIC. Every value is chosen here, so the analysis can be checked against it. The result is labelled Synthetic."
      footer={
        <>
          <Button variant="ghost" onClick={reset} disabled={busy || !schema}>
            Reset to demo
          </Button>
          <Button
            variant="primary"
            onClick={() => void generate()}
            loading={busy}
            disabled={!schema}
          >
            Generate
          </Button>
        </>
      }
    >
      {error ? <ErrorBox className="mb-3">{error}</ErrorBox> : null}
      {schema ? (
        <SchemaValueForm schema={schema} value={value} onValueChange={setValue} />
      ) : error ? (
        <Callout tone="warning">The generator settings could not be loaded.</Callout>
      ) : (
        <SkeletonRows rows={6} />
      )}
    </Dialog>
  );
}
