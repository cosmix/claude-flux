import type { DaemonState, FailureType, StageStatus, StageSummary } from "@/api/schema";

export type Tone =
  | "executing"
  | "completed"
  | "blocked"
  | "pending"
  | "queued"
  | "warning"
  | "merged"
  | "dimmed"
  | "neutral";

export interface StateMeta {
  tone: Tone;
  bold: boolean;
}

const STATE_META: Record<StageStatus, StateMeta> = {
  "waiting-for-deps": { tone: "pending", bold: false },
  queued: { tone: "queued", bold: true },
  executing: { tone: "executing", bold: true },
  "waiting-for-input": { tone: "warning", bold: true },
  blocked: { tone: "blocked", bold: true },
  completed: { tone: "completed", bold: true },
  "needs-handoff": { tone: "warning", bold: true },
  skipped: { tone: "dimmed", bold: false },
  "merge-conflict": { tone: "warning", bold: true },
  "completed-with-failures": { tone: "blocked", bold: true },
  "merge-blocked": { tone: "blocked", bold: true },
  "needs-human-review": { tone: "warning", bold: false },
  "needs-adjudication": { tone: "warning", bold: true },
};

export function formatElapsed(seconds: number): string {
  if (seconds < 60) {
    return `${seconds}s`;
  }
  if (seconds < 3600) {
    return `${Math.trunc(seconds / 60)}m${seconds % 60}s`;
  }
  return `${Math.trunc(seconds / 3600)}h${Math.trunc((seconds % 3600) / 60)}m`;
}

export function stateMeta(status: StageStatus): StateMeta {
  return STATE_META[status];
}

/// Which caution tape a state wears: red for a failure the daemon could not
/// get past, amber for a state waiting on a person or a merge, none otherwise.
export function hazardTone(status: StageStatus): "error" | "warning" | null {
  switch (status) {
    case "blocked":
    case "completed-with-failures":
    case "merge-blocked":
      return "error";
    case "merge-conflict":
    case "needs-human-review":
    case "needs-adjudication":
    case "waiting-for-input":
    case "needs-handoff":
      return "warning";
    default:
      return null;
  }
}

export function failureLabel(type: FailureType): string {
  const labels: Record<FailureType, string> = {
    "session-crash": "crash",
    "test-failure": "test",
    "build-failure": "build",
    "code-error": "code",
    timeout: "timeout",
    "context-exhausted": "context",
    "user-blocked": "user",
    "merge-conflict": "merge",
    "infrastructure-error": "infra",
    "sandbox-setup-failure": "sandbox",
    "startup-refusal": "startup",
    unknown: "error",
  };
  return labels[type];
}

function retryText(stage: StageSummary, label: string): { text: string; tone: Tone } {
  return {
    text: `${label} ${stage.retry_count}/${stage.max_retries ?? 3}`,
    tone: "blocked",
  };
}

function executingActivity(stage: StageSummary): { text: string; tone: Tone } {
  switch (stage.activity_status) {
    case "Working":
      return {
        text: stage.last_tool ? `working · ${stage.last_tool}` : "working",
        tone: "completed",
      };
    case "Idle":
      return stalenessText("idle", stage.staleness_secs, "dimmed");
    case "Stale":
      return stalenessText("stale", stage.staleness_secs, "warning");
    case "Orphaned":
      return { text: "orphaned", tone: "blocked" };
    case "Error":
      return { text: "crashed", tone: "blocked" };
  }
}

function stalenessText(
  prefix: string,
  seconds: number | null,
  tone: Tone,
): { text: string; tone: Tone } {
  return { text: seconds === null ? prefix : `${prefix} ${formatElapsed(seconds)}`, tone };
}

function baseActivity(stage: StageSummary): { text: string; tone: Tone } | null {
  if (stage.status === "executing" && stage.incoherence !== null) {
    return { text: "incoherent", tone: "blocked" };
  }
  switch (stage.status) {
    case "queued":
      return { text: "ready", tone: "queued" };
    case "executing":
      return executingActivity(stage);
    case "waiting-for-input":
      return { text: "awaiting input", tone: "warning" };
    case "needs-handoff":
      return { text: "handing off", tone: "warning" };
    case "blocked":
      return retryText(
        stage,
        stage.failure_info ? failureLabel(stage.failure_info.failure_type) : "error",
      );
    case "completed-with-failures":
      return retryText(stage, "failed");
    case "merge-conflict":
      return { text: "conflict", tone: "warning" };
    case "merge-blocked":
      return { text: "merge error", tone: "blocked" };
    case "needs-human-review":
      return { text: "awaiting you", tone: "warning" };
    case "needs-adjudication":
      return adjudicationText(stage);
    default:
      return null;
  }
}

function adjudicationText(stage: StageSummary): { text: string; tone: Tone } {
  const judge =
    stage.judge_heartbeat_secs === null
      ? "none"
      : stage.judge_heartbeat_secs <= 300
        ? "working"
        : "stale";
  return { text: `dispute ${stage.dispute_count} · judge ${judge}`, tone: "warning" };
}

export function activityText(stage: StageSummary): { text: string; tone: Tone } | null {
  const activity = baseActivity(stage);
  if (!activity || !stage.held) {
    return activity;
  }
  return { text: `held · ${activity.text}`, tone: "warning" };
}

export function contextUsage(stage: StageSummary): {
  tokens: number;
  ceiling: number;
  percent: number;
  /// Segments of the five-cell meter to light: a cell lights once usage
  /// passes its midpoint, so 39% shows two cells, not one.
  filled: number;
  health: "green" | "yellow" | "red";
} | null {
  const applicable = ["executing", "waiting-for-input", "needs-handoff"].includes(stage.status);
  const tokens = stage.context_tokens;
  const ceiling = stage.context_ceiling_tokens;
  if (!applicable || tokens === null || ceiling === null || ceiling === 0) {
    return null;
  }
  const ratio = tokens / ceiling;
  return {
    tokens,
    ceiling,
    percent: Math.round(ratio * 100),
    filled: Math.max(0, Math.min(5, Math.round(ratio * 5))),
    health: ratio >= 0.9 ? "red" : ratio >= 0.6 ? "yellow" : "green",
  };
}

export function modelsOf(stage: StageSummary): { model: string; execution: string[] } {
  return { model: stage.model, execution: [...stage.execution_models] };
}

export function timeText(stage: StageSummary): string | null {
  if (stage.status === "waiting-for-deps" || stage.status === "queued") {
    return null;
  }
  const seconds = stage.execution_secs ?? stage.elapsed_secs;
  return seconds === null ? "" : formatElapsed(seconds);
}

export function mergeText(stage: StageSummary): { text: string; tone: Tone } | null {
  if (
    stage.status === "completed" &&
    stage.cleanup_warning !== null &&
    stage.cleanup_warning !== undefined
  ) {
    return { text: "cleanup!", tone: "warning" };
  }
  if (stage.status === "completed" && stage.stage_type !== "knowledge") {
    if (!stage.merged) {
      return { text: "unmerged", tone: "warning" };
    }
    return stage.merge_assumed
      ? { text: "assumed", tone: "merged" }
      : { text: "merged", tone: "merged" };
  }
  if (stage.status === "merge-conflict") {
    return { text: "conflict", tone: "warning" };
  }
  if (stage.status === "merge-blocked") {
    return { text: "error", tone: "blocked" };
  }
  return null;
}

export function progressPercent(completed: number, total: number): number {
  if (total === 0) {
    return 0;
  }
  return Math.trunc((completed * 100 + Math.trunc(total / 2)) / total);
}

/// `staleSecs` is null while the page has a live feed; a number is how long
/// the page has been without one, in which case the daemon's real state is
/// unknown and must not be reported as running.
export function daemonLine(
  daemon: DaemonState,
  tickAgeSecs: number | null,
  staleSecs: number | null,
): { text: string; detail?: string; tone: Tone } {
  if (staleSecs !== null) {
    return {
      text: "daemon unknown",
      detail: `no data for ${formatElapsed(staleSecs)}`,
      tone: "dimmed",
    };
  }
  if (daemon === "process-only") {
    return { text: "daemon process alive, socket missing", tone: "warning" };
  }
  if (daemon === "not-running") {
    return { text: "daemon stopped", tone: "dimmed" };
  }
  // The stall check comes before `unreachable`: the tick file is read from
  // disk either way, so a stalled loop is a fact this caller can still see,
  // and it is the more urgent one.
  if (tickAgeSecs !== null && tickAgeSecs >= 60) {
    return { text: `loop stalled ${formatElapsed(tickAgeSecs)}`, tone: "warning" };
  }
  if (daemon === "unreachable") {
    return { text: "daemon running", detail: "socket unreachable", tone: "warning" };
  }
  return {
    text: "daemon running",
    detail: tickAgeSecs === null ? "tick unknown" : `tick ${tickAgeSecs}s ago`,
    tone: "completed",
  };
}

const CLOCK_FORMAT = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hourCycle: "h23",
});

const STAMP_FORMAT = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hourCycle: "h23",
});

/// 24-hour "14:03:22"; "—" for an unparseable input.
export function formatClock(value: string | number | Date): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "—" : CLOCK_FORMAT.format(date);
}

/// 24-hour date + time; "—" for an unparseable input.
export function formatStamp(value: string | number | Date): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "—" : STAMP_FORMAT.format(date);
}

export function summaryCounts(
  stages: readonly StageSummary[],
  attentionCount: number,
): { executing: number; queued: number; waiting: number; attention: number; done: number } {
  const count = (status: StageStatus) => stages.filter((stage) => stage.status === status).length;
  return {
    executing: count("executing"),
    queued: count("queued"),
    waiting: count("waiting-for-deps"),
    attention: attentionCount,
    done: count("completed"),
  };
}
