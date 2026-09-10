import { cn } from "cn";
import { useAtomValue } from "jotai/react";

import type { Snapshot, StageStatus } from "@/api/schema";
import { useNow } from "@/components/hooks/use-now";
import { StageStrip } from "@/components/stage-strip";
import { StateGlyph, toneClass } from "@/components/state-badge";
import { daemonLine, progressPercent, summaryCounts } from "@/lib/format";
import { connectionAtom, type ConnectionPhase } from "@/state/atoms";

const CONNECTION_LINE: Record<ConnectionPhase, ReturnType<typeof daemonLine>> = {
  connecting: { text: "connecting", tone: "dimmed" },
  live: { text: "awaiting data", tone: "dimmed" },
  reconnecting: { text: "reconnecting", tone: "warning" },
  offline: { text: "connection offline", tone: "blocked" },
  error: { text: "connection error", tone: "blocked" },
};

/// "● daemon running · tick 4s ago", toned by `daemonLine`. Before a valid
/// frame arrives it shows transport state. Once the feed drops, daemon state
/// is unknown regardless of the frozen snapshot, so connection staleness
/// overrides it.
export function DaemonLine({ snapshot }: { snapshot: Snapshot | null }) {
  const connection = useAtomValue(connectionAtom);
  const now = useNow();
  const stale =
    connection.phase === "reconnecting" ||
    connection.phase === "offline" ||
    connection.phase === "error";
  const staleSecs = stale ? Math.max(0, Math.round((now - connection.since) / 1000)) : null;
  const line = snapshot
    ? daemonLine(snapshot.daemon, snapshot.tick_age_secs, staleSecs)
    : CONNECTION_LINE[connection.phase];
  const diagnostics = [connection.message, snapshot?.notice].filter(Boolean).join("\n");
  return (
    <span
      className={cn("inline-flex items-center gap-1.5 text-xs", toneClass(line.tone))}
      title={diagnostics || undefined}
    >
      <span aria-hidden="true" className="size-2 rounded-full bg-(--tone)" />
      <span>{line.text}</span>
      {line.detail && <span className="text-muted-foreground">· {line.detail}</span>}
    </span>
  );
}

/// "1 of 7 stages complete", the warp strip, and the percent.
export function ProgressLine({ snapshot }: { snapshot: Snapshot }) {
  const { completed, total } = snapshot.status.progress;
  const percent = progressPercent(completed, total);
  return (
    <div className="flex items-center gap-3 text-sm">
      <span className="tabular-nums">
        {completed} of {total} stages complete
      </span>
      <StageStrip />
      <span className="text-xs tabular-nums text-muted-foreground">{percent}%</span>
    </div>
  );
}

const COUNTS: { key: "executing" | "queued" | "waiting" | "done"; status: StageStatus }[] = [
  { key: "executing", status: "executing" },
  { key: "queued", status: "queued" },
  { key: "waiting", status: "waiting-for-deps" },
  { key: "done", status: "completed" },
];

/// "● 1 executing · ▶ 1 queued · ○ 2 waiting · 3 need attention · ✓ 1 done".
export function SummaryLine({ snapshot, attention }: { snapshot: Snapshot; attention: number }) {
  const counts = summaryCounts(snapshot.status.stages, attention);
  const [executing, queued, waiting, done] = COUNTS;
  const item = ({ key, status }: (typeof COUNTS)[number]) => (
    <span key={key} className="inline-flex items-center gap-1.5 tabular-nums">
      <StateGlyph status={status} />
      <span>{counts[key]}</span>
      <span className="text-muted-foreground">{key}</span>
    </span>
  );
  return (
    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
      {item(executing)}
      <Dot />
      {item(queued)}
      <Dot />
      {item(waiting)}
      <Dot />
      <span
        className={cn(
          "tabular-nums",
          counts.attention === 0
            ? "text-muted-foreground"
            : cn(toneClass("blocked"), "font-medium"),
        )}
      >
        {counts.attention} need attention
      </span>
      <Dot />
      {item(done)}
    </div>
  );
}

/// "merged 0 · unmerged 1 · conflicts 1".
export function MergeLine({ snapshot }: { snapshot: Snapshot }) {
  const { merged, pending, conflicts } = snapshot.status.merge;
  return (
    <p className="text-xs tabular-nums text-muted-foreground">
      merged {merged.length} · unmerged {pending.length} ·{" "}
      <span className={cn(conflicts.length > 0 && toneClass("warning"))}>
        conflicts {conflicts.length}
      </span>
    </p>
  );
}

function Dot() {
  return (
    <span aria-hidden="true" className="text-muted-foreground/60">
      ·
    </span>
  );
}
