# Network configuration and planning

Two XML formats describe EtherCAT devices and networks. FlowLab reads the parts it needs:

- **ESI** (EtherCAT Slave Information, ETG.2000), shipped by device vendors: identity,
  SyncManagers, PDOs and their default assignment, supported mailbox protocols,
  distributed clocks.
- **ENI** (EtherCAT Network Information, ETG.2100), exported by a configuration tool such as
  TwinCAT: the configured slaves, their mailboxes, and the cyclic frames with the working
  counter the master expects for each datagram.

Unknown elements are ignored. Documents with a DTD are rejected.

## Analysing with an ENI

```bash
flowlab analyze run.pcapng --eni network.xml
flowlab serve run.pcapng --eni network.xml        # or Attach ENI… on the Overview
```

With an ENI:

- each cyclic datagram it lists is judged against its configured working counter instead of the
  most frequent value;
- stations get their configured names;
- mailbox addresses are known for stations whose SyncManager configuration is not in the
  capture.

Datagrams that the ENI does not list fall back to the most frequent value. The basis is
reported per datagram, so a partial ENI is visible as such. **Remove** on the Overview
re-analyses without it.

`flowlab synth SPEC -o x.pcapng --eni x.eni.xml` writes the network of a synthetic capture as
an ENI, which is useful for trying this out.

## Reading an ESI

```bash
flowlab esi vendor-devices.xml          # summary; --json for the full import
```

In the **Planner** view, **Import ESI…** adds the file's devices to the plan, with their default
process data sizes, mailbox sizes and protocols.

## Planning a bus budget

```bash
flowlab plan [plan.json] [--json]        # without a file: a built-in example
```

The planner takes a cycle time, a link rate and a line of slaves (process data sizes, mailboxes,
ESC RAM, required protocols). It lays out the cyclic frames as a master would: FRMW for
distributed clocks if requested, LRW over the process image split at the 1,486-byte datagram
limit, and BRD of AL status. Then it reports:

- each frame's size and wire time, and when the cyclic frames are back after the ring;
- the **idle time** left in the cycle, and how many mailbox frames fit into it: the bus-side
  mailbox headroom;
- **checks**, each with a severity and its reason: cycle overrun, too little idle time, ESC RAM
  overflow (mailboxes + 3 × process data), mailboxes larger than the device allows, and
  required protocols that are missing or not declared.

```text
cycle 1.000 ms  link 100 Mbit/s  process image 34 B out / 42 B in
  frame 0: 138 B, 12.96 µs on the wire  [FRMW 8 B, LRW 76 B, BRD 2 B]
  cyclic frames back after 17.16 µs (1.3 % of the cycle on the wire, ring 4.20 µs); idle 982.84 µs (98.3 %)
  mailbox: 87.04 µs per frame, 10 frames fit per cycle, ≤ 5060 kB/s per direction
```

The headroom is a bus-side upper bound. Whether a slave can take mailbox messages that fast
depends on its application and PDI, which is what the [simulator](simulation.md) models.

The planner's spec format is described in [Input files](../reference/specs.md). The algorithm
is described in [flowlab-devices](../workspace/flowlab-devices.md).
