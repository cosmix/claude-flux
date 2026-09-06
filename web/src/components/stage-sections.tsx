import { cn } from "cn";

import type { StageSummary } from "@/api/schema";
import { HazardPanel } from "@/aurora-ui/feedback/HazardPanel";
import { ContextMeter } from "@/components/context-meter";
import {
  present,
  row,
  Rows,
  Section,
  secs,
  tokens,
  yesNo,
  type SectionSpec,
} from "@/components/stage-detail";
import { contextUsage, failureLabel, formatStamp } from "@/lib/format";

/// What each field means, in the words a person reading the ledger needs.
const HINT = {
  type: "What the stage is for: standard work, a knowledge bootstrap, the integration-verify gate, or a knowledge distillation. It decides what the stage's signal asks of the agent.",
  level:
    "Dependency depth: the longest chain of dependencies above this stage. Level 0 stages start first.",
  model: "The model the stage's own session runs, from the plan or the stage type's default.",
  executionModels:
    "Models the stage's subagents have run on, first seen first. Empty until a subagent spawns.",
  elapsed:
    "Wall-clock time from the stage's first start to its completion, or to now while it is still open. Includes waits and retry backoff, which execution excludes. Shown once the stage has started.",
  execution:
    "Time spent in a session, excluding waits and retry backoff. For an executing stage it counts the attempt in flight.",
  tokens: "Resident tokens in the active session's context window, as the hook last reported them.",
  ceiling: "The resident-token count at which the session hands off to a fresh one.",
  usage: "Tokens against the ceiling: amber from 60%, red from 90%.",
  pid: "Process id of the stage's session.",
  alive: "Whether that process is still running.",
  backend: "Where the session runs: a native terminal window or a tmux pane.",
  sessionType:
    "What the session was spawned to do: stage work, a merge, a base-conflict fix, knowledge, or adjudication.",
  activity:
    "Working: a tool ran recently. Idle: no live session, or the stage is finished. Stale: no heartbeat for 5 min. Orphaned: executing with no session record. Error: the process died.",
  lastTool: "The tool the session used most recently, from its heartbeat.",
  lastActivity:
    "The heartbeat's description of what the session last did. Usually names the last tool, but session start and a subagent finishing are recorded here with no tool.",
  staleness: "Time since the last heartbeat.",
  retries: "Automatic retries so far, of the stage's limit.",
  disputes: "Acceptance criteria the stage's agent disputed. A judge session settles them.",
  judge: "Time since the adjudication judge for this stage last reported in.",
  merged:
    "Whether the stage's branch has been merged into the merge point. Assumed: an operator completed the stage with --force-unsafe --assume-merged, so loom did not perform the merge itself.",
  baseBranch:
    "The branch the stage's worktree was cut from: the target branch, or a base built by merging several dependencies.",
  baseMergedFrom:
    "The dependencies merged together to build that base, when there was more than one.",
  cleanup: "The worktree or branch could not be removed after the merge and is still on disk.",
  reviewReason: "Why the stage asked a person for a decision.",
  incoherence:
    "Why an executing stage does not describe a working agent, for example because its session is an adjudication session.",
  held: "Held by you: the daemon leaves the stage alone until you release it.",
  failureType: "How the daemon classified the failure.",
  detectedAt: "When the daemon detected it.",
  evidence: "Lines the daemon kept as evidence. Untrusted text, shown verbatim.",
} as const;

const FAILURE_STATES = new Set<StageSummary["status"]>(["blocked", "completed-with-failures"]);
const MERGE_STATES = new Set<StageSummary["status"]>([
  "completed",
  "merge-conflict",
  "merge-blocked",
]);

function identityRows(stage: StageSummary, level: number | null) {
  return present([
    row("type", HINT.type, stage.stage_type, true),
    row("level", HINT.level, level === null ? null : String(level), true),
    row("model", HINT.model, stage.model, true),
    row("execution models", HINT.executionModels, stage.execution_models.join(", "), true),
  ]);
}

function timingRows(stage: StageSummary) {
  return present([
    row("elapsed", HINT.elapsed, secs(stage.elapsed_secs), true),
    row("execution", HINT.execution, secs(stage.execution_secs), true),
  ]);
}

function contextRows(stage: StageSummary) {
  return present([
    row("tokens", HINT.tokens, tokens(stage.context_tokens), true),
    row("ceiling", HINT.ceiling, tokens(stage.context_ceiling_tokens), true),
    row("usage", HINT.usage, contextUsage(stage) && <ContextMeter stage={stage} detail />),
  ]);
}

function sessionRows(stage: StageSummary) {
  const hasSession =
    stage.pid !== null ||
    stage.session_type !== null ||
    stage.last_tool !== null ||
    stage.last_activity !== null;
  if (!hasSession) return [];
  return present([
    row("pid", HINT.pid, stage.pid === null ? null : String(stage.pid), true),
    row("alive", HINT.alive, yesNo(stage.session_alive), true),
    row("backend", HINT.backend, stage.session_backend, true),
    row("session type", HINT.sessionType, stage.session_type, true),
    row("activity", HINT.activity, stage.activity_status, true),
    row("last tool", HINT.lastTool, stage.last_tool, true),
    row("last activity", HINT.lastActivity, stage.last_activity, true),
    row("staleness", HINT.staleness, secs(stage.staleness_secs), true),
  ]);
}

function retryRows(stage: StageSummary) {
  if (stage.retry_count === 0 && !FAILURE_STATES.has(stage.status)) return [];
  return present([
    row("retries", HINT.retries, `${stage.retry_count} of ${stage.max_retries ?? 3}`, true),
  ]);
}

function adjudicationRows(stage: StageSummary) {
  const applies =
    stage.status === "needs-adjudication" ||
    stage.dispute_count > 0 ||
    stage.judge_heartbeat_secs !== null;
  if (!applies) return [];
  return present([
    row("disputes", HINT.disputes, String(stage.dispute_count), true),
    row(
      "judge heartbeat",
      HINT.judge,
      stage.judge_heartbeat_secs === null ? null : `${secs(stage.judge_heartbeat_secs)} ago`,
      true,
    ),
  ]);
}

function mergeRows(stage: StageSummary) {
  const applies =
    MERGE_STATES.has(stage.status) ||
    stage.merged ||
    stage.base_branch !== null ||
    stage.base_merged_from.length > 0 ||
    Boolean(stage.cleanup_warning);
  if (!applies) return [];
  return present([
    row(
      "merged",
      HINT.merged,
      MERGE_STATES.has(stage.status) || stage.merged
        ? stage.merge_assumed
          ? "assumed"
          : yesNo(stage.merged)
        : null,
      true,
    ),
    row("base branch", HINT.baseBranch, stage.base_branch, true),
    row("base merged from", HINT.baseMergedFrom, stage.base_merged_from.join(", "), true),
    row("cleanup warning", HINT.cleanup, stage.cleanup_warning ?? null),
  ]);
}

function noteRows(stage: StageSummary) {
  return present([
    row("review reason", HINT.reviewReason, stage.review_reason),
    row("incoherence", HINT.incoherence, stage.incoherence, true),
    row("held", HINT.held, stage.held ? "yes" : null, true),
  ]);
}

/// Every section that has something to say about this stage.
export function stageSections(stage: StageSummary, level: number | null): SectionSpec[] {
  return [
    { title: "identity", rows: identityRows(stage, level) },
    { title: "timing", rows: timingRows(stage) },
    { title: "context", rows: contextRows(stage) },
    { title: "session", rows: sessionRows(stage) },
    { title: "retries", rows: retryRows(stage) },
    { title: "adjudication", rows: adjudicationRows(stage) },
    { title: "merge", rows: mergeRows(stage) },
    { title: "notes", rows: noteRows(stage) },
  ].filter((section) => section.rows.length > 0);
}

/// The sections as cards; `wide` adds the third column the page has room for.
export function StageSectionGrid({
  stage,
  level,
  wide = false,
}: {
  stage: StageSummary;
  level: number | null;
  wide?: boolean;
}) {
  return (
    <div className={cn("grid gap-4 md:grid-cols-2", wide && "xl:grid-cols-3")}>
      {stageSections(stage, level).map((section) => (
        <Section key={section.title} {...section} />
      ))}
      {stage.failure_info && (
        <div className={cn("md:col-span-2", wide && "xl:col-span-3")}>
          <FailureSection failure={stage.failure_info} />
        </div>
      )}
    </div>
  );
}

/// The failure under the kit's red caution tape. Evidence is untrusted
/// text: rendered as text nodes only.
function FailureSection({ failure }: { failure: NonNullable<StageSummary["failure_info"]> }) {
  return (
    <HazardPanel
      tone="error"
      title={`failure · ${failureLabel(failure.failure_type)}`}
      className="bg-card"
    >
      <Rows
        rows={present([
          row("type", HINT.failureType, failure.failure_type, true),
          row("detected at", HINT.detectedAt, formatStamp(failure.detected_at), true),
          row(
            "evidence",
            HINT.evidence,
            failure.evidence.length > 0 && (
              <pre className="max-h-64 overflow-auto rounded-md border border-hairline bg-background p-2.5 text-[11px] leading-relaxed whitespace-pre-wrap">
                {failure.evidence.join("\n")}
              </pre>
            ),
          ),
        ])}
      />
    </HazardPanel>
  );
}
