import { cn } from "cn";
import { Link, useNavigate } from "react-router";

import type { StageSummary } from "@/api/schema";
import { ActivityRoundel } from "@/components/activity-roundel";
import { ContextMeter } from "@/components/context-meter";
import { stageHref } from "@/components/stage-href";
import { useOpenTerminal } from "@/components/stage-modal";
import { StateBadge, toneClass } from "@/components/state-badge";
import { TerminalGlyph } from "@/components/terminal/terminal-glyph";
import { Badge } from "@/components/ui/badge";
import { TableCell, TableRow } from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { activityText, mergeText, modelsOf, stateMeta, timeText } from "@/lib/format";

/// One ledger row. Re-mounted by the table when the status changes (the key
/// carries the status) so the wash animation marks exactly that row.
export function LedgerRow({ stage, level }: { stage: StageSummary; level: number }) {
  const navigate = useNavigate();
  const href = stageHref(stage.id);
  const activity = activityText(stage);
  const merge = mergeText(stage);
  const time = timeText(stage);
  const models = modelsOf(stage);
  const marked = stage.held || stage.incoherence !== null;

  return (
    <TableRow
      className={cn(
        // The tone class supplies `--tone` for the wash; the text stays foreground.
        "row-wash cursor-pointer align-top text-foreground",
        toneClass(stateMeta(stage.status).tone),
        marked && "border-l-2 border-l-(--tone-warning)",
      )}
      onClick={() => navigate(href)}
    >
      <TableCell data-label="state" data-span="" className="text-foreground">
        <StateBadge status={stage.status} />
      </TableCell>
      <TableCell data-label="stage" data-span="" className="text-foreground">
        <StageName stage={stage} level={level} href={href} />
      </TableCell>
      <TableCell data-label="depends on">
        <Dependencies ids={stage.dependencies} />
      </TableCell>
      <TableCell data-label="models" className="font-mono text-xs text-foreground">
        <Models model={models.model} execution={models.execution} />
      </TableCell>
      <TableCell
        data-label="activity"
        className={cn("text-xs", activity && toneClass(activity.tone))}
      >
        {activity && (
          <span className="inline-flex items-center gap-1.5">
            <ActivityRoundel stage={stage} size={14} />
            {activity.text}
          </span>
        )}
      </TableCell>
      <TableCell data-label="context">
        <ContextMeter stage={stage} />
      </TableCell>
      <TableCell data-label="time" className="font-mono text-xs text-muted-foreground tabular-nums">
        {time}
      </TableCell>
      <TableCell data-label="merge" className={cn("text-xs", merge && toneClass(merge.tone))}>
        {merge?.text}
      </TableCell>
    </TableRow>
  );
}

/// The orchestrator model dimmed, then `›` and the execution models.
function Models({ model, execution }: { model: string; execution: string[] }) {
  return (
    <>
      <span className="text-muted-foreground">{model}</span>
      {execution.length > 0 && (
        <span>
          <span aria-hidden="true"> › </span>
          <span className="sr-only">, execution </span>
          {execution.join(", ")}
        </span>
      )}
    </>
  );
}

function StageName({ stage, level, href }: { stage: StageSummary; level: number; href: string }) {
  const openTerminal = useOpenTerminal();
  return (
    <div className="flex flex-col gap-0.5" style={{ paddingLeft: `${level * 1.25}rem` }}>
      <span className="flex items-center gap-2">
        {level > 0 && (
          <span aria-hidden="true" className="text-muted-foreground/60">
            └
          </span>
        )}
        <Link
          to={href}
          onClick={(event) => event.stopPropagation()}
          className="font-medium underline-offset-4 hover:underline"
        >
          {stage.name}
        </Link>
        <TerminalGlyph stage={stage} onOpen={openTerminal} />
        {stage.held && (
          <Badge variant="outline" className={cn("h-4 px-1.5", toneClass("warning"))}>
            held
          </Badge>
        )}
        {stage.incoherence !== null && (
          <Tooltip>
            <TooltipTrigger asChild>
              <Badge
                variant="outline"
                className={cn("h-4 cursor-help px-1.5", toneClass("blocked"))}
              >
                incoherent
              </Badge>
            </TooltipTrigger>
            <TooltipContent className="font-mono text-[11px]">{stage.incoherence}</TooltipContent>
          </Tooltip>
        )}
      </span>
      <span className="font-mono text-[11px] text-muted-foreground">{stage.id}</span>
    </div>
  );
}

function Dependencies({ ids }: { ids: string[] }) {
  if (ids.length === 0) return null;
  return (
    <span className="flex flex-wrap gap-1">
      {ids.map((id) => (
        <Badge
          key={id}
          variant="secondary"
          asChild
          className="h-5 font-mono text-[11px] font-normal"
        >
          <Link to={stageHref(id)} onClick={(event) => event.stopPropagation()}>
            {id}
          </Link>
        </Badge>
      ))}
    </span>
  );
}
