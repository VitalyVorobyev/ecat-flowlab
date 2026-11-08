Calibration examples: studies with measured targets, run against a capture of the real network
([Investigating your own network](https://vitalyvorobyev.github.io/ecat-flowlab/book/guide/own-network.html#fit-the-model-to-a-capture)).

| Spec | What it fits |
|---|---|
| `foe-span.json` | PDI rate, per-access overhead and application time of slave 2, against the message spans of the first FoE write and the first SDO transfer in the capture |

```bash
flowlab study configs/calibration/foe-span.json --capture lab.pcapng --eni lab.eni.xml --result fit.json
```

Before running it on your own capture, edit `base` to match your network: the line of slaves and
their mailbox sizes (from `flowlab master scan` or the ENI), the cycle time, the master's policy
(`flowlab policy`), and the payload of each transfer (the sizes the capture shows in the Mailbox
view). Then choose parameter values around what you expect. A target compares a span both sides
observe: the simulator's `exchange_time_s` with the capture's FoE or SDO duration.

The values in `base` are placeholders, not measurements. A good fit shows that the model can
reproduce the capture with those parameters, not that they are the physical ones: read the
residuals, and check whether other runs fit as well.
