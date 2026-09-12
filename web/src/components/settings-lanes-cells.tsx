import { cn } from "cn";
import { UserRoundIcon } from "lucide-react";
import type { ReactElement } from "react";

import type { ConfigEntry, ConfigSnapshot } from "@/api/config";
import { EmptyState } from "@/aurora-ui/feedback/EmptyState";
import { LaneSlot } from "@/components/settings-control";
import {
  formatValue,
  statusFor,
  type OnWrite,
  type PairRow,
  type WriteStatus,
} from "@/components/settings-model";

/// The `<td>` building blocks of a lanes-table row, split out of
/// settings-lanes.tsx to keep that file under the line limit.

function laneCellClass(base: string, status: WriteStatus): string {
  return cn(base, status.phase === "error" && "hazard-error");
}

/// A pair row's key cell: its label, caption, and the model · effort the
/// row resolves to.
export function PairKeyCell({ row }: { row: PairRow }): ReactElement {
  return (
    <td className="settings-key">
      <span className="settings-key-name" title={`${row.model.help} · ${row.effort.help}`}>
        {row.label}
      </span>
      {row.caption && <span className="settings-help">{row.caption}</span>}
      <span className="settings-res">
        runs <b>{formatValue(row.model.kind, row.model.effective.value)}</b> ·{" "}
        <b>{formatValue(row.effort.kind, row.effort.effective.value)}</b>
      </span>
    </td>
  );
}

/// The model+effort `<td>` pair for one scope (`user` or `project`) of a
/// pair row — identical shape for both, so `PairRowTr` calls it twice.
export function PairScopeCells({
  scope,
  idBase,
  row,
  statuses,
  onWrite,
}: {
  scope: "user" | "project";
  idBase: string;
  row: PairRow;
  statuses: Record<string, WriteStatus>;
  onWrite: OnWrite;
}): ReactElement {
  const modelStatus = statusFor(statuses, scope, row.model.name);
  const effortStatus = statusFor(statuses, scope, row.effort.name);
  return (
    <>
      <td className={laneCellClass(`settings-ls settings-lane-${scope}`, modelStatus)}>
        <LaneSlot
          entry={row.model}
          lane={scope}
          status={modelStatus}
          controlId={`${idBase}-${scope}-model`}
          onWrite={(value) => onWrite(scope, row.model.name, value)}
        />
      </td>
      <td className={laneCellClass(`settings-lane-${scope}`, effortStatus)}>
        <LaneSlot
          entry={row.effort}
          lane={scope}
          status={effortStatus}
          controlId={`${idBase}-${scope}-effort`}
          onWrite={(value) => onWrite(scope, row.effort.name, value)}
        />
      </td>
    </>
  );
}

/// The full-width `<td>` for one scope of a single-key row.
export function SingleScopeCell({
  scope,
  idBase,
  entry,
  statuses,
  onWrite,
}: {
  scope: "user" | "project";
  idBase: string;
  entry: ConfigEntry;
  statuses: Record<string, WriteStatus>;
  onWrite: OnWrite;
}): ReactElement {
  const status = statusFor(statuses, scope, entry.name);
  return (
    <td className={laneCellClass(`settings-ls settings-lane-${scope}`, status)} colSpan={2}>
      <LaneSlot
        entry={entry}
        lane={scope}
        status={status}
        controlId={`${idBase}-${scope}`}
        onWrite={(value) => onWrite(scope, entry.name, value)}
      />
    </td>
  );
}

/// The project lane's one aurora-ui empty state, spanning every row of a
/// section that has no project tier (or, dialog-wide, no project workspace).
export function ProjectPane({
  data,
  rowCount,
}: {
  data: ConfigSnapshot;
  rowCount: number;
}): ReactElement {
  const short = rowCount < 3;
  const available = data.project.available;
  return (
    <td
      className="settings-lane-project settings-ls settings-project-cell"
      colSpan={2}
      rowSpan={rowCount}
    >
      <div className={cn("settings-pane", short && "settings-pane-short")}>
        <EmptyState
          variant="bare"
          size="sm"
          tone="muted"
          icon={UserRoundIcon}
          title={available ? "User-only keys" : "No project workspace"}
          description={
            available ? (
              "The project file has no tier for this section."
            ) : (
              <>
                <code>{data.project.path}</code> cannot be written, so these keys keep their user
                values.
              </>
            )
          }
          // EmptyState's own root div hardcodes `flex-col items-center
          // text-center`; `short` needs the mock's row form (icon left, text
          // stacked right) instead, which only a className override on that
          // root can reach — the wrapping `.settings-pane-short` rule in
          // settings.css has nothing else to apply `flex-direction: row` to.
          // The icon medallion is shrunk the same way, by targeting its own
          // (stable, kit-internal) size utility classes.
          className={cn(
            "gap-2 px-0 py-0",
            short &&
              "flex-row items-center text-left [&_.size-12]:size-8 [&_.size-6]:size-4 [&_p]:max-w-none [&_p]:mx-0",
          )}
        />
      </div>
    </td>
  );
}
