import { Table, type Column } from "@vitavision/ui";

import type { Distribution } from "../api";
import { count, ns } from "../lib/format";

type Row = { name: string; d: Distribution | null };

const COLUMNS: Column<Row>[] = [
  { key: "name", header: "Quantity", cell: (r) => r.name },
  { key: "n", header: "n", numeric: true, cell: (r) => (r.d ? count(r.d.count) : "–") },
  { key: "min", header: "min", numeric: true, cell: (r) => (r.d ? ns(r.d.min) : "–") },
  { key: "p1", header: "p1", numeric: true, cell: (r) => (r.d ? ns(r.d.p1) : "–") },
  { key: "p50", header: "median", numeric: true, cell: (r) => (r.d ? ns(r.d.p50) : "–") },
  { key: "p99", header: "p99", numeric: true, cell: (r) => (r.d ? ns(r.d.p99) : "–") },
  { key: "max", header: "max", numeric: true, cell: (r) => (r.d ? ns(r.d.max) : "–") },
  { key: "mean", header: "mean", numeric: true, cell: (r) => (r.d ? ns(r.d.mean) : "–") },
  { key: "std", header: "σ", numeric: true, cell: (r) => (r.d ? ns(r.d.std) : "–") },
];

/** Summary statistics of several time distributions, one per row. */
export function DistributionTable({ rows }: { rows: Row[] }) {
  return <Table columns={COLUMNS} rows={rows} rowKey={(r) => r.name} />;
}
