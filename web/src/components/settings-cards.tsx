import { cn } from "cn";
import type { ReactElement } from "react";
import { useId } from "react";

import type { ConfigEntry, ConfigScope, ConfigSnapshot } from "@/api/config";
import { BuiltinValue, LaneSlot, PairSummary } from "@/components/settings-control";
import {
  LANES,
  effectiveLane,
  laneScope,
  rowEntries,
  rowLabel,
  statusFor,
  type Lane,
  type SectionRows,
  type SettingsRow,
  type WriteStatus,
} from "@/components/settings-model";

/// Identical to `SettingsLanes`' props — the dialog swaps between the two
/// below 700px without changing what it passes down.
export interface SettingsTableProps {
  data: ConfigSnapshot;
  sections: SectionRows[];
  query: string;
  statuses: Record<string, WriteStatus>;
  onWrite: (scope: ConfigScope, name: string, value: string | null) => void;
}

const TIER_LABEL: Readonly<Record<Lane, string>> = {
  builtin: "built-in",
  user: "user",
  project: "project",
};

/// The phone rendering of the settings model: one card per row, three
/// tiers stacked inside each, in place of the desktop lanes table.
export function SettingsCards({
  data,
  sections,
  query,
  statuses,
  onWrite,
}: SettingsTableProps): ReactElement {
  const id = useId();

  if (sections.length === 0) {
    return <p className="settings-help settings-empty">nothing matches “{query}”</p>;
  }

  return (
    <div className="settings-cards">
      <div className="settings-cards-key">
        {LANES.map((lane) => (
          <span key={lane.lane} data-lane={lane.lane}>
            {laneKeyLabel(lane)}
          </span>
        ))}
      </div>
      {sections.map((section) => (
        <SectionCards
          key={section.section}
          id={id}
          section={section}
          data={data}
          statuses={statuses}
          onWrite={onWrite}
        />
      ))}
    </div>
  );
}

/// `LANES` gives the full lane blurb; the legend wants just the label and,
/// for the two file-backed lanes, the directory it lives in.
function laneKeyLabel(lane: (typeof LANES)[number]): string {
  if (lane.path === null) return lane.label;
  return `${lane.label} · ${lane.path.replace(/\/config\.toml$/, "")}`;
}

function rowKey(row: SettingsRow): string {
  return row.kind === "pair" ? row.model.name : row.entry.name;
}

function SectionCards({
  id,
  section,
  data,
  statuses,
  onWrite,
}: {
  id: string;
  section: SectionRows;
  data: ConfigSnapshot;
  statuses: Record<string, WriteStatus>;
  onWrite: SettingsTableProps["onWrite"];
}): ReactElement {
  return (
    <>
      <div className="settings-cards-sec">
        <span className="eyebrow">{section.section}</span>
        {section.caption && <span className="settings-help">{section.caption}</span>}
      </div>
      {section.rows.map((row) => (
        <SettingsCard
          key={rowKey(row)}
          id={id}
          section={section}
          row={row}
          data={data}
          statuses={statuses}
          onWrite={onWrite}
        />
      ))}
    </>
  );
}

function SettingsCardHeader({
  headingId,
  row,
  help,
}: {
  headingId: string;
  row: SettingsRow;
  help: string | null | undefined;
}): ReactElement {
  return (
    <header className="settings-card-h">
      <span>
        <span id={headingId} className="settings-key-name">
          {rowLabel(row)}
        </span>
        {help && <span className="settings-help">{help}</span>}
      </span>
      {row.kind === "pair" && (
        <span className="settings-res">
          <PairSummary row={row} />
        </span>
      )}
    </header>
  );
}

function ProjectTierNa({ available }: { available: boolean }): ReactElement {
  return (
    <div className="settings-tier settings-tier-na">
      <span className="settings-tl">project</span>
      <span>
        {available ? "user-only keys, no project tier" : "no project workspace to write to"}
      </span>
    </div>
  );
}

function SettingsCard({
  id,
  section,
  row,
  data,
  statuses,
  onWrite,
}: {
  id: string;
  section: SectionRows;
  row: SettingsRow;
  data: ConfigSnapshot;
  statuses: Record<string, WriteStatus>;
  onWrite: SettingsTableProps["onWrite"];
}): ReactElement {
  const headingId = `${id}-${rowKey(row)}-h`;
  const entries = rowEntries(row);
  const help = row.kind === "pair" ? row.caption : row.entry.help;
  const projectFits = section.projectAllowed && data.project.available;

  return (
    <article className="settings-card" aria-labelledby={headingId}>
      <SettingsCardHeader headingId={headingId} row={row} help={help} />
      {row.kind === "pair" && (
        <div className="settings-card-subh">
          <span className="sr-only">tier</span>
          <span>model</span>
          <span>effort</span>
        </div>
      )}
      <Tier lane="builtin" entries={entries} statuses={statuses} onWrite={onWrite} />
      <Tier lane="user" entries={entries} statuses={statuses} onWrite={onWrite} />
      {projectFits ? (
        <Tier lane="project" entries={entries} statuses={statuses} onWrite={onWrite} />
      ) : (
        <ProjectTierNa available={data.project.available} />
      )}
    </article>
  );
}

function Tier({
  lane,
  entries,
  statuses,
  onWrite,
}: {
  lane: Lane;
  entries: ConfigEntry[];
  statuses: Record<string, WriteStatus>;
  onWrite: SettingsTableProps["onWrite"];
}): ReactElement {
  const scope = laneScope(lane);
  const isEffective = entries.some((entry) => effectiveLane(entry) === lane);
  const hasError =
    scope !== null &&
    entries.some((entry) => statusFor(statuses, scope, entry.name).phase === "error");

  return (
    <div
      className={cn("settings-tier", hasError && "hazard-error")}
      data-lane={lane}
      data-effective={isEffective || undefined}
    >
      <span className="settings-tl">{TIER_LABEL[lane]}</span>
      <div className={cn("settings-tc", entries.length > 1 && "settings-tc-pair")}>
        {entries.map((entry) =>
          scope === null ? (
            <BuiltinValue key={entry.name} entry={entry} />
          ) : (
            <LaneCell
              key={entry.name}
              entry={entry}
              scope={scope}
              statuses={statuses}
              onWrite={onWrite}
            />
          ),
        )}
      </div>
    </div>
  );
}

function LaneCell({
  entry,
  scope,
  statuses,
  onWrite,
}: {
  entry: ConfigEntry;
  scope: ConfigScope;
  statuses: Record<string, WriteStatus>;
  onWrite: SettingsTableProps["onWrite"];
}): ReactElement {
  const controlId = useId() + scope + entry.name;
  const status = statusFor(statuses, scope, entry.name);
  return (
    <LaneSlot
      entry={entry}
      lane={scope}
      status={status}
      controlId={controlId}
      onWrite={(value) => onWrite(scope, entry.name, value)}
    />
  );
}
