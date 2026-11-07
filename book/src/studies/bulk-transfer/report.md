# Report: moving 10 MB through an ESC on SPI

This report covers what limits a 10,000,000-byte transfer between a PC master and an application
behind a LAN9252-class EtherCAT slave controller on SPI, and which changes help, by how much
and on what conditions. It is based on **simulations only**. No lab measurement exists yet. The
slave-side parameters that matter most (the SPI's effective rate and per-access cost) are
placeholders until they are measured. Read the numbers as *how the outcome depends on the
system*. They are not predictions of the lab result. The same results are on the interactive
[findings page](https://vitalyvorobyev.github.io/ecat-flowlab/findings/), which also checks
every configuration against a deadline and link and SPI limits you set.

> **In short**
>
> 1. **The floor is 0.8 s.** That is how long 10 MB alone occupies a 100 Mbit/s link. TCP over a
>    dedicated Ethernet link needs about 0.84 s. No modelled EtherCAT configuration comes within
>    a factor of 2.5 of that.
> 2. **The master's mailbox scheduling decides the first order of magnitude.** If the master
>    serves the mailbox a fixed number of times per cycle, FoE and CoE move one mailbox of data
>    every two cycles, whatever the SPI does. If the master polls in idle bus time, the same
>    transfer is 1.5–12× faster, and the SPI becomes the limit.
> 3. **Once the master polls freely, the SPI is the bottleneck.** Transfer time is roughly
>    inversely proportional to its effective rate. Reading only the message from the mailbox,
>    rather than the whole SyncManager buffer, gains as much as doubling the rate.
> 4. **Mailbox size: larger is better, with one trap for EoE.** EoE fragments carry multiples of
>    32 bytes, so a 512 B mailbox needs four fragments per Ethernet frame where 522 B needs three
>    and 778 B needs two.
> 5. **A process-data channel is the fastest and most predictable path, if both ends are yours.**
>    It does not depend on mailbox scheduling. Its throughput is bytes per cycle ÷ cycle time,
>    capped by the 4 KiB of ESC RAM (at most 1,260 B per cycle here). It fails badly if the SPI
>    cannot copy every cycle's chunk.
>
> **First actions:** fix the requirement (deadline, frequency, direction). Measure the SPI's
> effective rate and per-access overhead on the Verdin with the driver you will ship. Capture the
> real master's mailbox polling. Then re-run this report with those values.

## The modelled system

```text
PC master ──100BASE-TX── coupler ── I/O slave (8 B out / 8 B in) ── bulk endpoint ──SPI── application
                                                                     ESC, 4 KiB RAM
```

| Parameter | Value in the base case | Status |
|---|---|---|
| Link | 100 Mbit/s, 700 ns forwarding delay per slave | standard / chosen |
| Cycle | 1 ms, 2 µs master jitter (σ); cyclic LRW over the process image and BRD of AL status | chosen; varied |
| Bulk endpoint ESC RAM | 4,096 bytes; SyncManagers need mailbox out + mailbox in + 3 × outputs + 3 × inputs | LAN9252 datasheet |
| Mailboxes | 512 B each way | chosen; varied 128–1,484 B |
| PDI (SPI) | 20 Mbit/s effective each way, 5 µs per access, whole-buffer mailbox access | **placeholder**; varied 2.5–80 Mbit/s, 1–100 µs |
| Slave application | 20 µs to handle a message | **placeholder** |
| Master mailbox policy | fixed 2 datagrams per cycle | **unknown**; varied, see below |
| Transfer | 10,000,000 B, master → slave, one at a time | requirement still open |

The simulator treats both directions alike when the PDI's read and write rates are equal. The
master → slave numbers then apply to uploads too. A real SPI driver may well be asymmetric, so
measure both directions.

**The master policies compared:**

- **fixed, *n*/cycle**: up to *n* mailbox datagrams (writes and reads together) per cycle, in
  one frame after the cyclic frame.
- **idle time ≤ *p* %**: after each mailbox frame returns, another is sent while within *p* %
  of the cycle (10 µs turnaround).

Which of these a given master does is a question for a capture, not an assumption. See the
[lab plan](lab.md).

## Where the time goes

Each row below changes one thing from the row above it. Every row is a separate simulation that
delivered all 10,000,000 bytes and matched the payload's SHA-256.

{{#include generated/steps.md}}

The first configuration is a plausible default: small mailboxes and a master that handles the
mailbox once per cycle. It needs almost three minutes. The largest single gains come from
larger mailboxes and from polling in idle time. After that, every further gain is on the SPI
side.

## Findings

### 1. Request/response at a fixed service rate costs two cycles per mailbox

FoE (DATA → ACK) and CoE segmented transfers (segment → response) are stop-and-wait. With a
master that serves the mailbox once per cycle, a DATA write goes out in one cycle and the ACK is
read in a later one. If the slave turns the message around within one cycle, that is two cycles
per mailbox:

    goodput ≈ (mailbox − 12 B) / (2 × cycle time)

For 512 B mailboxes at 1 ms that is 500 B per 2 ms, so 40 s for 10 MB, which is what the
simulation gives. Allowing more mailbox datagrams per cycle changes nothing with one bulk slave:
the mailbox holds one message, and the second write in a cycle would be rejected. EoE (modelled
as a UDP-like tunnel without acknowledgements) writes one fragment per cycle and is faster for
the same reason.

{{#include generated/01-protocol-policy.md}}

Polling in idle time removes the cycle from the critical path. The cost is link load: the
mailbox frames fill up to the allowed share of every cycle (see finding 4).

### 2. Mailbox size: linear at a fixed service rate, limited by the SPI otherwise

At a fixed service rate, FoE time falls in proportion to the mailbox's data size. The exception
is the largest mailbox: the whole-buffer SPI reads and the ACK write no longer fit in one cycle,
which costs a third cycle now and then. With idle-time polling, larger mailboxes help less,
because the SPI already limits the rate.

{{#include generated/02-mailbox-foe.md}}

For EoE, the fragment granularity dominates. Fragments other than the last carry a multiple of
32 bytes, so a 1,514-byte Ethernet frame (1,500 B IP MTU) needs
⌈1514 / (32 · ⌊(mailbox − 10) / 32⌋)⌉ fragments:

| Mailbox | Data per fragment | Fragments per frame |
|---:|---:|---:|
| 128 B | 96 B | 16 |
| 512 B | 480 B | 4 |
| 522 B | 512 B | 3 |
| 778 B | 768 B | 2 |
| 1,024 B | 992 B | 2 |

At a fixed service rate (one fragment per cycle), 522 B beats 512 B by a quarter, and 778 B is
as good as 1,024 or 1,484 B. With idle-time polling and whole-buffer SPI access, the larger
mailboxes are *slower*: every short trailing fragment still costs a full-buffer read.

{{#include generated/03-mailbox-eoe.md}}

### 3. A shorter cycle helps only a fixed-rate master, and only while the SPI keeps up

At a fixed service rate, time is proportional to the cycle time from 1 ms up. Below that, the
slave's turnaround (SPI read of the mailbox, handling, SPI write of the reply) no longer fits
in one cycle. At 250 µs with 1,024 B mailboxes and a 20 Mbit/s SPI, each mailbox takes five
cycles instead of two, so the transfer takes 12.35 s instead of about 5 s. With idle-time polling, the cycle
time does not matter at all.

{{#include generated/04-cycle.md}}

### 4. Idle-time polling fills the link

The opportunistic master reaches its speed by sending mailbox frames back to back. In these runs
it occupies 50–91 % of the link, depending on the share it is allowed. That is harmless on a segment with nothing else to do. It is a
problem if other slaves need their own mailbox traffic, or if the cycle's cyclic frames grow. Size
the allowed share with the [planner](../../guide/devices.md)'s idle time and mailbox headroom,
not by maximising throughput.

### 5. Once the master polls freely, the SPI decides

Transfer time is close to inversely proportional to the SPI's effective rate. The per-access
overhead matters little up to about 20 µs with whole-buffer access, because each message is a
single access. Reading only the message (header, then the announced length, then the last byte
that releases the buffer) takes three accesses instead of one. In exchange it skips the unused
part of the buffer. FoE's ACKs and short fragments benefit most: at low SPI rates the transfer
time halves.

{{#include generated/05-pdi-whole-buffer.md}}

{{#include generated/06-pdi-message-only.md}}

These two tables are the reason the SPI must be measured before anything is tuned. Across the
plausible range of effective rates and overheads, the same configuration spans more than an
order of magnitude.

### 6. A process-data channel is fast and deterministic, within ESC RAM and SPI limits

A custom channel in cyclic process data works like this: a sequence-numbered chunk per cycle,
cumulative ACKs in the inputs, and go-back-N on a stall. It does not use the mailbox at all, so
the master's mailbox policy is irrelevant. Its rate is fixed by the configuration: bytes per
cycle ÷ cycle time. ESC RAM caps it. The ESC keeps three buffers of each process-data direction,
so with 128 B mailboxes and 8 B of I/O each way, 1,260 B per cycle is the most that fits in
4 KiB.

{{#include generated/07-pdo-size.md}}

The channel needs the application to copy every cycle's chunk over the SPI before the next one
arrives. When it cannot, which happens above about 90 % PDI load in these runs, chunks are
overwritten in the three-buffer exchange and go-back-N resends them. Throughput then collapses
erratically: a *slower* SPI or a *larger* window can be faster. Rate-match the channel to the
measured SPI with margin; do not size it to ESC RAM alone.

{{#include generated/08-pdo-pdi.md}}

### 7. FoE, CoE and EoE in the model, and what the model leaves out

FoE and CoE segmented transfers behave almost identically: both are stop-and-wait at mailbox
granularity. EoE looks faster because the model carries UDP-like datagrams with no
acknowledgement traffic. EoE carrying TCP adds TCP's ACKs and window in the opposite direction,
through the same mailboxes. The model's EoE numbers are therefore optimistic for TCP
applications. VoE is not modelled. It would behave like EoE without the IP and Ethernet headers,
plus whatever acknowledgement scheme the application adds.

## Recommendations

In order. Each step narrows the uncertainty that the next one depends on.

1. **Fix the requirement before optimising.** Is 10 MB moved once (configuration, firmware),
   periodically, or continuously? In which direction, and by when? If it must take less than
   about 3 s, or recur often, plan a separate Ethernet path (the Verdin has its own port)
   instead of tuning EtherCAT. Even the model's best cases with a fast SPI stay above 2 s.
   If tens of seconds are acceptable, a standard FoE path with sensible mailbox sizes is
   enough.
2. **Measure the SPI on the target**, in both directions:
   - effective read and write rates for mailbox-sized transfers;
   - the fixed cost per access with the driver that will ship (a kernel driver and user-space
     spidev can differ widely);
   - the time from the ESC's mailbox interrupt to the application reading the message.

   Put these numbers into `pdi.read_bps`, `pdi.write_bps`, `pdi.access_overhead_ns` and
   `app_processing_ns`, and re-run this report.
3. **Capture the real master's mailbox behaviour.** Record a capture at the master while it
   moves a file with FoE and open it in FlowLab's [Mailbox view](../../guide/mailbox.md). The
   view shows the polling cadence, empty reads and rejected writes. These decide whether
   findings 1–3 (fixed rate) or 5 (SPI-bound) apply. If the master can poll mailboxes outside
   the cyclic frame, enable it, within the link share the other slaves can spare.
4. **Choose mailbox sizes deliberately**, within what the ESC RAM and the device's ESI/SII
   allow. For FoE and CoE, use the largest mailbox that leaves room for the process data (here
   1,024 B). For EoE, use 522 B or 778 B rather than 512 B or 1,024 B.
5. **Make the slave application cheap per message.** Read the mailbox header first and then only
   the announced length. Write replies of their real length. React to the SyncManager interrupt
   rather than polling.
6. **Use a process-data channel when you control both ends and want a fixed rate.** Size its
   bytes per cycle from the measured SPI rate, keeping the application's PDI load well below
   saturation. Use a window of 2 and add end-to-end integrity (SHA-256, as the simulator does).
   It also leaves the mailbox free for diagnostics and parameters.
7. **Calibrate, then optimise.** With lab captures, fit the simulator's PDI and master
   parameters (`flowlab study --capture`, targets from the FoE and SDO message spans). Then
   repeat these sweeps with the fitted values and check the best candidates in the lab against
   the unoptimised baseline and the plain-Ethernet transfer.

## What this report does not show

- **No measurement.** Every number comes from the simulator with the parameters above. The SPI
  and application parameters are placeholders.
- **No operating system.** Neither the PC master's nor the Verdin's scheduling is modelled
  beyond a 2 µs cycle jitter; real interrupt and thread latency adds to every turnaround.
- **One bulk slave, no faults.** Other mailbox slaves share a fixed-rate master's service. The
  simulator supports frame loss, but these runs have none.
- **The masters are idealised policies**, not a model of any particular product.

## Reproduce

```bash
cargo build --release -p flowlab-cli
python3 scripts/bulk-transfer-report.py
```

The script writes the specs to `configs/studies/bulk-transfer/report/`, runs them (a few seconds),
and writes the tables included above to `book/src/studies/bulk-transfer/generated/`. CI runs it
and fails if the committed tables no longer match the code. Any spec can be opened in the UI
(**Study** or **Simulate**) or run alone: `flowlab study configs/studies/bulk-transfer/report/02-mailbox-foe.json`.
