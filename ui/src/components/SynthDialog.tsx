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

/** Configure and run the discrete-event simulator; its frames load as a capture. */
export function SimulateDialog(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (info: CaptureInfo) => void;
}) {
  return (
    <SpecDialog
      {...props}
      title="Simulate transfers"
      description="A model of master, link, ESC memory and PDI. Every number in the result follows from these parameters; the frames it writes are analysed like a capture and labelled Simulated."
      action="Simulate"
      loadSchema={api.simSchema}
      loadExample={api.simExample}
      run={api.simulate}
    />
  );
}
