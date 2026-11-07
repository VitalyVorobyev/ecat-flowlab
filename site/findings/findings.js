// Findings page: renders data.json (written by scripts/bulk-transfer-report.py) with
// Observable Plot. Every value is a simulation result; nothing here is computed beyond
// filtering and formatting.
import * as Plot from "https://cdn.jsdelivr.net/npm/@observablehq/plot@0.6.16/+esm";
import * as d3 from "https://cdn.jsdelivr.net/npm/d3@7.9.0/+esm";

const REPO = "https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/";

// What each study holds fixed, for describing its runs outside the study.
const CONTEXT = {
  "01-protocol-policy": { text: "512 B mailboxes, 1 ms cycle" },
  "02-mailbox-foe": { text: "FoE, 1 ms cycle", family: "mailbox" },
  "03-mailbox-eoe": { text: "EoE, 1 ms cycle", family: "eoe" },
  "04-cycle": { text: "FoE, 1024 B mailboxes", family: "mailbox" },
  "05-pdi-whole-buffer": {
    text: "FoE, 1024 B mailboxes, idle-time master ≤ 90 %, whole-buffer SPI access",
    family: "mailbox",
  },
  "06-pdi-message-only": {
    text: "FoE, 1024 B mailboxes, idle-time master ≤ 90 %, message-only SPI access",
    family: "mailbox",
  },
  "07-pdo-size": { text: "PDO channel, 128 B mailboxes", family: "pdo" },
  "08-pdo-pdi": { text: "PDO channel, 250 µs cycle, 128 B mailboxes", family: "pdo" },
};

const FAMILIES = ["mailbox", "eoe", "pdo"];
const FAMILY_LABEL = { mailbox: "FoE / CoE", eoe: "EoE", pdo: "PDO channel" };
const SYMBOL = { mailbox: "circle", eoe: "triangle", pdo: "square" };

const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
const series = () => [css("--series-1"), css("--series-2"), css("--series-3")];
const seq = () => [1, 2, 3, 4, 5, 6, 7].map((i) => css(`--seq-${i}`));

const fmtS = (s) =>
  !Number.isFinite(s) ? "—" : s >= 100 ? `${s.toFixed(0)} s` : s >= 10 ? `${s.toFixed(1)} s` : `${s.toFixed(2)} s`;
const LOG_TICKS = [1, 2, 10, 20, 100, 200];
const fmtPct = (p) => (Number.isFinite(p) ? `${p.toFixed(0)} %` : "—");

function el(tag, attrs = {}, ...children) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v;
    else e.setAttribute(k, v);
  }
  for (const c of children) e.append(c instanceof Node ? c : document.createTextNode(String(c)));
  return e;
}

function table(target, columns, rows) {
  const t = el("table");
  const head = el("tr");
  for (const c of columns) head.append(el("th", c.num ? { class: "num" } : {}, c.label));
  t.append(el("thead", {}, head));
  const body = el("tbody");
  for (const r of rows) {
    const tr = el("tr");
    for (const c of columns) {
      const v = c.value(r);
      tr.append(el("td", c.num ? { class: "num" } : {}, v instanceof Node ? v : (v ?? "—")));
    }
    body.append(tr);
  }
  t.append(body);
  target.replaceChildren(t);
}

function specLink(path) {
  return el("a", { href: REPO + path }, path.split("/").pop());
}

function frame(target, title, sub) {
  target.replaceChildren();
  if (title) target.append(el("h3", {}, title));
  if (sub) target.append(el("p", { class: "sub" }, sub));
  return Math.max(280, target.clientWidth - 32);
}

function style() {
  return {
    background: "transparent",
    color: css("--text-secondary"),
    fontFamily: css("--font-sans") || "system-ui",
    fontSize: "12px",
    overflow: "visible",
  };
}

function grid() {
  return { stroke: css("--line"), strokeOpacity: 1 };
}

// -- data --------------------------------------------------------------------------------------

function allRuns(data) {
  const out = [];
  for (const [name, s] of Object.entries(data.studies)) {
    const ctx = CONTEXT[name] ?? { text: s.description };
    for (const r of s.runs) {
      const family =
        ctx.family ?? (r.labels[0] === "EoE" ? "eoe" : r.labels[0].startsWith("PDO") ? "pdo" : "mailbox");
      const params = s.parameters.map((p, i) => `${p} ${r.labels[i]}`).join(" · ");
      out.push({
        config: `${ctx.text} · ${params}`,
        family,
        spec: s.spec,
        feasible: r.feasible,
        reason: r.reason,
        time: r.objectives?.[0],
        link: r.objectives?.[1],
        pdi: r.objectives?.[2],
      });
    }
  }
  for (const s of data.steps) {
    out.push({
      config: s.description.replace(/^\d+\.\s*/, `Step ${s.step}: `),
      family: s.protocol === "PDO channel" ? "pdo" : "mailbox",
      spec: s.spec,
      feasible: true,
      time: s.duration_s,
      link: s.link_percent,
      pdi: s.pdi_busy_percent,
    });
  }
  // The same configuration appears in several studies; keep one of each result.
  const seen = new Set();
  return out.filter((r) => {
    if (!r.feasible) return false;
    const key = `${r.family}|${r.time}|${r.link}|${r.pdi}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

// -- tiles -------------------------------------------------------------------------------------

function tiles(data) {
  const steps = data.steps;
  const mailbox = steps.filter((s) => s.protocol !== "PDO channel");
  const pdo = steps.filter((s) => s.protocol === "PDO channel");
  const best = (xs) => xs.reduce((a, b) => (b.duration_s < a.duration_s ? b : a));
  const items = [
    ["Payload alone on the 100 Mbit/s link", fmtS(data.physical_bound_s), "the floor"],
    ["TCP over a dedicated Ethernet link", fmtS(data.tcp_reference_s), "reference"],
    ["Plausible default: FoE, 128 B mailboxes, fixed rate", fmtS(steps[0].duration_s), "step 1"],
    ["Best mailbox path in the steps", fmtS(best(mailbox).duration_s), `step ${best(mailbox).step}`],
    ["Best process-data channel in the steps", fmtS(best(pdo).duration_s), `step ${best(pdo).step}`],
  ];
  document.getElementById("tiles").replaceChildren(
    ...items.map(([label, value, note]) =>
      el("div", { class: "tile" }, el("div", { class: "label" }, label), el("div", { class: "value" }, value), el("div", { class: "note" }, note)),
    ),
  );
}

// -- charts ------------------------------------------------------------------------------------

function stepsChart(data) {
  const target = document.getElementById("chart-steps");
  const width = frame(target, "10 MB transfer time per optimisation step", "simulated · log scale · bars coloured by path");
  const [c1, , c3] = series();
  const rows = data.steps.map((s) => ({ ...s, label: s.description.replace(/^\d+\.\s*/, `${s.step}. `) }));
  const narrow = width < 620;
  const marginLeft = narrow ? 8 : Math.min(330, width * 0.42);
  const lo = 0.5;
  target.append(
    Plot.plot({
      width,
      height: narrow ? 640 : 360,
      marginLeft,
      marginRight: 56,
      marginTop: 28,
      marginBottom: 44,
      style: style(),
      x: { type: "log", domain: [lo, 300], label: "10 MB transfer time, s →", labelAnchor: "center", labelOffset: 34, ticks: LOG_TICKS, tickFormat: (d) => `${d}` },
      y: { domain: rows.map((r) => r.label), label: null, axis: null, padding: narrow ? 0.62 : 0.3 },
      color: { domain: ["FoE", "PDO channel"], range: [c1, c3], legend: true },
      marks: [
        Plot.gridX(LOG_TICKS, { ...grid() }),
        Plot.barX(rows, {
          x1: lo,
          x2: "duration_s",
          y: "label",
          fill: "protocol",
          rx2: 4,
          tip: {
            fill: css("--surface-1"),
            stroke: css("--line"),
            format: { x1: false, x2: false, y: true, fill: false },
          },
          channels: {
            "10 MB time": (d) => fmtS(d.duration_s),
            Goodput: (d) => `${d.goodput_mbps.toFixed(2)} Mbit/s`,
            "Link share": (d) => fmtPct(d.link_percent),
            "PDI busy": (d) => fmtPct(d.pdi_busy_percent),
          },
        }),
        Plot.text(rows, {
          x: "duration_s",
          y: "label",
          text: (d) => fmtS(d.duration_s),
          dx: 6,
          textAnchor: "start",
          fill: css("--text-primary"),
        }),
        narrow
          ? Plot.text(rows, {
              x: lo,
              y: "label",
              text: "label",
              dy: -15,
              lineAnchor: "bottom",
              textAnchor: "start",
              fill: css("--text-secondary"),
              lineWidth: (width - 40) / 12,
            })
          : null,
        narrow ? null : Plot.axisY({ tickSize: 0, lineWidth: (marginLeft - 16) / 12, fill: css("--text-secondary") }),
        Plot.ruleX([data.physical_bound_s], { stroke: css("--ref"), strokeWidth: 1 }),
        Plot.text([data.physical_bound_s], {
          x: (d) => d,
          frameAnchor: "top",
          dy: -18,
          textAnchor: "start",
          text: () => `link bound ${fmtS(data.physical_bound_s)} · TCP ${fmtS(data.tcp_reference_s)}`,
          fill: css("--text-muted"),
        }),
      ],
    }),
  );
  table(
    document.getElementById("table-steps"),
    [
      { label: "Configuration", value: (r) => r.description },
      { label: "10 MB time", num: true, value: (r) => fmtS(r.duration_s) },
      { label: "Goodput", num: true, value: (r) => `${r.goodput_mbps.toFixed(2)} Mbit/s` },
      { label: "Link", num: true, value: (r) => fmtPct(r.link_percent) },
      { label: "PDI busy", num: true, value: (r) => fmtPct(r.pdi_busy_percent) },
      { label: "Spec", value: (r) => specLink(r.spec) },
    ],
    data.steps,
  );
}

function policyChart(data) {
  const s = data.studies["01-protocol-policy"];
  const target = document.getElementById("chart-policy");
  const width = frame(target, "Protocol × master mailbox policy", "simulated · 512 B mailboxes · 1 ms cycle · log scale");
  const rows = s.runs.filter((r) => r.feasible).map((r) => ({ protocol: r.labels[0], policy: r.labels[1], time: r.objectives[0], link: r.objectives[1], pdi: r.objectives[2] }));
  const protocols = [...new Set(rows.map((r) => r.protocol))];
  target.append(
    Plot.plot({
      width,
      height: 260,
      marginLeft: 120,
      marginRight: 24,
      marginBottom: 44,
      style: style(),
      x: { type: "log", label: "10 MB transfer time, s →", labelAnchor: "center", labelOffset: 34, ticks: LOG_TICKS, tickFormat: (d) => `${d}` },
      y: { domain: [...new Set(rows.map((r) => r.policy))], label: null },
      color: { domain: protocols, range: series(), legend: true },
      symbol: { domain: protocols, range: ["circle", "triangle", "square"] },
      marks: [
        Plot.gridX(LOG_TICKS, grid()),
        Plot.ruleY([...new Set(rows.map((r) => r.policy))], { stroke: css("--line") }),
        Plot.dot(rows, {
          x: "time",
          y: "policy",
          fill: "protocol",
          symbol: "protocol",
          r: 5.5,
          stroke: css("--surface-1"),
          strokeWidth: 2,
          tip: { fill: css("--surface-1"), stroke: css("--line"), format: { x: false, symbol: false } },
          channels: { "10 MB time": (d) => fmtS(d.time), "Link share": (d) => fmtPct(d.link), "PDI busy": (d) => fmtPct(d.pdi) },
        }),
      ],
    }),
  );
  table(
    document.getElementById("table-policy"),
    [
      { label: "Protocol", value: (r) => r.protocol },
      { label: "Master policy", value: (r) => r.policy },
      { label: "10 MB time", num: true, value: (r) => fmtS(r.time) },
      { label: "Link", num: true, value: (r) => fmtPct(r.link) },
      { label: "PDI busy", num: true, value: (r) => fmtPct(r.pdi) },
    ],
    rows,
  );
}

function linesChart(targetId, title, sub, rows, { x, xLabel, series: key, xType = "linear", xTicks, rules = [] }) {
  const target = document.getElementById(targetId);
  const width = frame(target, title, sub);
  const keys = [...new Set(rows.map((r) => r[key]))];
  target.append(
    Plot.plot({
      width,
      height: 280,
      marginLeft: 48,
      marginRight: 24,
      marginTop: 24,
      marginBottom: 44,
      style: style(),
      x: { type: xType, label: xLabel, labelAnchor: "center", labelOffset: 34, ticks: xTicks, tickFormat: xType === "log" ? "~g" : undefined, grid: false },
      y: { type: "log", label: "↑ 10 MB time, s", ticks: LOG_TICKS, tickFormat: (d) => `${d}` },
      color: { domain: keys, range: series(), legend: true },
      marks: [
        Plot.gridY(LOG_TICKS, grid()),
        ...rules,
        Plot.line(rows, { x, y: "time", stroke: key, strokeWidth: 2, curve: "linear" }),
        Plot.dot(rows, {
          x,
          y: "time",
          fill: key,
          r: 4.5,
          stroke: css("--surface-1"),
          strokeWidth: 2,
          tip: { fill: css("--surface-1"), stroke: css("--line"), format: { y: false } },
          channels: { "10 MB time": (d) => fmtS(d.time), "Link share": (d) => fmtPct(d.link), "PDI busy": (d) => fmtPct(d.pdi) },
        }),
      ],
    }),
  );
}

function runsOf(study, map) {
  return study.runs.filter((r) => r.feasible).map((r) => ({ ...map(r), time: r.objectives[0], link: r.objectives[1], pdi: r.objectives[2] }));
}

function mailboxCharts(data) {
  const foe = runsOf(data.studies["02-mailbox-foe"], (r) => ({ mailbox: r.numeric[0], policy: r.labels[1], protocol: "FoE" }));
  const eoe = runsOf(data.studies["03-mailbox-eoe"], (r) => ({ mailbox: r.numeric[0], policy: r.labels[1], protocol: "EoE" }));
  const opts = { x: "mailbox", xLabel: "mailbox size, bytes →", series: "policy", xTicks: [128, 512, 1024, 1484] };
  linesChart("chart-mbx-foe", "FoE: mailbox size × master policy", "simulated · 1 ms cycle", foe, opts);
  const marks = [522, 778].map((m) =>
    Plot.ruleX([m], { stroke: css("--line"), strokeWidth: 1 }),
  );
  linesChart("chart-mbx-eoe", "EoE (1500 B MTU): mailbox size × master policy", "simulated · 1 ms cycle · rules at 522 and 778 B", eoe, {
    ...opts,
    rules: marks,
  });
  table(
    document.getElementById("table-mbx"),
    [
      { label: "Protocol", value: (r) => r.protocol },
      { label: "Mailbox", num: true, value: (r) => `${r.mailbox} B` },
      { label: "Master policy", value: (r) => r.policy },
      { label: "10 MB time", num: true, value: (r) => fmtS(r.time) },
      { label: "Link", num: true, value: (r) => fmtPct(r.link) },
      { label: "PDI busy", num: true, value: (r) => fmtPct(r.pdi) },
    ],
    [...foe, ...eoe],
  );
}

function heatmap(targetId, title, sub, rows, domain) {
  const target = document.getElementById(targetId);
  const width = frame(target, title, sub);
  const ramp = seq();
  const scale = d3.scaleLog().domain(domain).range([0, 1]).clamp(true);
  const interp = d3.interpolateRgbBasis(ramp);
  const fill = (t) => interp(scale(t));
  const ink = (t) => (d3.lab(fill(t)).l > 58 ? "#141413" : "#ffffff");
  const rates = [...new Set(rows.map((r) => r.rateLabel))];
  const overheads = [...new Set(rows.map((r) => r.overheadLabel))];
  target.append(
    Plot.plot({
      width,
      height: 250,
      marginLeft: 64,
      marginBottom: 44,
      padding: 0,
      style: style(),
      x: { domain: rates, label: "PDI rate (effective) →", labelAnchor: "center", labelOffset: 34, tickSize: 0 },
      y: { domain: overheads, label: "↑ cost per access", tickSize: 0 },
      marks: [
        Plot.cell(rows, {
          x: "rateLabel",
          y: "overheadLabel",
          fill: (d) => fill(d.time),
          inset: 1,
          rx: 3,
          tip: { fill: css("--surface-1"), stroke: css("--line"), format: { fill: false } },
          channels: { "10 MB time": (d) => fmtS(d.time), "PDI busy": (d) => fmtPct(d.pdi) },
        }),
        Plot.text(rows, {
          x: "rateLabel",
          y: "overheadLabel",
          text: (d) => (d.time >= 10 ? d.time.toFixed(0) : d.time.toFixed(1)),
          fill: (d) => ink(d.time),
          fontSize: 11,
        }),
      ],
    }),
  );
  return { fill, domain };
}

function pdiCharts(data) {
  const map = (r) => ({ rate: r.numeric[0], overhead: r.numeric[1], rateLabel: r.labels[0].replace(" Mbit/s", ""), overheadLabel: r.labels[1] });
  const whole = runsOf(data.studies["05-pdi-whole-buffer"], map);
  const msg = runsOf(data.studies["06-pdi-message-only"], map);
  const all = [...whole, ...msg].map((r) => r.time);
  const domain = [Math.min(...all), Math.max(...all)];
  const sub = "simulated · seconds for 10 MB · rate in Mbit/s · shared colour scale";
  heatmap("chart-pdi-whole", "Whole-buffer SPI access", sub, whole, domain);
  heatmap("chart-pdi-msg", "Message-only SPI access", sub, msg, domain);
  table(
    document.getElementById("table-pdi"),
    [
      { label: "Access", value: (r) => r.access },
      { label: "PDI rate", num: true, value: (r) => `${r.rateLabel} Mbit/s` },
      { label: "Per access", num: true, value: (r) => r.overheadLabel },
      { label: "10 MB time", num: true, value: (r) => fmtS(r.time) },
      { label: "PDI busy", num: true, value: (r) => fmtPct(r.pdi) },
    ],
    [...whole.map((r) => ({ ...r, access: "whole buffer" })), ...msg.map((r) => ({ ...r, access: "message only" }))],
  );
}

function pdoCharts(data) {
  const size = data.studies["07-pdo-size"];
  const sizeRows = runsOf(size, (r) => ({ bytes: parseInt(r.labels[0], 10), cycle: r.labels[1] }));
  const limit = Plot.ruleX([1260], { stroke: css("--ref"), strokeWidth: 1 });
  const limitText = Plot.text([1260], {
    x: (d) => d,
    frameAnchor: "top",
    dy: -16,
    textAnchor: "end",
    text: () => "ESC RAM limit 1,260 B",
    fill: css("--text-muted"),
  });
  linesChart("chart-pdo-size", "Bytes per cycle × cycle time", "simulated · window 2 · 1,270 B does not fit in 4 KiB", sizeRows, {
    x: "bytes",
    xLabel: "bytes per cycle →",
    series: "cycle",
    xTicks: [256, 512, 768, 1024],
    rules: [limit, limitText],
  });
  const pdi = runsOf(data.studies["08-pdo-pdi"], (r) => ({ rate: r.numeric[1] / 1e6, window: `window ${r.labels[0].split("window ")[1]}` }));
  linesChart("chart-pdo-pdi", "Window × PDI rate at 1,024 B per 250 µs", "simulated · the channel needs ~33 Mbit/s of SPI", pdi, {
    x: "rate",
    xLabel: "PDI rate (effective), Mbit/s →",
    series: "window",
    xType: "log",
    xTicks: [10, 20, 40, 80],
  });
  table(
    document.getElementById("table-pdo"),
    [
      { label: "Channel", value: (r) => r.channel },
      { label: "Varied", value: (r) => r.varied },
      { label: "10 MB time", num: true, value: (r) => fmtS(r.time) },
      { label: "Link", num: true, value: (r) => fmtPct(r.link) },
      { label: "PDI busy", num: true, value: (r) => fmtPct(r.pdi) },
    ],
    [
      ...sizeRows.map((r) => ({ ...r, channel: `${r.bytes} B per cycle, window 2`, varied: `cycle ${r.cycle}` })),
      ...size.runs.filter((r) => !r.feasible).map((r) => ({ channel: r.labels[0], varied: `cycle ${r.labels[1]}: infeasible (${r.reason})` })),
      ...pdi.map((r) => ({ ...r, channel: `1024 B per 250 µs, ${r.window}`, varied: `PDI ${r.rate} Mbit/s` })),
    ],
  );
}

// -- requirement explorer ----------------------------------------------------------------------

function readRequirement() {
  const num = (id, dflt) => {
    const v = parseFloat(document.getElementById(id).value);
    return Number.isFinite(v) && v > 0 ? v : dflt;
  };
  const families = [...document.querySelectorAll("#req fieldset input:checked")].map((i) => i.value);
  return { deadline: num("deadline", 10), maxLink: num("maxlink", 100), maxPdi: num("maxpdi", 100), families };
}

function requirement(data, runs) {
  const req = readRequirement();
  const inScope = runs.filter((r) => req.families.includes(r.family));
  const meets = (r) => r.time <= req.deadline && r.link <= req.maxLink && r.pdi <= req.maxPdi;
  const ok = inScope.filter(meets).sort((a, b) => a.time - b.time);
  const fastest = [...inScope].sort((a, b) => a.time - b.time)[0];
  const verdict = document.getElementById("verdict");
  if (!inScope.length) {
    verdict.replaceChildren("Select at least one path.");
  } else if (!ok.length) {
    verdict.replaceChildren(
      el("b", {}, "No simulated configuration meets these limits. "),
      `The fastest of the selected paths takes ${fmtS(fastest.time)} (${fastest.config}). `,
      req.deadline < 3
        ? `Below about 3 s, plan a separate Ethernet path: TCP over a dedicated 100 Mbit/s link needs about ${fmtS(data.tcp_reference_s)}.`
        : "Relax the link or SPI limit, or measure the SPI: the placeholders may be pessimistic.",
    );
  } else {
    const leanest = [...ok].sort((a, b) => a.link - b.link)[0];
    verdict.replaceChildren(
      el("b", {}, `${ok.length} of ${inScope.length}`),
      ` simulated configurations meet ${fmtS(req.deadline)} within ${fmtPct(req.maxLink)} link and ${fmtPct(req.maxPdi)} PDI busy. `,
      `Lowest link share among them: ${fmtPct(leanest.link)} (${leanest.config}, ${fmtS(leanest.time)}).`,
    );
  }

  const target = document.getElementById("chart-req");
  const width = frame(target, "Every simulated configuration: time against link share", "simulated · filled marks meet the requirement · log scale");
  const colors = series();
  const shown = inScope.map((r) => ({ ...r, meets: meets(r), path: FAMILY_LABEL[r.family] }));
  const domain = FAMILIES.filter((f) => req.families.includes(f)).map((f) => FAMILY_LABEL[f]);
  const range = FAMILIES.map((f, i) => (req.families.includes(f) ? colors[i] : null)).filter(Boolean);
  const symbols = FAMILIES.filter((f) => req.families.includes(f)).map((f) => SYMBOL[f]);
  target.append(
    Plot.plot({
      width,
      height: 320,
      marginLeft: 48,
      marginRight: 24,
      marginTop: 24,
      marginBottom: 44,
      style: style(),
      x: { type: "log", label: "10 MB transfer time, s →", labelAnchor: "center", labelOffset: 34, domain: [0.5, 300], ticks: LOG_TICKS, tickFormat: (d) => `${d}` },
      y: { label: "↑ link share, %", domain: [0, 100], grid: true },
      color: { domain, range, legend: true },
      symbol: { domain, range: symbols },
      marks: [
        Plot.gridY(grid()),
        Plot.rectX([0], { x1: 0.5, x2: req.deadline, y1: 0, y2: Math.min(100, req.maxLink), fill: css("--accent"), fillOpacity: 0.07 }),
        Plot.ruleX([req.deadline], { stroke: css("--accent"), strokeWidth: 1.5 }),
        Plot.text([req.deadline], { x: (d) => d, frameAnchor: "top", dy: -16, text: () => `deadline ${fmtS(req.deadline)}`, fill: css("--text-secondary") }),
        Plot.ruleX([data.tcp_reference_s], { stroke: css("--ref"), strokeWidth: 1 }),
        Plot.dot(shown.filter((r) => !r.meets), {
          x: "time",
          y: "link",
          stroke: "path",
          symbol: "path",
          r: 4.5,
          strokeWidth: 1.5,
          fill: "none",
          strokeOpacity: 0.55,
          tip: { fill: css("--surface-1"), stroke: css("--line"), format: { x: false, y: false, stroke: false, symbol: false } },
          channels: { Configuration: "config", "10 MB time": (d) => fmtS(d.time), "Link share": (d) => fmtPct(d.link), "PDI busy": (d) => fmtPct(d.pdi) },
        }),
        Plot.dot(shown.filter((r) => r.meets), {
          x: "time",
          y: "link",
          fill: "path",
          symbol: "path",
          r: 5,
          stroke: css("--surface-1"),
          strokeWidth: 1.5,
          tip: { fill: css("--surface-1"), stroke: css("--line"), format: { x: false, y: false, fill: false, symbol: false } },
          channels: { Configuration: "config", "10 MB time": (d) => fmtS(d.time), "Link share": (d) => fmtPct(d.link), "PDI busy": (d) => fmtPct(d.pdi) },
        }),
      ],
    }),
  );

  const top = ok.slice(0, 12);
  const holder = document.getElementById("table-req");
  if (!top.length) {
    holder.replaceChildren();
    return;
  }
  table(
    holder,
    [
      {
        label: "Configuration that meets it",
        value: (r) => {
          const i = FAMILIES.indexOf(r.family);
          const sw = el("span", { class: "swatch", style: `background:${colors[i]}`, "aria-hidden": "true" });
          return el("span", {}, sw, `${FAMILY_LABEL[r.family]} — ${r.config}`);
        },
      },
      { label: "10 MB time", num: true, value: (r) => fmtS(r.time) },
      { label: "Link", num: true, value: (r) => fmtPct(r.link) },
      { label: "PDI busy", num: true, value: (r) => fmtPct(r.pdi) },
      { label: "Spec", value: (r) => specLink(r.spec) },
    ],
    top,
  );
  if (ok.length > top.length) holder.append(el("p", { class: "muted" }, `…and ${ok.length - top.length} more, slower.`));
}

// -- main --------------------------------------------------------------------------------------

async function main() {
  const res = await fetch("data.json");
  const data = await res.json();
  const runs = allRuns(data);
  const render = () => {
    tiles(data);
    stepsChart(data);
    requirement(data, runs);
    policyChart(data);
    mailboxCharts(data);
    pdiCharts(data);
    pdoCharts(data);
  };
  render();
  for (const input of document.querySelectorAll("#req input")) input.addEventListener("input", () => requirement(data, runs));
  let t;
  const later = () => {
    clearTimeout(t);
    t = setTimeout(render, 150);
  };
  window.addEventListener("resize", later);
  window.addEventListener("themechange", render);
}

main().catch((e) => {
  document.getElementById("tiles").replaceChildren(el("p", { class: "muted" }, `Could not load the results: ${e.message}`));
});
