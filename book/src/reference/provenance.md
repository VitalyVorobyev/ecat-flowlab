# Provenance and assumptions

## Provenance

Every capture FlowLab loads, and every report and result it produces, carries one of three
labels:

| Label | Source | Written by |
|---|---|---|
| `measured` | a capture file from a real network; the lab master's results; PDI timing samples from hardware and their fit | any capture file without a FlowLab marker; `flowlab capture`; `flowlab master`; `flowlab pdi-bench --device`, `pdi-fit` |
| `simulated` | the discrete-event simulator, and everything run on it | `flowlab simulate`, `what-if`, `study`, Simulate; `pdi-bench --sim` |
| `synthetic` | the capture generator, with ground truth | `flowlab synth`, Generate |

FlowLab writes the label into the pcapng section comment of every capture it generates, and
reads it back when the file is loaded. A file without the marker is taken as measured. PDI
timing samples carry it in a comment line (`# flowlab:provenance=simulated` from
`pdi-bench --sim`). Lab inputs take only measured data. The label appears in the report's
`meta.source`, on every capture badge in the UI and in the CLI summary (`[MEASURED]`,
`[SYNTHETIC]`, `[SIMULATED]`).

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
