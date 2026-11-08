# Live capture

FlowLab can watch a running segment: it reads the traffic as it is captured and analyses it every
second. It does not open network interfaces itself. Wireshark's `dumpcap` (or `tcpdump`) captures
the EtherCAT frames and writes them to FlowLab through a pipe; FlowLab saves them to a file and
runs the same analysis as on a capture file. Nothing is sent on the interface.

## Requirements

One capture tool, allowed to capture without root:

| System | Install | Permission to capture |
|---|---|---|
| Linux | `sudo apt install wireshark-common` (or `tshark`), or tcpdump | answer *yes* to "non-superusers capture", or `sudo dpkg-reconfigure wireshark-common`, then `sudo usermod -aG wireshark $USER` and log in again |
| macOS | Wireshark (`brew install --cask wireshark`) | the ChmodBPF helper that comes with the Wireshark package (`brew install --cask wireshark-chmodbpf` with Homebrew) |
| Windows | [Wireshark](https://www.wireshark.org) with Npcap (its installer offers Npcap) | none, unless Npcap was installed with access restricted to administrators |

FlowLab uses `dumpcap` when it finds one (on `PATH`, in the Wireshark app bundle on macOS, in
`C:\Program Files\Wireshark` on Windows), otherwise `tcpdump`. tcpdump does not exist on Windows,
so there it uses `dumpcap.exe` only.

`flowlab capture --list` shows the interfaces the tool can open; if capturing is not allowed,
the tool's own error message is shown. Windows interface names look like `\Device\NPF_{GUID}`;
the list prints each with its description (for example `Ethernet 2`), and that name goes to
`flowlab capture` or the Live view.

## Where to capture

Capture on the master's port, or with a tap or a mirror port between the master and the first
slave, so that both the sent frames and the returning ones are seen: directions, round trips,
lost frames and mailbox messages depend on it. The timestamps are the capturing host's. Software
timestamps include its scheduling jitter, so capturing on the master PC itself shows the master's
send jitter plus the host's. A tap with hardware timestamps measures the wire.

The filter keeps EtherCAT only: EtherType 0x88A4 and EtherCAT over UDP (port 34980).
`--all-traffic` keeps everything.

## On the command line

```bash
flowlab capture --list                       # interfaces the capture tool can open
flowlab capture eth1                         # until Ctrl-C; saves results/live/live-<time>.pcapng
flowlab capture eth1 --duration 60 --eni network.xml -o run.pcapng --report last-window.json
flowlab capture --replay run.pcapng --speed 5    # play a file back at five times its pace
```

One line per second:

```text
  time   frames       cycle  jitter p99     RTT p99    WKC  unans  missed  mailbox
    1s     2807    1.000 ms    10.59 µs    16.52 µs      1      1       0      235
    2s     1998    1.000 ms     9.65 µs    16.52 µs      1      2       0        0
```

| Column | Meaning, for the frames that arrived in that second |
|---|---|
| frames | EtherCAT frames, both directions |
| cycle | period of the main cyclic stream (the most frequent frame layout) |
| jitter p99 | 99th percentile of its send times' deviation from the cycle grid |
| RTT p99 | 99th percentile of its round trip at the capture point |
| WKC | frames of cyclic streams whose working counter differs from the expected value |
| unans | sent frames that did not come back |
| missed | cycles missing in gaps of the cyclic streams |
| mailbox | mailbox messages, both directions |

At the end the totals are printed, and `--report` writes the full analysis of the last window
(`--window`, default 10 s) as a [CaptureReport](../reference/reports.md#capturereport). The saved
file is an ordinary capture: `flowlab analyze` and the UI read it in full.

## In the UI

Capturing from interfaces in the UI is off by default. Turn it on when you start the server:

```bash
cargo ui -- --live                   # or: flowlab serve --live [--live-dir DIR]
```

The **Live** view then lists the interfaces. Choose the master's port, set the analysis window
and press **Start**. The view shows:

- the latest second and the session's totals: cycle, jitter, round trip, WKC mismatches,
  unanswered frames, missed cycles, mailbox messages;
- jitter and round trip per second, and problems per second, since the start;
- the findings and the stations (AL states, datagrams without response) of the last window.

**Stop** ends the capture and loads the saved file as a capture, so every analysis view can open
it. **Replay a capture** plays a loaded capture back at its recorded pace (1× to 10×); it works
without `--live` and needs no hardware, which makes it a way to try the view.

## How the live figures are made

- Each second's frames are analysed on their own, which gives one point per second and the
  totals. Cyclic streams need at least 20 frames, so cycles slower than about 50 ms show no
  period or jitter per point; the window's report still has them.
- The window (the last *N* seconds of capture time) is analysed in full every second.
- The master configures the mailboxes once, at start-up. Addresses learned from that
  configuration, or from the ENI, are kept for the whole session and passed on as hints, so
  mailbox traffic is still recognised after the configuration has left the window.
- A frame sent less than 50 ms before the end of a second is not counted as unanswered in that
  second's point, because its answer may arrive in the next.

The view lists these under *How the live figures are made*. The saved capture's full analysis has
none of these limits.
