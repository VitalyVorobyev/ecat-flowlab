# flowlab-devices

`flowlab-devices` reads what devices can do and how a network is configured, and checks whether a
plan fits. It holds subset readers for ESI (EtherCAT Slave Information, ETG.2000) and ENI
(EtherCAT Network Information, ETG.2100) files, an ENI writer, and a bus-budget planner that lays
out the cyclic frames of a network with arithmetic only.

API: [`flowlab_devices`](../../api/flowlab_devices/index.html) ·
source: [crates/flowlab-devices](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-devices)

## Position in the pipeline

| | |
|---|---|
| Depends on | `ecat-wire` (`wire_bytes`, command address modes), `roxmltree`, `serde`, `schemars`, `ts-rs` |
| Used by | `flowlab-analysis` (`AnalysisOptions::network` holds a `NetworkConfig`), `flowlab-synth` (`network_config` builds the ENI of a synthetic network), `flowlab-server` (`/api/esi`, `/api/plan`, `/api/captures/{id}/eni`), `flowlab-cli` (`esi`, `plan`, `--eni`), `flowlab-master` (`scan --eni` writes the scanned network as ENI) |

It does not depend on capture or analysis code.

## Main types and functions

| Item | What it is |
|---|---|
| `parse_esi(xml) -> Result<EsiFile, DeviceError>` | Read an `<EtherCATInfo>` document |
| `EsiFile` | `vendor_id`, `vendor_name`, `devices` |
| `DeviceDescription` | Identity (vendor, product code, revision, type name, name), `sync_managers`, `rx_pdos`, `tx_pdos`, `mailbox`, `dc`, `default_outputs_bits`, `default_inputs_bits`; `sm_size(kind)` |
| `SmDescription` | SyncManager `kind` (`MBoxOut`, `MBoxIn`, `Outputs`, `Inputs` …), start, default/min/max size, control byte, enable |
| `PdoDescription`, `PdoEntry` | PDO index, name, default `sm`, `fixed`, `mandatory`, entries, `bit_len` |
| `MailboxSupport` | `coe`, `foe`, `eoe`, `soe`, `voe`, `aoe`, `coe_sdo_info`, `coe_complete_access`; `supports(name)`, `names()` |
| `parse_eni(xml) -> Result<NetworkConfig, DeviceError>` | Read an `<EtherCATConfig>` document |
| `write_eni(&NetworkConfig) -> String` | Write the same subset; `parse_eni` reads it back |
| `NetworkConfig` | `master`, `cycle_time_ns`, `slaves: Vec<SlaveConfig>`, `cyclic: Vec<CyclicFrame>`; `expected_wkc(command, address, length)`, `slave(station)` |
| `SlaveConfig` | Name, station, ring position, identity, `outputs_bits`, `inputs_bits`, `mailbox: Option<MailboxConfig>` |
| `MailboxConfig` | `out_start`, `out_len`, `in_start`, `in_len`, `protocols` |
| `CyclicFrame`, `CyclicCmd` | A cyclic frame's datagrams: `command`, raw 32-bit `address`, `length`, `expected_wkc` |
| `plan(&PlanSpec) -> PlanResult` | Frame layout, wire time, idle time, mailbox headroom, checks |
| `PlanSpec`, `PlanSlave` | Input of the planner; `PlanSlave::from_device` builds an entry from an ESI device |
| `PlanResult`, `PlannedFrame`, `PlannedDatagram`, `PlanCheck` | Output of the planner |
| `MAX_DATAGRAM_DATA` | 1486: 1500 − 2 (EtherCAT header) − 12 (datagram header + WKC) |
| `plan_schema()` | JSON Schema (draft 2020-12) of `PlanSpec` |
| `DeviceError` | `Xml` (not well-formed) or `Format` (unexpected structure) |

## ESI subset

From `<Vendor>` the id and name; from each `<Descriptions><Devices><Device>`: `<Type>` with its
`ProductCode` and `RevisionNo` attributes, the first `<Name>`, every `<Sm>` (in file order, which
gives its index), `<RxPdo>` and `<TxPdo>` with their entries, the protocols listed under
`<Mailbox>` (presence of `<CoE>`, `<FoE>` …, plus CoE's `SdoInfo` and `CompleteAccess`
attributes), and whether `<Dc>` is present. The default process-data size is the sum of the PDOs
that carry an `Sm` attribute (padding entries included). Numbers may be decimal, `#x` hex or `0x`
hex. Unknown elements are ignored, so vendor extensions do not break import.

## ENI subset and `expected_wkc`

From `<Config>`: the master's name; each `<Slave>`'s `<Info>` (`PhysAddr` is required,
`AutoIncAddr` gives the ring position as its two's-complement negation), the bit lengths under
`<ProcessData><Send>`/`<Recv>`, and the `<Mailbox>` `Send`/`Recv` start and length with its
`<Protocol>` list; from each `<Cyclic>` the `CycleTime` (µs in the file, stored in ns) and every
`<Frame><Cmd>` with `Cmd`, `Addr` (or `Adp` | `Ado` << 16), `DataLength` (or the length of
`<Data>` hex text) and `Cnt`.

`NetworkConfig::expected_wkc(command, address, length)` returns the configured `Cnt` of the
first cyclic datagram with exactly that command code, raw address and length. The analysis uses
it to judge working counters before falling back to the most frequent value; each WKC slot
reports which basis it used (see [Network configuration and planning](../guide/devices.md)).

`write_eni` emits a minimal document: master name, slaves with identity and process-data bit
lengths, mailboxes, the cycle time and the cyclic commands (`Addr` for logical commands,
`Adp`/`Ado` otherwise). `flowlab synth --eni` uses it to write the configuration of a synthetic
network.

## The planner

`plan()` assumes the layout most masters use and does no simulation:

1. The first frame holds a DC system-time FRMW (8 bytes, when `distributed_clocks`), the first
   LRW part of the process image, and a BRD of AL status (2 bytes).
2. The rest of the image goes into further frames, one LRW of at most `MAX_DATAGRAM_DATA` bytes
   each.
3. Frames go back to back at cycle start; each occupies the link for its wire time
   (`ecat_wire::wire_bytes`: padding to 60 bytes, preamble, FCS and inter-frame gap). The last
   frame returns after its wire time plus the ring delay (sum of `forwarding_delay_ns`).

`PlanResult` reports `cycle_ns`, `link_bps`, `image_out_bytes`, `image_in_bytes`, the `frames`,
`cyclic_wire_ns`, `cyclic_percent`, `ring_delay_ns`, `cyclic_done_ns`, `idle_ns`, `idle_percent`
and, for the slave with the largest mailboxes, `mailbox_frame_ns` (one write of "mailbox out" and
one read of "mailbox in" in one frame), `mailbox_frames_per_cycle` (such frames that fit the idle
time when the master waits for each to return) and `mailbox_bytes_per_s` (an upper bound per
direction for that slave, one message per frame). `assumptions` lists these premises; slave-side
PDI time is left to the simulator.

`PlanCheck` has a `severity`, a `subject` and a `message` that states its threshold:

| Severity | Subject | When |
|---|---|---|
| `defect` | `cycle` | The cycle time is 0, or the cyclic frames plus the ring take longer than the cycle |
| `warn` | `cycle` | Idle share below `min_idle_percent` (default 25 %) |
| `info` | `process image` | The image needs more than one frame |
| `defect` | slave name | ESC RAM: `mailbox_out + mailbox_in + 3 × (outputs + inputs)` > `dpram_bytes` |
| `defect` | slave name | A mailbox larger than `max_mailbox_bytes` (ESI `MaxSize`) |
| `defect` | slave name | A required protocol with no mailbox configured, or not declared in `supports` |
| `info` | slave name | A required protocol whose support is unknown (`supports` absent) |

`PlanSpec` defaults: `link_bps` 100,000,000, `distributed_clocks` true, `min_idle_percent` 25;
`PlanSlave` defaults `forwarding_delay_ns` 700 and `dpram_bytes` 4096.

## Invariants and boundaries

- XML is parsed with `roxmltree` and `allow_dtd: false`: a document with a DTD is an error,
  which also rules out entity expansion.
- Malformed input is an `Err`, never a panic.
- No vendor ESI or ENI files are in the repository. The fixtures in
  `crates/flowlab-devices/tests/fixtures/` (`flowlab-demo.esi.xml`, `flowlab-demo.eni.xml`) are
  hand-written in ETG layout with vendor id 0xFFF.
- The planner is arithmetic; PDI and slave timing belong to `flowlab-sim`.

## Tests

`crates/flowlab-devices/src/tests.rs`: the ESI fixture's identity, SyncManagers, PDO default
assignment and mailbox protocols (`esi_reads_identity_sync_managers_pdos_and_mailbox`); the ENI
fixture's slaves, mailboxes and `expected_wkc`, including a length taken from `<Data>`
(`eni_reads_slaves_mailboxes_and_expected_wkc`); `parse_eni(write_eni(n)) == n`
(`eni_round_trips_through_the_writer`); malformed XML and a DTD rejected
(`malformed_files_are_errors_not_panics`); the example plan against hand arithmetic
(`plan_layout_matches_hand_arithmetic`); cycle, ESC RAM and capability checks; multi-frame images;
the schema draft.

## Example

```rust,ignore
use flowlab_devices::{PlanSpec, plan};

let result = plan(&PlanSpec::example());
assert!(result.cyclic_wire_ns > 0.0 && result.idle_ns < result.cycle_ns);
for check in &result.checks {
    println!("[{}] {}: {}", check.severity, check.subject, check.message);
}
```

The built-in example from the command line:

```bash
cargo run -q --release -p flowlab-cli -- plan
```

```text
cycle 1.000 ms  link 100 Mbit/s  process image 34 B out / 42 B in
  frame 0: 138 B, 12.96 µs on the wire  [FRMW 8 B, LRW 76 B, BRD 2 B]
  cyclic frames back after 17.16 µs (1.3 % of the cycle on the wire, ring 4.20 µs); idle 982.84 µs (98.3 %)
  mailbox: 87.04 µs per frame, 10 frames fit per cycle, ≤ 5060 kB/s per direction
  [INFO] Servo-1: CoE support is unknown (no ESI).
  ...
```
