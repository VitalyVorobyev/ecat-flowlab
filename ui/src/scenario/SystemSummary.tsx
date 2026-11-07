import type { MailboxPolicy, SimSpec } from "../api";
import { bitRate, hex, ns } from "../lib/format";
import { protocolDetail, slaveLine, transferLine } from "./spec";

const FIRST_STATION = 0x1001;

export function policyText(p: MailboxPolicy): string {
  switch (p.kind) {
    case "fixed_quota":
      return `fixed quota, ${p.accesses_per_cycle} mailbox datagrams per cycle`;
    case "round_robin":
      return `round robin, ${p.slaves_per_cycle} slave(s) per cycle`;
    case "opportunistic":
      return `opportunistic, up to ${p.bus_budget_percent} % of the cycle, ${ns(p.turnaround_ns)} turnaround`;
  }
}

/** What a scenario is, in words: master, slaves, transfers. */
export function SystemSummary({ spec }: { spec: SimSpec }) {
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
      <dt className="text-fg-muted">Master</dt>
      <dd>
        {ns(spec.cycle_time_us * 1000)} cycle on {bitRate(spec.link_bps)},{" "}
        {policyText(spec.mailbox_policy)}
        {spec.master_jitter_ns ? `, jitter σ ${ns(spec.master_jitter_ns)}` : ""}
      </dd>
      <dt className="text-fg-muted">Slaves</dt>
      <dd className="flex flex-col">
        {spec.slaves.map((s, i) => (
          // oxlint-disable-next-line react/no-array-index-key -- position is the identity (ring order, transfer number)
          <span key={i}>
            <span className="font-mono text-xs text-fg-muted">{hex(FIRST_STATION + i)}</span>{" "}
            {s.name} <span className="font-mono text-xs text-fg-muted">{slaveLine(s)}</span>
          </span>
        ))}
      </dd>
      <dt className="text-fg-muted">Transfers</dt>
      <dd className="flex flex-col">
        {spec.transfers.map((t, i) => (
          // oxlint-disable-next-line react/no-array-index-key -- position is the identity (ring order, transfer number)
          <span key={i}>
            <span className="font-mono text-xs text-fg-muted">#{i}</span> {transferLine(t, spec)}{" "}
            <span className="font-mono text-xs text-fg-muted">{protocolDetail(t.protocol)}</span>
          </span>
        ))}
      </dd>
    </dl>
  );
}
