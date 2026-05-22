import { Box, Stack, Tooltip, Typography } from "@mui/material";

type MetricLegendRow = {
  label: string;
  value: string;
};

type Props = {
  title: string;
  value: string;
  values: number[];
  rows: MetricLegendRow[];
  palette: string[];
  className?: string;
  chartHeight?: number;
  compact?: boolean;
  rowsLimit?: number;
};

export function MetricBarCard({
  title,
  value,
  values,
  rows,
  palette,
  className = "",
  compact = false,
  rowsLimit,
}: Props) {
  const visibleRows =
    typeof rowsLimit === "number" && rowsLimit > 0 && rows.length > rowsLimit
      ? rows.slice(Math.max(0, rows.length - rowsLimit))
      : rows;
  const hasMeaningfulData =
    values.some((entry) => entry > 0) || rows.length > 0;

  const positiveTotal = values.reduce(
    (sum, v) => sum + (Number.isFinite(v) && v > 0 ? v : 0),
    0,
  );
  const MAX_METER_SEGMENTS = 8;
  const rawSegments = values.map((entry, index) => {
    const safe = Number.isFinite(entry) && entry > 0 ? entry : 0;
    const pct = positiveTotal > 0 ? (safe / positiveTotal) * 100 : 0;
    const color = palette[index % palette.length];
    return {
      pct,
      value: safe,
      color,
      label: rows[index]?.label ?? `Series ${index + 1}`,
      displayValue: rows[index]?.value ?? "",
    };
  });
  const positiveSegments = rawSegments.filter((seg) => seg.pct > 0);
  let segments: typeof rawSegments;
  if (positiveSegments.length > MAX_METER_SEGMENTS) {
    const sorted = [...positiveSegments].sort((a, b) => b.pct - a.pct);
    const head = sorted.slice(0, MAX_METER_SEGMENTS - 1);
    const tail = sorted.slice(MAX_METER_SEGMENTS - 1);
    const tailPct = tail.reduce((sum, seg) => sum + seg.pct, 0);
    const tailValue = tail.reduce((sum, seg) => sum + seg.value, 0);
    const previewLabels = tail
      .slice(0, 4)
      .map((seg) => seg.label)
      .join(", ");
    segments = [
      ...head,
      {
        pct: tailPct,
        value: tailValue,
        color: "#c8d8c9",
        label: `Other (${tail.length} more)`,
        displayValue:
          tail.length > 4
            ? `${previewLabels} +${tail.length - 4} more`
            : previewLabels,
      },
    ];
  } else {
    segments = positiveSegments;
  }

  return (
    <Box
      className={`list-shell metric-bar-card stat-card rise-in${hasMeaningfulData ? "" : " metric-bar-card-empty"}${compact ? " metric-bar-card-compact" : ""} ${className}`.trim()}
      sx={{
        p: compact ? 1.15 : 1.6,
        borderRadius: "8px",
        border: "1px solid var(--ui-rgba-108-156-212-180)",
        background: "var(--ui-rgba-12-18-28-860)",
      }}
    >
      <Typography variant="subtitle1" className="metric-bar-card-title">
        {title}
      </Typography>
      <Typography variant="h4" className="metric-bar-card-value">
        {value}
      </Typography>
      {hasMeaningfulData ? (
        visibleRows.length <= 1 ? (
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
              mt: compact ? 0.5 : 0.75,
              display: "block",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
            title={visibleRows[0]?.label ?? ""}
          >
            {visibleRows.length === 1 ? `via ${visibleRows[0].label}` : ""}
          </Typography>
        ) : (
        <>
          <Box
            className="metric-bar-card-meter"
            sx={{
              mt: compact ? 1.0 : 1.4,
              mb: compact ? 0.6 : 0.9,
              height: 10,
              borderRadius: 999,
              display: "flex",
              gap: "2px",
              padding: "1px",
              background: "var(--ui-rgba-108-156-212-060)",
              border: "1px solid var(--ui-rgba-108-156-212-120)",
              boxShadow: "inset 0 1px 0 var(--ui-rgba-255-255-255-030)",
              overflow: "hidden",
              position: "relative",
            }}
          >
            {positiveTotal > 0 ? (
              segments
                .filter((seg) => seg.pct > 0)
                .map((seg, idx) => (
                  <Tooltip
                    key={`${title}-seg-${idx}`}
                    arrow
                    placement="top"
                    title={
                      <Box sx={{ px: 0.25, py: 0.15 }}>
                        <Typography
                          variant="caption"
                          sx={{
                            display: "block",
                            color: "#fff8ed",
                            fontWeight: 600,
                          }}
                        >
                          {seg.label}
                        </Typography>
                        <Typography
                          variant="caption"
                          sx={{
                            display: "block",
                            color: "#d8d0c4",
                          }}
                        >
                          {seg.displayValue} · {seg.pct.toFixed(1)}%
                        </Typography>
                      </Box>
                    }
                  >
                    <Box
                      sx={{
                        flex: `0 0 calc(${seg.pct}% - ${segments.length > 1 ? 2 : 0}px)`,
                        minWidth: 4,
                        borderRadius: 999,
                        background: `linear-gradient(90deg, ${seg.color} 0%, ${seg.color}B8 55%, ${seg.color}66 100%)`,
                        boxShadow: `0 0 14px ${seg.color}55, inset 0 1px 0 var(--ui-rgba-255-255-255-180)`,
                        position: "relative",
                        overflow: "hidden",
                        transition:
                          "filter 180ms ease, box-shadow 180ms ease, transform 180ms ease",
                        "&:hover": {
                          filter: "brightness(1.15)",
                          boxShadow: `0 0 22px ${seg.color}99, inset 0 1px 0 var(--ui-rgba-255-255-255-250)`,
                        },
                        "&::after": {
                          content: '""',
                          position: "absolute",
                          inset: 0,
                          background:
                            "linear-gradient(90deg, transparent 0%, var(--ui-rgba-255-255-255-220) 50%, transparent 100%)",
                          transform: "translateX(-100%)",
                          animation:
                            "metric-meter-shimmer 3.2s ease-in-out infinite",
                          animationDelay: `${idx * 0.35}s`,
                        },
                        "@media (prefers-reduced-motion: reduce)": {
                          "&::after": { animation: "none" },
                        },
                      }}
                    />
                  </Tooltip>
                ))
            ) : (
              <Box
                sx={{
                  flex: 1,
                  borderRadius: 999,
                  background:
                    "linear-gradient(90deg, var(--ui-rgba-108-156-212-100), var(--ui-rgba-108-156-212-040))",
                }}
              />
            )}
          </Box>
          <Stack spacing={compact ? 0.2 : 0.5} sx={{ mt: compact ? 0.4 : 0.6 }}>
            {visibleRows.map((row, index) => (
              <Stack
                key={`${title}-${row.label}-${index}`}
                className="metric-bar-card-row"
                direction="row"
                sx={{
                  justifyContent: "space-between",
                  alignItems: "center",
                }}
              >
                <Stack
                  direction="row"
                  spacing={0.8}
                  sx={{
                    alignItems: "center",
                    minWidth: 0,
                  }}
                >
                  <Box
                    sx={{
                      width: 8,
                      height: 8,
                      borderRadius: "50%",
                      bgcolor: palette[index % palette.length],
                      flex: "0 0 auto",
                      boxShadow: `0 0 8px ${palette[index % palette.length]}88`,
                    }}
                  />
                  <Typography
                    variant="body2"
                    className="metric-bar-card-row-label"
                    noWrap
                    title={row.label}
                  >
                    {row.label}
                  </Typography>
                </Stack>
                <Typography
                  variant="body2"
                  className="metric-bar-card-row-value"
                >
                  {row.value}
                </Typography>
              </Stack>
            ))}
          </Stack>
        </>
        )
      ) : (
        <Typography variant="body2" className="metric-bar-card-empty-copy">
          No usage in the selected range yet.
        </Typography>
      )}
    </Box>
  );
}
