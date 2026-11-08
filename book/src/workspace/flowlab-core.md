# flowlab-core

`flowlab-core` is the shared vocabulary of the workspace: where a result came from
(`Provenance`), the metadata every report carries (`ReportMeta`, `InputRef`), unit constants,
the one percentile definition (`stats::quantile`) and the PDI timing samples file
(`pdi_samples`), hex and SHA-256, and the deterministic generator (`rng`). It is deliberately
small so that every other crate can depend on it.

API reference: [`flowlab_core`](../../api/flowlab_core/index.html) ·
source: [`crates/flowlab-core`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-core)

## Position in the pipeline

| | |
|---|---|
| Depends on | `serde`, `ts-rs` (TypeScript export), `sha2` |
| Used by | `flowlab-capture`, `flowlab-analysis`, `flowlab-synth`, `flowlab-sim`, `flowlab-study`, `flowlab-live`, `flowlab-pdi`, `flowlab-master`, `flowlab-server`, `flowlab-cli` |

It must not depend on anything heavier than serialization and hashing.

## Types

| Item | Meaning |
|---|---|
| `Provenance` | `Measured`, `Simulated` or `Synthetic`; serialized in snake case |
| `Provenance::COMMENT_PREFIX` | `"flowlab:provenance="`, the marker FlowLab writers put into the pcapng section comment |
| `Provenance::as_str()` / `from_comment()` | the lower-case name, and parsing the marker out of free text |
| `InputRef` | file name (no directories), size in bytes, SHA-256 of the contents (lower-case hex); `InputRef::of(name, bytes)` computes the last two |
| `hex(bytes)`, `sha256_hex(bytes)` | lower-case hex, and the SHA-256 in it |
| `rng::SplitMix64`, `rng::mix64` | the deterministic generator of the simulator, the studies and the PDI benchmark (`next_u64`, `unit`, `chance`, `normal`, `below`), and its mix function, which the generated payload uses too; implemented here so a seed gives the same numbers across dependency upgrades |
| `ReportMeta` | `schema_version`, `tool` (`"ecat-flowlab"`), `tool_version`, `source: Provenance`, `input: InputRef`, `assumptions: Vec<String>` |
| `ReportMeta::new(source, input, assumptions)` | fills in the schema and tool version |
| `SCHEMA_VERSION` | version of the JSON report schemas (currently `1`) |

`Provenance`, `InputRef` and `ReportMeta` derive `ts_rs::TS`; `cargo test` exports them to
`ui/src/api/gen/` together with the other API types.

## Provenance rules

- `Measured` is the default for a file that does not declare otherwise. FlowLab cannot verify
  that a capture came from a real network; it only notices the absence of a marker.
- `Simulated` is written by the discrete-event simulator, `Synthetic` by `flowlab synth`.
- `from_comment` looks for the first `flowlab:provenance=` in a comment and reads the
  alphabetic word after it, so the marker can sit among other `key=value` text:

```text
flowlab:provenance=synthetic; generator=flowlab-synth 0.1.0; seed=42; Demo line ...
```

`flowlab-capture` applies it to the comments of a pcapng file when it builds a `Trace`; see
[Provenance and assumptions](../reference/provenance.md) for how results carry the label.

## Units

Units are carried in field names (`_ns`, `_bytes`, `_bps`) rather than newtypes, so the JSON
reports stay self-describing for tools outside Rust. The `units` module holds the constants:

| Constant | Value |
|---|---|
| `units::MB` | 10^6 bytes (decimal megabyte) |
| `units::MIB` | 2^20 bytes (binary mebibyte) |
| `units::NS_PER_S` | 10^9 |
| `units::NS_PER_US` | 10^3 |
| `units::FAST_ETHERNET_BPS` | 100 000 000 bit/s (100BASE-TX, the default link rate) |

`units::serialization_ns(bytes, bps)` is the time to serialize `bytes` at `bps`, in ns,
rounded up. Its unit test checks that 1538 wire bytes (a maximum-size frame plus overhead)
take 123 040 ns at 100 Mbit/s, and that 10 MB (decimal) take 800 000 000 ns.

## Percentiles and PDI samples

`stats::quantile(sorted, q)` interpolates linearly between the two closest ranks, at position
*q* · (*n* − 1); every percentile in a FlowLab result comes from it.

`pdi_samples` is the samples file `flowlab pdi-bench` writes and `flowlab pdi-fit` reads
([PDI timing samples](../reference/specs.md#pdi-timing-samples)): `Direction`, `Sample`
(`duration_ns` may have a fraction), `to_csv`, `parse` (errors name the line) and `provenance`
(the `flowlab:provenance=` marker, else measured).

## Tests

`provenance_marker_round_trip` writes each provenance into a comment with surrounding text and
reads it back, and checks that a comment without the marker gives `None`.

## Example

```rust,ignore
use flowlab_core::{InputRef, Provenance, ReportMeta};

let comment = format!("{}{}; seed=1", Provenance::COMMENT_PREFIX, Provenance::Simulated.as_str());
assert_eq!(Provenance::from_comment(&comment), Some(Provenance::Simulated));

let input = InputRef { name: "run.pcapng".into(), bytes: 0, sha256: String::new() };
let meta = ReportMeta::new(Provenance::Simulated, input, vec!["link rate 100 Mbit/s".into()]);
assert_eq!(meta.tool, "ecat-flowlab");
```
