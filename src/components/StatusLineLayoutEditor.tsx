import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { ansiRuns } from "../ansi";
import type { saveAppSettings } from "../appSettings";
import type {
  AppSettings,
  ProviderLabelStyle,
  StatusLineColour,
  StatusLineLayout,
  StatusLineQuotaFormat,
  StatusLineSegment,
  StatusLineSeparators,
} from "../types";

const ROWS = [1, 2, 3];

const SEGMENT_NAMES: Record<string, string> = {
  model: "Model",
  mode: "Effort, thinking and fast mode",
  sessionName: "Session name",
  agent: "Agent",
  outputStyle: "Output style, when not the default",
  vimMode: "Vim mode",
  version: "Claude Code version",
  directory: "Directory",
  branch: "Branch and uncommitted changes",
  gitOperation: "Rebase, merge or cherry-pick in progress",
  stash: "Stash entries",
  lastCommit: "Time since the last commit",
  tag: "Latest tag and commits since it",
  pullRequest: "Pull request",
  context: "Context",
  largeContext: "Context over 200k warning",
  cache: "Cache hit",
  sessionCost: "Session cost",
  linesChanged: "Lines added and removed this session",
  duration: "Session time and API time",
  today: "Today's tokens and cost, every provider",
  week: "Last 7 days' tokens and cost, every provider",
  lastReset: "Most recent quota reset",
  resetCredits: "Reset credits",
  quotaAge: "How old the other providers' quota is",
};

const PROVIDER_NAMES: Record<string, string> = { claude: "Claude", codex: "Codex" };

function segmentName(id: string): string {
  if (id.startsWith("quota:")) {
    const provider = id.slice("quota:".length);
    return `${PROVIDER_NAMES[provider] ?? provider} quota`;
  }
  return SEGMENT_NAMES[id] ?? id;
}

const QUOTA_PARTS: [keyof StatusLineQuotaFormat, string][] = [
  ["used", "Used"],
  ["remaining", "Remaining"],
  ["countdown", "Countdown"],
  ["pace", "Pace marker"],
  ["bar", "Bar"],
];

/** Swaps a segment with its neighbour among the segments sharing its row. */
function moved(segments: StatusLineSegment[], id: string, step: -1 | 1): StatusLineSegment[] {
  const index = segments.findIndex((segment) => segment.id === id);
  const peers = segments.flatMap((segment, at) =>
    segment.row === segments[index].row ? [at] : [],
  );
  const other = peers[peers.indexOf(index) + step];
  if (other === undefined) return segments;
  const next = [...segments];
  [next[index], next[other]] = [next[other], next[index]];
  return next;
}

/** Moves a segment to the end of another row. */
function placed(segments: StatusLineSegment[], id: string, row: number): StatusLineSegment[] {
  return [
    ...segments.filter((segment) => segment.id !== id),
    ...segments.filter((segment) => segment.id === id).map((segment) => ({ ...segment, row })),
  ];
}

/**
 * The status line's layout. The preview is the core's own rendering of a sample session,
 * so what it shows is what Claude Code will print.
 */
export function StatusLineLayoutEditor({
  settings,
  disabled,
  onChange,
}: {
  settings: AppSettings;
  disabled: boolean;
  onChange: (patch: Parameters<typeof saveAppSettings>[0]) => Promise<void>;
}) {
  const layout = settings.statusLineLayout;
  const labels = settings.statusLineProviderLabels;
  const [preview, setPreview] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    void invoke<string>("preview_claude_status_line", { layout, labels }).then((line) => {
      if (current) setPreview(line);
    });
    return () => {
      current = false;
    };
  }, [layout, labels]);

  // Built from the saved record at the front of the queue, so two quick clicks never
  // start from the same stale layout.
  const update = (change: (layout: StatusLineLayout) => Partial<StatusLineLayout>) =>
    void onChange((saved) => ({
      statusLineLayout: { ...saved.statusLineLayout, ...change(saved.statusLineLayout) },
    }));
  const updateSegments = (change: (segments: StatusLineSegment[]) => StatusLineSegment[]) =>
    update((saved) => ({ segments: change(saved.segments) }));

  let offset = 0;
  const runs = (preview ? ansiRuns(preview) : []).map((run) => {
    const key = offset;
    offset += run.text.length;
    return { ...run, key };
  });

  return (
    <div className="consent-options">
      <pre className="status-line-preview">
        {runs.map((run) => (
          <span key={run.key} className={run.level ? `ansi-${run.level}` : undefined}>
            {run.text}
          </span>
        ))}
      </pre>
      {ROWS.map((row) => {
        const segments = layout.segments.filter((segment) => segment.row === row);
        return (
          <fieldset key={row} className="status-line-row">
            <legend>Row {row}</legend>
            {segments.map((segment, position) => {
              const name = segmentName(segment.id);
              return (
                <div key={segment.id} className="status-line-segment">
                  <label>
                    <input
                      type="checkbox"
                      checked={segment.enabled}
                      disabled={disabled}
                      onChange={(event) => {
                        const enabled = event.target.checked;
                        updateSegments((all) =>
                          all.map((item) => (item.id === segment.id ? { ...item, enabled } : item)),
                        );
                      }}
                    />
                    {name}
                  </label>
                  <button
                    type="button"
                    aria-label={`Move ${name} earlier`}
                    disabled={disabled || position === 0}
                    onClick={() => updateSegments((all) => moved(all, segment.id, -1))}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    aria-label={`Move ${name} later`}
                    disabled={disabled || position === segments.length - 1}
                    onClick={() => updateSegments((all) => moved(all, segment.id, 1))}
                  >
                    ↓
                  </button>
                  <select
                    aria-label={`Row for ${name}`}
                    value={row}
                    disabled={disabled}
                    onChange={(event) => {
                      const target = Number(event.target.value);
                      updateSegments((all) => placed(all, segment.id, target));
                    }}
                  >
                    {ROWS.map((option) => (
                      <option key={option} value={option}>
                        Row {option}
                      </option>
                    ))}
                  </select>
                </div>
              );
            })}
          </fieldset>
        );
      })}
      <div className="status-line-parts">
        Quota shows
        {QUOTA_PARTS.map(([part, label]) => (
          <label key={part}>
            <input
              type="checkbox"
              checked={layout.quota[part]}
              disabled={disabled}
              onChange={(event) => {
                const value = event.target.checked;
                update((saved) => ({ quota: { ...saved.quota, [part]: value } }));
              }}
            />
            {label}
          </label>
        ))}
      </div>
      <label>
        Separators
        <select
          value={layout.separators}
          disabled={disabled}
          onChange={(event) => {
            const separators = event.target.value as StatusLineSeparators;
            update(() => ({ separators }));
          }}
        >
          <option value="classic">Classic ( · and | )</option>
          <option value="arrow">Arrow ( · and › )</option>
          <option value="powerline">Powerline (needs a Powerline or Nerd Font)</option>
        </select>
      </label>
      <label>
        Colour
        <select
          value={layout.colour}
          disabled={disabled}
          onChange={(event) => {
            const colour = event.target.value as StatusLineColour;
            update(() => ({ colour }));
          }}
        >
          <option value="full">Quota and context</option>
          <option value="quotaOnly">Quota only</option>
          <option value="none">None</option>
        </select>
      </label>
      <label>
        Provider names
        <select
          value={labels}
          disabled={disabled}
          onChange={(event) =>
            void onChange({ statusLineProviderLabels: event.target.value as ProviderLabelStyle })
          }
        >
          <option value="short">Short (CDX, CLD)</option>
          <option value="full">Full (Codex, Claude Code)</option>
        </select>
      </label>
    </div>
  );
}
