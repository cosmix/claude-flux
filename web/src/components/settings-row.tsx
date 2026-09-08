import { cn } from "cn";
import { Undo2Icon } from "lucide-react";
import { useId } from "react";

import type { ConfigEntry, ConfigScope } from "@/api/config";
import { ValueControl } from "@/components/settings-control";
import {
  displayValue,
  fallbackFor,
  fieldOf,
  provenanceAt,
  railStops,
  valueAt,
  type RailStop,
} from "@/components/settings-model";
import { toneClass } from "@/components/state-badge";
import { Button } from "@/components/ui/button";

/// One key's write in flight or just settled; the control shows the
/// pending value until the server answers, then the entry it returned.
export type WriteStatus =
  | { phase: "idle" }
  | { phase: "pending"; value: string | null }
  | { phase: "saved" }
  | { phase: "error"; message: string };

export interface SettingRowProps {
  entry: ConfigEntry;
  scope: ConfigScope;
  status: WriteStatus;
  onWrite: (value: string | null) => void;
  onSwitchScope: (scope: ConfigScope) => void;
}

export function SettingRow({ entry, scope, status, onWrite, onSwitchScope }: SettingRowProps) {
  const id = useId();
  const controlId = `${id}-control`;
  const helpId = `${id}-help`;
  const statusId = `${id}-status`;
  const provenance = provenanceAt(entry, scope);
  const pending = status.phase === "pending";
  const fallback = fallbackFor(entry, scope);
  const served = valueAt(entry, scope);
  // While clearing, the control previews what the key falls back to.
  const shown = pending ? (status.value ?? fallback.value ?? served) : served;

  return (
    <li className="settings-row" data-provenance={provenance} data-key={entry.name}>
      <div className="settings-row-main">
        <div className="settings-row-text">
          <label htmlFor={controlId} className="settings-label">
            {fieldOf(entry.name)}
          </label>
          <p id={helpId} className="settings-help">
            {entry.help}
          </p>
        </div>
        <div className="settings-row-control">
          {provenance === "unavailable" || shown === null ? (
            <Unavailable entry={entry} onSwitchScope={onSwitchScope} />
          ) : (
            <ValueControl
              id={controlId}
              kind={entry.kind}
              value={shown}
              pending={pending}
              invalid={status.phase === "error"}
              describedBy={`${helpId} ${statusId}`}
              onCommit={onWrite}
            />
          )}
        </div>
      </div>
      <div className="settings-row-meta">
        <ProvenanceRail entry={entry} stops={railStops(entry)} />
        <span className="settings-row-actions">
          {scope === "user" && entry.project?.set && (
            <span className={cn("settings-tag", toneClass("warning"))}>
              overridden by project: {displayValue(entry.kind, entry.project.value)}
            </span>
          )}
          {provenance === "inherited" && (
            <span className="settings-tag">
              {scope === "project" ? "inherited from user" : "built-in default"}
            </span>
          )}
          {provenance === "set" && (
            <Button
              type="button"
              variant="ghost"
              size="xs"
              className="settings-clear"
              disabled={pending}
              onClick={() => onWrite(null)}
            >
              <Undo2Icon aria-hidden="true" />
              {scope === "project" ? "clear override" : "reset"}
              <span className="text-muted-foreground">
                · back to {fallback.tier} {displayValue(entry.kind, fallback.value)}
              </span>
            </Button>
          )}
        </span>
      </div>
      <p
        id={statusId}
        className={cn("settings-status", status.phase === "error" && toneClass("blocked"))}
        aria-live="polite"
        aria-atomic="true"
      >
        {statusText(status)}
      </p>
    </li>
  );
}

function statusText(status: WriteStatus): string {
  switch (status.phase) {
    case "idle":
      return "";
    case "pending":
      return "saving…";
    case "saved":
      return "saved";
    case "error":
      return status.message;
  }
}

/// Why there is no control at this scope: the key is machine-wide, or
/// there is no project workspace to write into.
function Unavailable({
  entry,
  onSwitchScope,
}: {
  entry: ConfigEntry;
  onSwitchScope: (scope: ConfigScope) => void;
}) {
  if (!entry.scopes.includes("project")) {
    return (
      <span className="settings-unavailable">
        <span>machine-wide preference</span>
        <Button type="button" variant="link" size="xs" onClick={() => onSwitchScope("user")}>
          set at user scope
        </Button>
      </span>
    );
  }
  return <span className="settings-unavailable">no project workspace to write to</span>;
}

/// built-in → user → project, the effective stop lit: the value loom uses
/// is the nearest set stop, read right to left.
function ProvenanceRail({ entry, stops }: { entry: ConfigEntry; stops: RailStop[] }) {
  return (
    <ol className="settings-rail" aria-label={`where ${entry.name} comes from`}>
      {stops.map((stop) => (
        <li
          key={stop.tier}
          className="settings-stop"
          data-set={stop.set}
          data-applicable={stop.applicable}
          aria-current={stop.effective || undefined}
        >
          <span className="settings-stop-dot" aria-hidden="true" />
          <span className="settings-stop-tier">{stop.tier}</span>
          <span className="settings-stop-value">{stopValue(entry, stop)}</span>
          {stop.effective && <span className="sr-only">(in effect)</span>}
        </li>
      ))}
    </ol>
  );
}

function stopValue(entry: ConfigEntry, stop: RailStop): string {
  if (!stop.applicable) return "n/a";
  if (stop.value === null) return "not set";
  return displayValue(entry.kind, stop.value);
}
