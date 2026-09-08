import { z } from "zod";

/// The `/api/config` wire model: every registry key with its resolved value
/// at each scope, and where the value loom will actually use came from.
export const configScopeSchema = z.enum(["user", "project"]);
export const configKindSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("bool") }),
  z.object({ type: z.literal("u32") }),
  z.object({ type: z.literal("enum"), variants: z.array(z.string()) }),
]);
/// `value` is the value resolved at that scope (the inherited one when the
/// file does not set it); `set` is whether the file itself sets it.
export const scopeValueSchema = z.object({ value: z.string(), set: z.boolean() });
export const configEntrySchema = z.object({
  name: z.string(),
  help: z.string(),
  kind: configKindSchema,
  scopes: z.array(configScopeSchema),
  default: z.string(),
  user: scopeValueSchema,
  project: scopeValueSchema.nullable(),
  effective: z.object({
    value: z.string(),
    source: z.enum(["project", "user", "default"]),
  }),
});
export const configResponseSchema = z.object({
  csrf_token: z.string(),
  project: z.object({ available: z.boolean(), path: z.string() }),
  entries: z.array(configEntrySchema),
});
const writeResponseSchema = z.object({ entry: configEntrySchema });
const errorResponseSchema = z.object({ error: z.string() });

export type ConfigScope = z.infer<typeof configScopeSchema>;
export type ConfigKind = z.infer<typeof configKindSchema>;
export type ConfigEntry = z.infer<typeof configEntrySchema>;
export type ConfigSnapshot = z.infer<typeof configResponseSchema>;

export interface ConfigWrite {
  scope: ConfigScope;
  name: string;
  /// `null` unsets the key at that scope so it falls back to the tier below.
  value: string | null;
}

export type WriteResult =
  | { ok: true; entry: ConfigEntry }
  | { ok: false; status: number | null; message: string };

export interface ConfigClient {
  load: () => Promise<ConfigSnapshot>;
  write: (csrf: string, request: ConfigWrite) => Promise<WriteResult>;
}

export interface ConfigClientDeps {
  fetch?: typeof fetch;
}

const CSRF_HEADER = "X-Loom-Csrf";

function issueMessage(issues: { path: PropertyKey[]; message: string }[]): string {
  const issue = issues[0];
  const path = issue.path.map(String).join(".");
  return path ? `${path}: ${issue.message}` : issue.message;
}

/// What to say for a failed write when the server sent no message of its
/// own; the 400 validator text always wins over these.
function fallbackMessage(status: number): string {
  switch (status) {
    case 403:
      return "the server refused this dashboard's write token; reload to get a fresh one";
    case 409:
      return "no project workspace is available to write to";
    default:
      return `the server answered HTTP ${status}`;
  }
}

async function readErrorMessage(response: Response): Promise<string> {
  try {
    const parsed = errorResponseSchema.safeParse(await response.json());
    if (parsed.success) return parsed.data.error;
  } catch {
    // Not JSON: fall through to the status-derived text.
  }
  return fallbackMessage(response.status);
}

export function createConfigClient(deps: ConfigClientDeps = {}): ConfigClient {
  // Called through a closure rather than stored and invoked as `this.fetch`:
  // native fetch rejects a foreign receiver with "Illegal invocation" in a
  // real browser, which jsdom's mock never does.
  const request: typeof fetch = deps.fetch ?? ((input, init) => globalThis.fetch(input, init));

  return {
    async load() {
      const response = await request("/api/config", { cache: "no-store" });
      if (!response.ok) throw new Error(`config fetch failed: HTTP ${response.status}`);
      const parsed = configResponseSchema.safeParse(await response.json());
      if (!parsed.success) {
        throw new Error(`malformed config: ${issueMessage(parsed.error.issues)}`);
      }
      return parsed.data;
    },

    async write(csrf, body) {
      let response: Response;
      try {
        response = await request("/api/config", {
          method: "POST",
          headers: { "Content-Type": "application/json", [CSRF_HEADER]: csrf },
          body: JSON.stringify(body),
        });
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        return { ok: false, status: null, message: `request failed: ${message}` };
      }
      if (!response.ok) {
        return { ok: false, status: response.status, message: await readErrorMessage(response) };
      }
      let reply: unknown;
      try {
        reply = await response.json();
      } catch {
        return { ok: false, status: response.status, message: "malformed reply: not JSON" };
      }
      const parsed = writeResponseSchema.safeParse(reply);
      if (!parsed.success) {
        return {
          ok: false,
          status: response.status,
          message: `malformed reply: ${issueMessage(parsed.error.issues)}`,
        };
      }
      return { ok: true, entry: parsed.data.entry };
    },
  };
}
