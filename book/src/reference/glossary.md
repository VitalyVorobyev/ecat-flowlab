# Glossary

EtherCAT and FlowLab terms used in this book, in alphabetical order. Register addresses and
codes are those FlowLab uses; the protocol definitions are in the ETG standards listed under
[Sources](sources.md).

**3-buffer mode** — Buffered SyncManager mode with three buffers per direction, so that the
master and the slave application never wait for each other and the reader always gets the
newest complete buffer; intermediate updates can be overwritten unread. The simulator and the
planner charge 3 × the process-data bytes of each direction to ESC RAM.

**ADP / ADO** — The two 16-bit halves of a datagram's 32-bit address in physical addressing:
ADP is the position (auto-increment), station address (configured) or a counter (broadcast);
ADO is the offset in ESC memory. FlowLab stores the raw field as `ADP | ADO << 16`.

**AL state** — Application-layer state of a slave: INIT (1), PREOP (2), BOOT (3), SAFEOP (4),
OP (8). The master requests a state in AL control (0x0120); the slave reports it in AL status
(0x0130), with bit 4 as the error indication. Mailbox traffic needs PREOP or higher; cyclic
outputs are applied in OP.

**AoE** — ADS over EtherCAT, mailbox type 1. FlowLab counts its messages by type only.

**APRD, APWR, APRW, ARMW** — Auto-increment (position) physical read, write, read/write, and
read-multiple-write. Each slave increments ADP as the frame passes; the slave at position *p*
is addressed with ADP = −*p*. Used at start-up, before station addresses exist; an APWR to
0x0010 assigns a slave its configured station address.

**Assumption** — A sentence in a result's `meta.assumptions` that states a heuristic or a
modelling choice a reader needs to interpret the numbers. See
[Provenance and assumptions](provenance.md).

**BRD, BWR, BRW** — Broadcast read, write, read/write: every slave processes the datagram and
increments its working counter. Cyclic frames commonly carry a BRD of AL status (0x0130); the
largest working counter of a returning BRD is the report's `slaves_responding`, a lower bound
on the slaves that answered.

**Bus budget** — The planner's account of one cycle: how the cyclic frames are laid out, their
wire time, the ring delay, the idle time left and how many mailbox frames fit in it.
Arithmetic only (`flowlab plan`).

**Calibration** — A study with targets: measured values the simulator should reproduce. The
first objective is the fit error, the weighted RMS of the relative errors, and the residuals of
the best run are always reported. Only quantities both a capture and the simulator observe are
fitted, such as mailbox message spans. A good fit shows the model can reproduce the numbers, not
that its parameters are the physical ones.

**Channel source** — Where the analysis learned a station's mailbox addresses, reported per
channel: `sm_config` (SyncManager configuration in the capture), `eni`, `hint` (`--mailbox`) or
`heuristic` (datagrams whose data parses as a mailbox header), in that order of preference.

**CoE** — CANopen over EtherCAT, mailbox type 3: SDO transfers, emergencies and SDO
Information messages.

**Configured station address** — The 16-bit address the master assigns each slave (APWR
0x0010) and uses with the FP… commands. Synthetic and simulated networks use 0x1001 for the
first slave, 0x1002 for the next, and so on.

**Cyclic stream** — In a report, a datagram layout (command, address, length of every
datagram) sent repeatedly at a regular interval: at least `min_stream_frames` (default 20)
frames with ≥ 60 % of the intervals within ±50 % of the median, and not made only of mailbox
accesses. Streams are numbered by frame count, 0 first.

**Datagram** — One EtherCAT command in a frame: a 10-byte header (command, index, 32-bit
address, length with circulating and more-follows flags, IRQ), the data, and a 2-byte working
counter. A frame carries several, chained by the more-follows flag.

**DC (distributed clocks)** — Synchronised system time in the ESCs. A master keeps the clocks
aligned by sending an FRMW of the system-time register (0x0910) every cycle: the first slave's
time is read and written to the others. FlowLab's generator and planner add that FRMW when
`distributed_clocks` is on; an ESI's `<Dc>` element marks a device that supports DC.

**Direction (TX / RX)** — TX is a frame as sent by the master, RX the same frame returning.
FlowLab infers directions from the locally-administered bit of the source MAC, which ESCs set
on returning frames, else from all-zero working counters, else reports them as unknown
(`direction_mode`).

**DPRAM, ESC RAM** — The ESC's process memory, from which SyncManager buffers are allocated.
`dpram_bytes` defaults to 4096 in the simulator and the planner; a spec whose mailboxes plus
3 × process data exceed it is refused (simulator) or flagged as a defect (planner).

**Emergency** — A CoE message from the slave with a CiA 301 error code, the error register and
five manufacturer-specific bytes.

**ENI** — EtherCAT Network Information (ETG.2100): how a master configured one network,
including the expected working counter of each cyclic datagram. FlowLab reads a subset and
writes one for synthetic networks; see [Input files](specs.md#eni-and-esi-xml).

**EoE** — Ethernet over EtherCAT, mailbox type 2: Ethernet frames cut into fragments whose
offsets count 32-byte blocks. The analysis reassembles them and can export them as a pcapng.

**ESC** — EtherCAT Slave Controller: the hardware that processes frames on the fly as they pass
and holds the registers, SyncManagers, FMMUs and process memory. An ordinary Ethernet MAC is not
an ESC, and traffic through one is not EtherCAT.

**ESI** — EtherCAT Slave Information (ETG.2000): an XML description of a device type, its
SyncManagers, PDOs and supported mailbox protocols. FlowLab reads a subset (`flowlab esi`).

**Exchange time, message span** — First to last mailbox message of a transfer, timed at the
master (`exchange_ns`, metric `exchange_time_s`). It is what a capture shows, and the quantity
to compare with a capture's FoE, SDO or EoE durations. The transfer time (`duration_ns`) ends
instead when the receiving application has the last byte.

**Expedited, normal, segmented** — The three forms of an SDO transfer: data of up to 4 bytes
inside the initiate message; more data in one initiate message; or data continued in segment
messages with an alternating toggle bit.

**Finding** — A judgement or note in a report: `info` (context), `warn` or `defect`. `warn` and
`defect` are verdicts against a declared threshold, which the finding's text states.

**FMMU** — Fieldbus Memory Management Unit: maps a range of the logical address space onto ESC
memory, so that LRD, LWR and LRW reach every slave's process data (registers 0x0600–0x06FF).

**FoE** — File access over EtherCAT, mailbox type 4: a read (RRQ) or write (WRQ) request
followed by DATA and ACK packets, with ERR and BUSY replies.

**FPRD, FPWR, FPRW, FRMW** — Configured-address physical read, write, read/write, and
read-multiple-write, addressed by station address. Mailbox accesses are FPWR (to "mailbox out")
and FPRD (from "mailbox in"); FRMW distributes DC time.

**Frame** — An Ethernet frame with EtherType 0x88A4 (or UDP port 34980) that carries an
EtherCAT header and datagrams. One frame holds up to 1500 bytes of EtherCAT payload; the
planner fits at most 1486 data bytes into one datagram (`MAX_DATAGRAM_DATA`).

**Gap, missed cycle** — In a cyclic stream, an interval longer than 1.5 periods is a gap; the
cycles that fit inside it are missed cycles.

**Goodput** — Payload bits delivered per second of a transfer: bytes × 8 / duration, in bit/s
(`goodput_bps`) or Mbit/s (`goodput_mbps`).

**Ground truth** — What `flowlab synth` actually generated (frames, faults, mailbox counts),
written by `--truth`. Analysis tests assert against it.

**LAN9252** — A Microchip EtherCAT slave controller with 4 KiB of process RAM and SPI or a
host-bus interface as the PDI (see [Sources](sources.md)). The bulk-transfer study models a
LAN9252-class ESC on SPI; the simulator's default `dpram_bytes` of 4096 matches its RAM, and
FlowLab's register names follow the ET1100 and LAN9252 datasheets.

**LRD, LWR, LRW** — Logical read, write, read/write: addressed by a 32-bit logical address that
FMMUs map onto each slave's memory. One LRW typically exchanges the whole process image; its
expected working counter in FlowLab's synthetic networks is 2 per slave with outputs plus 1 per
slave with inputs.

**Mailbox** — The acyclic, confirmed message channel between the master and one slave, through
two SyncManagers in mailbox mode: "mailbox out" (master → slave, usually SM0) and "mailbox in"
(slave → master, usually SM1). Each message starts with a 6-byte header (length, address,
channel, priority, type, counter). A write returning WKC 0 was rejected (mailbox still full); a
read returning WKC 0 found nothing. FlowLab takes messages only from returning datagrams with
WKC > 0.

**Mailbox counter, repeat** — The 3-bit counter in the mailbox header (1–7, 0 reserved). A
message whose counter equals the previous one of the same station and direction is counted as a
repeat and dropped.

**Mailbox hint** — `--mailbox STATION=OUT,IN`: mailbox start addresses for a station whose
SyncManager configuration is not in the capture. See [Input files](specs.md#mailbox-hints).

**Mailbox policy** — In the simulator, how the master chooses mailbox datagrams per cycle:
`fixed_quota`, `round_robin` or `opportunistic`. None of them is what every master does.

**Measured** — Provenance of a capture from a real network; the default for any file without a
FlowLab provenance marker. FlowLab cannot verify it.

**Pareto frontier** — The feasible runs of a study that no other feasible run matches or beats
on every objective while beating them on at least one. The best run is chosen from it.

**PDI** — Process Data Interface: the link between the ESC and the slave application, e.g. SPI.
In the simulator, `PdiSpec` gives its read and write rates and per-access overhead; mailbox and
process-data accesses queue on it one at a time.

**PDO** — Process Data Object: a CoE object that lists the entries mapped into cyclic process
data. RxPDOs carry outputs (master → slave), TxPDOs inputs (slave → master); an ESI gives their
default assignment to SyncManagers.

**PDO channel** — A bulk-transfer method modelled by the simulator: sequence-numbered chunks in
reserved bytes of cyclic process data, cumulative acknowledgements in the other direction and
go-back-N on a stalled window.

**Period** — A cyclic stream's nominal cycle time: the least-squares slope of send time over
cycle number, so gaps do not bias it.

**Phase jitter** — Each send time's deviation from the ideal grid `t0 + k·period` of its
stream. For independent send errors of standard deviation σ, phase jitter has σ and
cycle-to-cycle intervals have √2 σ.

**Physical bound** — The time a transfer's payload needs on the link at zero protocol overhead
(`physical_bound_ns`). No transfer can be faster.

**Process image** — The cyclic outputs and inputs of all slaves, mapped into the logical
address space and exchanged by LRW each cycle. The generator and the simulator accept at most
1400 bytes, one LRW.

**Provenance** — Where a result came from: `measured`, `simulated` or `synthetic`. Every
capture, report and result carries it; results of different provenance are never combined
without labels.

**Round trip** — Time from a TX frame to its returning RX frame, at the capture point. In a
synthetic capture it is the frame's wire time plus the slaves' forwarding delays.

**SDO** — Service Data Object: a confirmed CoE read (upload) or write (download) of one object
dictionary entry, addressed by index and subindex, ending in success or an abort code.

**Simulated** — Provenance of frames and results produced by the discrete-event simulator from a
model; every number follows from the spec's parameters.

**SoE** — Servo drive profile over EtherCAT, mailbox type 5. FlowLab counts its error responses.

**SyncManager** — An ESC unit that guards one memory area for consistent exchange between
EtherCAT and the PDI, in mailbox mode (one buffer with a full/empty handshake) or buffered mode
(3-buffer mode). Configured at 0x0800 + 8·*n* for *n* = 0…15; the analysis reads these writes
to find mailbox addresses.

**Synthetic** — Provenance of a capture generated by `flowlab synth` with known ground truth,
for tests and demonstrations. Deterministic: the same spec gives a byte-identical file.

**TCP reference** — `ethernet_tcp_reference_ns`: the time TCP would need for the same payload
over a dedicated Ethernet link at the same rate (MSS 1460, steady state, no loss, no slow
start). A lower bound for comparison, not a prediction.

**Unanswered frame** — A TX frame whose returning frame is not in the capture (only with known
directions). TX frames within `in_flight_window_ns` (default 50 ms) of the capture end are
not counted.

**Utilization** — Wire bytes per time bin against the link rate, TX and RX separately, percent.

**VoE** — Vendor-specific over EtherCAT, mailbox type 15.

**Wire bytes** — A frame's occupancy of the link: the frame padded to 60 bytes, plus 8 bytes
preamble and SFD, 4 bytes FCS and a 12-byte inter-frame gap.

**Working counter (WKC)** — The 16-bit counter at the end of each datagram. The master sends 0;
each slave that processes the datagram successfully increments it. Comparing it with the
expected value (from an ENI, else the most frequent value; `expected_from`) shows slaves that
did not respond or did not process their part. On mailbox datagrams it signals full or empty
buffers instead and is not counted as a mismatch.
