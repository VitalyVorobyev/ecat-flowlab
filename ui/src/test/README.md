`report.fixture.json` is real `flowlab analyze` output for a small synthetic capture.
Regenerate it after a report schema change:

```bash
python3 -c "import json; s=json.load(open('configs/synth/demo.json')); s['cycles']=400; s['faults']={'lost_frame_probability':0.01,'wkc_error_probability':0.01}; json.dump(s,open('results/fixture-spec.json','w'))"
cargo run -p flowlab-cli -- synth results/fixture-spec.json -o results/fixture.pcapng
cargo run -p flowlab-cli -- analyze results/fixture.pcapng --out ui/src/test/report.fixture.json
```

`sim.fixture.json` is real `flowlab simulate` output (the example spec with 8 kB per transfer):

```bash
python3 -c "import json; s=json.load(open('configs/sim/example.json')); [t.update(payload_bytes=8000) for t in s['transfers']]; s['capture']={'enabled':False,'max_frames':0}; json.dump(s,open('results/sim-fixture-spec.json','w'))"
cargo run -p flowlab-cli -- simulate results/sim-fixture-spec.json --result ui/src/test/sim.fixture.json
```
