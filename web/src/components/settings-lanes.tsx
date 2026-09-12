import { cn } from "cn";
import { useId, type ReactElement } from "react";

import type { ConfigSnapshot } from "@/api/config";
import { BuiltinValue } from "@/components/settings-control";
import {
  PairKeyCell,
  PairScopeCells,
  ProjectPane,
  SingleScopeCell,
} from "@/components/settings-lanes-cells";
import {
  fieldOf,
  LANES,
  type OnWrite,
  type PairRow,
  type SectionRows,
  type SettingsRow,
  type SingleRow,
  type WriteStatus,
} from "@/components/settings-model";
import { toneClass } from "@/components/state-badge";

export interface SettingsTableProps {
  data: ConfigSnapshot;
  sections: SectionRows[];
  query: string;
  statuses: Record<string, WriteStatus>;
  onWrite: OnWrite;
}

const COLUMN_WIDTHS = [186, 104, 78, 132, 98, 132, 98];

/// The three writable-file lanes as table columns, model/effort pairs as
/// rows: a key reads left to right in loom's own resolution order, and the
/// rightmost lane that sets a value is the one in effect.
export function SettingsLanes({
  data,
  sections,
  query,
  statuses,
  onWrite,
}: SettingsTableProps): ReactElement {
  return (
    <table className="settings-table" aria-label="Settings across built-in, user and project files">
      <colgroup>
        {COLUMN_WIDTHS.map((width, i) => (
          <col key={i} style={{ width }} />
        ))}
      </colgroup>
      <LaneHead data={data} />
      {sections.length === 0 ? (
        <NoMatchBody query={query} />
      ) : (
        sections.map((section) => (
          <SectionBlock
            key={section.section}
            section={section}
            data={data}
            statuses={statuses}
            onWrite={onWrite}
          />
        ))
      )}
    </table>
  );
}

/// One head per lane: its label, the file it writes (or loom's defaults),
/// and a tag when the project file cannot be written.
function LaneHead({ data }: { data: ConfigSnapshot }): ReactElement {
  return (
    <thead>
      <tr>
        <th scope="col" />
        {LANES.map((lane) => (
          <th
            key={lane.lane}
            scope="colgroup"
            colSpan={2}
            className={cn("settings-ls", `settings-lane-${lane.lane}`)}
          >
            <b>{lane.label}</b>
            <span className="mt-0.5 block font-mono text-[11px] font-normal text-muted-foreground">
              {lane.lane === "builtin"
                ? lane.blurb
                : lane.lane === "project"
                  ? data.project.path
                  : lane.path}
            </span>
            {lane.lane === "project" && !data.project.available && (
              <span className={cn("settings-tag", toneClass("warning"))}>not available</span>
            )}
          </th>
        ))}
      </tr>
    </thead>
  );
}

function NoMatchBody({ query }: { query: string }): ReactElement {
  return (
    <tbody>
      <tr>
        <td colSpan={7} className="settings-key settings-empty">
          nothing matches “{query}”
        </td>
      </tr>
    </tbody>
  );
}

interface RowContext {
  index: number;
  section: SectionRows;
  data: ConfigSnapshot;
  statuses: Record<string, WriteStatus>;
  onWrite: SettingsTableProps["onWrite"];
}

function SectionBlock({
  section,
  data,
  statuses,
  onWrite,
}: {
  section: SectionRows;
  data: ConfigSnapshot;
  statuses: Record<string, WriteStatus>;
  onWrite: SettingsTableProps["onWrite"];
}): ReactElement {
  return (
    <tbody>
      <SectionHead section={section} />
      {section.rows.map((row, index) => (
        <RowTr
          key={row.kind === "pair" ? row.label : row.entry.name}
          row={row}
          index={index}
          section={section}
          data={data}
          statuses={statuses}
          onWrite={onWrite}
        />
      ))}
    </tbody>
  );
}

/// A section's heading row: its name and caption, then model/effort
/// sub-heads under each lane when the section holds pairs.
function SectionHead({ section }: { section: SectionRows }): ReactElement {
  // `some`, not `every`: a section can mix a stray unpaired key in among its
  // pairs (see `sectionRows`), and that lone single row still wants the
  // model/effort sub-header above the pairs around it.
  const paired = section.rows.some((row) => row.kind === "pair");
  return (
    <tr className="settings-sech">
      <th className="settings-key">
        <span className="eyebrow">{section.section}</span>
        {section.caption && <span className="settings-help">{section.caption}</span>}
      </th>
      {paired ? (
        <>
          <th className="settings-ls settings-lane-builtin">model</th>
          <th className="settings-lane-builtin">effort</th>
          <th className="settings-ls settings-lane-user">model</th>
          <th className="settings-lane-user">effort</th>
          {section.projectAllowed ? (
            <>
              <th className="settings-ls settings-lane-project">model</th>
              <th className="settings-lane-project">effort</th>
            </>
          ) : (
            <th className="settings-ls settings-lane-project" colSpan={2} />
          )}
        </>
      ) : (
        <>
          <th className="settings-ls settings-lane-builtin" colSpan={2} />
          <th className="settings-ls settings-lane-user" colSpan={2} />
          <th className="settings-ls settings-lane-project" colSpan={2} />
        </>
      )}
    </tr>
  );
}

function RowTr({ row, ...rest }: RowContext & { row: SettingsRow }): ReactElement {
  return row.kind === "pair" ? (
    <PairRowTr row={row} {...rest} />
  ) : (
    <SingleRowTr row={row} {...rest} />
  );
}

function PairRowTr({
  row,
  index,
  section,
  data,
  statuses,
  onWrite,
}: RowContext & { row: PairRow }): ReactElement {
  const idBase = useId();
  const projectShown = section.projectAllowed && data.project.available;
  const cells = { idBase, row, statuses, onWrite };

  return (
    <tr>
      <PairKeyCell row={row} />
      <td className="settings-ls settings-lane-builtin">
        <BuiltinValue entry={row.model} />
      </td>
      <td className="settings-lane-builtin">
        <BuiltinValue entry={row.effort} />
      </td>
      <PairScopeCells scope="user" {...cells} />
      {projectShown ? (
        <PairScopeCells scope="project" {...cells} />
      ) : (
        index === 0 && <ProjectPane data={data} rowCount={section.rows.length} />
      )}
    </tr>
  );
}

function SingleRowTr({
  row,
  index,
  section,
  data,
  statuses,
  onWrite,
}: RowContext & { row: SingleRow }): ReactElement {
  const idBase = useId();
  const entry = row.entry;
  const projectShown = section.projectAllowed && data.project.available;
  const cells = { idBase, entry, statuses, onWrite };

  return (
    <tr>
      <td className="settings-key">
        <span className="settings-key-name">{fieldOf(entry.name)}</span>
        {entry.help && <span className="settings-help">{entry.help}</span>}
      </td>
      <td className="settings-ls settings-lane-builtin" colSpan={2}>
        <BuiltinValue entry={entry} />
      </td>
      <SingleScopeCell scope="user" {...cells} />
      {projectShown ? (
        <SingleScopeCell scope="project" {...cells} />
      ) : (
        index === 0 && <ProjectPane data={data} rowCount={section.rows.length} />
      )}
    </tr>
  );
}
