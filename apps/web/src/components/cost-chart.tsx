"use client";

import { useMemo, useState } from "react";

interface DayCost {
  date: string;
  cost_usd: number;
}

const DAYS = 14;
const W = 600;
const H = 180;
const PAD = { top: 10, right: 8, bottom: 22, left: 44 };

function lastNDays(n: number): string[] {
  const days: string[] = [];
  const now = new Date();
  for (let i = n - 1; i >= 0; i--) {
    const d = new Date(now);
    d.setDate(now.getDate() - i);
    days.push(
      `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`,
    );
  }
  return days;
}

function formatUsd(v: number): string {
  return v >= 100 ? `$${v.toFixed(0)}` : `$${v.toFixed(2)}`;
}

/**
 * Daily cost over a fixed recent window. Padding the window with zero-cost
 * days keeps a single day of spend from rendering as one full-width bar.
 */
export function CostChart({ costByDay }: { costByDay: DayCost[] }) {
  const [hoverIdx, setHoverIdx] = useState<number | null>(null);

  const series = useMemo(() => {
    const byDate = new Map(costByDay.map((d) => [d.date, d.cost_usd]));
    return lastNDays(DAYS).map((date) => ({
      date,
      cost: byDate.get(date) ?? 0,
    }));
  }, [costByDay]);

  const maxCost = Math.max(...series.map((d) => d.cost));
  const total = series.reduce((sum, d) => sum + d.cost, 0);

  if (maxCost === 0) {
    return (
      <div className="flex h-36 items-center justify-center text-sm text-zinc-400 dark:text-zinc-600">
        No spend in the last {DAYS} days.
      </div>
    );
  }

  const yMax = maxCost * 1.15;
  const plotW = W - PAD.left - PAD.right;
  const plotH = H - PAD.top - PAD.bottom;
  const slot = plotW / DAYS;
  const barW = Math.min(slot * 0.62, 30);
  const x = (i: number) => PAD.left + slot * i + (slot - barW) / 2;
  const barH = (cost: number) => (cost / yMax) * plotH;
  const yTicks = [0, 1, 2].map((i) => (yMax * (i + 1)) / 3);

  const hover = hoverIdx != null ? series[hoverIdx] : null;

  return (
    <div>
      <p className="mb-2 text-xs text-zinc-500">
        Last {DAYS} days ·{" "}
        <span className="font-medium text-zinc-700 dark:text-zinc-300">
          {formatUsd(total)} total
        </span>
      </p>
      <div className="relative">
        <svg
          viewBox={`0 0 ${W} ${H}`}
          className="w-full"
          role="img"
          aria-label={`Daily cost for the last ${DAYS} days`}
          onMouseLeave={() => setHoverIdx(null)}
        >
          {yTicks.map((tick) => (
            <g key={tick}>
              <line
                x1={PAD.left}
                x2={W - PAD.right}
                y1={PAD.top + plotH - (tick / yMax) * plotH}
                y2={PAD.top + plotH - (tick / yMax) * plotH}
                className="stroke-zinc-200 dark:stroke-zinc-800"
                strokeWidth="1"
              />
              <text
                x={PAD.left - 6}
                y={PAD.top + plotH - (tick / yMax) * plotH + 3}
                textAnchor="end"
                className="fill-zinc-400 dark:fill-zinc-600 text-[10px] font-mono"
              >
                {formatUsd(tick)}
              </text>
            </g>
          ))}
          <line
            x1={PAD.left}
            x2={W - PAD.right}
            y1={PAD.top + plotH}
            y2={PAD.top + plotH}
            className="stroke-zinc-300 dark:stroke-zinc-700"
            strokeWidth="1"
          />

          {series.map((d, i) => (
            <g key={d.date}>
              {/* Hit area spans the full slot so hovering between bars works */}
              <rect
                x={PAD.left + slot * i}
                y={PAD.top}
                width={slot}
                height={plotH}
                fill="transparent"
                onMouseEnter={() => setHoverIdx(i)}
              />
              <rect
                x={x(i)}
                y={PAD.top + plotH - Math.max(barH(d.cost), d.cost > 0 ? 2 : 0)}
                width={barW}
                height={Math.max(barH(d.cost), d.cost > 0 ? 2 : 0)}
                rx="2"
                className={`pointer-events-none transition-colors ${
                  hoverIdx === i
                    ? "fill-violet-400"
                    : "fill-violet-500 dark:fill-violet-500/80"
                }`}
              />
              {(i === 0 || i === DAYS - 1 || i === Math.floor(DAYS / 2)) && (
                <text
                  x={PAD.left + slot * i + slot / 2}
                  y={H - 6}
                  textAnchor="middle"
                  className="fill-zinc-400 dark:fill-zinc-600 text-[10px] font-mono"
                >
                  {d.date.slice(5)}
                </text>
              )}
            </g>
          ))}
        </svg>

        {hover && (
          <div
            className="pointer-events-none absolute top-0 z-10 -translate-x-1/2 rounded-md border border-zinc-200 bg-white px-2.5 py-1.5 text-xs shadow-sm dark:border-zinc-700 dark:bg-zinc-900"
            style={{
              left: `${((PAD.left + slot * hoverIdx! + slot / 2) / W) * 100}%`,
            }}
          >
            <p className="font-mono font-medium text-zinc-900 dark:text-white">
              {formatUsd(hover.cost)}
            </p>
            <p className="whitespace-nowrap font-mono text-zinc-500">
              {hover.date}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
