import { cn } from "cn";
import type { ReactElement } from "react";

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

export function laneCellClass(base: string, status: WriteStatus): string {
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
