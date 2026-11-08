# flowlab-live

Live capture through an external capture tool, analysed in a rolling window. Used by
`flowlab capture`, by the server's `/api/live` endpoints ([Live capture](../guide/live.md)) and
by the lab master's check for another master (`flowlab_master::safety`).

## Design

Live capture never opens a network interface itself ([ADR-035](decisions.md#live-capture-and-hardware)). A
`LiveSession` runs a program that writes pcap or pcapng to its standard output: `dumpcap -i IF -n
-q -w - -f FILTER`, or `tcpdump -i IF -U --immediate-mode -s 0 --time-stamp-precision nano -w -
FILTER` (`Tool::capture_args`).

```text
 dumpcap/tcpdump ─stdout─▶ reader thread ─▶ StreamSplitter ─▶ blocks ─┬─▶ save file (whole blocks only)
                                                                      └─▶ window: header + packet blocks
 analysis thread, every tick:
   frames since the last tick ─▶ analyze ─▶ LivePoint, totals
   window ─▶ drop what is older than the window ─▶ analyze ─▶ CaptureReport, learned mailbox addresses
```

- `flowlab_capture::StreamSplitter` cuts the byte stream into whole blocks. The section header
  and interface blocks, followed by any run of packet blocks, form a valid file, so excerpts are
  parsed by `CaptureFile::parse` and analysed by `analyze` without a second code path.
- The save file receives whole blocks only. A tool killed in the middle of a block leaves a
  readable file.
- Mailbox addresses from SyncManager configuration or the ENI are learned and passed to later
  analyses as `mailbox_hints` (user hints win).
- The window is also capped at `MAX_WINDOW_BYTES` (256 MiB).
- `Source::Replay` plays a file back at its recorded pace instead, through the same path.
- Stopping kills the child; the reader sees the end of the pipe, the analysis thread runs a last
  tick, and the session ends. Dropping a session stops it.

## API

| Item | Purpose |
|---|---|
| `LiveSession::start(Source, LiveOptions)` | start; `snapshot()`, `points_after(t)`, `is_done()`, `stop()`, `wait()` |
| `Source::{Command, Replay}`, `Source::interface(tool, path, iface, filter)` | where packets come from |
| `LiveOptions` | `window` (10 s), `tick` (1 s), `max_points` (600), `save_to`, `analysis` |
| `LiveSnapshot` | state, source, message, provenance, packets, `totals`, `series[]`, the window's `CaptureReport`, assumptions |
| `find_tool`, `list_interfaces`, `Tool`, `ETHERCAT_FILTER`, `default_save_path` | the capture tools |

## Tests

`crates/flowlab-live/src/tests.rs`:

- A replay of a synthetic capture counts every EtherCAT frame exactly once across the ticks, and
  saves a byte-identical file.
- The window holds only its span, and mailbox messages after the configuration has left the
  window are still found.
- A replay stops early.
- `cat` stands in for the capture tool: the output is read and saved byte for byte.
- A failing tool reports its error output, output that is not a capture fails the session, and a
  missing program does not start.
- Interface lists of both tools parse.

The server test `live_replay_runs_and_interfaces_need_opt_in` covers the endpoints.
