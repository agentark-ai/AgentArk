import { Box, Card, CardContent, Stack, Typography } from "@mui/material";
import type { ReactNode } from "react";
import { useId, useMemo } from "react";

type Props = {
  label: string;
  value: string | number;
  icon?: ReactNode;
  hint?: string;
  compact?: boolean;
  sparkline?: number[];
  className?: string;
};

function Sparkline({ values, className }: { values: number[]; className?: string }) {
  const id = useId();
  const points = useMemo(() => {
    if (!values || values.length < 2) return "";
    const min = Math.min(...values);
    const max = Math.max(...values);
    const range = Math.max(1e-9, max - min);
    const w = 140;
    const h = 34;
    const padX = 2;
    const padY = 3;
    const innerW = w - padX * 2;
    const innerH = h - padY * 2;

    const xs = values.map((_, i) =>
      padX + (innerW * i) / (values.length - 1)
    );
    const ys = values.map((v) => padY + (innerH * (1 - (v - min) / range)));

    const line = xs
      .map((x, i) => `${x.toFixed(2)},${ys[i].toFixed(2)}`)
      .join(" ");

    const area = `${padX},${h - padY} ${line} ${w - padX},${h - padY}`;
    return { line, area, w, h };
  }, [values]);

  if (!points) return null;

  return (
    <svg
      className={className}
      width="100%"
      height="34"
      viewBox={`0 0 ${points.w} ${points.h}`}
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={`spark-${id}`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="rgba(47, 212, 255, 0.32)" />
          <stop offset="100%" stopColor="rgba(47, 212, 255, 0.00)" />
        </linearGradient>
      </defs>
      <polygon points={points.area} fill={`url(#spark-${id})`} />
      <polyline
        points={points.line}
        fill="none"
        stroke="rgba(47, 212, 255, 0.95)"
        strokeWidth="2"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function StatCard({
  label,
  value,
  icon,
  hint,
  compact = false,
  sparkline,
  className
}: Props) {
  return (
    <Card className={className} sx={compact ? { height: "100%" } : undefined}>
      <CardContent sx={compact ? { p: 1.25 } : undefined}>
        <Stack
          direction="row"
          sx={{
            alignItems: "center",
            justifyContent: "space-between",
            mb: 1
          }}>
          <Typography variant="body2" sx={{
            color: "text.secondary"
          }}>
            {label}
          </Typography>
          {icon}
        </Stack>
        <Typography variant={compact ? "h6" : "h5"} sx={{
          fontWeight: 700
        }}>
          {value}
        </Typography>
        <Typography variant="caption" sx={{
          color: "text.secondary"
        }}>
          {hint || "Live"}
        </Typography>
        {sparkline && sparkline.length >= 2 ? (
          <Box sx={{ mt: 0.6, opacity: 0.95 }}>
            <Sparkline values={sparkline} className="stat-spark" />
          </Box>
        ) : null}
      </CardContent>
    </Card>
  );
}
