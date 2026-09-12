import { render, screen } from "@testing-library/react";
import { createStore } from "jotai";
import { Provider } from "jotai/react";
import { createMemoryRouter, RouterProvider } from "react-router";

import type { ConfigClient, ConfigEntry, ConfigSnapshot, ConfigWrite } from "@/api/config";
import { SettingsDialog } from "@/components/settings-dialog";

/// Shared harness for the settings dialog test suites (settings-dialog,
/// settings-cards, settings-entry, settings-writes): a canonical registry
/// fixture plus the render/write/lookup helpers every suite needs. Each
/// suite only ever asserts on the specific keys it cares about, so the
/// extra entries other suites rely on are inert noise for the rest.
export function entry(
  overrides: Partial<ConfigEntry> & Pick<ConfigEntry, "name" | "kind">,
): ConfigEntry {
  return {
    help: `help for ${overrides.name}`,
    scopes: ["user"],
    default: "x",
    user: { value: "x", set: false },
    project: null,
    effective: { value: "x", source: "default" },
    ...overrides,
  };
}

/// A slice of the registry: ceiling has a project override, backend
/// inherits, pressure.claude_model is user-only, the standard pair covers
/// both shapes a key-level tier can take — set at project (standard_model)
/// and set at user (standard_effort) — and the knowledge pair is fully
/// inherited from the built-in at every tier.
const SNAPSHOT: ConfigSnapshot = {
  csrf_token: "deadbeef",
  project: { available: true, path: ".loom/work/config.toml" },
  entries: [
    entry({
      name: "update.check",
      kind: { type: "bool" },
      default: "true",
      user: { value: "true", set: false },
      effective: { value: "true", source: "default" },
    }),
    entry({
      name: "update.check_interval_hours",
      kind: { type: "u32" },
      default: "24",
      user: { value: "24", set: true },
      effective: { value: "24", source: "user" },
    }),
    entry({
      name: "terminal.backend",
      kind: { type: "enum", variants: ["native", "tmux"] },
      scopes: ["user", "project"],
      default: "native",
      user: { value: "native", set: false },
      project: { value: "native", set: false },
      effective: { value: "native", source: "default" },
    }),
    entry({
      name: "context.ceiling_tokens",
      kind: { type: "u32" },
      scopes: ["user", "project"],
      default: "800000",
      user: { value: "800000", set: false },
      project: { value: "900000", set: true },
      effective: { value: "900000", source: "project" },
    }),
    entry({
      name: "pressure.claude_model",
      kind: { type: "enum", variants: ["haiku", "sonnet", "opus", "fable"] },
      default: "opus",
      user: { value: "opus", set: true },
      effective: { value: "opus", source: "user" },
    }),
    entry({
      name: "models.standard_model",
      kind: { type: "enum", variants: ["haiku", "sonnet", "opus", "fable"] },
      scopes: ["user", "project"],
      default: "opus",
      user: { value: "opus", set: false },
      project: { value: "sonnet", set: true },
      effective: { value: "sonnet", source: "project" },
    }),
    entry({
      name: "models.standard_effort",
      kind: { type: "enum", variants: ["low", "medium", "high", "xhigh", "max"] },
      scopes: ["user", "project"],
      default: "medium",
      user: { value: "high", set: true },
      project: { value: "high", set: false },
      effective: { value: "high", source: "user" },
    }),
    entry({
      name: "models.knowledge_model",
      kind: { type: "enum", variants: ["haiku", "sonnet", "opus", "fable"] },
      scopes: ["user", "project"],
      default: "haiku",
      user: { value: "haiku", set: false },
      project: { value: "haiku", set: false },
      effective: { value: "haiku", source: "default" },
    }),
    entry({
      name: "models.knowledge_effort",
      kind: { type: "enum", variants: ["low", "medium", "high", "xhigh", "max"] },
      scopes: ["user", "project"],
      default: "low",
      user: { value: "low", set: false },
      project: { value: "low", set: false },
      effective: { value: "low", source: "default" },
    }),
  ],
};

/// A fresh deep copy of the fixture each call: callers mutate what they get
/// (`applyWrite` rewrites `entries`), so no two cases may share one.
export function snapshot(): ConfigSnapshot {
  return structuredClone(SNAPSHOT);
}

/// Applies a write to the fixture the way the server would, so the entry
/// that comes back carries the new scope values and effective source.
export function applyWrite(data: ConfigSnapshot, write: ConfigWrite): ConfigEntry {
  const current = data.entries.find((candidate) => candidate.name === write.name);
  if (!current) throw new Error(`no such key ${write.name}`);
  const next = structuredClone(current);
  if (write.scope === "project" && next.project) {
    next.project =
      write.value === null
        ? { value: next.user.value, set: false }
        : { value: write.value, set: true };
  } else if (write.scope === "user") {
    next.user =
      write.value === null
        ? { value: next.default, set: false }
        : { value: write.value, set: true };
    if (next.project && !next.project.set) {
      next.project = { ...next.project, value: next.user.value };
    }
  }
  next.effective = next.project?.set
    ? { value: next.project.value, source: "project" }
    : next.user.set
      ? { value: next.user.value, source: "user" }
      : { value: next.user.value, source: "default" };
  data.entries = data.entries.map((candidate) => (candidate.name === next.name ? next : candidate));
  return next;
}

export function fakeClient(data: ConfigSnapshot = snapshot(), failWith?: string) {
  const writes: ConfigWrite[] = [];
  const client: ConfigClient = {
    load: async () => structuredClone(data),
    write: async (csrf, request) => {
      writes.push(request);
      if (csrf !== data.csrf_token) return { ok: false, status: 403, message: "bad token" };
      if (failWith !== undefined) return { ok: false, status: 400, message: failWith };
      return { ok: true, entry: applyWrite(data, request) };
    },
  };
  return { client, writes };
}

/// Every case renders the dialog through a memory router in a jotai
/// Provider, the same shell the real app mounts it in, with the fake
/// `ConfigClient` threaded straight through instead of hitting `fetch`.
export function renderAt(search: string, client: ConfigClient) {
  const router = createMemoryRouter([{ path: "/", element: <SettingsDialog client={client} /> }], {
    initialEntries: [`/${search}`],
  });
  render(
    <Provider store={createStore()}>
      <RouterProvider router={router} />
    </Provider>,
  );
  return router;
}

/// A row is found by the text of its key cell — `rowLabel` for a pair
/// (`"standard"`), `fieldOf` for a single key (`"ceiling_tokens"`) — never by
/// the full dotted registry name.
export function row(label: string): HTMLElement {
  const heading = screen.getByText(label, { selector: ".settings-key-name" });
  const element = heading.closest("tr");
  if (!(element instanceof HTMLElement)) throw new Error(`no row for ${label}`);
  return element;
}
