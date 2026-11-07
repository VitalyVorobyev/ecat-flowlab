import { SchemaValueForm, type JsonSchema } from "@vitavision/forms";
import { Button, Callout, Dialog, ErrorBox, SkeletonRows } from "@vitavision/ui";
import { useEffect, useState } from "react";

import { type CaptureInfo, errorMessage } from "../api";

/**
 * Edit a spec rendered from its JSON Schema, then run it on the server. The schema and the
 * example come from the server, so the form always matches the Rust types.
 */
export function SpecDialog({
  open,
  onOpenChange,
  onCreated,
  title,
  description,
  action,
  loadSchema,
  loadExample,
  run,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (info: CaptureInfo) => void;
  title: string;
  description: string;
  action: string;
  loadSchema: () => Promise<Record<string, unknown>>;
  loadExample: () => Promise<Record<string, unknown>>;
  run: (spec: unknown) => Promise<CaptureInfo>;
}) {
  const [schema, setSchema] = useState<JsonSchema | null>(null);
  const [value, setValue] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open || schema) return;
    let live = true;
    Promise.all([loadSchema(), loadExample()]).then(
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
  }, [open, schema, loadSchema, loadExample]);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const info = await run(value);
      onCreated(info);
      onOpenChange(false);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const reset = () => {
    void loadExample().then(setValue, (e: unknown) => setError(errorMessage(e)));
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      className="w-[min(44rem,calc(100vw-2rem))]"
      description={description}
      footer={
        <>
          <Button variant="ghost" onClick={reset} disabled={busy || !schema}>
            Reset to example
          </Button>
          <Button variant="primary" onClick={() => void submit()} loading={busy} disabled={!schema}>
            {action}
          </Button>
        </>
      }
    >
      {error ? <ErrorBox className="mb-3">{error}</ErrorBox> : null}
      {schema ? (
        <SchemaValueForm schema={schema} value={value} onValueChange={setValue} />
      ) : error ? (
        <Callout tone="warning">The settings could not be loaded.</Callout>
      ) : (
        <SkeletonRows rows={6} />
      )}
    </Dialog>
  );
}
