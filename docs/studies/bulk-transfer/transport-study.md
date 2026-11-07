# Transport landscape and decision framework

## First principle

The transport must be supported **by both master and slave**. Replacing the TwinCAT master alone cannot add VoE/PDO/FoE abilities to a slave that only exposes EoE, and adding an EtherCAT stack to a Toradex Linux application does not substitute for ESC hardware.

| Transport | Wire/path | Transfer 10 MB? | Potential strength | Main risks |
|---|---|---|---|---|
| EoE over EtherCAT | Mailbox fragments of Ethernet packets | Yes, as many packets/fragments | IP app compatibility, standard | Fragment overhead, queues, master/slave scheduling, TCP over tunnel |
| FoE | EtherCAT mailbox protocol | If supported by both sides | Transfer/file oriented | Service availability by state, device constraints, semantics, throughput |
| VoE | Vendor-defined mailbox | If supported by both sides | Custom opaque binary envelopes | Master API support, no built-in reliability/application semantics |
| CoE/SDO | Mailbox object dictionary | Segmented/block when supported | Standard interoperable parameter access | Request/reply and object semantics may be inefficient for bulk |
| PDO custom channel | Cyclic process data | Yes, chunked across cycles | Predictable service slots, real-time integration | Fixed reservation, ESC memory, frame count, master timing, buffering |
| Plain Ethernet TCP | Independent ordinary network | Yes | Strong throughput baseline | Not physically through EtherCAT segment |

## EoE

EoE tunnels Ethernet frames in mailbox messages. For 10 MB application transfers with IP/TCP there are multiple nested layers: application chunks → TCP/IP packets → Ethernet frames → EoE fragments → EtherCAT mailbox datagrams. Benchmark both TCP and UDP-based EoE workload only if both endpoints genuinely support it. Record packet sizes, mailbox sizes and reassembly flush delays. Link MTU does not directly set slave mailbox capacity.

## PDO

Use a reserved process-data area with your own data protocol (`wire-protocol.md`). A 10 MB transfer requires many cycles; quantify bytes/cycle, bus occupancy, ESC buffering, and aggregate process image constraints. Do not assume the process image supports arbitrary 512/1024-byte PDO mappings or that host software updates it fast enough. Compare schedules keeping control PDO load fixed. A custom PDO channel is valid when both endpoints are under our control and the master configures the mapping.

## VoE

Raw vendor-specific mailbox protocol potentially reduces encapsulation vs EoE, but not mailbox service bottlenecks. Require verified support from chosen master stack and slave firmware; no assumption of a generic public TwinCAT API for arbitrary VoE traffic.

## FoE

File over EtherCAT looks especially relevant to an occasional 10 MB *file-like transfer*. Verify implementation, state requirements, block sizes, master API, and whether writing arbitrary application bytes is suitable. It may be limited to firmware/file services on some devices. Avoid asserting universal performance superiority.

## CoE

Use as a compatibility baseline when supported. Prefer verified segmented/block transfers; never turn 10 MB into thousands of individual scalar SDO writes without explicitly measuring the inefficiency.

## Questions for protocol selection

- Hard transfer deadline? Data arrival frequency? Unidirectional or full duplex?
- Is dynamic variable-length delivery required or is a fixed throughput channel acceptable?
- Can both endpoints be modified? Does the system already use TwinCAT and fixed slave firmware?
- Can a separate physical Ethernet interface be used instead?
- Is data persistent (file/FoE) or transient (stream/PDO)?
- What is the ESC's actual mailbox/process memory and PDI speed?

Decision must be evidence-based: compare *transfer completion time and useful application goodput*, not hypothetical header savings alone.
