import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { createStore } from "jotai";
import { Provider } from "jotai/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ConfigClient, ConfigEntry, ConfigSnapshot, ConfigWrite } from "@/api/config";
import { SettingsDialog } from "@/components/settings-dialog";
import { routes } from "@/router";

function entry(overrides: Partial<ConfigEntry> & Pick<ConfigEntry, "name" | "kind">): ConfigEntry {
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
/// inherits, pressure.claude_model is user-only, and the models pair covers
/// both shapes a key-level tier can take - set (standard_model) and left for
/// the user tier (standard_effort).
function snapshot(): ConfigSnapshot {
  return {
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
        project: { value: "sonnet", set: true },
        effective: { value: "sonnet", source: "project" },
      }),
      entry({
        name: "models.standard_effort",
        kind: { type: "enum", variants: ["low", "medium", "high", "xhigh", "max"] },
        scopes: ["user", "project"],
        project: { value: "x", set: false },
      }),
    ],
  };
}

/// Applies a write to the fixture the way the server would, so the entry
/// that comes back carries the new scope values and effective source.
function applyWrite(data: ConfigSnapshot, write: ConfigWrite): ConfigEntry {
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
      write.value === null ? { value: "default", set: false } : { value: write.value, set: true };
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

function fakeClient(data: ConfigSnapshot = snapshot(), failWith?: string) {
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

function renderDialog(path: string, client: ConfigClient) {
  const router = createMemoryRouter([{ path: "/", element: <SettingsDialog client={client} /> }], {
    initialEntries: [path],
  });
  render(<RouterProvider router={router} />);
  return router;
}

function row(name: string): HTMLElement {
  const element = document.body.querySelector(`[data-key="${name}"]`);
  if (!(element instanceof HTMLElement)) throw new Error(`no row for ${name}`);
  return element;
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("settings dialog", () => {
  it("opens from ?settings=1 at user scope and renders each control from its kind", async () => {
    renderDialog("/?settings=1", fakeClient().client);

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(await screen.findByRole("switch", { name: "check" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: /^user/ })).toHaveProperty("checked", true);
    expect(screen.getByRole("combobox", { name: "backend" })).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "ceiling_tokens" })).toHaveProperty(
      "value",
      "800000",
    );
    expect(screen.getByText(/writing to/).textContent).toContain("~/.loom/config.toml");
  });

  it("closes when the settings param goes away, as the back button does", async () => {
    const router = renderDialog("/?settings=1", fakeClient().client);
    await screen.findByRole("switch", { name: "check" });

    fireEvent.click(screen.getByRole("button", { name: "Close" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(router.state.location.search).toBe("");
  });

  it("switches scope through the radio group and marks user-only keys instead of faking a control", async () => {
    const router = renderDialog("/?settings=1", fakeClient().client);
    await screen.findByRole("switch", { name: "check" });

    fireEvent.click(screen.getByRole("radio", { name: /^project/ }));

    expect(router.state.location.search).toBe("?settings=project");
    expect(screen.getByText(/writing to/).textContent).toContain(".loom/work/config.toml");
    const check = row("update.check");
    expect(within(check).queryByRole("switch")).toBeNull();
    expect(within(check).getByText("machine-wide preference")).toBeTruthy();
    expect(check.dataset.provenance).toBe("unavailable");

    fireEvent.click(within(check).getByRole("button", { name: "set at user scope" }));
    expect(router.state.location.search).toBe("?settings=user");
  });

  it("draws an inherited project value differently from a set override, and lights the effective stop", async () => {
    renderDialog("/?settings=project", fakeClient().client);
    await screen.findByRole("combobox", { name: "backend" });

    const backend = row("terminal.backend");
    expect(backend.dataset.provenance).toBe("inherited");
    expect(within(backend).getByText("inherited from user")).toBeTruthy();
    expect(within(backend).queryByRole("button", { name: /clear override/ })).toBeNull();
    const backendRail = within(backend).getByRole("list", { name: /terminal.backend comes from/ });
    expect(backendRail.querySelector('[aria-current="true"]')?.textContent).toContain("built-in");

    const ceiling = row("context.ceiling_tokens");
    expect(ceiling.dataset.provenance).toBe("set");
    expect(within(ceiling).getByRole("textbox")).toHaveProperty("value", "900000");
    expect(within(ceiling).getByRole("button", { name: /clear override/ }).textContent).toContain(
      "back to user 800000",
    );
    const ceilingRail = within(ceiling).getByRole("list", { name: /ceiling_tokens comes from/ });
    expect(ceilingRail.querySelector('[aria-current="true"]')?.textContent).toContain("project");
  });

  it("names the built-in on the rail and in the reset affordance even once the user scope sets the key", async () => {
    const data = snapshot();
    const check = data.entries.find((candidate) => candidate.name === "update.check");
    if (!check) throw new Error("no update.check entry");
    // The fixture's own shape: `update.check` set to false at user scope,
    // with a true built-in. `user.value` alone cannot name that built-in
    // once `user.set` is true, which is exactly what `default` is for.
    check.default = "true";
    check.user = { value: "false", set: true };
    check.effective = { value: "false", source: "user" };
    renderDialog("/?settings=1", fakeClient(data).client);

    const toggle = await screen.findByRole("switch", { name: "check" });
    expect(toggle).toHaveProperty("checked", false);

    const check_ = row("update.check");
    const rail = within(check_).getByRole("list", { name: /update.check comes from/ });
    const [builtIn] = within(rail).getAllByRole("listitem");
    expect(builtIn.textContent).toContain("built-in");
    expect(builtIn.textContent).toContain("on");

    expect(within(check_).getByRole("button", { name: /^reset/ }).textContent).toContain(
      "back to built-in on",
    );
  });

  it("warns at user scope when a project override is what actually applies", async () => {
    renderDialog("/?settings=user", fakeClient().client);
    await screen.findByRole("switch", { name: "check" });

    expect(
      within(row("context.ceiling_tokens")).getByText("overridden by project: 900000"),
    ).toBeTruthy();
  });

  it("clears a project override with a null write and falls back to the user value", async () => {
    const { client, writes } = fakeClient();
    renderDialog("/?settings=project", client);
    await screen.findByRole("combobox", { name: "backend" });

    fireEvent.click(
      within(row("context.ceiling_tokens")).getByRole("button", { name: /clear override/ }),
    );

    expect(writes).toEqual([{ scope: "project", name: "context.ceiling_tokens", value: null }]);
    await waitFor(() => expect(row("context.ceiling_tokens").dataset.provenance).toBe("inherited"));
    const ceiling = row("context.ceiling_tokens");
    expect(within(ceiling).getByRole("textbox")).toHaveProperty("value", "800000");
    expect(within(ceiling).getByText("saved")).toBeTruthy();
  });

  it("writes a select change at the scope being edited", async () => {
    const { client, writes } = fakeClient();
    renderDialog("/?settings=project", client);
    const select = await screen.findByRole("combobox", { name: "backend" });

    fireEvent.change(select, { target: { value: "tmux" } });

    expect(writes).toEqual([{ scope: "project", name: "terminal.backend", value: "tmux" }]);
    await waitFor(() => expect(row("terminal.backend").dataset.provenance).toBe("set"));
    expect(screen.getByRole("combobox", { name: "backend" })).toHaveProperty("value", "tmux");
  });

  it("shows a 400 verbatim beside the control and reverts it to the server's value", async () => {
    const message = 'context.ceiling_tokens: "abc" is not a u32 (expected a non-negative integer)';
    const { client, writes } = fakeClient(snapshot(), message);
    renderDialog("/?settings=user", client);
    const input = await screen.findByRole("textbox", { name: "ceiling_tokens" });

    fireEvent.change(input, { target: { value: "abc" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(writes).toEqual([{ scope: "user", name: "context.ceiling_tokens", value: "abc" }]);
    const ceiling = row("context.ceiling_tokens");
    await within(ceiling).findByText(message);
    const reverted = within(ceiling).getByRole("textbox");
    expect(reverted).toHaveProperty("value", "800000");
    expect(reverted.getAttribute("aria-invalid")).toBe("true");
    expect(reverted.getAttribute("aria-describedby")).toContain(
      within(ceiling).getByText(message).id,
    );
  });

  it("reverts a switch after a failed write and keeps the status on the scope it was written at", async () => {
    const { client } = fakeClient(snapshot(), "update.check: only true or false");
    renderDialog("/?settings=user", client);
    const toggle = await screen.findByRole("switch", { name: "check" });
    expect(toggle).toHaveProperty("checked", true);

    fireEvent.click(toggle);

    const check = row("update.check");
    await within(check).findByText("update.check: only true or false");
    expect(within(check).getByRole("switch")).toHaveProperty("checked", true);

    fireEvent.click(screen.getByRole("radio", { name: /^project/ }));
    expect(within(row("update.check")).queryByText(/only true or false/)).toBeNull();
  });

  it("drops an unsaved draft on Escape and only closes on the next one", async () => {
    const { client, writes } = fakeClient();
    renderDialog("/?settings=user", client);
    const input = await screen.findByRole("textbox", { name: "ceiling_tokens" });

    fireEvent.change(input, { target: { value: "123" } });
    expect(input.getAttribute("data-draft")).toBe("true");
    fireEvent.keyDown(input, { key: "Escape" });

    expect(input).toHaveProperty("value", "800000");
    expect(input.getAttribute("data-draft")).toBeNull();
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(writes).toEqual([]);

    fireEvent.keyDown(input, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("does not write when the number field is left unchanged", async () => {
    const { client, writes } = fakeClient();
    renderDialog("/?settings=user", client);
    const input = await screen.findByRole("textbox", { name: "ceiling_tokens" });

    fireEvent.change(input, { target: { value: "800000" } });
    fireEvent.blur(input);

    expect(writes).toEqual([]);
  });

  it("keeps the project side coherent when no workspace file is available", async () => {
    const data = snapshot();
    data.project = { available: false, path: ".loom/work/config.toml" };
    for (const item of data.entries) item.project = null;
    renderDialog("/?settings=project", fakeClient(data).client);
    await screen.findByText(/no project workspace here/);

    const projectCell = screen.getByRole("radio", { name: /^project/ }).closest("label");
    if (projectCell === null) throw new Error("project radio has no label");
    expect(within(projectCell).getByText("not available")).toBeTruthy();
    const backend = row("terminal.backend");
    expect(within(backend).queryByRole("combobox")).toBeNull();
    expect(within(backend).getByText("no project workspace to write to")).toBeTruthy();
    expect(within(row("update.check")).getByText("machine-wide preference")).toBeTruthy();
  });

  it("offers a retry when the config fetch fails", async () => {
    let calls = 0;
    const client: ConfigClient = {
      load: async () => {
        calls += 1;
        if (calls === 1) throw new Error("config fetch failed: HTTP 500");
        return snapshot();
      },
      write: async () => ({ ok: false, status: null, message: "unused" }),
    };
    renderDialog("/?settings=1", client);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("HTTP 500");
    fireEvent.click(screen.getByRole("button", { name: "retry" }));
    expect(await screen.findByRole("switch", { name: "check" })).toBeTruthy();
  });
});

describe("header entry point", () => {
  it("opens the dialog from the header and fetches /api/config", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      if (String(input) === "/api/config") {
        return new Response(JSON.stringify(snapshot()), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });
    vi.stubGlobal("fetch", fetchMock);
    const router = createMemoryRouter(routes, { initialEntries: ["/ledger"] });
    render(
      <Provider store={createStore()}>
        <RouterProvider router={router} />
      </Provider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "open settings" }));

    expect(router.state.location.search).toBe("?settings=user");
    expect(await screen.findByRole("switch", { name: "check" })).toBeTruthy();
    expect(fetchMock).toHaveBeenCalledWith("/api/config", { cache: "no-store" });
  });
});
