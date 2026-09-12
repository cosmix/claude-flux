import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { ConfigClient, WriteResult } from "@/api/config";
import {
  filterSections,
  formatValue,
  rowLabel,
  sectionRows,
  USER_CONFIG_PATH,
} from "@/components/settings-model";
import { entry, fakeClient, renderAt, row, snapshot } from "@/test/settings-kit";

afterEach(() => {
  cleanup();
});

describe("settings dialog", () => {
  it("opens from ?settings=1, no radio group, one control per writable lane", async () => {
    const data = snapshot();
    renderAt("?settings=1", fakeClient(data).client);

    expect(screen.getByRole("dialog")).toBeTruthy();
    await screen.findByRole("switch", { name: "check at user scope" });
    expect(screen.getByText("built-in")).toBeTruthy();
    expect(screen.getByText("user")).toBeTruthy();
    expect(screen.getByText("project")).toBeTruthy();
    expect(screen.getByText(USER_CONFIG_PATH)).toBeTruthy();
    expect(screen.getByText(data.project.path)).toBeTruthy();
    expect(screen.queryAllByRole("radio")).toHaveLength(0);

    const backend = row("backend");
    expect(within(backend).getAllByRole("combobox")).toHaveLength(2);
    expect(within(backend).getByRole("combobox", { name: "backend at user scope" })).toBeTruthy();
    expect(
      within(backend).getByRole("combobox", { name: "backend at project scope" }),
    ).toBeTruthy();
  });

  it("closes when the settings param goes away, as the back button does", async () => {
    const router = renderAt("?settings=1", fakeClient().client);
    await screen.findByRole("switch", { name: "check at user scope" });

    fireEvent.click(screen.getByRole("button", { name: "Close" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(router.state.location.search).toBe("");
  });

  it("draws a pair as one row with four comboboxes and a summary from the effective values", async () => {
    const data = snapshot();
    renderAt("?settings=1", fakeClient(data).client);
    await screen.findByRole("switch", { name: "check at user scope" });

    const standard = row("standard");
    expect(within(standard).getAllByRole("combobox")).toHaveLength(4);

    const model = data.entries.find((candidate) => candidate.name === "models.standard_model");
    const effort = data.entries.find((candidate) => candidate.name === "models.standard_effort");
    if (!model || !effort) throw new Error("fixture missing the standard pair");
    const summary = standard.querySelector(".settings-res");
    expect(summary?.textContent).toBe(
      `runs ${formatValue(model.kind, model.effective.value)} · ${formatValue(effort.kind, effort.effective.value)}`,
    );

    const modelUser = within(standard).getByRole("combobox", {
      name: "standard_model at user scope",
    });
    expect(modelUser.closest(".settings-slot")?.getAttribute("data-provenance")).toBe("inherited");

    const modelProject = within(standard).getByRole("combobox", {
      name: "standard_model at project scope",
    });
    const modelProjectSlot = modelProject.closest(".settings-slot");
    expect(modelProjectSlot?.getAttribute("data-provenance")).toBe("set");
    expect(modelProjectSlot?.getAttribute("data-effective")).toBe("true");
    if (!(modelProjectSlot instanceof HTMLElement)) throw new Error("no slot for standard_model");
    expect(within(modelProjectSlot).getByText("in effect")).toBeTruthy();

    const effortUser = within(standard).getByRole("combobox", {
      name: "standard_effort at user scope",
    });
    const effortUserSlot = effortUser.closest(".settings-slot");
    if (!(effortUserSlot instanceof HTMLElement)) throw new Error("no slot for standard_effort");
    expect(effortUserSlot.getAttribute("data-effective")).toBe("true");
    expect(within(effortUserSlot).getByText("in effect")).toBeTruthy();

    const effortProject = within(standard).getByRole("combobox", {
      name: "standard_effort at project scope",
    });
    expect(effortProject.closest(".settings-slot")?.getAttribute("data-effective")).toBeNull();
  });

  it("draws a user-only section's pane once and lets a project-capable section write there too", async () => {
    renderAt("?settings=1", fakeClient().client);
    await screen.findByRole("switch", { name: "check at user scope" });

    const check = row("check");
    expect(within(check).getByText("User-only keys")).toBeTruthy();
    const checkSection = check.closest("tbody");
    if (!checkSection) throw new Error("check row has no section tbody");
    expect(within(checkSection).getAllByText("User-only keys")).toHaveLength(1);

    const checkInterval = row("check_interval_hours");
    expect(within(checkInterval).queryByRole("textbox", { name: /at project scope/ })).toBeNull();

    const ceiling = row("ceiling_tokens");
    expect(
      within(ceiling).getByRole("textbox", { name: "ceiling_tokens at project scope" }),
    ).toBeTruthy();
  });

  it("writes at the project lane and shows the server's answer", async () => {
    const { client, writes } = fakeClient();
    renderAt("?settings=1", client);
    await screen.findByRole("switch", { name: "check at user scope" });

    const projectSelect = within(row("backend")).getByRole("combobox", {
      name: "backend at project scope",
    });
    fireEvent.change(projectSelect, { target: { value: "tmux" } });

    expect(writes).toEqual([{ scope: "project", name: "terminal.backend", value: "tmux" }]);
    await waitFor(() =>
      expect(
        within(row("backend")).getByRole("combobox", { name: "backend at project scope" }),
      ).toHaveProperty("value", "tmux"),
    );
  });

  it("clears a project override with a null write and falls back to the user value", async () => {
    const { client, writes } = fakeClient();
    renderAt("?settings=1", client);
    await screen.findByRole("switch", { name: "check at user scope" });

    const clearButton = within(row("ceiling_tokens")).getByRole("button", {
      name: "clear ceiling_tokens at project scope",
    });
    fireEvent.click(clearButton);

    expect(writes).toEqual([{ scope: "project", name: "context.ceiling_tokens", value: null }]);
    await waitFor(() =>
      expect(
        within(row("ceiling_tokens")).getByRole("textbox", {
          name: "ceiling_tokens at project scope",
        }),
      ).toHaveProperty("value", "800000"),
    );
    expect(screen.getByText("cleared, falls back to user 800,000")).toBeTruthy();
  });

  it("shows a 400 beside the control, marks the cell, and lets Escape drop a fresh draft before closing", async () => {
    const message = 'context.ceiling_tokens: "abc" is not a u32 (expected a non-negative integer)';
    const { client, writes } = fakeClient(snapshot(), message);
    renderAt("?settings=1", client);
    const input = await screen.findByRole("textbox", { name: "ceiling_tokens at user scope" });

    fireEvent.change(input, { target: { value: "abc" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(writes).toEqual([{ scope: "user", name: "context.ceiling_tokens", value: "abc" }]);
    const ceiling = row("ceiling_tokens");
    const alert = await within(ceiling).findByRole("alert");
    expect(alert.textContent).toBe(message);
    let control = within(ceiling).getByRole("textbox", { name: "ceiling_tokens at user scope" });
    expect(control.getAttribute("aria-describedby")).toContain(alert.id);
    expect(control.closest("td")?.className).toContain("hazard-error");

    fireEvent.change(control, { target: { value: "123" } });
    expect(control.getAttribute("data-draft")).toBe("true");
    fireEvent.keyDown(control, { key: "Escape" });

    control = within(row("ceiling_tokens")).getByRole("textbox", {
      name: "ceiling_tokens at user scope",
    });
    expect(control).toHaveProperty("value", "800000");
    expect(control.getAttribute("data-draft")).toBeNull();
    expect(screen.getByRole("dialog")).toBeTruthy();

    fireEvent.keyDown(control, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the clear button visible after a rejected write to an already-set key", async () => {
    const message = "models.standard_effort: not a valid effort";
    const { client, writes } = fakeClient(snapshot(), message);
    renderAt("?settings=1", client);
    await screen.findByRole("switch", { name: "check at user scope" });

    const standard = row("standard");
    const effortUser = within(standard).getByRole("combobox", {
      name: "standard_effort at user scope",
    });
    fireEvent.change(effortUser, { target: { value: "max" } });

    expect(writes).toEqual([{ scope: "user", name: "models.standard_effort", value: "max" }]);
    const alert = await within(standard).findByRole("alert");
    expect(alert.textContent).toBe(message);
    expect(
      within(standard).getByRole("button", { name: "clear standard_effort at user scope" }),
    ).toBeTruthy();
  });

  it("shows MODEL/EFFORT sub-headers and spans a stray single row across a mixed section", async () => {
    const data = snapshot();
    data.entries.push(
      entry({
        name: "models.stray_budget",
        kind: { type: "u32" },
        scopes: ["user", "project"],
        default: "1000",
        user: { value: "1000", set: false },
        project: { value: "1000", set: false },
        effective: { value: "1000", source: "default" },
      }),
    );
    renderAt("?settings=1", fakeClient(data).client);
    await screen.findByRole("switch", { name: "check at user scope" });

    const strayRow = row("stray_budget");
    const tbody = strayRow.closest("tbody");
    if (!tbody) throw new Error("stray row has no section tbody");
    const headerLabels = within(tbody).getAllByText(/^(model|effort)$/);
    expect(headerLabels.map((label) => label.textContent)).toEqual([
      "model",
      "effort",
      "model",
      "effort",
      "model",
      "effort",
    ]);

    const userCell = within(strayRow)
      .getByRole("textbox", { name: "stray_budget at user scope" })
      .closest("td");
    if (!(userCell instanceof HTMLTableCellElement)) throw new Error("no user td for stray_budget");
    expect(userCell.colSpan).toBe(2);

    const builtinCell = strayRow.querySelector(".settings-lane-builtin");
    if (!(builtinCell instanceof HTMLTableCellElement)) {
      throw new Error("no builtin td for stray_budget");
    }
    expect(builtinCell.colSpan).toBe(2);
  });

  it("reverts a switch after a failed write and keeps the message off other rows", async () => {
    const { client } = fakeClient(snapshot(), "update.check: only true or false");
    renderAt("?settings=1", client);
    const toggle = await screen.findByRole("switch", { name: "check at user scope" });
    expect(toggle).toHaveProperty("checked", true);

    fireEvent.click(toggle);

    const check = row("check");
    await within(check).findByText("update.check: only true or false");
    expect(within(check).getByRole("switch", { name: "check at user scope" })).toHaveProperty(
      "checked",
      true,
    );
    expect(within(row("check_interval_hours")).queryByText(/only true or false/)).toBeNull();
  });

  it("does not write when the number field is left unchanged", async () => {
    const { client, writes } = fakeClient();
    renderAt("?settings=1", client);
    const input = await screen.findByRole("textbox", { name: "ceiling_tokens at user scope" });

    fireEvent.change(input, { target: { value: "800000" } });
    fireEvent.blur(input);

    expect(writes).toEqual([]);
  });

  it("filters rows by value or help text, focuses on /, and clears with Escape", async () => {
    const data = snapshot();
    renderAt("?settings=1", fakeClient(data).client);
    const filter = await screen.findByRole("searchbox", { name: "filter settings" });

    fireEvent.change(filter, { target: { value: "sonnet" } });

    const expected = new Set(
      filterSections(sectionRows(data.entries), "sonnet").flatMap((section) =>
        section.rows.map((candidate) => rowLabel(candidate)),
      ),
    );
    expect(expected.size).toBeGreaterThan(0);
    const shown = new Set(
      Array.from(document.querySelectorAll(".settings-key-name")).map(
        (element) => element.textContent,
      ),
    );
    expect(shown).toEqual(expected);

    fireEvent.change(filter, { target: { value: "zzzzznotfound" } });
    expect(screen.getByText(/^nothing matches /)).toBeTruthy();

    fireEvent.change(filter, { target: { value: "" } });
    await screen.findByRole("switch", { name: "check at user scope" });

    fireEvent.keyDown(screen.getByRole("dialog"), { key: "/" });
    expect(document.activeElement).toBe(filter);

    filter.blur();
    const combobox = within(row("backend")).getByRole("combobox", {
      name: "backend at user scope",
    });
    combobox.focus();
    fireEvent.keyDown(combobox, { key: "/" });
    expect(document.activeElement).toBe(combobox);

    fireEvent.change(filter, { target: { value: "sonnet" } });
    fireEvent.keyDown(filter, { key: "Escape" });
    expect(filter).toHaveProperty("value", "");
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("shows no project workspace when the project file is unavailable", async () => {
    const data = snapshot();
    data.project = { available: false, path: data.project.path };
    renderAt("?settings=1", fakeClient(data).client);
    await screen.findByRole("switch", { name: "check at user scope" });

    expect(screen.getByText("not available")).toBeTruthy();
    expect(screen.getAllByText("No project workspace").length).toBeGreaterThan(0);
    expect(document.querySelectorAll('[aria-label*="at project scope"]')).toHaveLength(0);
  });

  it("disables the control and shows the pending roundel while a write is in flight", async () => {
    const client: ConfigClient = {
      load: async () => snapshot(),
      write: () => new Promise<WriteResult>(() => {}),
    };
    renderAt("?settings=1", client);
    const toggle = await screen.findByRole("switch", { name: "check at user scope" });

    fireEvent.click(toggle);

    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "check at user scope" })).toHaveProperty(
        "disabled",
        true,
      ),
    );
    expect(screen.getByText("saving")).toBeTruthy();
  });

  it("hides the clear button for a project-scope key while its own write is pending", async () => {
    const client: ConfigClient = {
      load: async () => snapshot(),
      write: () => new Promise<WriteResult>(() => {}),
    };
    renderAt("?settings=1", client);
    await screen.findByRole("switch", { name: "check at user scope" });

    expect(
      within(row("ceiling_tokens")).getByRole("button", {
        name: "clear ceiling_tokens at project scope",
      }),
    ).toBeTruthy();

    const input = within(row("ceiling_tokens")).getByRole("textbox", {
      name: "ceiling_tokens at project scope",
    });
    fireEvent.change(input, { target: { value: "123" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() =>
      expect(
        within(row("ceiling_tokens")).getByRole("textbox", {
          name: "ceiling_tokens at project scope",
        }),
      ).toHaveProperty("disabled", true),
    );
    expect(
      within(row("ceiling_tokens")).queryByRole("button", {
        name: "clear ceiling_tokens at project scope",
      }),
    ).toBeNull();
  });
});
