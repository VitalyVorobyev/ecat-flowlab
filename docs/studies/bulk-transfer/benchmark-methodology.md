# Benchmark methodology

## Principle

Compare full logical transfer completion in a controlled test. A 10 MB transfer must include segmentation, buffering, protocol headers, roundtrips, retries, and final integrity verification. Distinguish model predictions from real measurements. Benchmark both directions independently.

## Standard workloads

1. `tiny`: 64 B / 256 B / 1 KiB, periodic and burst, to expose scheduler overhead.
2. `boundary`: values around one application chunk, EoE fragment, mailbox service and maximum PDO payload.
3. `bulk`: 10,000,000 B and 10,485,760 B one-shot file-like transfers.
4. `bulk-repeated`: 10 MB repeated at a configurable period (may overload link).
5. `saturated`: streaming source to measure sustainable goodput.
6. `mixed`: 10 MB bulk while cyclic control PDO traffic and other mailboxes are serviced.
7. `bidirectional`: reverse and simultaneous transfer (if supported).
8. `fault`: jitter, service starvation, frame loss, duplicate, dropped ACK, slave receiver backpressure.

## Measured/estimated metrics

- Sender enqueue-to-receiver `COMPLETE` end-to-end time and integrity status.
- `verified_payload_bytes / completion_time` as application goodput; record decimal MB and MiB labels.
- First-byte latency and per-chunk latency histogram p50/p95/p99.
- Physical frame count/bytes and utilization; EtherCAT datagrams; mailbox transactions and EoE fragments; retransmission/CRC/working-counter errors where observable.
- Master CPU load, user/kernel time, ESC access and Toradex CPU load; SPI read/write rate.
- Queue occupancy and drops/overruns; process-data cycle deadline misses and jitter.
- Provenance (simulation vs Linux vs Windows vs TwinCAT), hardware/firmware revisions, master library version, network interface, timestamp source and uncertainty.

## Statistical protocol

- Deterministic simulator: record seed, scenario hash, commit SHA and build target.
- Real hardware: warm-up, repeat >= 5 trials (tune statistically), log median, range and confidence intervals where meaningful; avoid claiming precision beyond clock accuracy.
- Do not combine different directions, payload definitions, CPU frequencies or master settings into a single mean.
- Baseline: raw direct TCP over dedicated Ethernet between PC and Toradex (ideally Gigabit if available). It is not EtherCAT and illustrates the opportunity cost of EtherCAT.
- Secondary baseline: existing TwinCAT EoE system if safely available, with exact topology and matching data application.

## Validation instruments

- Raw EtherCAT PCAP/PCAPNG capture with suitable access (a virtual EoE adapter capture alone hides mailbox scheduling).
- EtherCAT master diagnostics (WKC, missing frames, state transitions), ESC counters, CPU traces, netlink stats, SPI metrics.
- Independent SHA-256 at both ends; optional CRC32 per chunk for early detection.

## Report format

JSON = canonical data; CSV = comparison tables; Markdown/HTML = presentation. Every chart/table displays `Measured` or `Simulated`, scenario parameters, count of trials and uncertainties. Check in small test fixtures, not gigabytes of generated binary dumps.

## Pitfalls

- TCP receive buffer size or sender file I/O may be bottleneck; pre-generate bytes or use repeatable memory sources for isolated network tests.
- Physical link 100 Mbit/s is not 100 Mbit/s useful payload.
- Background PDO traffic competes with bulk data; don't remove it to exaggerate a protocol's real production throughput.
- EoE/VoE/FoE implementation quality is separate from theoretical header efficiency.
