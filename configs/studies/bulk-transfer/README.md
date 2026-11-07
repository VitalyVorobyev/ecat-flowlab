Simulator and study specs for the
[bulk-transfer study](https://vitalyvorobyev.github.io/ecat-flowlab/studies/bulk-transfer/index.html)
(`book/src/studies/bulk-transfer/`). Results are simulated, never measured.

| Spec | What it runs |
|---|---|
| `smoke.json` | 65,536 B master → slave by EoE, FoE, CoE and a 256 B/cycle PDO channel at 1 ms |
| `large-payload.json` | 10,000,000 B master → slave by FoE, then EoE, at 500 µs |
| `mixed-load.json` | 10,000,000 B slave → master by EoE next to 768 B of cyclic process data at 250 µs |
| `foe-sweep.json` | A study: mailbox size × PDI rate × policy for a 100 kB FoE write; Pareto frontier of time vs link load |
| `report/` | The report's sweeps and steps, written by `scripts/bulk-transfer-report.py` — do not edit by hand |

```bash
flowlab simulate configs/studies/bulk-transfer/{smoke,large-payload,mixed-load}.json
flowlab study configs/studies/bulk-transfer/foe-sweep.json
```

Every parameter, above all the PDI rate and per-access overhead, is an assumption until it is
calibrated against a capture of the real system.
