# TCP over EoE

Ethernet over EtherCAT (EoE) carries ordinary Ethernet frames through the slave's mailbox, so a
TCP connection to a service on the slave's host runs over the EtherCAT cable. Three FlowLab
commands measure it end to end:

- on the slave's host, [`flowlab pdi-slave --eoe-bridge`](pdi-slave.md#eoe-to-a-network-interface)
  puts EoE onto a Linux network interface (or the slave's own application does, if it offers
  EoE);
- on the master's host, [`flowlab master gateway`](master.md#gateway) brings the EoE link up
  with its own TCP/IP stack and forwards a local port to the slave;
- `flowlab tcp-bench` sends and receives through that port, with a SHA-256 check of every byte.

```text
 master's host                                     slave's host
 tcp-bench run ─▶ 127.0.0.1:18000                   tcp-bench serve (:8000)
                  master gateway ═══ EtherCAT ═══ ESC ─SPI─ pdi-slave ─▶ veth ─▶ host stack
                  (smoltcp, EoE)       (EoE)                (bridge)
```

## A run

```bash
# Slave's host, as root (the bridge's veth pair, then the slave application and the server).
ip link add fleoe0 type veth peer name fleoe0b
for i in fleoe0 fleoe0b; do ethtool -K $i tx off rx off gso off tso off gro off sg off; done
ip link set fleoe0b up; ip link set fleoe0 up
flowlab pdi-slave --device /dev/spidev1.0 --clock-hz 20000000 --replace-slave-application \
    --max-mailbox-bytes 1024 --eoe-bridge fleoe0b --eoe-host fleoe0 &
flowlab tcp-bench serve --listen 0.0.0.0:8000 &

# Master's host: mailboxes of 1024 B, the slave's address set by EoE, port 18000 forwarded.
flowlab master gateway -i en7 --isolated-lab-network --station 0x1000 --mailbox-bytes 1024 \
    --slave-ip 10.0.0.2 --master-ip 10.0.0.1 --set-ip --slave-mac 02:00:00:00:00:02 \
    --forward 18000:8000 -o results/gw/run1.json &
flowlab tcp-bench run --to 127.0.0.1:18000 --download 1M --upload 1M \
    -o results/gw/run1-tcp.json --csv results/gw/tcp.csv --name run1
```

`tcp-bench run` prints one line per direction and fails if a SHA-256 differs:

```text
download    1048576 B in    2.576 s:     407.0 kB/s, SHA-256 ok
upload      1048576 B in    2.691 s:     389.7 kB/s, SHA-256 ok
```

(Measured on a Verdin iMX8MP with a LAN9252 on SPI at 20 MHz; see
[LAN9252 on a Verdin iMX8MP](../studies/bulk-transfer/lab-results.md).)

## tcp-bench

`serve` speaks a minimal HTTP/1.0, one thread per connection, so `curl` works against it too:

| Request | Answer |
|---|---|
| `GET /bytes/N?seed=S` | N bytes (at most 1 GiB) of the simulator's payload for seed S (default 1), with `Content-Length` and `X-SHA256` |
| `PUT` or `POST /upload` with `Content-Length` | `BYTES SHA256 SECONDS`: what arrived, its SHA-256 and the time from the end of the headers to the last byte |

`run` downloads `--download` bytes (1M) with `--seed` and uploads `--upload` bytes (1M) of the
payload for seed + 1; a `k` or `M` suffix means KiB or MiB, 0 skips a direction. A download
counts from connecting to its last byte, as does an upload, up to the server's answer. It
checks the download against the header and against the payload it computes itself, and the
upload against the server's count and SHA-256.

| Output | Content |
|---|---|
| stdout | bytes, seconds, kB/s and the check per direction |
| `-o FILE` | JSON, provenance `measured`: per direction the HTTP status, bytes, seconds, time to the first byte (download) or the server's receive time (upload), bytes per second and `sha256_ok` |
| `--csv FILE --name NAME` | one row per direction: `name,direction,http,bytes,seconds,ttfb_or_server_s,bytes_per_s,sha` (header written if the file is new) |

`--timeout` (300 s) bounds every connect, read and write. `tcp-bench` opens ordinary TCP
connections and sends no EtherCAT frames; the gateway is what reaches the segment.

## What limits it

TCP over EoE is a mailbox protocol: every frame moves in mailbox messages, one SyncManager
buffer at a time, and the slave's application copies each message through its PDI. A message
carries the mailbox size less 10 bytes of headers, cut to 32-byte steps except for the last
fragment, so a 1514-byte frame takes 7 messages with 256-byte mailboxes, 4 with 512 and 2 with
1024; every TCP acknowledgement takes one more. The slave's access pattern to its ESC therefore sets the
rate far more than the 100 Mbit/s line: see the
[transfer protocols](../reference/transfer-protocols.md) and the measured
[LAN9252 results](../studies/bulk-transfer/lab-results.md). The gateway's own options
(`--mtu`, `--ack-delay-ms`, `--tcp-buffer`, `--cycle-us`) are on the [Lab master](master.md#gateway)
page.
