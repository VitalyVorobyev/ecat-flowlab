# Logical application transfer protocol (transport independent)

This document is a proposed **application layer**, NOT a change to the EtherCAT standard and NOT an assertion about any standard mailbox on-wire layout. Transport-specific adapters encapsulate these messages appropriately and may use native FoE semantics instead of this exact format.

## Goals

- Bounded memory, arbitrary binary payloads, streaming segmentation and out-of-order/duplicate protection when needed.
- Exact payload integrity, independent of transport-layer checksums.
- Ability to compare data recovery overhead under faults.
- Minimal enough to embed over PDO, VoE, EoE application socket or simulated link.

## Logical messages

- `START`: version, 128-bit transfer ID, total byte count (`u64`), preferred chunk size, transfer options, expected SHA-256 digest (optional for streaming).
- `DATA`: transfer ID, `u64` offset, `u32` length, payload bytes. Transport must respect its configured maximum piece size.
- `ACK`: transfer ID, cumulative received offset + optional selective receipt ranges, advertised receive window, error code.
- `END`: transfer ID, sender final length/digest; receiver verifies and emits `COMPLETE` or `ERROR`.
- `ABORT`: reason and transfer ID.

**These are logical concepts, not a frozen binary packet format.** First implement a versioned typed message model and tests. Choose compact wire encoding only when transport requirements are concrete. Avoid allocating one contiguous 10 MB buffer unnecessarily; use streaming hash and bounded receive windows.

## Safety / correctness

- Reject offsets that overflow, exceed declared length, overlap inconsistently or exceed memory quotas.
- Protect against duplicate and reordered fragments where transport semantics allow them.
- Classify transport failures separately from application corruption and local cancellation.
- Consider direction-dependent retry and timeout semantics; a cyclic PDO channel may use explicit ACK/NACK fields while TCP-over-EoE inherits TCP reliability.
- Track measured *bytes successfully committed and verified*.
- Transfer may be successful only when the entire declared payload has passed validation.

## Cross-transport fairness

- Equal payload byte sequences and hash validation on all adapters.
- Define whether transfer starts at `START` enqueue vs first wire byte; report both when possible.
- Include control messages and retries in physical utilization; exclude them from application goodput numerator.
- Clearly distinguish file transfer semantics (FoE) from this native session protocol when comparing.

## Initial tests

Empty, 1B, near-fragment boundaries, 10 MB, 10 MiB, non-aligned tail, duplicate, missing chunk, reordered data, sender crash, receiver restart, checksum mismatch, partial allocation failure and timeout.
