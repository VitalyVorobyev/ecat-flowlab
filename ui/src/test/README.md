`report.fixture.json` is real `flowlab analyze` output for a small synthetic capture.
Regenerate it after a report schema change:

```bash
python3 -c "import json; s=json.load(open('configs/synth/demo.json')); s['cycles']=400; s['faults']={'lost_frame_probability':0.01,'wkc_error_probability':0.01}; json.dump(s,open('results/fixture-spec.json','w'))"
cargo run -p flowlab-cli -- synth results/fixture-spec.json -o results/fixture.pcapng
cargo run -p flowlab-cli -- analyze results/fixture.pcapng --out ui/src/test/report.fixture.json
```
