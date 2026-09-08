import { cn } from "cn";
import { useCallback, useEffect, useState } from "react";
import { useSearchParams } from "react-router";

import {
  configScopeSchema,
  createConfigClient,
  type ConfigClient,
  type ConfigEntry,
  type ConfigScope,
  type ConfigSnapshot,
} from "@/api/config";
import { groupBySection, SCOPES, scopePath } from "@/components/settings-model";
import { SettingRow, type WriteStatus } from "@/components/settings-row";
import { toneClass } from "@/components/state-badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";

/// The query parameter that opens the dialog, so it has a URL and the back
/// button closes it. Its value names the scope being edited; `?settings=1`
/// or any other value opens at user scope.
export const SETTINGS_PARAM = "settings";

function scopeFromParam(value: string | null): ConfigScope {
  const parsed = configScopeSchema.safeParse(value);
  return parsed.success ? parsed.data : "user";
}

export function useOpenSettings(): (scope?: ConfigScope) => void {
  const [, setParams] = useSearchParams();
  return useCallback(
    (scope: ConfigScope = "user") =>
      setParams((params) => {
        const next = new URLSearchParams(params);
        next.set(SETTINGS_PARAM, scope);
        return next;
      }),
    [setParams],
  );
}

const defaultClient = createConfigClient();

/// Loom's configuration at two scopes, opened from any route with
/// `?settings=<scope>`; every control writes on change, one key at a time.
export function SettingsDialog({ client = defaultClient }: { client?: ConfigClient }) {
  const [params, setParams] = useSearchParams();
  const param = params.get(SETTINGS_PARAM);
  const open = param !== null;
  const scope = scopeFromParam(param);
  const close = () =>
    setParams((current) => {
      const next = new URLSearchParams(current);
      next.delete(SETTINGS_PARAM);
      return next;
    });
  // Switching scope rewrites the entry in place: one back press still
  // closes the dialog rather than stepping through every scope visited.
  const setScope = (next: ConfigScope) =>
    setParams(
      (current) => {
        const copy = new URLSearchParams(current);
        copy.set(SETTINGS_PARAM, next);
        return copy;
      },
      { replace: true },
    );

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent
        className={cn("settings-dialog gap-0 overflow-hidden p-0 sm:max-w-2xl", `scope-${scope}`)}
        onEscapeKeyDown={(event) => {
          // Escape in a number field with an unsaved draft drops the draft
          // (the field handles that); a second Escape closes the dialog.
          if (event.target instanceof HTMLElement && event.target.matches("[data-draft]")) {
            event.preventDefault();
          }
        }}
      >
        {open && <Body client={client} scope={scope} onScope={setScope} />}
      </DialogContent>
    </Dialog>
  );
}

type LoadState =
  | { phase: "loading" }
  | { phase: "error"; message: string }
  | { phase: "ready"; data: ConfigSnapshot };

function Body({
  client,
  scope,
  onScope,
}: {
  client: ConfigClient;
  scope: ConfigScope;
  onScope: (scope: ConfigScope) => void;
}) {
  const [load, setLoad] = useState<LoadState>({ phase: "loading" });
  const [attempt, setAttempt] = useState(0);
  // Keyed by scope and name: a write issued at one scope must not show its
  // pending value or its error against the other scope's control.
  const [statuses, setStatuses] = useState<Record<string, WriteStatus>>({});

  useEffect(() => {
    let ignore = false;
    setLoad({ phase: "loading" });
    client
      .load()
      .then((data) => {
        if (!ignore) setLoad({ phase: "ready", data });
      })
      .catch((error: unknown) => {
        if (ignore) return;
        const message = error instanceof Error ? error.message : String(error);
        setLoad({ phase: "error", message });
      });
    return () => {
      ignore = true;
    };
  }, [client, attempt]);

  const write = async (name: string, value: string | null) => {
    if (load.phase !== "ready") return;
    const csrf = load.data.csrf_token;
    const key = statusKey(scope, name);
    setStatuses((current) => ({ ...current, [key]: { phase: "pending", value } }));
    const result = await client.write(csrf, { scope, name, value });
    if (result.ok) {
      setLoad((current) =>
        current.phase === "ready"
          ? { ...current, data: replaceEntry(current.data, result.entry) }
          : current,
      );
      setStatuses((current) => ({ ...current, [key]: { phase: "saved" } }));
      return;
    }
    setStatuses((current) => ({
      ...current,
      [key]: { phase: "error", message: result.message },
    }));
    // A CSRF rejection means the token this dialog holds is stale; reading
    // the config again fetches a fresh one along with the current values.
    if (result.status === 403) setAttempt((n) => n + 1);
  };

  const project = load.phase === "ready" ? load.data.project : null;
  return (
    <>
      <DialogHeader className="settings-head gap-3 p-5 pr-12">
        <div className="flex flex-col gap-1">
          <DialogTitle className="text-xl font-semibold tracking-tight">Settings</DialogTitle>
          <DialogDescription>
            Project overrides user, user overrides loom's built-in; the rail under each key shows
            which one is in effect.
          </DialogDescription>
        </div>
        <ScopeSwitch
          scope={scope}
          projectAvailable={project?.available ?? null}
          onScope={onScope}
        />
      </DialogHeader>
      <div className="settings-body max-h-[62dvh] overflow-y-auto">
        {load.phase === "loading" && (
          <p className="settings-notice" aria-busy="true">
            reading configuration…
          </p>
        )}
        {load.phase === "error" && (
          <div className={cn("settings-notice", toneClass("blocked"))} role="alert">
            <span>{load.message}</span>
            <Button
              type="button"
              variant="outline"
              size="xs"
              onClick={() => setAttempt((n) => n + 1)}
            >
              retry
            </Button>
          </div>
        )}
        {load.phase === "ready" && (
          <Entries
            data={load.data}
            scope={scope}
            statuses={statuses}
            onWrite={write}
            onSwitchScope={onScope}
          />
        )}
      </div>
      <footer className="flex items-center justify-between gap-3 border-t border-hairline bg-muted/40 px-5 py-2.5 text-xs text-muted-foreground">
        <span>
          <Kbd>Esc</Kbd> closes
        </span>
        <span>each change is written on its own as you make it</span>
      </footer>
    </>
  );
}

function statusKey(scope: ConfigScope, name: string): string {
  return `${scope}:${name}`;
}

function replaceEntry(data: ConfigSnapshot, entry: ConfigEntry): ConfigSnapshot {
  return {
    ...data,
    entries: data.entries.map((current) => (current.name === entry.name ? entry : current)),
  };
}

/// The file being written, as a two-cell radio group: name, path, and for
/// the project cell whether there is a workspace file to write at all.
function ScopeSwitch({
  scope,
  projectAvailable,
  onScope,
}: {
  scope: ConfigScope;
  projectAvailable: boolean | null;
  onScope: (scope: ConfigScope) => void;
}) {
  return (
    <fieldset className="settings-scopes">
      <legend className="sr-only">scope to edit</legend>
      {SCOPES.map(({ scope: candidate, path, blurb }) => {
        const missing = candidate === "project" && projectAvailable === false;
        return (
          <label
            key={candidate}
            className={cn("settings-scope", scope === candidate && "is-active")}
            data-scope={candidate}
          >
            <input
              type="radio"
              name="settings-scope"
              value={candidate}
              checked={scope === candidate}
              onChange={() => onScope(candidate)}
              className="sr-only"
            />
            <span className="settings-scope-name">
              {candidate}
              {missing && (
                <span className={cn("settings-tag", toneClass("warning"))}>not available</span>
              )}
            </span>
            <span className="settings-scope-path">{path}</span>
            <span className="settings-scope-blurb">{blurb}</span>
          </label>
        );
      })}
    </fieldset>
  );
}

function Entries({
  data,
  scope,
  statuses,
  onWrite,
  onSwitchScope,
}: {
  data: ConfigSnapshot;
  scope: ConfigScope;
  statuses: Record<string, WriteStatus>;
  onWrite: (name: string, value: string | null) => void;
  onSwitchScope: (scope: ConfigScope) => void;
}) {
  const path = scopePath(scope, data.project.path);
  return (
    <>
      <p className="settings-writing">
        {scope === "project" && !data.project.available ? (
          <span className={toneClass("warning")}>
            no project workspace here: {data.project.path} cannot be written, so these keys keep
            their user values
          </span>
        ) : (
          <>
            writing to <code>{path}</code>
          </>
        )}
      </p>
      {groupBySection(data.entries).map(({ section, entries }) => (
        <section key={section} className="settings-section" aria-labelledby={`settings-${section}`}>
          <h3 id={`settings-${section}`} className="eyebrow settings-section-head">
            {section}
          </h3>
          <ul className="settings-rows">
            {entries.map((entry) => (
              <SettingRow
                key={entry.name}
                entry={entry}
                scope={scope}
                status={statuses[statusKey(scope, entry.name)] ?? { phase: "idle" }}
                onWrite={(value) => onWrite(entry.name, value)}
                onSwitchScope={onSwitchScope}
              />
            ))}
          </ul>
        </section>
      ))}
    </>
  );
}
