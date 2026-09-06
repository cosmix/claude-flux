import { z } from "zod";

export const STAGE_STATUSES = [
  "waiting-for-deps",
  "queued",
  "executing",
  "waiting-for-input",
  "blocked",
  "completed",
  "needs-handoff",
  "skipped",
  "merge-conflict",
  "completed-with-failures",
  "merge-blocked",
  "needs-human-review",
  "needs-adjudication",
] as const;
export const stageStatusSchema = z.enum(STAGE_STATUSES);
export const stageTypeSchema = z.enum([
  "standard",
  "knowledge",
  "integration-verify",
  "knowledge-distill",
]);
export const activityStatusSchema = z.enum(["Idle", "Working", "Error", "Stale", "Orphaned"]);
export const failureTypeSchema = z.enum([
  "session-crash",
  "context-exhausted",
  "test-failure",
  "build-failure",
  "code-error",
  "timeout",
  "user-blocked",
  "merge-conflict",
  "infrastructure-error",
  "sandbox-setup-failure",
  "startup-refusal",
  "unknown",
]);
export const sessionTypeSchema = z.enum([
  "stage",
  "merge",
  "baseconflict",
  "knowledge",
  "adjudication",
]);
export const sessionBackendSchema = z.enum(["native", "tmux"]);
export const failureInfoSchema = z.object({
  failure_type: failureTypeSchema,
  detected_at: z.string(),
  evidence: z.array(z.string()),
});
export const stageSummarySchema = z
  .object({
    id: z.string(),
    name: z.string(),
    status: stageStatusSchema,
    stage_type: stageTypeSchema,
    dependencies: z.array(z.string()),
    context_tokens: z.number().int().nullable(),
    elapsed_secs: z.number().int().nullable(),
    execution_secs: z.number().int().nullable(),
    base_branch: z.string().nullable(),
    base_merged_from: z.array(z.string()),
    failure_info: failureInfoSchema.nullable(),
    activity_status: activityStatusSchema,
    last_tool: z.string().nullable(),
    last_activity: z.string().nullable(),
    staleness_secs: z.number().int().nullable(),
    context_ceiling_tokens: z.number().int().nullable(),
    review_reason: z.string().nullable(),
    merged: z.boolean(),
    merge_assumed: z.boolean(),
    cleanup_warning: z.string().nullable().optional(),
    held: z.boolean(),
    retry_count: z.number().int(),
    max_retries: z.number().int().nullable(),
    pid: z.number().int().nullable(),
    session_alive: z.boolean(),
    model: z.string(),
    session_type: sessionTypeSchema.nullable(),
    incoherence: z.string().nullable(),
    execution_models: z.array(z.string()),
    dispute_count: z.number().int(),
    judge_heartbeat_secs: z.number().int().nullable(),
    session_backend: sessionBackendSchema.nullable(),
  })
  .strict();
export const mergeSummarySchema = z.object({
  merged: z.array(z.string()),
  pending: z.array(z.string()),
  conflicts: z.array(z.string()),
});
export const progressSummarySchema = z.object({
  total: z.number().int(),
  completed: z.number().int(),
  executing: z.number().int(),
  pending: z.number().int(),
  blocked: z.number().int(),
});
export const windowKindSchema = z.enum(["five-hour", "seven-day"]);
export const quotaWindowSchema = z.object({
  kind: windowKindSchema,
  used_percent: z.number(),
  resets_at: z.number().int().nullable(),
});
export const providerQuotaSchema = z.object({
  observed_at: z.number().int(),
  windows: z.array(quotaWindowSchema),
  plan: z.string().nullable(),
  error: z.string().nullable(),
});
export const quotaSnapshotSchema = z.object({
  claude: providerQuotaSchema.nullable(),
  codex: providerQuotaSchema.nullable(),
});
export const statusDataSchema = z.object({
  stages: z.array(stageSummarySchema),
  merge: mergeSummarySchema,
  progress: progressSummarySchema,
  plan_name: z.string().nullable(),
  quota: quotaSnapshotSchema,
});
export const attentionSchema = z.object({
  id: z.string(),
  name: z.string(),
  label: z.string(),
  hint: z.string(),
  failure_type: failureTypeSchema.nullable(),
  failure_label: z.string().nullable(),
  evidence: z.array(z.string()),
  review_reason: z.string().nullable(),
  cleanup_warning: z.string().nullable(),
  has_human_review_choices: z.boolean(),
  dispute_count: z.number().int().nullable(),
  judge_heartbeat_secs: z.number().int().nullable(),
});
export const alertSchema = z.object({
  severity: z.enum(["info", "warning", "critical"]),
  text: z.string(),
});
export const daemonStateSchema = z.enum(["running", "process-only", "not-running", "unreachable"]);
export const snapshotSchema = z
  .object({
    status: statusDataSchema,
    attention: z.array(attentionSchema),
    alerts: z.array(alertSchema),
    daemon: daemonStateSchema,
    tick_age_secs: z.number().int().nullable(),
    source: z.enum(["daemon", "files"]),
    notice: z.string().optional(),
    generated_at: z.string(),
    version: z.string(),
  })
  .strict();

export type StageStatus = z.infer<typeof stageStatusSchema>;
export type StageSummary = z.infer<typeof stageSummarySchema>;
export type StatusData = z.infer<typeof statusDataSchema>;
export type Attention = z.infer<typeof attentionSchema>;
export type Alert = z.infer<typeof alertSchema>;
export type DaemonState = z.infer<typeof daemonStateSchema>;
export type FailureType = z.infer<typeof failureTypeSchema>;
export type ActivityStatus = z.infer<typeof activityStatusSchema>;
export type Snapshot = z.infer<typeof snapshotSchema>;
export type WindowKind = z.infer<typeof windowKindSchema>;
export type QuotaWindow = z.infer<typeof quotaWindowSchema>;
export type ProviderQuota = z.infer<typeof providerQuotaSchema>;
export type QuotaSnapshot = z.infer<typeof quotaSnapshotSchema>;
