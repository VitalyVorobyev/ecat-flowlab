import { Button, Field, Input, NumberInput, SegmentedControl, Select, cn } from "@vitavision/ui";
import { ChevronDown, ChevronRight, Copy, Plus, Trash2 } from "lucide-react";
import { useState, type ReactNode } from "react";

import type {
  MailboxPolicy,
  PdiSpec,
  SimSlave,
  SimSpec,
  TransferProtocol,
  TransferSpec,
} from "../api";
import { hex } from "../lib/format";
import {
  PAYLOAD_UNITS,
  POLICY_OPTIONS,
  PROTOCOL_OPTIONS,
  type PayloadUnit,
  type PolicyKind,
  type ProtocolKind,
  defaultPolicy,
  defaultProtocol,
  newSlave,
  newTransfer,
  payloadUnit,
  protocolDetail,
  removeSlave,
  slaveLine,
  transferLine,
} from "./spec";

/** First station address the simulator assigns (ring order from here). */
const FIRST_STATION = 0x1001;

function Group({
  title,
  hint,
  actions,
  children,
}: {
  title: string;
  hint?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="border-b border-line px-4 py-3">
      <div className="mb-2 flex items-center gap-2">
        <h3 className="text-xs font-semibold tracking-wide text-fg-muted uppercase">{title}</h3>
        {hint ? <span className="truncate text-xs text-fg-subtle">{hint}</span> : null}
        <div className="ml-auto flex items-center gap-1">{actions}</div>
      </div>
      {children}
    </section>
  );
}

function Num({
  label,
  value,
  unit,
  onChange,
  min = 0,
  max,
  scale = 1,
  integer = true,
  hint,
}: {
  label: string;
  value: number;
  unit?: string;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  /** Shown value = stored value / scale (e.g. 1e6 to show bit/s as Mbit/s). */
  scale?: number;
  /** Round the stored value to an integer. */
  integer?: boolean;
  hint?: string;
}) {
  return (
    <Field label={label} annotation={hint}>
      <NumberInput
        value={value / scale}
        unit={unit}
        min={min}
        max={max === undefined ? undefined : max / scale}
        onValueChange={(v) => onChange(integer ? Math.round(v * scale) : v * scale)}
      />
    </Field>
  );
}

function Row({
  open,
  onToggle,
  index,
  title,
  sub,
  actions,
}: {
  open: boolean;
  onToggle: () => void;
  index: string;
  title: string;
  sub: string;
  actions: ReactNode;
}) {
  return (
    <div className={cn("flex items-center gap-1 rounded-md", open && "bg-raised")}>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 py-1.5 text-left hover:bg-raised"
      >
        {open ? (
          <ChevronDown className="size-3.5 shrink-0 text-fg-subtle" />
        ) : (
          <ChevronRight className="size-3.5 shrink-0 text-fg-subtle" />
        )}
        <span className="w-12 shrink-0 font-mono text-[11px] text-fg-subtle">{index}</span>
        <span className="min-w-0">
          <span className="block truncate text-sm text-fg">{title}</span>
          <span className="block truncate font-mono text-[11px] text-fg-muted">{sub}</span>
        </span>
      </button>
      <div className="flex shrink-0 items-center pr-1">{actions}</div>
    </div>
  );
}

function IconButton({
  label,
  onClick,
  disabled,
  children,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <Button
      variant="ghost"
      size="sm"
      aria-label={label}
      title={label}
      onClick={onClick}
      disabled={disabled}
      className="px-1.5"
    >
      {children}
    </Button>
  );
}

function PolicyFields({
  policy,
  onChange,
}: {
  policy: MailboxPolicy;
  onChange: (p: MailboxPolicy) => void;
}) {
  const hint = POLICY_OPTIONS.find((o) => o.value === policy.kind)?.hint;
  return (
    <div className="flex flex-col gap-2">
      <Field label="Mailbox policy" as="group" description={hint}>
        <SegmentedControl
          aria-label="Mailbox policy"
          value={policy.kind}
          options={POLICY_OPTIONS.map(({ value, label }) => ({ value, label }))}
          onValueChange={(v) => v && onChange(defaultPolicy(v as PolicyKind))}
        />
      </Field>
      <div className="grid grid-cols-2 gap-x-3 gap-y-2">
        {policy.kind === "fixed_quota" ? (
          <Num
            label="Datagrams per cycle"
            value={policy.accesses_per_cycle}
            min={1}
            onChange={(v) => onChange({ ...policy, accesses_per_cycle: v })}
          />
        ) : null}
        {policy.kind === "round_robin" ? (
          <Num
            label="Slaves per cycle"
            value={policy.slaves_per_cycle}
            min={1}
            onChange={(v) => onChange({ ...policy, slaves_per_cycle: v })}
          />
        ) : null}
        {policy.kind === "opportunistic" ? (
          <>
            <Num
              label="Share of the cycle"
              unit="%"
              value={policy.bus_budget_percent}
              max={100}
              integer={false}
              onChange={(v) => onChange({ ...policy, bus_budget_percent: v })}
            />
            <Num
              label="Master turnaround"
              unit="µs"
              scale={1e3}
              value={policy.turnaround_ns}
              onChange={(v) => onChange({ ...policy, turnaround_ns: v })}
            />
          </>
        ) : null}
      </div>
    </div>
  );
}

function SlaveFields({ slave, onChange }: { slave: SimSlave; onChange: (s: SimSlave) => void }) {
  const set = <K extends keyof SimSlave>(k: K, v: SimSlave[K]) => onChange({ ...slave, [k]: v });
  const pdi = <K extends keyof PdiSpec>(k: K, v: PdiSpec[K]) =>
    onChange({ ...slave, pdi: { ...slave.pdi, [k]: v } });
  return (
    <div className="grid grid-cols-2 gap-x-3 gap-y-2 px-2 pt-1 pb-3">
      <Field label="Name" className="col-span-2">
        <Input value={slave.name} onChange={(e) => set("name", e.target.value)} />
      </Field>
      <Num
        label="Outputs"
        unit="B"
        value={slave.outputs_bytes}
        max={65535}
        onChange={(v) => set("outputs_bytes", v)}
      />
      <Num
        label="Inputs"
        unit="B"
        value={slave.inputs_bytes}
        max={65535}
        onChange={(v) => set("inputs_bytes", v)}
      />
      <Num
        label="Mailbox out"
        unit="B"
        value={slave.mailbox_out_bytes}
        max={1484}
        hint="0 = none"
        onChange={(v) => set("mailbox_out_bytes", v)}
      />
      <Num
        label="Mailbox in"
        unit="B"
        value={slave.mailbox_in_bytes}
        max={1484}
        onChange={(v) => set("mailbox_in_bytes", v)}
      />
      <Num
        label="ESC RAM"
        unit="B"
        value={slave.dpram_bytes}
        onChange={(v) => set("dpram_bytes", v)}
      />
      <Num
        label="Forwarding delay"
        unit="ns"
        value={slave.forwarding_delay_ns}
        onChange={(v) => set("forwarding_delay_ns", v)}
      />
      <p className="col-span-2 mt-1 text-xs font-medium text-fg-muted">
        PDI: the application's access to the ESC (SPI, parallel bus)
      </p>
      <Num
        label="Read rate"
        unit="Mbit/s"
        scale={1e6}
        value={slave.pdi.read_bps}
        integer
        onChange={(v) => pdi("read_bps", v)}
      />
      <Num
        label="Write rate"
        unit="Mbit/s"
        scale={1e6}
        value={slave.pdi.write_bps}
        onChange={(v) => pdi("write_bps", v)}
      />
      <Num
        label="Overhead per access"
        unit="µs"
        scale={1e3}
        value={slave.pdi.access_overhead_ns}
        onChange={(v) => pdi("access_overhead_ns", v)}
      />
      <Field label="Mailbox access">
        <Select
          aria-label="Mailbox access"
          value={slave.pdi.mailbox_access}
          options={[
            { value: "whole_buffer", label: "Whole buffer" },
            { value: "message_only", label: "Message only" },
          ]}
          onValueChange={(v) => v && pdi("mailbox_access", v as PdiSpec["mailbox_access"])}
        />
      </Field>
      <Num
        label="Application per message"
        unit="µs"
        scale={1e3}
        value={slave.app_processing_ns}
        onChange={(v) => set("app_processing_ns", v)}
      />
    </div>
  );
}

function PayloadField({ bytes, onChange }: { bytes: number; onChange: (n: number) => void }) {
  const [unit, setUnit] = useState<PayloadUnit>(() => payloadUnit(bytes));
  return (
    <Field label="Payload" className="col-span-2">
      <div className="flex gap-2">
        <NumberInput
          className="min-w-0 flex-1"
          value={bytes / unit.factor}
          min={1 / unit.factor}
          onValueChange={(v) => onChange(Math.max(1, Math.round(v * unit.factor)))}
        />
        <Select
          aria-label="Payload unit"
          className="w-24"
          value={unit.label}
          options={PAYLOAD_UNITS.map((u) => ({ value: u.label, label: u.label }))}
          onValueChange={(v) => {
            const u = PAYLOAD_UNITS.find((x) => x.label === v);
            if (!u) return;
            // Keep the number typed, change what it means: 10 MB → 10 MiB.
            onChange(Math.max(1, Math.round((bytes / unit.factor) * u.factor)));
            setUnit(u);
          }}
        />
      </div>
    </Field>
  );
}

function TransferFields({
  transfer,
  spec,
  onChange,
}: {
  transfer: TransferSpec;
  spec: SimSpec;
  onChange: (t: TransferSpec) => void;
}) {
  const set = <K extends keyof TransferSpec>(k: K, v: TransferSpec[K]) =>
    onChange({ ...transfer, [k]: v });
  const p = transfer.protocol;
  const proto = (next: TransferProtocol) => set("protocol", next);
  const mailbox = p.kind !== "pdo_channel";
  return (
    <div className="grid grid-cols-2 gap-x-3 gap-y-2 px-2 pt-1 pb-3">
      <Field label="Protocol" className="col-span-2">
        <Select
          aria-label="Protocol"
          value={p.kind}
          options={PROTOCOL_OPTIONS}
          onValueChange={(v) => v && proto(defaultProtocol(v as ProtocolKind))}
        />
      </Field>
      <Field label="Direction" as="group" className="col-span-2">
        <SegmentedControl
          aria-label="Direction"
          value={transfer.direction}
          options={[
            { value: "master_to_slave", label: "Master → slave" },
            { value: "slave_to_master", label: "Slave → master" },
          ]}
          onValueChange={(v) => v && set("direction", v as TransferSpec["direction"])}
        />
      </Field>
      <Field label="Slave" className="col-span-2">
        <Select
          aria-label="Slave"
          value={String(transfer.slave)}
          options={spec.slaves.map((s, i) => ({
            value: String(i),
            label: `${hex(FIRST_STATION + i)} ${s.name}`,
            ...(mailbox && s.mailbox_out_bytes === 0 ? { note: "no mailbox" } : {}),
          }))}
          onValueChange={(v) => v && set("slave", Number(v))}
        />
      </Field>
      <PayloadField bytes={transfer.payload_bytes} onChange={(n) => set("payload_bytes", n)} />
      {p.kind === "eoe" ? (
        <Num
          label="IP MTU"
          unit="B"
          value={p.mtu_bytes}
          min={68}
          max={1500}
          onChange={(v) => proto({ ...p, mtu_bytes: v })}
        />
      ) : null}
      {p.kind === "voe" ? (
        <Num
          label="Window"
          unit="DATA per ACK"
          value={p.window}
          min={1}
          max={4096}
          onChange={(v) => proto({ ...p, window: v })}
        />
      ) : null}
      {p.kind === "pdo_channel" ? (
        <>
          <Num
            label="Bytes per cycle"
            unit="B"
            value={p.bytes_per_cycle}
            min={9}
            max={65535}
            onChange={(v) => proto({ ...p, bytes_per_cycle: v })}
          />
          <Num
            label="Window"
            unit="chunks"
            value={p.window}
            min={1}
            onChange={(v) => proto({ ...p, window: v })}
          />
          <Num
            label="Resend after"
            unit="cycles"
            value={p.retransmit_after_cycles}
            min={1}
            onChange={(v) => proto({ ...p, retransmit_after_cycles: v })}
          />
        </>
      ) : null}
      <Num
        label="Start after"
        unit="ms"
        value={transfer.start_ms}
        integer={false}
        onChange={(v) => set("start_ms", v)}
      />
    </div>
  );
}

/**
 * The scenario as a form: master and link, the slaves in ring order, the transfers. Values are
 * shown in the units people use (Mbit/s, µs, MB) and stored in the spec's units.
 */
export function SetupEditor({
  spec,
  slaveTemplate,
  onChange,
}: {
  spec: SimSpec;
  /** A slave with the simulator's defaults; adding a slave waits for it. */
  slaveTemplate: SimSlave | null;
  onChange: (s: SimSpec) => void;
}) {
  const [openSlave, setOpenSlave] = useState<number | null>(null);
  const [openTransfer, setOpenTransfer] = useState<number | null>(0);
  const set = <K extends keyof SimSpec>(k: K, v: SimSpec[K]) => onChange({ ...spec, [k]: v });
  const setSlave = (i: number, s: SimSlave) =>
    set(
      "slaves",
      spec.slaves.map((x, k) => (k === i ? s : x)),
    );
  const setTransfer = (i: number, t: TransferSpec) =>
    set(
      "transfers",
      spec.transfers.map((x, k) => (k === i ? t : x)),
    );

  return (
    <div className="flex flex-col">
      <Group title="Master and link">
        <div className="grid grid-cols-2 gap-x-3 gap-y-2">
          <Num
            label="Cycle time"
            unit="µs"
            value={spec.cycle_time_us}
            min={1}
            onChange={(v) => set("cycle_time_us", v)}
          />
          <Num
            label="Link rate"
            unit="Mbit/s"
            scale={1e6}
            value={spec.link_bps}
            min={1e6}
            onChange={(v) => set("link_bps", v)}
          />
          <Num
            label="Cycle jitter (σ)"
            unit="µs"
            scale={1e3}
            value={spec.master_jitter_ns}
            onChange={(v) => set("master_jitter_ns", v)}
          />
        </div>
        <div className="mt-3">
          <PolicyFields policy={spec.mailbox_policy} onChange={(p) => set("mailbox_policy", p)} />
        </div>
      </Group>

      <Group
        title="Slaves"
        hint="in ring order"
        actions={
          <Button
            variant="ghost"
            size="sm"
            icon={<Plus className="size-3.5" />}
            disabled={!slaveTemplate}
            onClick={() => {
              if (!slaveTemplate) return;
              set("slaves", [...spec.slaves, newSlave(slaveTemplate, spec.slaves.length)]);
              setOpenSlave(spec.slaves.length);
            }}
          >
            Add
          </Button>
        }
      >
        <div className="flex flex-col gap-0.5">
          {spec.slaves.map((s, i) => (
            // oxlint-disable-next-line react/no-array-index-key -- position is the identity (ring order, transfer number)
            <div key={i}>
              <Row
                open={openSlave === i}
                onToggle={() => setOpenSlave(openSlave === i ? null : i)}
                index={hex(FIRST_STATION + i)}
                title={s.name}
                sub={slaveLine(s)}
                actions={
                  <>
                    <IconButton
                      label={`Duplicate ${s.name}`}
                      onClick={() =>
                        set("slaves", [
                          ...spec.slaves.slice(0, i + 1),
                          { ...structuredClone(s), name: `${s.name}-copy` },
                          ...spec.slaves.slice(i + 1),
                        ])
                      }
                    >
                      <Copy className="size-3.5" />
                    </IconButton>
                    <IconButton
                      label={`Remove ${s.name} and its transfers`}
                      disabled={spec.slaves.length <= 1}
                      onClick={() => {
                        onChange(removeSlave(spec, i));
                        setOpenSlave(null);
                      }}
                    >
                      <Trash2 className="size-3.5" />
                    </IconButton>
                  </>
                }
              />
              {openSlave === i ? <SlaveFields slave={s} onChange={(x) => setSlave(i, x)} /> : null}
            </div>
          ))}
        </div>
      </Group>

      <Group
        title="Transfers"
        hint="one after another on the same slave"
        actions={
          <Button
            variant="ghost"
            size="sm"
            icon={<Plus className="size-3.5" />}
            onClick={() => {
              set("transfers", [...spec.transfers, newTransfer(spec)]);
              setOpenTransfer(spec.transfers.length);
            }}
          >
            Add
          </Button>
        }
      >
        <div className="flex flex-col gap-0.5">
          {spec.transfers.map((t, i) => (
            // oxlint-disable-next-line react/no-array-index-key -- position is the identity (ring order, transfer number)
            <div key={i}>
              <Row
                open={openTransfer === i}
                onToggle={() => setOpenTransfer(openTransfer === i ? null : i)}
                index={`#${i}`}
                title={transferLine(t, spec)}
                sub={protocolDetail(t.protocol)}
                actions={
                  <>
                    <IconButton
                      label={`Duplicate transfer ${i}`}
                      onClick={() =>
                        set("transfers", [
                          ...spec.transfers.slice(0, i + 1),
                          structuredClone(t),
                          ...spec.transfers.slice(i + 1),
                        ])
                      }
                    >
                      <Copy className="size-3.5" />
                    </IconButton>
                    <IconButton
                      label={`Remove transfer ${i}`}
                      disabled={spec.transfers.length <= 1}
                      onClick={() => {
                        set(
                          "transfers",
                          spec.transfers.filter((_, k) => k !== i),
                        );
                        setOpenTransfer(null);
                      }}
                    >
                      <Trash2 className="size-3.5" />
                    </IconButton>
                  </>
                }
              />
              {openTransfer === i ? (
                <TransferFields transfer={t} spec={spec} onChange={(x) => setTransfer(i, x)} />
              ) : null}
            </div>
          ))}
        </div>
      </Group>

      <Group title="Run">
        <div className="grid grid-cols-2 gap-x-3 gap-y-2">
          <Num
            label="Time limit"
            unit="ms"
            value={spec.max_duration_ms}
            min={1}
            onChange={(v) => set("max_duration_ms", v)}
          />
          <Num label="Seed" value={spec.seed} onChange={(v) => set("seed", v)} />
          <Num
            label="Frame loss"
            unit="%"
            scale={0.01}
            value={spec.faults.frame_loss_probability}
            integer={false}
            max={0.99}
            onChange={(v) => set("faults", { ...spec.faults, frame_loss_probability: v })}
          />
        </div>
      </Group>
    </div>
  );
}
