import { useCallback, useEffect, useRef, useState } from "react";
import {
  axisScale,
  bandGeometry,
  columnPath,
  labelStride,
  linePath,
  stackSegments,
} from "../charts";
import { formatRangeHour } from "../dateRanges";
import { formatAxisDate, formatAxisHour, hourDate } from "../format";

/**
 * The one chart the dashboard draws.
 *
 * Every chart on this dashboard plots the same x-axis — the buckets of the selected range —
 * and differs only in what it stacks or traces on it, so there is one component and four
 * configurations of it rather than four charts with four copies of an axis, a legend, a
 * hover layer and a tooltip.
 *
 * A bucket is a calendar day, or an hour when the range is short enough to be read that
 * way. Only the labels and the tooltip heading know which; everything below them is the
 * same axis either way.
 */

export type ChartResolution = "day" | "hour";

export interface ChartSeries {
  key: string;
  label: string;
  /** The mark's colour. Text never wears it; the swatch beside the text carries identity. */
  color: string;
  /** One value per bucket; `null` where it has no reading, which is drawn as a gap. */
  values: Array<number | null>;
}

/** A dated annotation drawn on the axis rather than in the data, such as a quota restart. */
export interface ChartMarker {
  /** The bucket key the annotation sits on. */
  bucket: string;
  label: string;
  tone: "muted" | "warning";
}

interface TrendChartProps {
  title: string;
  subtitle?: string;
  buckets: string[];
  resolution: ChartResolution;
  series: ChartSeries[];
  mode: "stacked" | "line";
  /** The exact value, for the tooltip. */
  formatValue: (value: number) => string;
  /** The short value, for the axis. Defaults to `formatValue`. */
  formatTick?: (value: number) => string;
  /** A fixed axis top, for scales that mean something at 100 whatever the data reaches. */
  maxValue?: number;
  markers?: ChartMarker[];
  selectedBucket?: string | null;
  onSelectBucket?: (bucket: string | null) => void;
  emptyCopy: string;
  /** Holds the previous render at reduced opacity while the next range arrives. */
  loading?: boolean;
}

const PLOT_HEIGHT = 168;
const AXIS_HEIGHT = 22;
const TOP_PADDING = 10;
const LEFT_GUTTER = 52;
const RIGHT_PADDING = 12;
const HEIGHT = PLOT_HEIGHT + AXIS_HEIGHT + TOP_PADDING;

/** The rendered width of the plot, which only the browser can tell us. */
function useMeasuredWidth(): [(node: HTMLDivElement | null) => void, number] {
  const [width, setWidth] = useState(0);
  const observer = useRef<ResizeObserver | null>(null);
  const attach = useCallback((node: HTMLDivElement | null) => {
    observer.current?.disconnect();
    if (node === null) return;
    setWidth(node.clientWidth);
    observer.current = new ResizeObserver((entries) => {
      setWidth(entries[0]?.contentRect.width ?? 0);
    });
    observer.current.observe(node);
  }, []);
  useEffect(() => () => observer.current?.disconnect(), []);
  return [attach, width];
}

/** The hourly axis carries a date and a clock time, so its labels need more room. */
const HOUR_LABEL_SPACING = 96;

/**
 * Which buckets carry an axis label, and what it says.
 *
 * An hourly axis repeats the same date across a whole day, so only the first label of each
 * day names it and the rest are the clock alone. The last bucket is always labelled: it is
 * the end of the range, which is what a reader looks for first.
 */
function axisLabels(
  buckets: string[],
  resolution: ChartResolution,
  plotWidth: number,
): Array<{ bucket: string; index: number; text: string }> {
  const stride = labelStride(
    buckets.length,
    plotWidth,
    resolution === "hour" ? HOUR_LABEL_SPACING : undefined,
  );
  let labelledDate: string | null = null;
  return buckets.flatMap((bucket, index) => {
    if (index % stride !== 0 && index !== buckets.length - 1) return [];
    if (resolution === "day") return [{ bucket, index, text: formatAxisDate(bucket) }];
    const date = hourDate(bucket);
    const text =
      date === labelledDate
        ? formatAxisHour(bucket)
        : `${formatAxisDate(date)} ${formatAxisHour(bucket)}`;
    labelledDate = date;
    return [{ bucket, index, text }];
  });
}

/** What the tooltip calls the bucket under the pointer. */
function bucketHeading(bucket: string, resolution: ChartResolution): string {
  return resolution === "day" ? formatAxisDate(bucket) : formatRangeHour(bucket);
}

export function TrendChart({
  title,
  subtitle,
  buckets,
  resolution,
  series,
  mode,
  formatValue,
  formatTick,
  maxValue,
  markers = [],
  selectedBucket = null,
  onSelectBucket,
  emptyCopy,
  loading = false,
}: TrendChartProps) {
  const [attachPlot, width] = useMeasuredWidth();
  const [hovered, setHovered] = useState<number | null>(null);

  const plotWidth = Math.max(0, width - LEFT_GUTTER - RIGHT_PADDING);
  const totals = buckets.map((_, index) =>
    mode === "stacked"
      ? series.reduce((sum, entry) => sum + Math.max(0, entry.values[index] ?? 0), 0)
      : series.reduce((peak, entry) => Math.max(peak, entry.values[index] ?? 0), 0),
  );
  const highest = totals.reduce((peak, value) => Math.max(peak, value), 0);
  const scale = axisScale(maxValue ?? highest);
  const axisMax = maxValue ?? scale.max;
  const ticks = maxValue === undefined ? scale.ticks : axisScale(maxValue).ticks;
  const bands = bandGeometry(plotWidth, buckets.length);
  const labelled = axisLabels(buckets, resolution, plotWidth);
  const toY = (value: number) =>
    TOP_PADDING + PLOT_HEIGHT - (Math.min(value, axisMax) / axisMax) * PLOT_HEIGHT;

  const hasData = highest > 0;
  const activeIndex = hovered ?? (selectedBucket ? buckets.indexOf(selectedBucket) : -1);
  const tooltipIndex = activeIndex >= 0 ? activeIndex : null;

  function moveHover(step: number) {
    if (buckets.length === 0) return;
    const from = hovered ?? (selectedBucket ? buckets.indexOf(selectedBucket) : -1);
    const next = Math.min(buckets.length - 1, Math.max(0, (from < 0 ? 0 : from) + step));
    setHovered(next);
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowRight") moveHover(1);
    else if (event.key === "ArrowLeft") moveHover(-1);
    else if (event.key === "Escape") setHovered(null);
    else if ((event.key === "Enter" || event.key === " ") && hovered !== null) {
      onSelectBucket?.(buckets[hovered] === selectedBucket ? null : buckets[hovered]);
    } else return;
    event.preventDefault();
  }

  const markersByBucket = new Map(markers.map((marker) => [marker.bucket, marker]));
  const tooltipRows =
    tooltipIndex === null
      ? []
      : series.filter((entry) => mode === "line" || (entry.values[tooltipIndex] ?? 0) > 0);

  return (
    <article className={`chart-card${loading ? " chart-loading" : ""}`}>
      <div className="card-heading">
        <div>
          <h3>{title}</h3>
          {subtitle ? <span>{subtitle}</span> : null}
        </div>
      </div>
      {/* Identity never rests on colour alone: two or more series always carry a legend,
          and a single one is already named by the card's own title. */}
      {series.length > 1 || markers.length > 0 ? (
        <ul className="chart-legend">
          {series.map((entry) => (
            <li key={entry.key}>
              <i
                className={mode === "line" ? "legend-line" : "legend-swatch"}
                style={{ background: entry.color }}
              />
              {entry.label}
            </li>
          ))}
          {markers.length > 0 ? (
            <li>
              <i className="legend-marker" />
              Quota restart
            </li>
          ) : null}
        </ul>
      ) : null}
      <div className="chart-plot" ref={attachPlot}>
        {hasData ? null : <p className="empty-copy chart-empty">{emptyCopy}</p>}
        <div
          className="chart-surface"
          tabIndex={0}
          role="application"
          aria-label={`${title}. Use the arrow keys to read each ${resolution}.`}
          onKeyDown={onKeyDown}
          onPointerLeave={() => setHovered(null)}
          onBlur={() => setHovered(null)}
        >
          <svg width={width} height={HEIGHT} role="presentation" focusable="false">
            <g transform={`translate(${LEFT_GUTTER} 0)`}>
              {ticks.map((tick) => (
                <line
                  key={tick}
                  className="chart-gridline"
                  x1={0}
                  x2={plotWidth}
                  y1={toY(tick)}
                  y2={toY(tick)}
                />
              ))}
              {buckets.map((bucket, index) => {
                const marker = markersByBucket.get(bucket);
                if (!marker) return null;
                return (
                  <g
                    key={`marker-${bucket}`}
                    className={`chart-marker chart-marker-${marker.tone}`}
                  >
                    <line
                      x1={bands.center(index)}
                      x2={bands.center(index)}
                      y1={TOP_PADDING}
                      y2={TOP_PADDING + PLOT_HEIGHT}
                    />
                    <circle cx={bands.center(index)} cy={TOP_PADDING} r={3.5} />
                  </g>
                );
              })}
              {mode === "stacked"
                ? buckets.map((bucket, index) => {
                    const segments = stackSegments(
                      series.map((entry) => Math.max(0, entry.values[index] ?? 0)),
                      (value) => toY(value),
                    );
                    // Only the segment on top of the stack ends in a rounded data-end; the
                    // ones under it end where the next begins.
                    const topSegment = segments.reduce(
                      (top, segment) => (segment === null ? top : segment.index),
                      -1,
                    );
                    return (
                      <g
                        key={bucket}
                        className={
                          index === activeIndex ? "chart-band chart-band-active" : "chart-band"
                        }
                      >
                        {segments.map((segment) =>
                          segment === null ? null : (
                            <path
                              key={series[segment.index].key}
                              d={columnPath(
                                bands.left(index),
                                segment.top,
                                bands.barWidth,
                                segment.height,
                                segment.index === topSegment ? 4 : 0,
                              )}
                              fill={series[segment.index].color}
                            />
                          ),
                        )}
                      </g>
                    );
                  })
                : series.map((entry) => {
                    const points = entry.values.map((value, index) =>
                      value === null || !Number.isFinite(value)
                        ? null
                        : { x: bands.center(index), y: toY(value) },
                    );
                    const drawn = points.filter((point) => point !== null);
                    const last = drawn.at(-1);
                    return (
                      <g key={entry.key}>
                        {series.length === 1 && drawn.length > 1 ? (
                          <path
                            className="chart-area"
                            d={`${linePath(points)} L${drawn.at(-1)!.x.toFixed(2)} ${toY(0)} L${drawn[0]!.x.toFixed(2)} ${toY(0)} Z`}
                            fill={entry.color}
                          />
                        ) : null}
                        <path className="chart-line" d={linePath(points)} stroke={entry.color} />
                        {last ? (
                          <circle
                            className="chart-endpoint"
                            cx={last.x}
                            cy={last.y}
                            r={4}
                            fill={entry.color}
                          />
                        ) : null}
                        {activeIndex >= 0 && points[activeIndex] ? (
                          <circle
                            className="chart-endpoint"
                            cx={points[activeIndex]!.x}
                            cy={points[activeIndex]!.y}
                            r={4}
                            fill={entry.color}
                          />
                        ) : null}
                      </g>
                    );
                  })}
              {labelled.map(({ bucket, index, text }) => (
                <text
                  key={`label-${bucket}`}
                  className="chart-axis-label"
                  x={bands.center(index)}
                  y={TOP_PADDING + PLOT_HEIGHT + 15}
                  textAnchor="middle"
                >
                  {text}
                </text>
              ))}
              {/* The hit target is the whole band, never the painted mark. */}
              {buckets.map((bucket, index) => (
                <rect
                  key={`hit-${bucket}`}
                  className="chart-hit"
                  x={bands.band * index}
                  y={TOP_PADDING}
                  width={bands.band}
                  height={PLOT_HEIGHT}
                  onPointerEnter={() => setHovered(index)}
                  onClick={() => onSelectBucket?.(bucket === selectedBucket ? null : bucket)}
                />
              ))}
            </g>
            {ticks.map((tick) => (
              <text
                key={`tick-${tick}`}
                className="chart-axis-label"
                x={LEFT_GUTTER - 10}
                y={toY(tick) + 4}
                textAnchor="end"
              >
                {(formatTick ?? formatValue)(tick)}
              </text>
            ))}
          </svg>
          {tooltipIndex !== null && width > 0 ? (
            <div
              className="chart-tooltip"
              style={{
                left: Math.min(
                  Math.max(LEFT_GUTTER + bands.center(tooltipIndex), 90),
                  Math.max(90, width - 90),
                ),
              }}
            >
              <strong>{bucketHeading(buckets[tooltipIndex], resolution)}</strong>
              {/* A stacked series that contributed nothing that day drew nothing either,
                  so listing it is a row of zeroes between the values being compared. A
                  line keeps its zero, which is a reading rather than an absence. */}
              {tooltipRows.map((entry) => (
                <p key={entry.key}>
                  <i style={{ background: entry.color }} />
                  <span>{entry.label}</span>
                  <b>
                    {entry.values[tooltipIndex] === null
                      ? "No reading"
                      : formatValue(entry.values[tooltipIndex] ?? 0)}
                  </b>
                </p>
              ))}
              {tooltipRows.length === 0 ? (
                <p className="chart-tooltip-note">Nothing recorded</p>
              ) : null}
              {mode === "stacked" && tooltipRows.length > 1 ? (
                <p className="chart-tooltip-total">
                  <span>Total</span>
                  <b>{formatValue(totals[tooltipIndex])}</b>
                </p>
              ) : null}
              {markersByBucket.has(buckets[tooltipIndex]) ? (
                <p className="chart-tooltip-note">
                  {markersByBucket.get(buckets[tooltipIndex])!.label}
                </p>
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
    </article>
  );
}
