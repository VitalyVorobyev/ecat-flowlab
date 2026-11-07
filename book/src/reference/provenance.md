# Provenance and assumptions

## Provenance

Every capture FlowLab loads, and every report and result it produces, carries one of three
labels:

| Label | Source | Written by |
|---|---|---|
| `measured` | a capture file from a real network | — (any file without a FlowLab marker) |
| `simulated` | the discrete-event simulator | `flowlab simulate`, Simulate… |
| `synthetic` | the capture generator, with ground truth | `flowlab synth`, Synthesize… |

FlowLab writes the label into the pcapng section comment of every capture it generates, and
reads it back when the file is loaded. A file without the marker is taken as measured. The
label appears in the report's `meta.source`, on every capture badge in the UI and in the CLI
summary (`[SYNTHETIC]`, `[SIMULATED]`).

Results of different provenance are never combined into one number or one chart series.
Compare shows two captures in separate, labelled columns, and a study fitted to a measurement
names the capture it was fitted to.

## Assumptions

Every report carries `meta.assumptions`: a list of sentences stating what the result rests on.
In the UI the list is at the bottom of the Overview, Mailbox, Sim run and Study views.
Examples:

- how directions were inferred (MAC bit, zero working counters, or not at all);
- that expected working counters are the most frequent value, or come from an ENI;
- where each mailbox channel's addresses came from;
- the link rate used for utilization;
- for a simulation: the model's simplifications and the policy used;
- for a calibration: that a good fit does not identify the physical parameters.

Every threshold behind a *warn* or *defect* finding is stated in the finding's text.

## Report metadata

`meta` also records the schema version, the tool version, and the input's name, size and
SHA-256 (or the spec's identity for generated data). Two reports with the same input hash and
tool version come from the same bytes.
