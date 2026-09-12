import { cn } from "cn";
import { SearchIcon } from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type RefObject,
} from "react";
import { useSearchParams } from "react-router";

import {
  createConfigClient,
  type ConfigClient,
  type ConfigEntry,
  type ConfigScope,
  type ConfigSnapshot,
} from "@/api/config";
import { SettingsCards } from "@/components/settings-cards";
import { SettingsLanes } from "@/components/settings-lanes";
import {
  fallbackFor,
  filterSections,
  formatValue,
  sectionRows,
  statusKey,
  type WriteStatus,
} from "@/components/settings-model";
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
import { useMediaQuery } from "@/lib/use-media-query";

/// The query parameter that opens the dialog, so it has a URL and the back
/// button closes it.
export const SETTINGS_PARAM = "settings";

export function useOpenSettings(): () => void {
  const [, setParams] = useSearchParams();
  return useCallback(
    () =>
      setParams((params) => {
        const next = new URLSearchParams(params);
        next.set(SETTINGS_PARAM, "1");
        return next;
      }),
    [setParams],
  );
}

const defaultClient = createConfigClient();

/// Whether the dialog's query parameter is present, and a `close` that
/// drops it.
function useSettingsParam(): { open: boolean; close: () => void } {
  const [params, setParams] = useSearchParams();
  const close = () =>
    setParams((current) => {
      const next = new URLSearchParams(current);
      next.delete(SETTINGS_PARAM);
      return next;
    });
  return { open: params.get(SETTINGS_PARAM) !== null, close };
}

/// `/` outside a field jumps to the filter box and selects its text.
function focusFilterOnSlash(
  event: ReactKeyboardEvent,
  filterRef: RefObject<HTMLInputElement | null>,
): void {
  if (event.key !== "/") return;
  if (event.target instanceof HTMLElement && event.target.matches("input, select, textarea")) {
    return;
  }
  event.preventDefault();
  filterRef.current?.focus();
  filterRef.current?.select();
}

/// Radix's Escape listener runs in the capture phase, ahead of the filter
/// input's own keydown handler: preventing default here is what keeps the
/// dialog open so the filter's handler can clear the query on the way back
/// up instead of closing it. Also covers a number field with an unsaved
/// draft, today's rule.
function keepOpenOnEscape(event: KeyboardEvent): void {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return;
  if (target.matches("[data-filter]") && (target as HTMLInputElement).value) {
    event.preventDefault();
    return;
  }
  if (target.matches("[data-draft]")) {
    event.preventDefault();
  }
}

/// Loom's configuration across every writable file, opened from any route
/// with `?settings=1`; every cell writes on its own, one key at a time.
export function SettingsDialog({ client = defaultClient }: { client?: ConfigClient }) {
  const { open, close } = useSettingsParam();
  const [query, setQuery] = useState("");
  const filterRef = useRef<HTMLInputElement>(null);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent
        className={cn("settings-dialog gap-0 overflow-hidden p-0 sm:max-w-[980px]")}
        onKeyDown={(event) => focusFilterOnSlash(event, filterRef)}
        onEscapeKeyDown={keepOpenOnEscape}
      >
        {open && (
          <Body client={client} query={query} onQueryChange={setQuery} filterRef={filterRef} />
        )}
      </DialogContent>
    </Dialog>
  );
}

type LoadState =
  | { phase: "loading" }
  | { phase: "error"; message: string }
  | { phase: "ready"; data: ConfigSnapshot };

/// Loads the snapshot. `retry` bumps `attempt` to fetch it again, which is
/// both the manual "retry" action and how a stale CSRF token is replaced;
/// `applyEntry` folds one written entry back into a loaded snapshot.
function useSnapshot(client: ConfigClient) {
  const [load, setLoad] = useState<LoadState>({ phase: "loading" });
  const [attempt, setAttempt] = useState(0);

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

  const retry = () => setAttempt((n) => n + 1);
  const applyEntry = (entry: ConfigEntry) =>
    setLoad((current) =>
      current.phase === "ready" ? { ...current, data: replaceEntry(current.data, entry) } : current,
    );

  return { load, retry, applyEntry };
}

/// Tracks every in-flight or settled write keyed by scope and name, and
/// drives the footer toast. A CSRF rejection re-fetches the snapshot.
function useSettingsWrites(client: ConfigClient) {
  const { load, retry, applyEntry } = useSnapshot(client);
  const [statuses, setStatuses] = useState<Record<string, WriteStatus>>({});
  const [toast, setToast] = useState("each cell writes on its own");

  const setStatus = (key: string, status: WriteStatus) =>
    setStatuses((current) => ({ ...current, [key]: status }));

  const write = async (scope: ConfigScope, name: string, value: string | null) => {
    if (load.phase !== "ready") return;
    const key = statusKey(scope, name);
    setStatus(key, { phase: "pending", value });
    setToast(`writing ${name} at ${scope}…`);
    const result = await client.write(load.data.csrf_token, { scope, name, value });
    if (result.ok) {
      applyEntry(result.entry);
      setStatus(key, { phase: "saved" });
      setToast(savedToast(result.entry, scope, value));
      return;
    }
    setStatus(key, { phase: "error", message: result.message });
    setToast("the server rejected the write");
    // A CSRF rejection means the token this dialog holds is stale; reading
    // the config again fetches a fresh one along with the current values.
    if (result.status === 403) retry();
  };

  return { load, statuses, toast, write, retry };
}

/// The toast after an accepted write; a clear names the value it now falls
/// back to.
function savedToast(entry: ConfigEntry, scope: ConfigScope, value: string | null): string {
  if (value !== null) return "saved";
  const fallback = fallbackFor(entry, scope);
  return `cleared, falls back to ${fallback.tier} ${formatValue(entry.kind, fallback.value)}`;
}

/// Title, filter box and (>=700px only) description. Below 700px — the same
/// breakpoint the dialog swaps table for cards at — the header stacks into
/// "Settings" + close button, then a full-width filter, and the description
/// drops to `sr-only` rather than fighting the close button for room.
function SettingsHeader({
  query,
  onQueryChange,
  filterRef,
}: {
  query: string;
  onQueryChange: (query: string) => void;
  filterRef: RefObject<HTMLInputElement | null>;
}) {
  return (
    <DialogHeader
      className={cn(
        "settings-head",
        "flex flex-col items-stretch gap-3 p-4 pr-12",
        "min-[700px]:flex-row min-[700px]:items-start min-[700px]:justify-between min-[700px]:gap-4 min-[700px]:p-5 min-[700px]:pr-14",
      )}
    >
      <div>
        <DialogTitle className="text-xl font-semibold tracking-tight">Settings</DialogTitle>
        <DialogDescription className="max-[699px]:sr-only min-[700px]:max-w-[46ch]">
          A key takes the rightmost value that is set. Edit a cell to set it in that file; clear it
          to fall back. Sections with two sub-columns are read as one row per stage or step.
        </DialogDescription>
      </div>
      <label className={cn("settings-filter", "max-[699px]:w-full")}>
        <SearchIcon aria-hidden="true" className="size-3.5 shrink-0" />
        <input
          ref={filterRef}
          type="search"
          data-filter
          aria-label="filter settings"
          placeholder="filter keys, values, help"
          autoComplete="off"
          spellCheck={false}
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape" && event.currentTarget.value) {
              event.stopPropagation();
              onQueryChange("");
            }
          }}
        />
        <Kbd>/</Kbd>
      </label>
    </DialogHeader>
  );
}

function SettingsFooter({ toast }: { toast: string }) {
  return (
    <footer className="settings-foot">
      <span>
        <i className="settings-legend-eff" aria-hidden="true" /> in effect · <Kbd>Esc</Kbd> closes
      </span>
      <span className="settings-toast" aria-live="polite">
        {toast}
      </span>
    </footer>
  );
}

/// What the scroll area shows while the snapshot loads or after it fails.
function LoadNotice({ load, onRetry }: { load: LoadState; onRetry: () => void }) {
  if (load.phase === "loading") {
    return (
      <p className="settings-notice" aria-busy="true">
        reading configuration…
      </p>
    );
  }
  if (load.phase === "error") {
    return (
      <div className={cn("settings-notice", toneClass("blocked"))} role="alert">
        <span>{load.message}</span>
        <Button type="button" variant="outline" size="xs" onClick={onRetry}>
          retry
        </Button>
      </div>
    );
  }
  return null;
}

// Both layouts take the same props; below 700px the cards replace the table.
// Crossing the breakpoint swaps the component, so local UI state inside it
// (an uncommitted number-field draft) is dropped; write statuses live above
// it and survive.
function Body({
  client,
  query,
  onQueryChange,
  filterRef,
}: {
  client: ConfigClient;
  query: string;
  onQueryChange: (query: string) => void;
  filterRef: RefObject<HTMLInputElement | null>;
}) {
  const { load, statuses, toast, write, retry } = useSettingsWrites(client);
  const narrow = useMediaQuery("(max-width: 699px)");

  const sections = useMemo(
    () => (load.phase === "ready" ? filterSections(sectionRows(load.data.entries), query) : []),
    [load, query],
  );

  return (
    <>
      <SettingsHeader query={query} onQueryChange={onQueryChange} filterRef={filterRef} />
      <div className="settings-scroll">
        <LoadNotice load={load} onRetry={retry} />
        {load.phase === "ready" &&
          (narrow ? (
            <SettingsCards
              data={load.data}
              sections={sections}
              query={query}
              statuses={statuses}
              onWrite={write}
            />
          ) : (
            <SettingsLanes
              data={load.data}
              sections={sections}
              query={query}
              statuses={statuses}
              onWrite={write}
            />
          ))}
      </div>
      <SettingsFooter toast={toast} />
    </>
  );
}

function replaceEntry(data: ConfigSnapshot, entry: ConfigEntry): ConfigSnapshot {
  return {
    ...data,
    entries: data.entries.map((current) => (current.name === entry.name ? entry : current)),
  };
}
