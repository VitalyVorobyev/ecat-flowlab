#!/usr/bin/env python3
"""Cross-check FlowLab's mailbox analysis against Wireshark's EtherCAT dissector.

Usage: crosscheck-tshark.py CAPTURE REPORT.json

REPORT.json is `flowlab analyze CAPTURE --out REPORT.json`. First, no frame may be malformed or
carry a Wireshark expert warning. Then Wireshark (tshark) decodes every
mailbox datagram independently of FlowLab; this script counts what it decodes in *returning*
datagrams with WKC > 0 (the messages that actually went through) and compares the counts with
the report. Exits 1 on any mismatch.

Assumes the capture's directions split by the locally-administered source-MAC bit
(`direction_mode: mac_local_bit`), which is what flowlab-synth writes.
"""

import json
import subprocess
import sys
from collections import Counter

TYPES = {0: "Error", 1: "AoE", 2: "EoE", 3: "CoE", 4: "FoE", 5: "SoE", 15: "VoE"}
FPRD, FPWR = 0x04, 0x05


def flat(o, acc):
    if isinstance(o, dict):
        for k, v in o.items():
            if isinstance(v, (dict, list)):
                flat(v, acc)
            else:
                acc.setdefault(k, v)
    elif isinstance(o, list):
        for x in o:
            flat(x, acc)
    return acc


def tshark(capture, *args):
    """Run tshark on `capture` fed through stdin (some distributions confine tshark's file
    access with AppArmor) and return stdout; any tshark failure raises."""
    # IP is disabled: Wireshark does not reassemble EoE fragments, so it would dissect the
    # first fragment of a tunnelled IP datagram as a truncated packet. The captures checked
    # here carry EtherCAT as a raw EtherType, which does not need IP.
    with open(capture, "rb") as f:
        return subprocess.run(
            ["tshark", "--disable-protocol", "ip", "-r", "-", *args],
            stdin=f,
            check=True,
            capture_output=True,
            text=True,
        ).stdout


def tshark_problems(capture):
    """Frames Wireshark marks malformed or flags with an expert warning or error."""
    return tshark(capture, "-Y", "_ws.malformed || _ws.expert.severity >= warning").strip()


def tshark_counts(capture):
    out = tshark(capture, "-Y", "ecat_mailbox", "-T", "json")
    c = Counter()
    for pkt in json.loads(out or "[]"):
        layers = pkt["_source"]["layers"]
        src = layers["eth"]["eth.src"]
        if not int(src.split(":")[0], 16) & 0x02:
            continue  # TX copy; the returning frame says whether the access succeeded
        for key, dg in layers["ecat"].items():
            if not isinstance(dg, dict) or "ecat_mailbox" not in dg:
                continue
            if int(dg.get("ecat.cnt", "0")) == 0:
                continue  # rejected write / empty read
            cmd = int(dg["Header"]["ecat.cmd"], 16)
            if cmd not in (FPRD, FPWR):
                continue
            f = flat(dg["ecat_mailbox"], {})
            length = int(f.get("ecat_mailbox.length", "0"))
            if length == 0:
                continue
            proto = TYPES.get(int(f["ecat_mailbox.type"]), "Reserved")
            c[(proto, "out" if cmd == FPWR else "in")] += 1
            if proto == "CoE":
                if f.get("ecat_mailbox.coe.sdoreq") == "4" or f.get("ecat_mailbox.coe.sdores") == "4":
                    c["coe_aborts"] += 1
                if f.get("ecat_mailbox.coe.type") == "1":
                    c["coe_emergencies"] += 1
            elif proto == "EoE":
                if f.get("ecat_mailbox.eoe.type", "0") == "0" and f.get("ecat_mailbox.eoe.last") == "1":
                    c["eoe_frames"] += 1
            elif proto == "FoE":
                if f.get("ecat_mailbox.foe_opmode") == "0x03":
                    c["foe_data_bytes"] += length - 6
    return c


def report_counts(path):
    m = json.load(open(path))["mailbox"]
    c = Counter()
    for p in m["protocols"]:
        c[(p["protocol"], "out")] += p["messages_out"]
        c[(p["protocol"], "in")] += p["messages_in"]
    # Repeats are messages Wireshark also sees; FlowLab drops them from the protocol counts.
    repeats = sum(ch["repeats"] for ch in m["channels"])
    c["coe_aborts"] = m["coe"]["aborted"]
    c["coe_emergencies"] = m["coe"]["emergencies_total"]
    c["eoe_frames"] = m["eoe"]["frames_total"]
    c["foe_data_bytes"] = sum(t["bytes"] for t in m["foe"])
    return c, repeats


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    problems = tshark_problems(sys.argv[1])
    if problems:
        print("Wireshark reports malformed frames or expert warnings:")
        print(problems[:4000])
        return 1
    ws = tshark_counts(sys.argv[1])
    fl, repeats = report_counts(sys.argv[2])
    keys = sorted(set(ws) | set(fl), key=str)
    bad = 0
    print(f"{'quantity':<24}{'wireshark':>12}{'flowlab':>12}")
    for k in keys:
        name = k if isinstance(k, str) else f"{k[0]} {k[1]}"
        mark = "" if ws[k] == fl[k] else "   <-- differs"
        bad += ws[k] != fl[k]
        print(f"{name:<24}{ws[k]:>12}{fl[k]:>12}{mark}")
    if repeats:
        print(f"note: {repeats} repeated messages dropped by FlowLab are counted by Wireshark")
    print("OK" if bad == 0 else f"{bad} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
