# HTTP API

`flowlab serve` runs a local HTTP server that keeps captures in memory, analyses them, answers
the workbench's queries and serves the built UI. Everything the UI shows comes through this API,
so it can also be scripted with `curl`. The server's internals are described in
[flowlab-server and flowlab-cli](../workspace/server-cli.md).

## Starting the server

```bash
flowlab serve [CAPTURES]... [--port 7878] [--host 127.0.0.1] [--ui-dir ui/dist] [--open] \
              [--mailbox STATION=OUT,IN]... [--eni XML]
```

Captures named on the command line are loaded at start. `--mailbox` hints and `--eni` apply to
every capture the server analyses (see [Input files](specs.md)). Without a built UI in
`--ui-dir`, only the API is served. The base URL is `http://127.0.0.1:7878`.

## Security

The server binds to `127.0.0.1` by default. It has **no authentication**, and
`POST /api/captures/open` reads any file the user running it can read. Keep it on loopback:
`flowlab serve` logs a warning when `--host` is not a loopback address. Do not expose it to a
network.

Every request whose `Host` header is neither an IP address nor `localhost` gets `403`. A web page
that points its own domain name at 127.0.0.1 (DNS rebinding) therefore cannot reach the API. Open
the UI as `http://127.0.0.1:7878` or `http://localhost:7878`, also through an SSH tunnel.

## Endpoints

From the module documentation of
[`crates/flowlab-server/src/lib.rs`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-server/src/lib.rs),
checked against its router:

| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/api/health` | | `{ "version": "…" }` |
| GET | `/api/captures` | | `CaptureInfo[]` |
| POST | `/api/captures?name=` | raw capture bytes (pcap or pcapng) | `CaptureInfo` |
| POST | `/api/captures/open` | JSON `{ "path": "…" }` | `CaptureInfo` |
| DELETE | `/api/captures/{id}` | | 204 No Content |
| GET | `/api/captures/{id}/report` | | `CaptureReport` |
| GET | `/api/captures/{id}/frames` | `FrameFilter` fields, `offset`, `limit` | `FramePage` |
| GET | `/api/captures/{id}/frames/{no}` | | `FrameDetail` |
| GET | `/api/captures/{id}/eoe.pcapng` | | Ethernet frames reassembled from EoE, as pcapng |
| GET | `/api/captures/{id}/simulation` | | `SimResult` of a simulated capture |
| POST | `/api/captures/{id}/eni` | ENI XML | `CaptureInfo`, re-analysed with the configuration |
| DELETE | `/api/captures/{id}/eni` | | `CaptureInfo`, re-analysed without |
| POST | `/api/esi` | ESI XML | `EsiImport` |
| GET | `/api/plan/schema` | | JSON Schema of `PlanSpec` |
| GET | `/api/plan/example` | | `PlanSpec` |
| POST | `/api/plan` | `PlanSpec` | `PlanResult` |
| GET | `/api/study/schema` | | JSON Schema of `StudySpec` |
| GET | `/api/study/example` | | `StudySpec` |
| POST | `/api/study?capture=` | `StudySpec` | `StudyResult`, calibrated against capture `capture` if given |
| GET | `/api/synth/schema` | | JSON Schema of `SynthSpec` |
| GET | `/api/synth/example` | | `SynthSpec` |
| POST | `/api/synth` | `SynthSpec` | `CaptureInfo` of the generated capture |
| GET | `/api/sim/schema` | | JSON Schema of `SimSpec` |
| GET | `/api/sim/example` | | `SimSpec` |
| POST | `/api/simulate` | `SimSpec` | `CaptureInfo` of the run's capture; the result via `/simulation` |
| GET | `/api/sim/presets` | | `SimPreset[]`: `{ id, title, spec }`, the scenarios the Simulate view starts from |
| POST | `/api/sim/run` | `SimSpec` | `SimResult`, without a capture or a capture entry |
| POST | `/api/sim/what-if` | `SimSpec` | `WhatIfResult`: the scenario with one change at a time, and pairs, best first |
| GET | `/api/live/interfaces` | | `LiveInterfaces`: `{ enabled, tool, interfaces[], error }` |
| POST | `/api/live/start` | `LiveStart`: `{ source, window_s }` | `LiveSnapshot`; 403 for an interface without `--live`, 409 while a session runs |
| GET | `/api/live` | | `LiveSnapshot` of the running or last session, or `null` |
| POST | `/api/live/stop` | | `LiveStopped`: `{ snapshot, capture }`, the saved capture loaded as a `CaptureInfo` |

`LiveStart.source` is `{ "kind": "interface", "interface": "eth1", "tool": null, "all_traffic": false }`
or `{ "kind": "replay", "capture": 3, "speed": 1 }` (a loaded capture, 0 < speed ≤ 1000). One live
session runs at a time.

The request and response types are described in [Input files](specs.md) and
[Report and result files](reports.md); their TypeScript definitions are generated into
`ui/src/api/gen/`. Ids count up from 1 and are not reused. Attaching or removing an ENI keeps the
capture's id. When the UI is served, any other path, including an unknown `/api/…` path, returns
the UI's `index.html`.

### CaptureInfo

| Field | Meaning |
|---|---|
| `id` | server-assigned id |
| `name` | file name (directories stripped); `synth-seed<seed>.pcapng` or `sim-seed<seed>.pcapng` for generated captures |
| `provenance` | `"measured"`, `"simulated"` or `"synthetic"` |
| `format` | `"pcap"` or `"pcapng"` |
| `bytes`, `frames`, `ethercat_frames` | file size, all frames, EtherCAT frames |
| `duration_ns` | first to last frame |
| `streams`, `defects`, `warnings` | cyclic streams; findings with severity `defect` and `warn` |
| `sha256` | SHA-256 of the file |
| `has_simulation` | a `SimResult` is attached (`/simulation`) |
| `eni` | master name of the attached ENI, else `null` |

## The frames query

`GET /api/captures/{id}/frames` lists frames in capture order. All given conditions must hold.

| Parameter | Type | Default | Meaning |
|---|---|---|---|
| `offset` | u64 | 0 | rows to skip among the matches |
| `limit` | u64 | 200 | rows to return; values above 5000 are reduced to 5000 |
| `command` | string | — | a datagram has this mnemonic, case-insensitive (`LRW`, `fprd`) |
| `direction` | string | — | `tx`, `rx` or `unknown` |
| `station` | u16 (decimal) | — | a configured-address datagram addresses this station |
| `stream` | u16 | — | the frame belongs to this cyclic stream; `65535` = in no stream |
| `problems_only` | bool | `false` | only frames with a WKC mismatch, no answer, or a parse error |
| `include_other` | bool | `false` | include non-EtherCAT frames |
| `mailbox` | string | — | `any`: a datagram accesses a mailbox; `coe`, `foe`, `eoe`, `soe`, `voe`, `aoe` or `error`: the frame carries a message of that protocol |

An empty string parameter (`command=`) means no condition; an empty or non-decimal numeric
parameter (`station=`, `station=0x1001`) is rejected with 400, and so is an unknown command,
direction or mailbox name (`{"error":"unknown mailbox filter `eeo`: …"}`).

The response is a `FramePage`: `total` (rows matching the filter), `offset`, and `rows`, each a
`FrameRow` with `no` (1-based frame number), `t_ns` (since capture start), `direction`, `kind`
(`ethercat`, `other`, `malformed`), `len`, `datagrams`, `summary`, `stream`, `wkc_mismatch`,
`unanswered`, `paired` (the partner frame number) and `rtt_ns` (on a paired TX frame).

`GET /api/captures/{id}/frames/{no}` returns a `FrameDetail`: the `row`, `orig_len`,
`wire_bytes`, Ethernet `dst`, `src`, `vlan`, `ethertype`, `encapsulation`, EtherCAT `ecat_length`
and `ecat_type`, the frame as `hex`, labelled byte `ranges` for the hex view, a parse `error` if
any, and `datagrams[]`. Each `DatagramDetail` has `position`, `command`, `command_code`,
`index`, `address` (mode, station, position, `ado`, logical address, register name and area),
`len`, flags, `wkc`, `expected_wkc`, `al_state` (for 0x0120/0x0130) and `mailbox` (role, channel
source, header, summary and decoded fields).

## Requests and limits

| Rule | Value |
|---|---|
| Body size limit on every API route (`MAX_UPLOAD_BYTES`) | `2 << 30` = 2,147,483,648 bytes (2 GiB) |
| JSON bodies | need `Content-Type: application/json`; unknown fields are rejected |
| XML bodies (ENI, ESI) | sent as is; must be UTF-8 |
| Upload name (`?name=`) | reduced to its file name; default `upload.pcapng` |
| `POST /api/simulate` | forces `capture.enabled` on and limits `capture.max_frames` to 500,000 (`MAX_SIM_FRAMES`) |
| Frames per page | default 200, at most 5000 |

Parsing and analysing captures, frame queries, the EoE export, ENI re-analysis, synthesis,
simulation and studies run on blocking threads (`spawn_blocking`), so a long request does not
stall the others.

## Errors

Errors raised by the handlers are JSON with an HTTP status:

```json
{ "error": "no capture 9" }
```

| Status | When |
|---|---|
| 400 | bad input: a file that is not pcap/pcapng, an unreadable path, XML that is not an ENI/ESI, a spec the generator, simulator or study rejects, an unknown frame filter name |
| 404 | unknown capture id or frame number; `/simulation` of a capture that is not a simulation run; any other path under `/api` (never the UI's `index.html`) |
| 500 | internal error |

Requests that axum rejects before a handler runs get a **plain-text** body instead:

| Status | When |
|---|---|
| 400 | query string that does not parse (`limit=abc`, `station=`); body that is not valid JSON |
| 415 | JSON endpoint called without `Content-Type: application/json` |
| 422 | JSON that does not match the type, including an unknown field |

## Example session

A synthetic capture, served, queried and stopped. All output below is from these commands,
trimmed.

```bash
flowlab synth configs/synth/demo.json -o demo.pcapng
flowlab serve demo.pcapng &
```

```text
INFO flowlab: loaded demo.pcapng (10859 frames)
INFO flowlab_server: listening on http://127.0.0.1:7878
```

List the loaded captures:

```bash
curl -s http://127.0.0.1:7878/api/captures
```

```json
[
  {
    "id": 1,
    "name": "demo.pcapng",
    "provenance": "synthetic",
    "format": "pcapng",
    "bytes": 1787928,
    "frames": 10859,
    "ethercat_frames": 10859,
    "duration_ns": 5034616434.0,
    "streams": 1,
    "defects": 2,
    "warnings": 3,
    "sha256": "95d8517efcb8543b965a4543dcd0a8c2fdb1594de799677253a80dae396f813c",
    "has_simulation": false,
    "eni": null
  }
]
```

The first two sent frames of cyclic stream 0, and the first frame with a problem:

```bash
curl -s "http://127.0.0.1:7878/api/captures/1/frames?stream=0&direction=tx&limit=2"
curl -s "http://127.0.0.1:7878/api/captures/1/frames?problems_only=true&limit=1"
```

```json
{
  "total": 4994,
  "offset": 0,
  "rows": [
    {
      "no": 351,
      "t_ns": 35600162.0,
      "direction": "tx",
      "kind": "ethercat",
      "len": 130,
      "datagrams": 3,
      "summary": "FRMW 0x1001:0910 · LRW 0x00010000 [68] · BRD 0130",
      "stream": 0,
      "wkc_mismatch": false,
      "unanswered": false,
      "paired": 352,
      "rtt_ns": 16520.0
    },
    …
  ]
}
{
  "total": 17,
  "offset": 0,
  "rows": [
    { "no": 1965, "direction": "tx", "wkc_mismatch": true, "paired": 1966, … }
  ]
}
```

Decode the returning frame 1966; its LRW came back with WKC 8 where 10 was expected:

```bash
curl -s http://127.0.0.1:7878/api/captures/1/frames/1966
```

```json
{
  "row": { "no": 1966, "t_ns": 578619568.0, "direction": "rx", "wkc_mismatch": true, "paired": 1965, … },
  "orig_len": 130,
  "wire_bytes": 154,
  "dst": "ff:ff:ff:ff:ff:ff",
  "src": "02:00:5e:00:53:01",
  "ethertype": 34980,
  "encapsulation": "ethernet",
  "datagrams": [
    …,
    {
      "position": 1,
      "command": "LRW",
      "command_code": 12,
      "address": { "mode": "logical", "text": "logical 0x00010000", "logical": 65536, … },
      "len": 68,
      "wkc": 8,
      "expected_wkc": 10,
      "mailbox": null,
      …
    },
    …
  ],
  …
}
```

Frames carrying FoE messages:

```bash
curl -s "http://127.0.0.1:7878/api/captures/1/frames?mailbox=foe&direction=rx&limit=1"
```

```json
{
  "total": 74,
  "offset": 0,
  "rows": [
    { "no": 810, "direction": "rx", "summary": "FPWR 0x1004:1000 — FoE WRQ 'firmware.bin'", … }
  ]
}
```

Attach the ENI that `flowlab synth --eni` writes for the same spec; working counters are then
judged against its `Cnt` values (`expected_from: "eni"`). Detaching re-analyses without it:

```bash
flowlab synth configs/synth/demo.json -o demo.pcapng --eni demo.eni.xml
curl -s --data-binary @demo.eni.xml http://127.0.0.1:7878/api/captures/1/eni
curl -s -X DELETE http://127.0.0.1:7878/api/captures/1/eni
```

```json
{ "id": 1, "name": "demo.pcapng", "provenance": "synthetic", …, "eni": "flowlab-synth 0.1.0" }
{ "id": 1, "name": "demo.pcapng", "provenance": "synthetic", …, "eni": null }
```

Run the example simulation with smaller payloads and read its result:

```bash
curl -s http://127.0.0.1:7878/api/sim/example \
  | python3 -c 'import json,sys; s=json.load(sys.stdin)
for t in s["transfers"]: t["payload_bytes"] = 4000
print(json.dumps(s))' \
  | curl -s -H 'content-type: application/json' --data @- http://127.0.0.1:7878/api/simulate
curl -s http://127.0.0.1:7878/api/captures/3/simulation
```

```json
{ "id": 3, "name": "sim-seed1.pcapng", "provenance": "simulated", "frames": 210, "has_simulation": true, … }
{ "meta": { "source": "simulated", … }, "all_completed": true, "transfers": [ { "protocol": "EoE", "verified": true, … }, … ], … }
```

(Id 2 was a capture generated with `POST /api/synth` earlier in the session.) Errors:

```bash
curl -s http://127.0.0.1:7878/api/captures/9/report
curl -s "http://127.0.0.1:7878/api/captures/1/frames?station="
curl -s -H 'content-type: application/json' --data @sim-bad.json http://127.0.0.1:7878/api/simulate
```

```text
{"error":"no capture 9"}                                                          (404)
Failed to deserialize query string: station: cannot parse integer from empty string   (400)
{"error":"invalid spec: slave 2 (bulk-endpoint): SyncManagers need 1852 bytes of ESC RAM (mailboxes + 3 × process data), only 100 available"}   (400)
```

`sim-bad.json` is the example spec with `slaves[2].dpram_bytes` set to 100. Delete a capture and
stop the server:

```bash
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE http://127.0.0.1:7878/api/captures/2   # 204
kill %1   # the server also stops on Ctrl-C
```
