import { ChevronDownIcon } from "lucide-react";
import type { ReactElement } from "react";
import { useState } from "react";

import type { ConfigEntry, ConfigKind } from "@/api/config";
import { BusyRoundel } from "@/aurora-ui/feedback/BusyRoundel";
import {
  effectiveLane,
  fallbackFor,
  fieldOf,
  formatValue,
  laneState,
  type LaneProvenance,
  type PairRow,
  type WriteStatus,
} from "@/components/settings-model";

export interface ControlProps {
  id: string;
  kind: ConfigKind;
  /// The value the control shows: the in-flight one while a write is
  /// pending, otherwise the server's last known value at this lane.
  value: string;
  pending: boolean;
  invalid: boolean;
  /// Accessible name; the same key appears once per lane, so callers pass
  /// `${fieldOf(name)} at ${lane} scope`.
  label: string;
  describedBy?: string;
  onCommit: (value: string) => void;
}

/// One control per wire `kind`; the key name never decides the widget.
export function ValueControl(props: ControlProps): ReactElement {
  switch (props.kind.type) {
    case "bool":
      return <BoolSwitch {...props} />;
    case "u32":
      return <NumberField {...props} />;
    case "enum":
      return <EnumSelect {...props} variants={props.kind.variants} />;
  }
}

function BoolSwitch({ id, value, pending, invalid, label, describedBy, onCommit }: ControlProps) {
  return (
    <input
      id={id}
      type="checkbox"
      role="switch"
      className="settings-switch"
      checked={value === "true"}
      aria-checked={value === "true"}
      aria-label={label}
      aria-busy={pending || undefined}
      aria-invalid={invalid || undefined}
      aria-describedby={describedBy}
      disabled={pending}
      onChange={(event) => onCommit(event.target.checked ? "true" : "false")}
    />
  );
}

/// Text rather than `type="number"`: the server's validator owns the rules,
/// and a text field lets its message about "abc" reach the operator instead
/// of the browser silently refusing the keystrokes.
function NumberField({ id, value, pending, invalid, label, describedBy, onCommit }: ControlProps) {
  // Local draft only while editing; null means "show the server's value",
  // which is also how a failed write reverts without an effect.
  const [draft, setDraft] = useState<string | null>(null);
  const shown = draft ?? value;

  const commit = () => {
    if (draft === null) return;
    const next = draft.trim();
    setDraft(null);
    if (next !== value) onCommit(next);
  };

  return (
    <input
      id={id}
      type="text"
      inputMode="numeric"
      autoComplete="off"
      spellCheck={false}
      className="settings-ctl settings-ctl-num"
      value={shown}
      data-draft={draft !== null || undefined}
      aria-label={label}
      aria-busy={pending || undefined}
      aria-invalid={invalid || undefined}
      aria-describedby={describedBy}
      disabled={pending}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          commit();
        } else if (event.key === "Escape") {
          // Drops the draft; the dialog keeps itself open while one exists.
          setDraft(null);
        }
      }}
    />
  );
}

function EnumSelect({
  id,
  value,
  variants,
  pending,
  invalid,
  label,
  describedBy,
  onCommit,
}: ControlProps & { variants: string[] }) {
  // A value the registry no longer lists still needs an option, or the
  // select would silently show the first variant instead of the truth.
  const options = variants.includes(value) ? variants : [value, ...variants];
  return (
    <span className="settings-select-wrap">
      <select
        id={id}
        className="settings-ctl settings-select"
        value={value}
        aria-label={label}
        aria-busy={pending || undefined}
        aria-invalid={invalid || undefined}
        aria-describedby={describedBy}
        disabled={pending}
        onChange={(event) => onCommit(event.target.value)}
      >
        {options.map((variant) => (
          <option key={variant} value={variant}>
            {variant}
          </option>
        ))}
      </select>
      <ChevronDownIcon aria-hidden="true" className="settings-select-chevron" />
    </span>
  );
}

/// A pair row's "runs `<model>` · `<effort>`" summary, shared by the lanes
/// table's key cell and the cards layout's card header.
export function PairSummary({ row }: { row: PairRow }): ReactElement {
  return (
    <>
      runs <b>{formatValue(row.model.kind, row.model.effective.value)}</b> ·{" "}
      <b>{formatValue(row.effort.kind, row.effort.effective.value)}</b>
    </>
  );
}

export interface LaneSlotProps {
  entry: ConfigEntry;
  lane: "user" | "project";
  status: WriteStatus;
  controlId: string;
  onWrite: (value: string | null) => void;
}

/// The ✕ that clears a set value back to its fallback; a title spells out
/// what that fallback is before the operator commits to clicking it.
function ClearButton({
  field,
  lane,
  title,
  onClear,
}: {
  field: string;
  lane: "user" | "project";
  title: string;
  onClear: () => void;
}): ReactElement {
  return (
    <button
      type="button"
      className="settings-clear"
      aria-label={`clear ${field} at ${lane} scope`}
      title={title}
      onClick={onClear}
    >
      ✕
    </button>
  );
}

/// What a lane slot shows for one entry at one lane given its write status:
/// the value, the provenance to style by, and the clear button's title (null
/// when there is nothing to clear).
function slotView(entry: ConfigEntry, lane: "user" | "project", status: WriteStatus) {
  const state = laneState(entry, lane);
  const pending = status.phase === "pending";
  // `data-provenance` also needs to read "error"/"pending" for the control's
  // own styling, but the clear button must stay keyed to the entry's actual
  // provenance alone — a rejected write must not hide it when the key is
  // still set.
  const provenance: LaneProvenance | "error" | "pending" =
    status.phase === "error" ? "error" : pending ? "pending" : state.provenance;
  const unavailable = state.provenance === "unavailable";
  const fallback = unavailable ? null : fallbackFor(entry, lane);
  const shown = unavailable
    ? null
    : pending
      ? (status.value ?? fallback?.value ?? null)
      : state.value;
  const clearTitle =
    fallback && state.provenance === "set" && !pending
      ? `falls back to ${fallback.tier} ${formatValue(entry.kind, fallback.value)}`
      : null;
  return { effective: state.effective, pending, provenance, unavailable, shown, clearTitle };
}

export function LaneSlot({ entry, lane, status, controlId, onWrite }: LaneSlotProps): ReactElement {
  const view = slotView(entry, lane, status);
  const field = fieldOf(entry.name);
  const errorId = `${controlId}-error`;

  return (
    <>
      <span
        className="settings-slot"
        data-lane={lane}
        data-provenance={view.provenance}
        data-effective={view.effective || undefined}
      >
        {view.unavailable ? (
          <span className="settings-na">user only</span>
        ) : (
          <ValueControl
            id={controlId}
            kind={entry.kind}
            value={view.shown ?? ""}
            pending={view.pending}
            invalid={status.phase === "error"}
            label={`${field} at ${lane} scope`}
            describedBy={status.phase === "error" ? errorId : undefined}
            onCommit={onWrite}
          />
        )}
        {view.pending && <BusyRoundel busy size={12} busyLabel="saving" idleLabel="" />}
        {view.clearTitle !== null && (
          <ClearButton
            field={field}
            lane={lane}
            title={view.clearTitle}
            onClear={() => onWrite(null)}
          />
        )}
        {view.effective && <span className="sr-only">in effect</span>}
      </span>
      {status.phase === "error" && (
        <span className="hazard-text settings-error" role="alert" id={errorId}>
          {status.message}
        </span>
      )}
    </>
  );
}

export function BuiltinValue({ entry }: { entry: ConfigEntry }): ReactElement {
  const effective = effectiveLane(entry) === "builtin";
  return (
    <span
      className="settings-slot"
      data-lane="builtin"
      data-provenance="readonly"
      data-effective={effective || undefined}
    >
      <span className="settings-ctl settings-ctl-readonly">
        {formatValue(entry.kind, entry.default)}
      </span>
      {effective && <span className="sr-only">in effect</span>}
    </span>
  );
}
