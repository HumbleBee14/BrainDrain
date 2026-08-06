"use client";

import { useMemo, useRef, useState } from "react";

export interface LossPoint {
  step: number;
  loss: number;
  epoch?: number;
  phase?: string;
}

const W = 600;
const H = 220;
const PAD = { top: 12, right: 16, bottom: 24, left: 48 };

function formatLoss(v: number): string {
  if (v >= 100) return v.toFixed(0);
  if (v >= 1) return v.toFixed(2);
  return v.toFixed(3);
}

export function LossChart({ points }: { points: LossPoint[] }) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [hoverIdx, setHoverIdx] = useState<number | null>(null);

  const chart = useMemo(() => {
    if (points.length === 0) return null;

    const losses = points.map((p) => p.loss);
    const steps = points.map((p) => p.step);
    const minLoss = Math.min(...losses);
    const maxLoss = Math.max(...losses);
    const lossPad = Math.max((maxLoss - minLoss) * 0.08, maxLoss * 0.02, 1e-6);
    const yMin = Math.max(0, minLoss - lossPad);
    const yMax = maxLoss + lossPad;
    const xMin = Math.min(...steps);
    const xMax = Math.max(...steps);

    const plotW = W - PAD.left - PAD.right;
    const plotH = H - PAD.top - PAD.bottom;
    const x = (step: number) =>
      PAD.left +
      (xMax === xMin ? plotW / 2 : ((step - xMin) / (xMax - xMin)) * plotW);
    const y = (loss: number) =>
      PAD.top + plotH - ((loss - yMin) / (yMax - yMin)) * plotH;

    const line = points
      .map(
        (p, i) =>
          `${i === 0 ? "M" : "L"}${x(p.step).toFixed(1)},${y(p.loss).toFixed(1)}`,
      )
      .join(" ");
    const area = `${line} L${x(points[points.length - 1].step).toFixed(1)},${(
      PAD.top + plotH
    ).toFixed(1)} L${x(points[0].step).toFixed(1)},${(PAD.top + plotH).toFixed(1)} Z`;

    const yTicks = [0, 1, 2, 3].map((i) => yMin + ((yMax - yMin) * i) / 3);

    const phaseStarts: { step: number; phase: string }[] = [];
    for (let i = 1; i < points.length; i++) {
      const phase = points[i].phase;
      if (phase && phase !== points[i - 1].phase) {
        phaseStarts.push({ step: points[i].step, phase });
      }
    }

    return { x, y, line, area, yTicks, xMin, xMax, plotH, phaseStarts };
  }, [points]);

  if (!chart) {
    return (
      <div className="flex h-40 items-center justify-center text-sm text-zinc-400 dark:text-zinc-600">
        Waiting for training metrics...
      </div>
    );
  }

  const handleMove = (e: React.MouseEvent<SVGSVGElement>) => {
    const rect = svgRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return;
    const vx = ((e.clientX - rect.left) / rect.width) * W;
    let nearest = 0;
    let best = Infinity;
    for (let i = 0; i < points.length; i++) {
      const d = Math.abs(chart.x(points[i].step) - vx);
      if (d < best) {
        best = d;
        nearest = i;
      }
    }
    setHoverIdx(nearest);
  };

  const last = points[points.length - 1];
  const hover = hoverIdx != null ? points[hoverIdx] : null;

  return (
    <div className="relative">
      <svg
        ref={svgRef}
        viewBox={`0 0 ${W} ${H}`}
        className="w-full cursor-crosshair"
        role="img"
        aria-label="Training loss over steps"
        onMouseMove={handleMove}
        onMouseLeave={() => setHoverIdx(null)}
      >
        {chart.yTicks.map((tick) => (
          <g key={tick}>
            <line
              x1={PAD.left}
              x2={W - PAD.right}
              y1={chart.y(tick)}
              y2={chart.y(tick)}
              className="stroke-zinc-200 dark:stroke-zinc-800"
              strokeWidth="1"
            />
            <text
              x={PAD.left - 6}
              y={chart.y(tick) + 3}
              textAnchor="end"
              className="fill-zinc-400 dark:fill-zinc-600 text-[10px] font-mono"
            >
              {formatLoss(tick)}
            </text>
          </g>
        ))}

        {chart.phaseStarts.map(({ step, phase }) => (
          <g key={`${phase}-${step}`}>
            <line
              x1={chart.x(step)}
              x2={chart.x(step)}
              y1={PAD.top}
              y2={PAD.top + chart.plotH}
              className="stroke-zinc-300 dark:stroke-zinc-700"
              strokeWidth="1"
              strokeDasharray="3 3"
            />
            <text
              x={chart.x(step) + 4}
              y={PAD.top + 10}
              className="fill-zinc-400 dark:fill-zinc-500 text-[10px] uppercase"
            >
              {phase}
            </text>
          </g>
        ))}

        <path
          d={chart.area}
          className="fill-violet-500/10 dark:fill-violet-500/15"
        />
        <path
          d={chart.line}
          fill="none"
          className="stroke-violet-500"
          strokeWidth="1.75"
          strokeLinejoin="round"
          strokeLinecap="round"
        />

        <circle
          cx={chart.x(last.step)}
          cy={chart.y(last.loss)}
          r="3"
          className="fill-violet-500"
        />

        {hover && (
          <g>
            <line
              x1={chart.x(hover.step)}
              x2={chart.x(hover.step)}
              y1={PAD.top}
              y2={PAD.top + chart.plotH}
              className="stroke-zinc-400 dark:stroke-zinc-500"
              strokeWidth="1"
            />
            <circle
              cx={chart.x(hover.step)}
              cy={chart.y(hover.loss)}
              r="4"
              className="fill-violet-500 stroke-white dark:stroke-zinc-950"
              strokeWidth="1.5"
            />
          </g>
        )}

        <text
          x={PAD.left}
          y={H - 6}
          className="fill-zinc-400 dark:fill-zinc-600 text-[10px] font-mono"
        >
          step {chart.xMin}
        </text>
        <text
          x={W - PAD.right}
          y={H - 6}
          textAnchor="end"
          className="fill-zinc-400 dark:fill-zinc-600 text-[10px] font-mono"
        >
          step {chart.xMax}
        </text>
      </svg>

      {hover && (
        <div
          className="pointer-events-none absolute top-1 z-10 -translate-x-1/2 rounded-md border border-zinc-200 bg-white px-2.5 py-1.5 text-xs shadow-sm dark:border-zinc-700 dark:bg-zinc-900"
          style={{
            left: `${(chart.x(hover.step) / W) * 100}%`,
          }}
        >
          <p className="font-mono font-medium text-zinc-900 dark:text-white">
            loss {formatLoss(hover.loss)}
          </p>
          <p className="whitespace-nowrap font-mono text-zinc-500">
            step {hover.step}
            {hover.epoch != null && ` · ep ${hover.epoch}`}
            {hover.phase && ` · ${hover.phase}`}
          </p>
        </div>
      )}
    </div>
  );
}
