import { Box, Typography } from "@mui/material";
import { memo, useMemo } from "react";

const AGENTARK_CHART_LANGUAGE = "agentark-chart";
const MAX_CHART_ROWS = 160;
const MAX_CHART_SERIES = 8;
const AXIS_LABEL_COLOR = "#c8d8c9";
const AXIS_LINE_COLOR = "rgba(130, 170, 160, 0.28)";
const SPLIT_LINE_COLOR = "rgba(130, 170, 160, 0.16)";
const TOOLTIP_BG = "rgba(14, 18, 14, 0.96)";
const TOOLTIP_BORDER = "rgba(120, 242, 176, 0.24)";
const LINE_COLORS = ["#78f2b0", "#d8ad78", "#ffbe63", "#b7a7ff", "#e6d6c0", "#ff9b9b"];
const PIE_COLORS = ["#d8ad78", "#78f2b0", "#ffbe63", "#e6d6c0", "#b7a7ff", "#ff9b9b"];

type ChartRecord = Record<string, unknown>;
type ChartKind = "bar" | "line" | "area" | "scatter" | "pie" | "doughnut";

type ChartSeries = {
  key: string;
  name: string;
  kind?: ChartKind;
};

type ChartModel =
  | {
      ok: true;
      title: string;
      subtitle: string;
      option: ChartRecord;
      height: number;
      kind: ChartKind;
      rows: ChartRecord[];
      categoryKey: string;
      series: ChartSeries[];
    }
  | {
      ok: false;
      message: string;
    };

type ResolvedChartModel = Extract<ChartModel, { ok: true }>;

function isRecord(value: unknown): value is ChartRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function textValue(value: unknown, fallback = ""): string {
  if (typeof value === "string" && value.trim()) return value.trim();
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  return fallback;
}

function numberValue(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value !== "string") return null;
  const parsed = Number(value.replace(/[$,%\s,]/g, ""));
  return Number.isFinite(parsed) ? parsed : null;
}

function clampNumber(
  value: unknown,
  fallback: number,
  min: number,
  max: number,
): number {
  const parsed = numberValue(value);
  if (parsed == null) return fallback;
  return Math.min(max, Math.max(min, parsed));
}

function explicitChartKind(value: unknown): ChartKind | null {
  const normalized = textValue(value).toLowerCase();
  switch (normalized) {
    case "bar":
    case "line":
    case "area":
    case "scatter":
    case "pie":
    case "doughnut":
      return normalized;
    default:
      return null;
  }
}

function chartKind(value: unknown): ChartKind {
  return explicitChartKind(value) || "bar";
}

function chartColor(index: number, palette = LINE_COLORS): string {
  return palette[index % palette.length] || palette[0];
}

function hexToRgba(hex: string, alpha: number): string {
  const normalized = hex.replace("#", "");
  if (!/^[0-9a-f]{6}$/i.test(normalized)) return `rgba(216, 173, 120, ${alpha})`;
  const r = Number.parseInt(normalized.slice(0, 2), 16);
  const g = Number.parseInt(normalized.slice(2, 4), 16);
  const b = Number.parseInt(normalized.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function goldBarGradient(): ChartRecord {
  return {
    type: "linear",
    x: 0,
    y: 0,
    x2: 0,
    y2: 1,
    colorStops: [
      { offset: 0, color: "#f1d6ad" },
      { offset: 0.36, color: "#d8ad78" },
      { offset: 1, color: "#8d6841" },
    ],
  };
}

function firstDataKey(
  rows: ChartRecord[],
  predicate: (value: unknown) => boolean,
): string {
  const keys = new Set<string>();
  for (const row of rows) {
    Object.keys(row).forEach((key) => keys.add(key));
  }
  for (const key of keys) {
    if (rows.some((row) => predicate(row[key]))) return key;
  }
  return keys.values().next().value || "";
}

function inferCategoryKey(rows: ChartRecord[], preferred: unknown): string {
  const explicit = textValue(preferred);
  if (explicit) return explicit;
  return firstDataKey(rows, (value) => numberValue(value) == null);
}

function looksTemporalCategoryValue(value: unknown): boolean {
  const text = textValue(value);
  if (!text) return false;
  if (Number.isFinite(Date.parse(text))) return true;
  return /\d{1,2}:\d{2}/.test(text) || /\d{4}[-/]\d{1,2}[-/]\d{1,2}/.test(text);
}

function inferChartKind(spec: ChartRecord, rows: ChartRecord[]): ChartKind {
  const explicit = explicitChartKind(spec.type);
  if (explicit) return explicit;
  const categoryKey = inferCategoryKey(rows, spec.x);
  const temporalCount = rows.filter((row) =>
    looksTemporalCategoryValue(row[categoryKey]),
  ).length;
  return temporalCount >= Math.min(2, rows.length) ? "line" : "bar";
}

function inferNumericKeys(rows: ChartRecord[], categoryKey: string): string[] {
  const keys = new Set<string>();
  for (const row of rows) {
    Object.keys(row).forEach((key) => {
      if (key !== categoryKey) keys.add(key);
    });
  }
  return Array.from(keys)
    .filter((key) => rows.some((row) => numberValue(row[key]) != null))
    .slice(0, MAX_CHART_SERIES);
}

function seriesFromSpec(
  spec: ChartRecord,
  rows: ChartRecord[],
  categoryKey: string,
): ChartSeries[] {
  const rawSeries = spec.series;
  if (Array.isArray(rawSeries)) {
    const series = rawSeries
      .map((item): ChartSeries | null => {
        if (typeof item === "string") {
          const key = item.trim();
          return key ? { key, name: key } : null;
        }
        if (!isRecord(item)) return null;
        const key = textValue(item.key);
        if (!key) return null;
        const explicitKind = explicitChartKind(item.type);
        return {
          key,
          name: textValue(item.name, textValue(item.label, key)),
          kind: explicitKind || undefined,
        };
      })
      .filter((item): item is ChartSeries => item !== null);
    if (series.length > 0) return series.slice(0, MAX_CHART_SERIES);
  }

  return inferNumericKeys(rows, categoryKey).map((key) => ({ key, name: key }));
}

function valueForRow(row: ChartRecord, key: string): number | null {
  return numberValue(row[key]);
}

function buildDataZoom(rows: ChartRecord[]): ChartRecord[] | undefined {
  if (rows.length <= 16) return undefined;
  return [
    { type: "inside", xAxisIndex: 0, throttle: 80 },
    {
      type: "slider",
      xAxisIndex: 0,
      height: 18,
      bottom: 6,
      borderColor: "rgba(130, 170, 160, 0.22)",
      fillerColor: "rgba(120, 242, 176, 0.14)",
      handleStyle: { color: AXIS_LABEL_COLOR },
      textStyle: { color: AXIS_LABEL_COLOR },
      dataBackground: {
        lineStyle: { color: "rgba(120, 242, 176, 0.30)" },
        areaStyle: { color: "rgba(120, 242, 176, 0.10)" },
      },
      selectedDataBackground: {
        lineStyle: { color: "rgba(120, 242, 176, 0.48)" },
        areaStyle: { color: "rgba(120, 242, 176, 0.16)" },
      },
    },
  ];
}

function buildAxisOption(spec: ChartRecord, rows: ChartRecord[], kind: ChartKind): ChartRecord {
  const categoryKey = inferCategoryKey(rows, spec.x);
  const categories = rows.map((row, index) =>
    textValue(row[categoryKey], String(index + 1)),
  );
  const series = seriesFromSpec(spec, rows, categoryKey);
  const dataZoom = buildDataZoom(rows);
  const chartSeries = series.map((entry, index) => {
    const seriesKind =
      entry.kind && entry.kind !== "pie" && entry.kind !== "doughnut"
        ? entry.kind
        : kind;
    const lineColor = chartColor(index);
    if (seriesKind === "bar") {
      return {
        name: entry.name,
        type: "bar",
        barWidth: "42%",
        barMaxWidth: 34,
        barMinWidth: 3,
        itemStyle: {
          borderRadius: [5, 5, 1, 1],
          color: index === 0 ? goldBarGradient() : lineColor,
        },
        emphasis: {
          itemStyle: {
            color: index === 0 ? "#f4ddb9" : lineColor,
          },
        },
        data: rows.map((row) => valueForRow(row, entry.key)),
      };
    }
    return {
      name: entry.name,
      type: seriesKind === "area" ? "line" : seriesKind,
      smooth: seriesKind === "line" || seriesKind === "area",
      symbol: "circle",
      symbolSize: seriesKind === "scatter" ? 8 : 6,
      showSymbol: rows.length <= 32,
      itemStyle: {
        color: lineColor,
        borderColor: "#f7fbff",
        borderWidth: 1.5,
      },
      lineStyle:
        seriesKind === "scatter"
          ? undefined
          : {
              width: 2.2,
              type: index >= 2 ? "dashed" : "solid",
              color: lineColor,
            },
      areaStyle:
        seriesKind === "area"
          ? {
              color: {
                type: "linear",
                x: 0,
                y: 0,
                x2: 0,
                y2: 1,
                colorStops: [
                  { offset: 0, color: hexToRgba(lineColor, 0.30) },
                  { offset: 1, color: hexToRgba(lineColor, 0.04) },
                ],
              },
            }
          : undefined,
      data: rows.map((row) => valueForRow(row, entry.key)),
    };
  });
  const hasMultipleSeries = chartSeries.length > 1;

  return {
    backgroundColor: "transparent",
    animationDuration: 450,
    color: LINE_COLORS,
    tooltip: {
      trigger: "axis",
      confine: true,
      backgroundColor: TOOLTIP_BG,
      borderColor: TOOLTIP_BORDER,
      textStyle: { color: "#fff8ed" },
      axisPointer: {
        type: kind === "bar" ? "shadow" : "line",
        lineStyle: { color: "rgba(130, 170, 160, 0.52)" },
        shadowStyle: { color: "rgba(120, 242, 176, 0.06)" },
      },
    },
    legend: {
      top: 4,
      left: hasMultipleSeries ? "center" : undefined,
      right: hasMultipleSeries ? undefined : 8,
      icon: "circle",
      itemWidth: 11,
      itemHeight: 11,
      textStyle: { color: "#d8d0c4" },
    },
    grid: {
      left: 18,
      right: 12,
      top: 42,
      bottom: dataZoom ? 52 : 28,
      containLabel: true,
    },
    xAxis: {
      type: "category",
      data: categories,
      axisLabel: {
        color: AXIS_LABEL_COLOR,
        fontSize: 10,
        hideOverlap: true,
        rotate: rows.length > 10 ? 25 : 0,
      },
      axisLine: { lineStyle: { color: AXIS_LINE_COLOR } },
      axisTick: { show: false },
    },
    yAxis: {
      type: "value",
      axisLabel: { color: AXIS_LABEL_COLOR },
      splitLine: { lineStyle: { color: SPLIT_LINE_COLOR } },
    },
    dataZoom,
    series: chartSeries,
  };
}

function buildPieOption(spec: ChartRecord, rows: ChartRecord[], kind: ChartKind): ChartRecord {
  const categoryKey = inferCategoryKey(rows, spec.x);
  const series = seriesFromSpec(spec, rows, categoryKey);
  const valueKey = series[0]?.key;
  const data = valueKey
    ? rows
        .map((row, index) => ({
          name: textValue(row[categoryKey], String(index + 1)),
          value: valueForRow(row, valueKey),
        }))
        .filter((row) => row.value != null)
    : [];

  return {
    backgroundColor: "transparent",
    animationDuration: 450,
    color: PIE_COLORS,
    tooltip: {
      trigger: "item",
      confine: true,
      backgroundColor: TOOLTIP_BG,
      borderColor: TOOLTIP_BORDER,
      textStyle: { color: "#fff8ed" },
    },
    legend: {
      orient: "vertical",
      right: 8,
      top: 18,
      bottom: 18,
      textStyle: { color: "#d8d0c4" },
    },
    series: [
      {
        name: series[0]?.name || textValue(spec.title, "Value"),
        type: "pie",
        radius: kind === "doughnut" ? ["44%", "68%"] : "68%",
        center: ["40%", "52%"],
        avoidLabelOverlap: true,
        label: { color: "rgba(255, 248, 237, 0.8)" },
        labelLine: { lineStyle: { color: "rgba(255, 248, 237, 0.3)" } },
        data,
      },
    ],
  };
}

function buildChartModel(code: string): ChartModel {
  let parsed: unknown;
  try {
    parsed = JSON.parse(code);
  } catch {
    return { ok: false, message: "Chart block is not valid JSON." };
  }
  if (!isRecord(parsed)) {
    return { ok: false, message: "Chart block must be a JSON object." };
  }
  const rows = Array.isArray(parsed.data)
    ? parsed.data.filter(isRecord).slice(0, MAX_CHART_ROWS)
    : [];
  if (rows.length === 0) {
    return { ok: false, message: "Chart block does not include tabular data." };
  }
  const kind = inferChartKind(parsed, rows);
  const categoryKey = inferCategoryKey(rows, parsed.x);
  const series = seriesFromSpec(parsed, rows, categoryKey);
  const hasNumericData = series.some((entry) =>
    rows.some((row) => valueForRow(row, entry.key) != null),
  );
  if (!categoryKey || series.length === 0 || !hasNumericData) {
    return { ok: false, message: "Chart block does not include numeric series data." };
  }
  const option =
    kind === "pie" || kind === "doughnut"
      ? buildPieOption(parsed, rows, kind)
      : buildAxisOption(parsed, rows, kind);
  return {
    ok: true,
    title: textValue(parsed.title, "Chart"),
    subtitle: textValue(parsed.subtitle),
    option,
    height: clampNumber(parsed.height, 310, 220, 520),
    kind,
    rows,
    categoryKey,
    series,
  };
}

function chartValueLabel(value: number): string {
  if (Math.abs(value) >= 1000) {
    return value.toLocaleString(undefined, { maximumFractionDigits: 0 });
  }
  if (Math.abs(value) >= 10) {
    return value.toLocaleString(undefined, { maximumFractionDigits: 1 });
  }
  return value.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

function shortChartLabel(value: unknown, maxChars = 18): string {
  const text = textValue(value).replace(/\s+/g, " ").trim();
  if (!text) return "";
  return text.length > maxChars ? `${text.slice(0, maxChars - 3)}...` : text;
}

function chartSeriesColor(index: number): string {
  return index === 0 ? "#d8ad78" : chartColor(index);
}

function chartValues(model: ResolvedChartModel): number[] {
  return model.rows.flatMap((row) =>
    model.series
      .map((entry) => valueForRow(row, entry.key))
      .filter((value): value is number => value != null),
  );
}

function StaticAxisChart({ model }: { model: ResolvedChartModel }) {
  const width = 760;
  const height = Math.max(260, model.height);
  const left = 58;
  const right = 24;
  const top = 40;
  const bottom = model.rows.length > 8 ? 72 : 48;
  const plotWidth = width - left - right;
  const plotHeight = height - top - bottom;
  const values = chartValues(model);
  const rawMin = Math.min(0, ...values);
  const rawMax = Math.max(0, ...values);
  const span = rawMax === rawMin ? 1 : rawMax - rawMin;
  const min = rawMin;
  const max = rawMax === rawMin ? rawMax + 1 : rawMax;
  const yFor = (value: number) =>
    top + plotHeight - ((value - min) / (max - min || span)) * plotHeight;
  const rowCount = Math.max(1, model.rows.length);
  const categoryWidth = plotWidth / rowCount;
  const xFor = (index: number) => left + index * categoryWidth + categoryWidth / 2;
  const zeroY = yFor(0);
  const labelStep = Math.max(1, Math.ceil(model.rows.length / 8));
  const gridLines = Array.from({ length: 5 }, (_, index) => {
    const value = min + ((max - min) * index) / 4;
    const y = yFor(value);
    return { value, y };
  }).reverse();

  const axisSeries = model.series
    .map((entry, index) => ({
      ...entry,
      index,
      renderKind:
        entry.kind && entry.kind !== "pie" && entry.kind !== "doughnut"
          ? entry.kind
          : model.kind,
    }))
    .filter((entry) => entry.renderKind !== "pie" && entry.renderKind !== "doughnut");
  const barSeries = axisSeries.filter((entry) => entry.renderKind === "bar");
  const barGroupWidth = Math.min(categoryWidth * 0.72, 44);
  const barWidth = Math.max(3, barGroupWidth / Math.max(1, barSeries.length));

  return (
    <svg
      className="chat-inline-chart-svg"
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={model.title}
    >
      <rect x="0" y="0" width={width} height={height} rx="12" fill="transparent" />
      {gridLines.map((line, index) => (
        <g key={`grid-${index}`}>
          <line
            x1={left}
            y1={line.y}
            x2={width - right}
            y2={line.y}
            stroke="rgba(130, 170, 160, 0.18)"
          />
          <text
            x={left - 10}
            y={line.y + 4}
            textAnchor="end"
            fill="#c8d8c9"
            fontSize="11"
          >
            {chartValueLabel(line.value)}
          </text>
        </g>
      ))}
      <line x1={left} y1={top} x2={left} y2={top + plotHeight} stroke="rgba(130, 170, 160, 0.34)" />
      <line x1={left} y1={zeroY} x2={width - right} y2={zeroY} stroke="rgba(130, 170, 160, 0.38)" />

      {barSeries.map((entry, seriesOffset) =>
        model.rows.map((row, rowIndex) => {
          const value = valueForRow(row, entry.key);
          if (value == null) return null;
          const x =
            xFor(rowIndex) -
            (barWidth * barSeries.length) / 2 +
            seriesOffset * barWidth +
            1;
          const y = yFor(Math.max(0, value));
          const h = Math.max(1, Math.abs(yFor(value) - zeroY));
          return (
            <rect
              key={`bar-${entry.key}-${rowIndex}`}
              x={x}
              y={y}
              width={Math.max(2, barWidth - 2)}
              height={h}
              rx="4"
              fill={chartSeriesColor(entry.index)}
            />
          );
        }),
      )}

      {axisSeries
        .filter((entry) => entry.renderKind !== "bar")
        .map((entry) => {
          const points = model.rows
            .map((row, rowIndex) => {
              const value = valueForRow(row, entry.key);
              return value == null
                ? null
                : { x: xFor(rowIndex), y: yFor(value), value };
            })
            .filter((point): point is { x: number; y: number; value: number } => point != null);
          if (points.length === 0) return null;
          const path = points
            .map((point, index) => `${index === 0 ? "M" : "L"}${point.x.toFixed(1)} ${point.y.toFixed(1)}`)
            .join(" ");
          const color = chartSeriesColor(entry.index);
          const area =
            entry.renderKind === "area" && points.length > 1
              ? `${path} L ${points[points.length - 1].x.toFixed(1)} ${zeroY.toFixed(1)} L ${points[0].x.toFixed(1)} ${zeroY.toFixed(1)} Z`
              : "";
          return (
            <g key={`series-${entry.key}`}>
              {area ? <path d={area} fill={color} opacity="0.14" /> : null}
              {entry.renderKind !== "scatter" ? (
                <path d={path} fill="none" stroke={color} strokeWidth="2.6" />
              ) : null}
              {points.map((point, index) => (
                <circle
                  key={`point-${entry.key}-${index}`}
                  cx={point.x}
                  cy={point.y}
                  r={entry.renderKind === "scatter" ? 4.5 : 3.2}
                  fill={color}
                  stroke="#f7fbff"
                  strokeWidth="1"
                />
              ))}
            </g>
          );
        })}

      {model.rows.map((row, index) => {
        if (index % labelStep !== 0 && index !== model.rows.length - 1) {
          return null;
        }
        const label = shortChartLabel(row[model.categoryKey], 20);
        const x = xFor(index);
        const y = height - 20;
        const rotate = model.rows.length > 8;
        return (
          <text
            key={`x-${index}`}
            x={x}
            y={y}
            textAnchor={rotate ? "end" : "middle"}
            transform={rotate ? `rotate(-25 ${x} ${y})` : undefined}
            fill="#d8d0c4"
            fontSize="10"
          >
            {label}
          </text>
        );
      })}

      <g className="chat-inline-chart-legend">
        {axisSeries.slice(0, 4).map((entry, index) => (
          <g key={`legend-${entry.key}`} transform={`translate(${width - right - 156}, ${14 + index * 18})`}>
            <circle cx="0" cy="0" r="5" fill={chartSeriesColor(entry.index)} />
            <text x="10" y="4" fill="#fff8ed" fontSize="11">
              {shortChartLabel(entry.name, 22)}
            </text>
          </g>
        ))}
      </g>
    </svg>
  );
}

function polarToCartesian(
  cx: number,
  cy: number,
  radius: number,
  angleDegrees: number,
) {
  const angleRadians = ((angleDegrees - 90) * Math.PI) / 180;
  return {
    x: cx + radius * Math.cos(angleRadians),
    y: cy + radius * Math.sin(angleRadians),
  };
}

function describePieSlice(
  cx: number,
  cy: number,
  outerRadius: number,
  innerRadius: number,
  startAngle: number,
  endAngle: number,
): string {
  const startOuter = polarToCartesian(cx, cy, outerRadius, endAngle);
  const endOuter = polarToCartesian(cx, cy, outerRadius, startAngle);
  const largeArcFlag = endAngle - startAngle <= 180 ? "0" : "1";
  if (innerRadius <= 0) {
    return [
      `M ${cx} ${cy}`,
      `L ${startOuter.x} ${startOuter.y}`,
      `A ${outerRadius} ${outerRadius} 0 ${largeArcFlag} 0 ${endOuter.x} ${endOuter.y}`,
      "Z",
    ].join(" ");
  }
  const startInner = polarToCartesian(cx, cy, innerRadius, endAngle);
  const endInner = polarToCartesian(cx, cy, innerRadius, startAngle);
  return [
    `M ${startOuter.x} ${startOuter.y}`,
    `A ${outerRadius} ${outerRadius} 0 ${largeArcFlag} 0 ${endOuter.x} ${endOuter.y}`,
    `L ${endInner.x} ${endInner.y}`,
    `A ${innerRadius} ${innerRadius} 0 ${largeArcFlag} 1 ${startInner.x} ${startInner.y}`,
    "Z",
  ].join(" ");
}

function StaticPieChart({ model }: { model: ResolvedChartModel }) {
  const width = 760;
  const height = Math.max(250, model.height);
  const cx = 250;
  const cy = height / 2 + 8;
  const outerRadius = Math.min(118, height * 0.34);
  const innerRadius = model.kind === "doughnut" ? outerRadius * 0.56 : 0;
  const valueKey = model.series[0]?.key || "";
  const slices = model.rows
    .map((row, index) => ({
      label: textValue(row[model.categoryKey], String(index + 1)),
      value: Math.max(0, valueForRow(row, valueKey) ?? 0),
      color: PIE_COLORS[index % PIE_COLORS.length] || PIE_COLORS[0],
    }))
    .filter((slice) => slice.value > 0);
  const total = slices.reduce((sum, slice) => sum + slice.value, 0);
  let cursor = 0;

  return (
    <svg
      className="chat-inline-chart-svg"
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={model.title}
    >
      <rect x="0" y="0" width={width} height={height} rx="12" fill="transparent" />
      {total > 0 ? (
        slices.map((slice, index) => {
          const start = cursor;
          const end = cursor + (slice.value / total) * 360;
          cursor = end;
          return (
            <path
              key={`slice-${index}`}
              d={describePieSlice(cx, cy, outerRadius, innerRadius, start, end)}
              fill={slice.color}
              stroke="#111820"
              strokeWidth="2"
            />
          );
        })
      ) : (
        <text x={cx} y={cy} fill="#d8d0c4" textAnchor="middle">
          No positive values
        </text>
      )}
      {model.kind === "doughnut" && total > 0 ? (
        <>
          <text x={cx} y={cy - 4} fill="#fff8ed" textAnchor="middle" fontSize="22" fontWeight="700">
            {chartValueLabel(total)}
          </text>
          <text x={cx} y={cy + 18} fill="#d8d0c4" textAnchor="middle" fontSize="11">
            total
          </text>
        </>
      ) : null}
      <g transform={`translate(450 ${Math.max(34, cy - 74)})`}>
        {slices.slice(0, 8).map((slice, index) => (
          <g key={`legend-${index}`} transform={`translate(0 ${index * 22})`}>
            <rect x="0" y="-9" width="14" height="14" rx="4" fill={slice.color} />
            <text x="22" y="2" fill="#fff8ed" fontSize="12">
              {shortChartLabel(slice.label, 26)}
            </text>
            <text x="220" y="2" fill="#d8d0c4" fontSize="12" textAnchor="end">
              {chartValueLabel(slice.value)}
            </text>
          </g>
        ))}
      </g>
    </svg>
  );
}

function StaticInlineChart({ model }: { model: ResolvedChartModel }) {
  return model.kind === "pie" || model.kind === "doughnut" ? (
    <StaticPieChart model={model} />
  ) : (
    <StaticAxisChart model={model} />
  );
}

export function markdownFenceLanguage(className = ""): string {
  const token = className
    .split(/\s+/)
    .map((part) => part.trim())
    .find((part) => part.length > 0);
  return (token || "").replace(/^language-/, "").toLowerCase();
}

export function isAgentArkChartFence(className = ""): boolean {
  return markdownFenceLanguage(className) === AGENTARK_CHART_LANGUAGE;
}

// memo: the chart is a pure function of its fence code; parents re-render
// far more often than the code changes.
export const InlineAgentArkChart = memo(function InlineAgentArkChart({
  code,
}: {
  code: string;
}) {
  const model = useMemo(() => buildChartModel(code), [code]);

  if (!model.ok) {
    return (
      <Box className="chat-inline-chart chat-inline-chart-error">
        <Typography className="chat-inline-chart-title" variant="body2">
          Chart unavailable
        </Typography>
        <Typography className="chat-inline-chart-subtitle" variant="caption">
          {model.message}
        </Typography>
      </Box>
    );
  }

  return (
    <Box className="chat-inline-chart">
      <Box className="chat-inline-chart-header">
        <Typography className="chat-inline-chart-title" variant="body2">
          {model.title}
        </Typography>
        {model.subtitle ? (
          <Typography className="chat-inline-chart-subtitle" variant="caption">
            {model.subtitle}
          </Typography>
        ) : null}
      </Box>
      <StaticInlineChart model={model} />
    </Box>
  );
});
