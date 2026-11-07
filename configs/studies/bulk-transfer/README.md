Simulator specs (`SimSpec`, see `flowlab schema sim`) for the
[bulk-transfer study](../../../docs/studies/bulk-transfer/README.md). Run them side by side:

```bash
cargo run --release -p flowlab-cli -- simulate configs/studies/bulk-transfer/*.json
```

| Spec | What it compares |
|---|---|
| `smoke.json` | 65,536 B master → slave by EoE, FoE, CoE and a 256 B/cycle PDO channel at 1 ms |
| `large-payload.json` | 10,000,000 B master → slave by FoE, then EoE, at 500 µs (no capture written) |
| `mixed-load.json` | 10,000,000 B slave → master by EoE next to 768 B of cyclic process data at 250 µs |

They replace the starter design's scenario files (kept verbatim in
`docs/studies/bulk-transfer/archive/configs/`). Changes in the migration: the abstract
`synthetic_mailbox` transport became explicit protocols; `mode` was dropped (provenance is a
property of results, not inputs); `mailbox_services_per_cycle: 0.5` had no defined meaning and is
modelled as a round-robin policy; the ESC now has RAM and a PDI. Every parameter — especially the
PDI rate and overhead, which dominate several results — is an assumption until calibrated
against a lab capture. Results are simulated, not measured.
