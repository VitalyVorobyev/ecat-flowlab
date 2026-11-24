# On a Verdin board

How to run the lab toolbox on a Toradex Verdin module (aarch64, Torizon OS or another Linux
image) whose carrier has a LAN9252 on SPI. The master runs on a PC or Mac cabled to the
board's EtherCAT IN port; the Verdin runs the slave side.

| On the Verdin | On the master's host |
|---|---|
| [`flowlab pdi-slave`](pdi-slave.md): FlowLab's slave application on the LAN9252 | [`flowlab master`](master.md): scan, transfer, gateway |
| [`flowlab pdi-bench`](../studies/bulk-transfer/lab.md#measuring-the-spi): SPI access times | [`flowlab tcp-bench run`](tcp-bench.md) |
| [`flowlab pdi-phy`](pdi-phy.md): the EtherCAT ports' PHYs | `flowlab capture`, `flowlab analyze` |
| `flowlab tcp-bench serve` | |

A Verdin without an ESC can still be a slave for tests: the [emulated slave](slave.md) answers
on its ordinary Ethernet port.

## The binary

The release archive `flowlab-VERSION-aarch64-unknown-linux-musl.tar.gz` is static, runs on
any Verdin image and includes the lab master and the slave applications (features `master` and
`slave`). From source
([Releases and cross-compiling](../workspace/contributing.md#releases-and-cross-compiling)):

```bash
cargo zigbuild --release -p flowlab-cli --features master,slave --target aarch64-unknown-linux-musl
scp target/aarch64-unknown-linux-musl/release/flowlab torizon@verdin:
```

**Without a network.** When the master's host has a single Ethernet port and it carries
EtherCAT, the board's management port is unreachable. The serial console still works. Put the
console's shell into raw mode and send the bytes as they are:

```bash
# On the board's console:
stty raw -echo; head -c BYTES > flowlab.gz; stty sane
```

Then write the file's bytes to the serial device from the host, and compare `md5sum` on both
sides. At 115200 baud a 3 MB gzip takes about 4 minutes. Do not paste base64 into a shell
here-document: the line editor drops characters at that rate.

## Root

`pdi-slave`, the EoE bridge and `chrt` need root. With the `torizon` user's password, use
`sudo`. On Torizon OS, membership in the `docker` group is equivalent to root: a privileged
container that shares the host's process namespace and mounts `/` can `chroot` into the host:

```bash
docker run --rm -it --privileged --pid host --network host -v /:/host IMAGE chroot /host
```

`IMAGE` is any image on the board that has `chroot`. Use this only on a test device.

## Stopping the board's own slave application

The ESC has one PDI: FlowLab's application and the product's must not both serve it.

```bash
docker ps                          # find the container that holds /dev/spidevB.C
docker stop -t 3 NAME              # if it ignores SIGTERM, docker kills it after 3 s
docker start NAME                  # afterwards, to restore the product
```

A container with the restart policy `unless-stopped` stays stopped across reboots until it
is started again. A stopped application can leave the mailbox SyncManagers deactivated from
the PDI side (PDI Control bit 0); `pdi-slave` clears that on PRE-OP and sets it again when it
stops.

## SPI

- The LAN9252 is a spidev device, for example `/dev/spidev1.0`; `pdi-bench`, `pdi-phy` and
  `pdi-slave` check the link (`BYTE_TEST`, `HW_CFG`, `ID_REV`) before anything else.
- Find the highest clock the board passes with `pdi-bench` and a few clocks. A carrier's SPI
  path can fail well below the LAN9252's own limit: on one board, reads came back shifted by
  one bit from 30 MHz on ([LAN9252 on a Verdin iMX8MP](../studies/bulk-transfer/lab-results.md)).
- The i.MX ECSPI driver moves transfers over 64 bytes by DMA, which is slower at these sizes;
  `--spi-tuning fast` (the `pdi-slave` default) keeps every transfer at 64 bytes or less.

## Measurement hygiene

```bash
# As root. Both reset at reboot.
for p in /sys/devices/system/cpu/cpufreq/policy*; do echo performance > $p/scaling_governor; done
chrt -f -p 50 $(pidof flowlab)     # the polling loop as a real-time thread
```

`pdi-slave` polls the ESC continuously and keeps one core busy. Without the `performance`
governor, the core's clock follows the load and adds jitter to every SPI transfer; without a
real-time priority, other processes delay the loop by milliseconds.

## The EtherCAT ports

`flowlab pdi-phy` shows the link of both ports as the LAN9252's PHYs see it. Firmware on some
boards forces the PHYs (fixed speed, duplex and MDI/MDI-X, no auto-negotiation), and a USB
adapter on the master's side may then link on one port and not on the other; plug the master
into the port that links. Check the carrier's documentation before plugging a cable into an
unlabelled RJ45 socket: not every socket on a carrier is Ethernet.

## A complete run

```bash
# Verdin, as root: stop the product, bridge EoE to a veth pair, serve TCP.
docker stop -t 3 NAME
ip link add fleoe0 type veth peer name fleoe0b
for i in fleoe0 fleoe0b; do ethtool -K $i tx off rx off gso off tso off gro off sg off; done
ip link set fleoe0b up; ip link set fleoe0 up
./flowlab pdi-slave --device /dev/spidev1.0 --clock-hz 20000000 --replace-slave-application \
    --max-mailbox-bytes 1024 --eoe-bridge fleoe0b --eoe-host fleoe0 &
chrt -f -p 50 $!
./flowlab tcp-bench serve &

# Master's host.
flowlab master scan -i en7 --isolated-lab-network
flowlab master transfer -i en7 --isolated-lab-network --station 0x1000 --mailbox-bytes 1024 \
    --protocol voe --bytes 1000000 --seed 3
flowlab master gateway -i en7 --isolated-lab-network --station 0x1000 --mailbox-bytes 1024 \
    --slave-ip 10.0.0.2 --master-ip 10.0.0.1 --set-ip --slave-mac 02:00:00:00:00:02 \
    --forward 18000:8000 &
flowlab tcp-bench run --to 127.0.0.1:18000 --download 1M --upload 1M

# Verdin afterwards: Ctrl-C (or kill) pdi-slave, which sets AL Status to INIT, then
ip link del fleoe0
docker start NAME
```


## Web workbench on the Verdin

When built with `--features embed-ui,slave`, `flowlab` bundles the entire frontend inside the
single static binary so no Node/Bun runtime or separate asset folder is required on the board:

```bash
# On the development host (cross-compile UI + binary):
cd ui && bun run build && cd ..
cargo zigbuild --release --target aarch64-unknown-linux-musl -p flowlab-cli --features embed-ui,slave
```

On the Verdin (after stopping any product slave stack before enabling write/takeover flags):

```bash
# Management LAN or mDNS (<hostname>.local:7878), plus EtherCAT EoE forwarding:
./flowlab serve --host 0.0.0.0 --port 7878 \
    --device /dev/spidev1.0 \
    --replace-slave-application --allow-writes --allow-phy-writes
```

Open `http://<verdin-ip>:7878` or `http://<verdin-hostname>.local:7878` directly over management
LAN, or forward port `7878` over EtherCAT EoE via `flowlab master gateway --forward 17878:7878`
and open `http://127.0.0.1:17878`. Use the **Throughput** workspace to measure browser-to-Verdin
HTTP/TCP speed and RTT over either path without HTTPS/WebCrypto (SHA-256 runs in pure TypeScript),
and use **SPI Bench** -> **Apply to Simulate** to calibrate the workbench simulator directly from
hardware measurements. Pass `--sim-pdi` if you only want to rehearse the UI without touching the
LAN9252.
