import { api, type CaptureInfo } from "../api";
import { SpecDialog } from "./SpecDialog";

/** Configure and generate a synthetic capture with known ground truth. */
export function SynthDialog(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (info: CaptureInfo) => void;
}) {
  return (
    <SpecDialog
      {...props}
      title="Generate a synthetic capture"
      description="A master and a line of slaves, written as a capture at the master's NIC. Every value is chosen here, so the analysis can be checked against it. The result is labelled Synthetic."
      action="Generate"
      loadSchema={api.synthSchema}
      loadExample={api.synthExample}
      run={api.synth}
    />
  );
}
