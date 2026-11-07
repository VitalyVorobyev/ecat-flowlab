#!/usr/bin/env python3
"""Regenerate the simulated results of the bulk-transfer report.

Writes the study and simulator specs to configs/studies/bulk-transfer/report/, runs them with
`flowlab`, and renders Markdown tables to book/src/studies/bulk-transfer/generated/, which the
report includes, and the same results as findings.json for the interactive findings page. Every
number is a simulation result for the stated parameters.

    cargo build --release -p flowlab-cli
    python3 scripts/bulk-transfer-report.py [--flowlab target/release/flowlab]

CI runs it and fails if the committed specs or tables differ from what the code produces.
"""

import argparse
import copy
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "configs/studies/bulk-transfer/report"
TABLES = ROOT / "book/src/studies/bulk-transfer/generated"

PAYLOAD = 10_000_000  # 10 MB, decimal
SLAVE = 2  # index of the bulk endpoint in the line

# -- the modelled system ------------------------------------------------------------------------


def pdi(rate=20_000_000, overhead=5_000, access="whole_buffer"):
    return {"read_bps": rate, "write_bps": rate, "access_overhead_ns": overhead, "mailbox_access": access}


def slave(name, out=0, inp=0, mailbox=0, **kw):
    s = {
        "name": name,
        "outputs_bytes": out,
        "inputs_bytes": inp,
        "forwarding_delay_ns": 700,
        "mailbox_out_bytes": mailbox,
        "mailbox_in_bytes": mailbox,
        "dpram_bytes": 4096,
        "pdi": pdi(),
        "app_processing_ns": 20_000,
    }
    s.update(kw)
    return s


FOE = {"kind": "foe"}
EOE = {"kind": "eoe", "mtu_bytes": 1500}
COE = {"kind": "coe_segmented"}


def pdo(bytes_per_cycle, window=2):
    return {
        "kind": "pdo_channel",
        "bytes_per_cycle": bytes_per_cycle,
        "window": window,
        "retransmit_after_cycles": 8,
    }


def fixed(n):
    return {"kind": "fixed_quota", "accesses_per_cycle": n}


def opportunistic(budget):
    return {"kind": "opportunistic", "bus_budget_percent": float(budget), "turnaround_ns": 10_000}


def base(protocol=FOE, mailbox=512, cycle_us=1000, policy=None, description=""):
    """PC master → coupler → I/O slave → bulk endpoint (an ESC with 4 KiB RAM behind a PDI)."""
    return {
        "seed": 1,
        "description": description or "Bulk-transfer report: one 10 MB transfer to the bulk endpoint",
        "link_bps": 100_000_000,
        "cycle_time_us": cycle_us,
        "master_jitter_ns": 2_000,
        "slaves": [
            slave("coupler"),
            slave("io", 8, 8),
            slave("bulk-endpoint", 8, 8, mailbox),
        ],
        "mailbox_policy": policy or fixed(2),
        "transfers": [
            {
                "slave": SLAVE,
                "direction": "master_to_slave",
                "payload_bytes": PAYLOAD,
                "protocol": protocol,
                "start_ms": 0.0,
            }
        ],
        "max_duration_ms": 900_000,
        "faults": {"frame_loss_probability": 0.0},
        "capture": {"enabled": False, "max_frames": 200_000},
        "start_unix_s": 1_767_225_600,
    }


def param(path, values, also=()):
    return {"path": path, "also": list(also), "values": values}


MAILBOX = ("/slaves/2/mailbox_out_bytes", ["/slaves/2/mailbox_in_bytes"])
RATE = ("/slaves/2/pdi/read_bps", ["/slaves/2/pdi/write_bps"])
TIME = {"metric": {"kind": "transfer_time_s", "transfer": 0}, "goal": "minimize"}
LINK = {"metric": {"kind": "link_utilization_percent"}, "goal": "minimize"}
PDI_BUSY = {"metric": {"kind": "pdi_busy_percent_max"}, "goal": "minimize"}
RAM = [{"metric": {"kind": "dpram_used_percent_max"}, "max": 100.0, "min": None}]


def study(description, b, parameters, objectives=(TIME, LINK, PDI_BUSY)):
    return {
        "description": description,
        "base": b,
        "parameters": parameters,
        "sampling": {"kind": "grid"},
        "objectives": list(objectives),
        "constraints": RAM,
        "targets": [],
        "max_runs": 400,
    }


POLICIES = [fixed(1), fixed(2), fixed(4), opportunistic(60), opportunistic(90)]
TWO_POLICIES = [fixed(2), opportunistic(90)]
MAILBOXES = [128, 256, 512, 522, 778, 1024, 1484]
RATES = [2_500_000, 5_000_000, 10_000_000, 20_000_000, 40_000_000, 80_000_000]
OVERHEADS = [1_000, 5_000, 20_000, 50_000, 100_000]


def pdi_study(access):
    b = base(mailbox=1024, policy=opportunistic(90))
    b["slaves"][SLAVE]["pdi"]["mailbox_access"] = access
    return study(
        f"FoE write, 1024 B mailboxes, opportunistic master: PDI rate × per-access overhead ({access})",
        b,
        [param(RATE[0], RATES, RATE[1]), param("/slaves/2/pdi/access_overhead_ns", OVERHEADS)],
    )


def pdo_base(cycle_us=1000):
    b = base(protocol=pdo(512), mailbox=128, cycle_us=cycle_us)
    return b


STUDIES = {
    "01-protocol-policy": study(
        "Protocol × master mailbox policy, 512 B mailboxes, 1 ms cycle",
        base(),
        [param("/transfers/0/protocol", [FOE, EOE, COE]), param("/mailbox_policy", POLICIES)],
    ),
    "02-mailbox-foe": study(
        "FoE write: mailbox size × master policy, 1 ms cycle",
        base(),
        [param(MAILBOX[0], MAILBOXES, MAILBOX[1]), param("/mailbox_policy", TWO_POLICIES)],
    ),
    "03-mailbox-eoe": study(
        "EoE (UDP-like, 1500 B MTU): mailbox size × master policy, 1 ms cycle",
        base(protocol=EOE),
        [param(MAILBOX[0], MAILBOXES, MAILBOX[1]), param("/mailbox_policy", TWO_POLICIES)],
    ),
    "04-cycle": study(
        "FoE write, 1024 B mailboxes: cycle time × master policy",
        base(mailbox=1024),
        [param("/cycle_time_us", [250, 500, 1000, 2000, 4000]), param("/mailbox_policy", TWO_POLICIES)],
    ),
    "05-pdi-whole-buffer": pdi_study("whole_buffer"),
    "06-pdi-message-only": pdi_study("message_only"),
    "07-pdo-size": study(
        "PDO channel (window 2), 128 B mailboxes: bytes per cycle × cycle time",
        pdo_base(),
        [
            param("/transfers/0/protocol", [pdo(n) for n in (256, 512, 1024, 1260, 1270)]),
            param("/cycle_time_us", [250, 500, 1000]),
        ],
    ),
    "08-pdo-pdi": study(
        "PDO channel 1024 B/cycle at 250 µs, 128 B mailboxes: window × PDI rate",
        pdo_base(250),
        [
            param("/transfers/0/protocol", [pdo(1024, w) for w in (1, 2, 4)]),
            param(RATE[0], [10_000_000, 20_000_000, 40_000_000, 80_000_000], RATE[1]),
        ],
    ),
}


def steps():
    """One configuration per optimisation step, each changing one thing."""

    def with_pdi(b, **kw):
        b["slaves"][SLAVE]["pdi"].update(kw)
        return b

    out = []
    s = base(mailbox=128, description="1. FoE, 128 B mailboxes, master serves 2 mailbox datagrams per 1 ms cycle")
    out.append(s)
    s = base(mailbox=1024, description="2. … 1024 B mailboxes")
    out.append(s)
    s = base(mailbox=1024, cycle_us=250, description="3. … 250 µs cycle")
    out.append(s)
    s = base(mailbox=1024, policy=opportunistic(90), description="4. 1 ms cycle, master polls in idle time (≤ 90 %)")
    out.append(s)
    s = with_pdi(
        base(mailbox=1024, policy=opportunistic(90), description="5. … PDI moves only the message"),
        mailbox_access="message_only",
    )
    out.append(s)
    s = with_pdi(
        base(mailbox=1024, policy=opportunistic(90), description="6. … PDI at 80 Mbit/s effective"),
        mailbox_access="message_only",
        read_bps=80_000_000,
        write_bps=80_000_000,
    )
    out.append(s)
    s = base(protocol=pdo(1024), mailbox=128, cycle_us=500, description="7. PDO channel 1024 B per 500 µs cycle instead")
    out.append(s)
    s = with_pdi(
        base(protocol=pdo(1024), mailbox=128, cycle_us=250, description="8. … 250 µs cycle, PDI at 40 Mbit/s effective"),
        read_bps=40_000_000,
        write_bps=40_000_000,
    )
    out.append(s)
    return out


# -- rendering ----------------------------------------------------------------------------------


def label(path, value):
    if isinstance(value, dict):
        k = value.get("kind")
        if k == "fixed_quota":
            return f"fixed, {value['accesses_per_cycle']}/cycle"
        if k == "round_robin":
            return f"round robin, {value['slaves_per_cycle']}/cycle"
        if k == "opportunistic":
            return f"idle time ≤ {value['bus_budget_percent']:g} %"
        if k == "pdo_channel":
            if "window" in path:
                return str(value["window"])
            return f"{value['bytes_per_cycle']} B, window {value['window']}"
        return {"foe": "FoE", "eoe": "EoE", "coe_segmented": "CoE segmented"}.get(k, k)
    if path.endswith("_bps"):
        return f"{value / 1e6:g} Mbit/s"
    if path.endswith("_ns"):
        return f"{value / 1e3:g} µs"
    if path.endswith("_us"):
        return f"{value} µs"
    if path.endswith("_bytes"):
        return f"{value} B"
    return str(value)


HEADERS = {
    "/mailbox_policy": "master policy",
    "/transfers/0/protocol": "protocol",
    MAILBOX[0]: "mailbox",
    "/cycle_time_us": "cycle",
    RATE[0]: "PDI rate",
    "/slaves/2/pdi/access_overhead_ns": "PDI overhead per access",
}

OBJECTIVE = {
    0: ("10 MB transfer time, s", "{:.2f}"),
    1: ("link utilization, %", "{:.0f}"),
    2: ("PDI busy, %", "{:.0f}"),
}


def pivot(spec, result, objective, row_label=None):
    """Rows: first parameter; columns: second parameter; cell: one objective."""
    p0, p1 = spec["parameters"]
    rows, cols, cell = [], [], {}
    for run in result["runs"]:
        v0, v1 = (json.loads(v) for v in run["values"])
        a, b = label(p0["path"], v0), label(p1["path"], v1)
        rows += [a] if a not in rows else []
        cols += [b] if b not in cols else []
        if run["feasible"]:
            title, fmt = OBJECTIVE[objective]
            cell[a, b] = fmt.format(run["objectives"][objective])
        else:
            cell[a, b] = "✗"
    title = OBJECTIVE[objective][0]
    head = f"| {row_label or HEADERS.get(p0['path'], p0['path'])} ↓ · {HEADERS.get(p1['path'], p1['path'])} → |"
    lines = [head + "".join(f" {c} |" for c in cols), "|---|" + "---:|" * len(cols)]
    for a in rows:
        lines.append(f"| {a} |" + "".join(f" {cell[a, b]} |" for b in cols))
    reasons = sorted({r["reason"] for r in result["runs"] if not r["feasible"]})
    notes = "".join(f"\n\n✗ — infeasible: {r}" for r in reasons)
    return f"*{title}*\n\n" + "\n".join(lines) + notes


def ms_or_s(ns):
    return f"{ns / 1e9:.2f} s" if ns >= 1e9 else f"{ns / 1e6:.0f} ms"


def steps_table(results):
    lines = [
        "| Configuration | 10 MB time | Goodput | Link | PDI busy | Messages | Resent chunks |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for r in results:
        t = r["transfers"][0]
        s = r["slaves"][SLAVE]
        assert t["verified"], r["description"]
        msgs = t["messages_to_slave"] + t["messages_from_slave"]
        lines.append(
            f"| {r['description']} | {ms_or_s(t['duration_ns'])} | {t['goodput_bps'] / 1e6:.2f} Mbit/s"
            f" | {r['link_utilization_percent']:.0f} % | {s['pdi_busy_percent']:.0f} % | {msgs:,}"
            f" | {t['retransmitted_chunks']:,} |"
        )
    t = results[0]["transfers"][0]
    lines.append(
        f"| Payload alone on the 100 Mbit/s link (bound) | {ms_or_s(t['physical_bound_ns'])} | 100.00 Mbit/s | | | | |"
    )
    lines.append(
        f"| TCP over a dedicated 100 Mbit/s Ethernet link (reference) | {ms_or_s(t['ethernet_tcp_reference_ns'])}"
        f" | {PAYLOAD * 8 / t['ethernet_tcp_reference_ns'] * 1e3:.2f} Mbit/s | | | | |"
    )
    return "\n".join(lines)


TABLES_OF = {
    "01-protocol-policy": [(0, None), (1, None)],
    "02-mailbox-foe": [(0, None), (2, None)],
    "03-mailbox-eoe": [(0, None), (2, None)],
    "04-cycle": [(0, None), (1, None)],
    "05-pdi-whole-buffer": [(0, None)],
    "06-pdi-message-only": [(0, None)],
    "07-pdo-size": [(0, "channel"), (2, "channel")],
    "08-pdo-pdi": [(0, "window"), (2, "window")],
}


def sig(x, digits=5):
    """Round to significant digits so the committed JSON is compact and stable."""
    return float(f"{x:.{digits}g}") if isinstance(x, float) else x


def study_data(name, spec, result):
    return {
        "description": spec["description"],
        "spec": f"configs/studies/bulk-transfer/report/{name}.json",
        "parameters": [HEADERS.get(p["path"], p["path"]) for p in spec["parameters"]],
        "objectives": [OBJECTIVE[i][0] for i in range(len(spec["objectives"]))],
        "runs": [
            {
                "labels": [label(p["path"], json.loads(v)) for p, v in zip(spec["parameters"], run["values"])],
                "numeric": [sig(n) if n is not None else None for n in run["numeric"]],
                "feasible": run["feasible"],
                "reason": run["reason"],
                "objectives": [sig(o) for o in run["objectives"]] if run["feasible"] else None,
            }
            for run in result["runs"]
        ],
    }


def steps_data(specs, results):
    out = []
    for i, (spec, r) in enumerate(zip(specs, results), 1):
        t, s = r["transfers"][0], r["slaves"][SLAVE]
        out.append(
            {
                "step": i,
                "description": r["description"],
                "spec": f"configs/studies/bulk-transfer/report/step-{i}.json",
                "protocol": "PDO channel" if spec["transfers"][0]["protocol"]["kind"] == "pdo_channel" else "FoE",
                "duration_s": sig(t["duration_ns"] / 1e9),
                "goodput_mbps": sig(t["goodput_bps"] / 1e6),
                "link_percent": sig(r["link_utilization_percent"]),
                "pdi_busy_percent": sig(s["pdi_busy_percent"]),
                "messages": t["messages_to_slave"] + t["messages_from_slave"],
                "retransmitted_chunks": t["retransmitted_chunks"],
            }
        )
    return out


def dump(obj):
    return json.dumps(obj, indent=2, ensure_ascii=False) + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--flowlab", default=str(ROOT / "target/release/flowlab"))
    args = ap.parse_args()
    SPECS.mkdir(parents=True, exist_ok=True)
    TABLES.mkdir(parents=True, exist_ok=True)
    stamp = "<!-- Generated by scripts/bulk-transfer-report.py. Do not edit. -->\n"
    findings = {
        "source": "simulated",
        "generator": "scripts/bulk-transfer-report.py",
        "payload_bytes": PAYLOAD,
        "studies": {},
    }
    with tempfile.TemporaryDirectory() as tmp:
        for name, spec in STUDIES.items():
            path = SPECS / f"{name}.json"
            path.write_text(dump(spec))
            out = Path(tmp) / f"{name}.result.json"
            subprocess.run([args.flowlab, "study", str(path), "--result", str(out)], check=True, capture_output=True)
            result = json.loads(out.read_text())
            assert result["meta"]["source"] == "simulated"
            body = [f"{stamp}\n**Simulated.** {spec['description']}. `flowlab study {path.relative_to(ROOT)}`"]
            body += [pivot(spec, result, o, row) for o, row in TABLES_OF[name]]
            (TABLES / f"{name}.md").write_text("\n\n".join(body) + "\n")
            findings["studies"][name] = study_data(name, spec, result)
            print(f"{name}: {len(result['runs'])} runs", file=sys.stderr)
        files, step_specs = [], steps()
        for i, s in enumerate(step_specs, 1):
            path = SPECS / f"step-{i}.json"
            path.write_text(dump(s))
            files.append(str(path))
        out = Path(tmp) / "steps.json"
        subprocess.run([args.flowlab, "simulate", *files, "--result", str(out)], check=True, capture_output=True)
        results = json.loads(out.read_text())
        assert all(r["meta"]["source"] == "simulated" for r in results)
        cmd = "flowlab simulate configs/studies/bulk-transfer/report/step-{1..8}.json"
        (TABLES / "steps.md").write_text(f"{stamp}\n**Simulated.** `{cmd}`\n\n{steps_table(results)}\n")
        print(f"steps: {len(results)} runs", file=sys.stderr)
        t = results[0]["transfers"][0]
        findings["physical_bound_s"] = sig(t["physical_bound_ns"] / 1e9)
        findings["tcp_reference_s"] = sig(t["ethernet_tcp_reference_ns"] / 1e9)
        findings["steps"] = steps_data(step_specs, results)
        (TABLES / "findings.json").write_text(json.dumps(findings, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
