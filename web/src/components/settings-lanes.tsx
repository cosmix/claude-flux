import { cn } from "cn";
import { UserRoundIcon } from "lucide-react";
import { useId, type ReactElement } from "react";

import type { ConfigEntry, ConfigSnapshot } from "@/api/config";
import { EmptyState } from "@/aurora-ui/feedback/EmptyState";
import { BuiltinValue, LaneSlot } from "@/components/settings-control";
import { laneCellClass, PairKeyCell, PairScopeCells } from "@/components/settings-lanes-cells";
import {
  fieldOf,
  LANES,
  statusFor,
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

/// The full-width `<td>` for one scope of a single-key row.
function SingleScopeCell({
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
function ProjectPane({ data, rowCount }: { data: ConfigSnapshot; rowCount: number }): ReactElement {
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
