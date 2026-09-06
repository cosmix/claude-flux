---
name: loom-typescript
description: TypeScript language expertise for type-safe, production-quality code.
triggers:
  - typescript
  - ts
  - tsx
  - interface
  - generic
  - union
  - intersection
  - discriminated union
  - type guard
  - type assertion
  - utility types
  - conditional types
  - mapped types
  - satisfies
  - zod
  - trpc
  - prisma
  - react
  - node
  - nodejs
  - deno
  - bun
  - oxc
  - oxlint
  - oxfmt
  - rolldown
  - tsdown
  - vite
  - eslint
  - prettier
  - npm
  - pnpm
  - yarn
  - type-safe
  - type safety
  - tsconfig
  - strict mode
  - branded types
---

# TypeScript Language Expertise

## Overview

Type-safe, production-quality TypeScript: the type system's sharp edges, strict-mode config that actually moves the needle, the type-vs-runtime boundary, and framework patterns (zod/tRPC/Prisma/React/Express). Assumes fluency with JS and basic TS — this is reference for the traps and idioms that bite experienced engineers.

Toolchain assumed throughout: **Bun** (install/run/test), **Vite** (app dev server and bundler — Rolldown bundling, Oxc transforms and minifier, Lightning CSS), **tsdown** (library bundler on Rolldown), **oxlint** (lint), **oxfmt** (format), `tsc --noEmit` (type-check). ESLint, Prettier, esbuild, and Babel are the legacy equivalents — do not add them to a project that has this stack. See **Toolchain** below.

## Type System Essentials

### Generics

```typescript
// Constraints + default type param; `this` return for fluent chaining
interface HasId { id: string }
class Repo<T extends HasId = HasId> {
  private items = new Map<string, T>();
  save(item: T): this { this.items.set(item.id, item); return this; }
  find(id: string): T | undefined { return this.items.get(id); }
}

// Return `as const` to preserve a tuple/literal shape instead of widening
function pair<A, B>(a: A, b: B) { return [a, b] as const; } // readonly [A, B]
```

### Utility types

| Utility                       | Result                                        |
| ----------------------------- | --------------------------------------------- |
| `Partial<T>` / `Required<T>`  | all props optional / required                 |
| `Readonly<T>`                 | all props `readonly` (shallow)                |
| `Pick<T,K>` / `Omit<T,K>`     | keep / drop keys `K`                          |
| `Record<K,V>`                 | object with keys `K`, values `V`              |
| `Extract<U,V>` / `Exclude<U,V>` | keep / drop union members of `U` assignable to `V` |
| `NonNullable<T>`              | strip `null` / `undefined`                    |
| `ReturnType<F>` / `Parameters<F>` | function return type / param tuple        |
| `Awaited<T>`                  | recursively unwrap `Promise` (prefer over a hand-rolled `Unwrap`) |

```typescript
// Derive types from values so they can't drift:
function createUser(name: string, email: string): User { /* … */ }
type NewUser = ReturnType<typeof createUser>;   // User
type NewUserArgs = Parameters<typeof createUser>; // [string, string]
```

### Conditional types & `infer`

```typescript
type Elem<T> = T extends (infer U)[] ? U : never;
type Ret<T>  = T extends (...a: any[]) => infer R ? R : never;

// Conditionals DISTRIBUTE over naked union type params:
type ToArray<T> = T extends any ? T[] : never;
type A = ToArray<string | number>;   // string[] | number[]

// Wrap both sides in a 1-tuple to DISABLE distribution:
type ToArray1<T> = [T] extends [any] ? T[] : never;
type B = ToArray1<string | number>;  // (string | number)[]
```

### Mapped types

```typescript
type Mutable<T>  = { -readonly [K in keyof T]: T[K] };
type Optional<T> = { [K in keyof T]+?: T[K] };

// Key remapping via `as` (TS 4.1+): rename or filter keys
type Getters<T> = { [K in keyof T as `get${Capitalize<string & K>}`]: () => T[K] };
type PickByValue<T, V> = { [K in keyof T as T[K] extends V ? K : never]: T[K] };
// PickByValue<{a:string;b:number}, string> → { a: string }
```

### Discriminated unions & exhaustiveness

Model each valid state as a variant with a shared literal discriminant; `switch` on it narrows each arm, and an `assertNever` default turns "added a variant, forgot a case" into a compile error.

```typescript
type State =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "success"; data: User[] }
  | { status: "error"; error: Error };

function assertNever(x: never): never { throw new Error(`Unreachable: ${JSON.stringify(x)}`); }

function render(s: State): string {
  switch (s.status) {
    case "idle":    return "Click to load";
    case "loading": return "Loading…";
    case "success": return `Loaded ${s.data.length}`;   // s narrowed → data exists
    case "error":   return s.error.message;             // s narrowed → error exists
    default:        return assertNever(s);              // ← new unhandled variant = type error
  }
}
```

⚠ Exhaustiveness relies on a *finite* discriminant. A **numeric enum** discriminant accepts any `number`, so `assertNever` won't catch a missing case — use string-literal unions or `as const` objects (see Anti-Patterns).

### Type guards, assertion functions, `unknown` vs `any`

- `any` disables checking **transitively** — it silently poisons every expression it flows into. `unknown` is the safe top type: assignable *from* anything, assignable *to* nothing until you narrow.
- Type external inputs (`JSON.parse`, `fetch().json()`, `catch` vars, `process.env` shapes) as `unknown` and narrow with `typeof` / `instanceof` / `in` / a validator before use.

```typescript
function isCat(a: Cat | Dog): a is Cat { return "meow" in a; }

// Assertion function: narrows for the rest of the scope, throws otherwise
function assertNonNull<T>(v: T | null | undefined, msg?: string): asserts v is T {
  if (v == null) throw new Error(msg ?? "value is null/undefined");
}
```

⚠ An explicit `x is T` predicate is **trusted, not verified** by the compiler — a wrong body is as unsafe as `as`. Prefer letting TS *infer* the predicate (TS 5.5+, see Gotchas); reserve explicit `is` for what inference can't express, and unit-test those.

## tsconfig & Strict Mode

`module`/`moduleResolution` must match where the output runs — there is no universal template. Pick ONE of the two below; never copy a `nodenext` config into a bundler app or vice versa.

### Node app or published library — `module: nodenext`

Requires explicit `.js` extensions on relative imports plus `"type": "module"` in `package.json` (or `.mts`/`.cts`). `nodenext` implies a matching `lib`/`target`, so an explicit `"lib"` is redundant here.

```json
{
  "compilerOptions": {
    "module": "nodenext",
    "verbatimModuleSyntax": true,
    "outDir": "./dist",
    "rootDir": "./src",
    "declaration": true,
    "declarationMap": true,
    "sourceMap": true,

    "strict": true,
    "noUncheckedIndexedAccess": true,
    "exactOptionalPropertyTypes": true,
    "noImplicitReturns": true,
    "noFallthroughCasesInSwitch": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,

    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "isolatedModules": true
  },
  "include": ["src/**/*"],
  "exclude": ["node_modules", "dist"]
}
```

### Bundler app (Vite / Rolldown)

Extensionless relative imports work; the bundler emits, so `tsc` only type-checks (`noEmit`). ⚠ Never use `moduleResolution: bundler` for a **published library** — it's "infectious": emitted `.d.ts` files carry extensionless relative imports that break Node.js ESM consumers. Vite transpiles TS with Oxc, which strips types without checking them — `tsc --noEmit` stays the type gate (`oxlint --type-aware` covers the lint half, not the compiler's).

```json
{
  "compilerOptions": {
    "module": "esnext",
    "moduleResolution": "bundler",
    "verbatimModuleSyntax": true,
    "noEmit": true,

    "strict": true,
    "noUncheckedIndexedAccess": true,
    "exactOptionalPropertyTypes": true,
    "noImplicitReturns": true,
    "noFallthroughCasesInSwitch": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,

    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "isolatedModules": true
  },
  "include": ["src/**/*"]
}
```

### Flags beyond `strict`

`strict: true` does NOT include these — opt in explicitly (TS 5.9 `tsc --init` now enables them for new projects):

- **`noUncheckedIndexedAccess`** — adds `| undefined` to array subscripts and index-signature access (`arr[i]: T | undefined`). ⚠ NOT applied to named properties, NOT to `for...of` loop variables (by design, microsoft/TypeScript#42622), NOT to `Object.values()` — so it's no safety net when iterating.
- **`exactOptionalPropertyTypes`** — `obj.x = undefined` becomes an error for `x?: "a" | "b"`; only *deleting* the key makes the property absent. Matters for `"x" in obj` checks and serialization round-trips.
- **`useUnknownInCatchVariables`** (on via `strict` since 4.4) — `catch` vars are `unknown`; guard with `instanceof Error`.
- **`verbatimModuleSyntax`** — modern module-safety baseline; supersedes the now-no-op `importsNotUsedAsValues`/`preserveValueImports` (see Modules).
- **`isolatedModules`** — forbids constructs single-file transpilers can't handle (re-exporting a type without `type`, `const enum` inlining); required for single-file transpilers — Oxc (Vite, tsdown), Bun's transpiler, esbuild, swc, Babel.

### Checklist: tsconfig

- [ ] Exactly one of `nodenext` / `bundler` chosen to match the runtime — not mixed
- [ ] `strict: true` PLUS `noUncheckedIndexedAccess` + `exactOptionalPropertyTypes` (they aren't in `strict`)
- [ ] `verbatimModuleSyntax: true`; type-only imports carry the `type` modifier
- [ ] Library build: `declaration: true`, NOT `moduleResolution: bundler`, no `const enum` in public `.d.ts`
- [ ] `isolatedModules: true` if any non-`tsc` transpiler is in the build

## Toolchain: Bun + Vite + Oxc

One stack, no overlap: **Bun** installs, runs scripts, and runs `bun test`; **Vite** builds apps (Rolldown bundler, Oxc transform + minifier, Lightning CSS); **tsdown** builds libraries (Rolldown); **oxlint** lints; **oxfmt** formats; `tsc --noEmit` type-checks. Oxc replaces ESLint, Prettier, esbuild, and Babel — never add those to a project that has this stack, and never run two formatters over the same files; a second linter is acceptable only as a temporary migration bridge (see oxlint below).

```bash
bun add -D typescript oxlint oxfmt
bunx oxlint --init            # writes .oxlintrc.json
bunx oxfmt --init             # writes .oxfmtrc.json (--migrate=prettier converts an existing Prettier config)
bun add -D oxlint-tsgolint    # needed by "typeAware": true in the config below; drop both to skip type-aware rules
```

`package.json` scripts:

```json
{
  "scripts": {
    "typecheck": "tsc --noEmit",
    "lint": "oxlint --deny-warnings",
    "lint:fix": "oxlint --fix",
    "format": "oxfmt",
    "format:check": "oxfmt --check",
    "check": "bun run typecheck && bun run lint && bun run format:check"
  }
}
```

### oxlint

- Config is `.oxlintrc.json`. Rules are grouped in categories; only `correctness` is on by default — opt into `suspicious`, `perf`, `pedantic` per project rather than `all`.
- Setting `plugins` REPLACES the default set (`eslint`, `typescript`, `unicorn`, `oxc`) — list the defaults you keep alongside the ones you add (`import`, `promise`, `vitest`, `react`, `jsx-a11y`, ...).
- `oxlint --fix` applies only safe fixes; `--fix-suggestions` and `--fix-dangerously` can change behavior — review the diff.
- Type-aware rules (`typescript/no-floating-promises`, `typescript/no-unsafe-assignment`, ...) need `oxlint-tsgolint` plus `"options": { "typeAware": true }` (or `oxlint --type-aware`). They run on the native TypeScript compiler (tsgo), so the tsconfig must be tsgo-clean — no `baseUrl` — and `include` should stay tight or the pass gets slow.
- Migrating from ESLint: `bunx -p @oxlint/migrate oxlint-migrate` converts a flat config to `.oxlintrc.json`. If a rule oxlint lacks must stay, keep ESLint for that rule only, add `eslint-plugin-oxlint` to turn off everything oxlint already covers, and run `oxlint && eslint` so the fast pass fails first.

```json
{
  "$schema": "./node_modules/oxlint/configuration_schema.json",
  "plugins": ["eslint", "typescript", "unicorn", "oxc", "import", "promise"],
  "categories": { "correctness": "error", "suspicious": "warn" },
  "rules": {
    "typescript/no-explicit-any": "error",
    "typescript/explicit-function-return-type": "error",
    "typescript/method-signature-style": ["error", "property"],
    "import/no-cycle": "error"
  },
  "options": { "typeAware": true },
  "ignorePatterns": ["dist/**", "coverage/**"]
}
```

### oxfmt

- Prettier-compatible output, Prettier-style CLI: `oxfmt` writes in place, `oxfmt --check` for CI. Config is `.oxfmtrc.json`; it honors `.gitignore` and `.prettierignore` by default and formats TS/JS/JSX/TSX, JSON, CSS, Markdown, YAML and more.
- Default `printWidth` is 100 (Prettier's is 80). Matching an existing Prettier codebase: generate the config with `oxfmt --migrate=prettier` or set `printWidth` explicitly, then land one formatting-only commit.
- `sortPackageJson` is on by default; `sortImports` is opt-in. Drop Prettier and its ESLint bridges (`eslint-config-prettier`, `eslint-plugin-prettier`) — oxlint and oxfmt do not overlap under the default categories (`style` is off), so nothing needs silencing.

### tsdown for libraries

Bundle published packages with `tsdown` (Rolldown under the hood): `tsdown.config.ts` with `defineConfig({ entry: ["src/index.ts"], format: ["esm"], dts: true })`, `nodenext` resolution in tsconfig (see above), and `isolatedDeclarations: true` so `.d.ts` emit is per-file (see Performance). It replaces tsup/rollup/esbuild-based library builds.

### Bun as runtime

`bun run src/x.ts` and `bun test` transpile TypeScript themselves — no build step, no `ts-node`/`tsx` loader. Bun strips types without checking them, exactly like Vite, so `tsc --noEmit` is still the gate. Use `bun test` for Node/Bun-target code and Vitest (which runs on Vite's Oxc transform) for browser-target code.

## Modules & Declaration Merging

### Type-only imports

Under `verbatimModuleSyntax`, an import/export WITHOUT a `type` modifier is emitted verbatim; anything WITH `type` is erased. A purely-type import missing `type` ships as a runtime `import`/`require` — defeating tree-shaking and dragging CJS/ESM side effects in.

```typescript
import type { User, Order } from "./models";        // fully erased
import { type UserDTO, createUser } from "./user";   // UserDTO erased, createUser kept
export { Order, type OrderDTO } from "./order";      // OrderDTO erased
```

### Declaration merging

Interfaces (unlike `type` aliases) merge across declarations. Deliberate uses: augmenting third-party/global types; pairing an `interface` with a `namespace` or `class` of the same name.

```typescript
// Augment a library type (must be inside its module scope or a `declare module`):
declare global {
  namespace Express { interface Request { user?: User; requestId: string } }
}
declare module "untyped-pkg" { export function doThing(v: string): void }
export {}; // makes an ambient file a module
```

⚠ The flip side is a footgun: two same-named `interface`s in one scope **merge silently** — often an accidental collision. Use a `type` alias for shapes you don't want merged (a duplicate `type` is a hard error, which is what you want).

## Runtime Validation Boundary

Types are **erased at runtime** — they cannot validate data crossing a trust boundary (HTTP body, JSON, env, DB rows). Parse with a schema validator at the edge; inside the boundary, trust the types. Never bridge the boundary with `as`.

### Zod

```typescript
import { z } from "zod";

const UserSchema = z.object({
  id: z.string().uuid(),
  email: z.string().email(),
  role: z.enum(["admin", "user", "guest"]).default("user"),
  createdAt: z.coerce.date(),          // string → Date at parse time
});

type User = z.infer<typeof UserSchema>;   // OUTPUT type (post default/transform/coerce)
type UserIn = z.input<typeof UserSchema>; // INPUT type (what .parse ACCEPTS)

UserSchema.parse(input);      // throws ZodError
UserSchema.safeParse(input);  // { success: true; data } | { success: false; error }

// Compose instead of redefining:
UserSchema.partial();               // all optional
UserSchema.pick({ email: true });
UserSchema.omit({ createdAt: true });
UserSchema.extend({ age: z.number().int().positive() });

// Cross-field validation:
z.object({ pw: z.string().min(8), confirm: z.string() })
  .refine((d) => d.pw === d.confirm, { message: "mismatch", path: ["confirm"] });
```

⚠ `z.infer` is the **output** type. Whenever a schema uses `.default()`, `.transform()`, or `.coerce`, input ≠ output — annotate parse *inputs* with `z.input` and results with `z.infer`. Passing an `z.infer` value where `z.input` is expected is a common, silent shape bug.

### tRPC

```typescript
const t = initTRPC.context<Context>().create();

const appRouter = t.router({
  getUser: t.procedure
    .input(z.object({ id: z.string().uuid() }))          // runtime validation AND static input type
    .query(({ input, ctx }) => ctx.db.user.find(input.id)),
});
export type AppRouter = typeof appRouter;   // the ONLY thing the client imports

// Client — I/O types flow across the wire from the exported TYPE, no codegen:
import type { AppRouter } from "./server";
const client = createTRPCClient<AppRouter>({ url });
await client.getUser.query({ id: "…" });    // fully typed, autocompleted
```

The client imports the type only (`import type`), so no server code ships to the browser; the `.input()` schema doubles as runtime guard and static contract.

### Prisma

```typescript
const user = await prisma.user.findUnique({
  where: { id },
  include: { posts: { where: { published: true }, take: 10 } },
}); // return type NARROWS to User & { posts: Post[] } from the include

// Name a query shape instead of hand-writing the joined type:
type UserWithPosts = Prisma.UserGetPayload<{ include: { posts: true } }>;

await prisma.$transaction(async (tx) => { /* all-or-nothing */ });
```

`select`/`include` reshape the *result type*, not just the query — `select` prunes fields from the returned type. Use `Prisma.<Model>GetPayload<…>` to derive a shape rather than duplicating it by hand.

## Patterns

### Branded (nominal) types

TS is structural, so `UserId` and `OrderId` (both `string`) are interchangeable unless you brand them. Validate in the constructor; the brand makes mix-ups a compile error.

```typescript
declare const brand: unique symbol;
type Brand<T, B> = T & { readonly [brand]: B };

type UserId = Brand<string, "UserId">;
type Email  = Brand<string, "Email">;

function toEmail(s: string): Email {
  if (!s.includes("@")) throw new Error("invalid email");
  return s.toLowerCase() as Email;   // the ONE sanctioned cast, gated by validation
}
// getUser(orderId) // ← type error: OrderId not assignable to UserId
```

### Result type & async error handling

```typescript
type Result<T, E = Error> = { ok: true; value: T } | { ok: false; error: E };

async function fetchUser(id: string): Promise<Result<User>> {
  try {
    const res = await fetch(`/api/users/${id}`);
    if (!res.ok) return { ok: false, error: new Error(`HTTP ${res.status}`) };
    return { ok: true, value: (await res.json()) as User };
  } catch (error) {
    // `error` is `unknown` (useUnknownInCatchVariables). Normalize — never `error as Error`,
    // which yields an object whose `.message` is undefined when a non-Error is thrown.
    return { ok: false, error: error instanceof Error ? error : new Error(String(error)) };
  }
}

async function retry<T>(fn: () => Promise<T>, attempts: number, delayMs: number): Promise<T> {
  // `unknown` + sentinel: avoids the unsafe cast AND the uninitialized hazard if attempts <= 0
  let lastError: unknown = new Error("retry: no attempts made");
  for (let i = 0; i < attempts; i++) {
    try { return await fn(); }
    catch (e) {
      lastError = e instanceof Error ? e : new Error(String(e));
      if (i < attempts - 1) await new Promise((r) => setTimeout(r, delayMs * 2 ** i));
    }
  }
  throw lastError;
}
```

## React + TypeScript

```typescript
import { type ReactNode, type ComponentPropsWithoutRef, createContext, useContext, useState, useRef } from "react";

// Extend native element props instead of re-declaring them:
interface InputProps extends ComponentPropsWithoutRef<"input"> {
  label: string;
  error?: string;
}

// Generic component — T is inferred from `items`:
function List<T>({ items, render, keyOf }: {
  items: T[]; render: (t: T) => ReactNode; keyOf: (t: T) => string | number;
}) {
  return <ul>{items.map((it) => <li key={keyOf(it)}>{render(it)}</li>)}</ul>;
}

// Custom hooks: return `as const` so the tuple keeps positional types
function useToggle(init = false) {
  const [on, setOn] = useState(init);
  return [on, () => setOn((v) => !v)] as const; // [boolean, () => void], not (boolean | (() => void))[]
}

const ref = useRef<HTMLVideoElement>(null); // ref.current: HTMLVideoElement | null

// Context typed `T | undefined`; guard in the hook so consumers get a non-null value
const Ctx = createContext<AuthValue | undefined>(undefined);
function useAuth(): AuthValue {
  const c = useContext(Ctx);
  if (!c) throw new Error("useAuth must be used within AuthProvider");
  return c;
}
```

- ⚠ Avoid `React.FC`: it doesn't support generic components and its `children` semantics shifted in React 18 types. Annotate the props object directly; add `children: ReactNode` only when the component renders children.
- Polymorphic `as` prop — the one genuinely tricky component type:

```typescript
type Poly<C extends React.ElementType, P = {}> =
  P & { as?: C } & Omit<React.ComponentPropsWithoutRef<C>, keyof P | "as">;

function Text<C extends React.ElementType = "span">({ as, ...rest }: Poly<C, { size?: "sm" | "lg" }>) {
  const Tag = as ?? "span";
  return <Tag {...rest} />; // <Text as="a" href="…"/> type-checks href
}
```

## Node.js + Express

```typescript
import type { Request, Response, NextFunction } from "express";
import { z } from "zod";

// Augment Request via declaration merging (see Declaration merging):
declare global {
  namespace Express { interface Request { user?: User } }
}

// One validation-middleware factory, reused per route:
const validate = <T>(schema: z.ZodSchema<T>) =>
  (req: Request, res: Response, next: NextFunction) => {
    const r = schema.safeParse(req.body);
    if (!r.success) return res.status(400).json({ errors: r.error.issues });
    req.body = r.data;   // now typed T for downstream handlers
    next();
  };
app.post("/users", validate(CreateUserSchema), createUserHandler);

// Model API failures as a discriminated union, mapped to status codes centrally:
type ApiError =
  | { type: "validation"; error: z.ZodError }
  | { type: "not_found"; resource: string }
  | { type: "unauthorized"; message: string };
```

## Common Anti-Patterns

```typescript
// ❌ any bypasses checking      →  ✅ unknown + narrow
function bad(d: any) { return d.a.b.c; }
function good(d: unknown): string { if (isValid(d)) return d.a.b.c; throw new Error("invalid"); }

// ❌ assert across the runtime boundary  →  ✅ validate (zod), then trust the type
const u1 = JSON.parse(input) as User;      // lies if the shape is wrong
const u2 = UserSchema.parse(JSON.parse(input));

// ❌ non-null assertion abuse    →  ✅ handle / throw explicitly
const a = users.find((u) => u.id === id)!;               // crashes silently if missing
const b = users.find((u) => u.id === id) ?? throwMissing(id);

// ❌ overly-permissive object    →  ✅ generics preserve the type
function mergeBad(a: object, b: object): object { return { ...a, ...b }; }
function mergeGood<T extends object, U extends object>(a: T, b: U): T & U { return { ...a, ...b }; }

// ❌ numeric enum — `let s: Status = 999` compiles (assignment hole); also errors under
//    --erasableSyntaxOnly and can't be stripped by Node's native TS (22.18+).
enum Status { Pending, Active }
// ✅ const object + union — zero runtime cost, no hole, strips cleanly, keeps exhaustiveness
const Status = { Pending: "pending", Active: "active" } as const;
type Status = (typeof Status)[keyof typeof Status];

// ❌ async callback in forEach (fire-and-forget, unhandled rejections)
items.forEach(async (i) => { await save(i); });
// ✅ await the whole batch
await Promise.all(items.map((i) => save(i)));

// ❌ boolean-flag state permits impossible combos (loading && error && data)
type S1 = { isLoading: boolean; data?: User[]; error?: Error };
// ✅ discriminated union: only valid states are representable
type S2 = { status: "loading" } | { status: "success"; data: User[] } | { status: "error"; error: Error };
```

## Expert Practices: Idioms, Anti-Patterns & Gotchas

### Idioms

#### `satisfies` — validate without widening

`satisfies` (TS 4.9+) checks that an expression is assignable to a target type **without** replacing the expression's inferred type with that target, so downstream code keeps the narrowest per-property/literal types while still catching wrong shapes and typo'd keys. It resolves the dilemma between a `: Type` annotation (validates but WIDENS, losing literal/tuple precision) and an `as Type` assertion (preserves nothing and SUPPRESSES mismatches, so a misspelled key slips through). Mechanism: TypeScript verifies assignability to the target but records the original expression type for inference. Constraint: it applies only at an expression/initializer site — it is not a statement you can retroactively apply to an already-declared variable.

```typescript
type Colors = "red" | "green" | "blue";
type RGB = [number, number, number];

const palette = {
  red: [255, 0, 0],
  green: "#00ff00",
  // bleu: [0, 0, 255]  // ← Error: 'bleu' is not in Record<Colors, ...>
} satisfies Record<Colors, string | RGB>;

palette.green.toUpperCase(); // OK — still narrowed to string
palette.red.at(0); // OK — still narrowed to [number, number, number]

// An `as` assertion would suppress everything — a typo'd key compiles silently:
// const p = { red: [255,0,0], bleu: [0,0,255] } as Record<Colors, string | RGB>;
```

#### `NoInfer<T>` — mark a parameter validate-only

`NoInfer<T>` (TS 5.4+) tells TypeScript not to use a parameter as an inference candidate for a type variable, while still validating it against the `T` inferred from the principal parameters. Without it, every `T`-typed parameter contributes inference candidates, so a default value or callback can silently expand what `T` resolves to and accept out-of-range values. Use it when one parameter is the authoritative source of truth.

```typescript
function createStreetLight<C extends string>(
  colors: C[],
  defaultColor?: NoInfer<C>, // validated against C, never widens it
) {}

createStreetLight(["red", "yellow", "green"], "blue");
// Error: '"blue"' is not assignable to '"red" | "yellow" | "green" | undefined'
// Without NoInfer, 'blue' would be absorbed into C and accepted.
```

#### `const` type parameters — keep the `readonly` constraint

The `const` modifier on a type parameter (TS 5.0+) makes inline literal arguments infer const-like (literal/tuple) types, so callers no longer need `as const`. The silent trap: if the constraint is **mutable** (`T extends string[]`), the const-inferred candidate `readonly ['a','b']` is not assignable to it, so inference falls back to the widened mutable type with NO warning. Always use a `readonly` constraint. The modifier also affects only literals written directly at the call site — passing a pre-declared variable (already inferred as `string[]`) sees no benefit.

```typescript
declare function tags<const T extends readonly string[]>(args: T): T;
const t = tags(["a", "b"]); // readonly ['a', 'b']

// With `T extends string[]` (mutable), tags(['a','b']) silently widens to string[].
```

#### `verbatimModuleSyntax` and precise type-only imports

`verbatimModuleSyntax` (TS 5.0+) replaces the deprecated, now-no-op `importsNotUsedAsValues`/`preserveValueImports` with one rule: imports/exports WITHOUT a `type` modifier are emitted verbatim; anything WITH `type` is erased. So every purely-type import must be `import type { ... }` or use an inline `type` specifier — otherwise it is emitted as a runtime import even when unused, defeating tree-shaking, forcing unwanted CJS/ESM `require()` inclusion, and breaking cross-compiler consistency (Oxc/esbuild/swc/Babel all strip `type`-marked imports reliably). It is in TS 5.9's `tsc --init` defaults. (See Modules above.)

#### `using` / `await using` — deterministic cleanup (TS 5.2)

Explicit Resource Management: any object implementing `Symbol.dispose` can be declared with `using`, and TypeScript guarantees `dispose` runs on scope exit — including early returns and exceptions — in last-in-first-out order. `await using` calls and awaits `Symbol.asyncDispose`. This replaces error-prone `try/finally` cleanup for DB connections, file handles, timers, and test fixtures. Requires `lib` to include `esnext.disposable`; some runtimes need a `Symbol.dispose` polyfill.

```typescript
class DbConnection implements Disposable {
  constructor(private conn: Connection) {}
  [Symbol.dispose]() {
    this.conn.close();
  }
}
async function processRecords() {
  using db = new DbConnection(openConnection());
  return await db.conn.query("SELECT * FROM records"); // close() runs on every exit path
}
// tsconfig: { "lib": ["es2022", "esnext.disposable"] }
```

### Anti-Patterns

#### Explicit `x is T` predicates are trusted unconditionally — as unsafe as `as`

When you annotate a guard's return type as `x is T`, TypeScript does NOT verify the body actually narrows `x` to `T` — it trusts the assertion, making an explicit predicate semantically equivalent to a type assertion. A wrong or incomplete predicate compiles silently and causes runtime type confusion. Prefer letting TypeScript INFER the predicate from a simple narrowing body (TS 5.5+), because then the compiler derives it from the implementation. Reserve explicit `is` for cases inference cannot handle (multiple return paths, deep structural validation) — write them thoroughly and unit-test them.

```typescript
// Inferred & validated by the compiler:
const isString = (x: unknown) => typeof x === "string"; // inferred: x is string

// Compiles fine but is a lie — TypeScript never checks the body:
function isPositive(n: number): n is 1 | 2 | 3 {
  return n > 0; // also true for 4, 5, … → downstream runtime crash
}
```

#### Never publish `const enum` in a `.d.ts`

A published `const enum` is inlined into consumers' bundles at compile time. If a later patch changes member values, consumers keep the OLD inlined values while running the NEW library — a silent wrong-branch bug. It is also incompatible with `isolatedModules` and single-file transpilers (Oxc, Babel, esbuild, swc), which cannot inline cross-file values. For published APIs use a regular `enum`, an `as const` object, or `preserveConstEnums` to strip the `const` from declaration output.

```typescript
// Safe in a published package — value exists at runtime, no inlining:
const Direction = { Up: "UP", Down: "DOWN" } as const;
type Direction = (typeof Direction)[keyof typeof Direction];
```

### Gotchas

#### TS 5.5 inferred type predicates — but truthiness and `.filter(Boolean)` do NOT narrow

TS 5.5 infers a type predicate for a function with no explicit return annotation, a single return statement, no parameter mutation, and a boolean expression tied to a refinement of the parameter — so `arr.filter(x => x !== undefined)` finally returns `T[]`. The trap: truthiness checks (`x => !!x`, `x => x`) and `.filter(Boolean)` do NOT infer a predicate. Reason — the "if and only if" rule: `!!score` being false could mean `undefined` OR the valid value `0`, so `score is number` would be unsound; `Boolean` is also not itself recognized as a predicate. The result is doubly bad: the type stays `(T | undefined)[]` AND zero/empty-string values are silently dropped at runtime. Use explicit comparisons or a named guard.

```typescript
// Inferred predicate → Bird[]
const birds = countries.map((c) => birdMap.get(c)).filter((b) => b !== undefined);

// Reusable named guard for .filter(Boolean) situations:
function isDefined<T>(x: T | null | undefined): x is NonNullable<T> {
  return x != null;
}
const defined = countries.map((c) => birdMap.get(c)).filter(isDefined); // Bird[]

// BAD: type stays (number | undefined)[] AND zero scores are dropped:
// students.map(s => scoreMap.get(s)).filter(score => !!score);
```

#### `useUnknownInCatchVariables` types `catch` as `unknown` (on via `strict` since 4.4)

Catch-clause variables are `unknown`, not `any`, so touching `.message`/`.stack` without a guard fails to compile — and the break rides in silently via `strict` on upgrade. Use an `instanceof Error` guard; `error as Error` restores the old unsafe behavior and is only a temporary migration crutch (see the `fetchUser`/`retry` examples above).

```typescript
try {
  await riskyOperation();
} catch (err) {
  if (err instanceof Error) console.error(err.message);
  else console.error("Unknown error:", String(err));
}
```

#### Excess-property checking only fires on FRESH object literals

The "may only specify known properties" error fires only when a literal is assigned DIRECTLY to a typed target or passed DIRECTLY as an argument. Assigning the same literal to an intermediate variable first — even one with an explicit type annotation — strips its freshness, and structural typing then allows the extra properties silently. Refactoring a direct literal into a named variable "for readability" can suppress a real bug the compiler was catching.

```typescript
interface Duck {
  quack(): void;
}
const d: Duck = { quack() {}, woof() {} }; // Error: 'woof' is excess on a fresh literal

const obj = { quack() {}, woof() {} }; // freshness lost
const d2: Duck = obj; // No error — extra 'woof' silently allowed
```

#### Control-flow narrowing is discarded inside closures — copy to a `const`

TypeScript drops a variable's narrowing when it is captured by a closure, even if unconditionally assigned beforehand, because the captured binding could be reassigned between narrowing and execution (acknowledged design limitation, microsoft/TypeScript#37802). Copy the narrowed value into a fresh `const` so the closure captures an immutable binding.

```typescript
function deferred(value?: string): () => string {
  if (value == null) value = "";
  const v = value; // const captures the narrowed type
  return () => v; // v is string — returning () => value would widen to string | undefined
}
```

#### Method-shorthand syntax is checked bivariantly — `strictFunctionTypes` does not catch it

`strictFunctionTypes` enforces contravariant parameter checking for function-TYPED properties (`m: (x: T) => void`), but the docs explicitly exempt parameters of methods declared in shorthand syntax (`m(x: T): void`) — these stay BIVARIANT. The exemption lets `Array<T>` relate covariantly, but in user-defined interfaces it is a real soundness hole. oxlint's `typescript/method-signature-style` set to `"property"` forces property syntax to close the gap.

```typescript
interface Processor {
  process: (value: string | number) => void; // property syntax → contravariant
}
const p: Processor = {
  process: (value: string) => console.log(value), // Error: string not assignable to string | number
};
// With method shorthand `process(value: string | number): void`, the same
// assignment compiles — and crashes at runtime if called with a number.
```

#### Template-literal types produce Cartesian products

A template-literal type interpolating multiple unions expands to the full Cartesian product: unions of size N and M yield N*M members, growing multiplicatively and becoming a real compile-time cost for large schemas. The bounded event-name pattern is the canonical good use; for large route maps or i18n keys prefer code generation (`tsc --generateTrace` surfaces the cost). Note the intrinsic `Uppercase`/`Lowercase`/`Capitalize` types use raw JS `toUpperCase`/`toLowerCase` — they are NOT locale-aware.

```typescript
type PropEventSource<T> = {
  on<K extends string & keyof T>(event: `${K}Changed`, cb: (v: T[K]) => void): void;
};
// AVOID: `${Methods} ${Routes}` over 5 × 50 unions → 250 members; prefer codegen.
```

### Performance

#### Prefer `interface extends` over type intersection for object types

`interface Foo extends Bar, Baz` produces a single flat object type whose relationships the compiler caches, whereas `type Foo = Bar & Baz` forces a recursive merge of constituents on every comparison at each use site. Effects: faster type-checking / better language-server responsiveness in large codebases; conflicting properties are reported eagerly at the declaration instead of silently collapsing to `never` at use sites; cleaner IDE hover. The TS Performance wiki names this a high-impact optimization. Intersections remain necessary for composing non-object types (unions, primitives, mapped/conditional results).

```typescript
interface AdminUser extends BaseUser, AdminPermissions {
  adminLevel: number;
}
// type AdminUser = BaseUser & AdminPermissions & { adminLevel: number }; ← recomputed per comparison
```

#### `isolatedDeclarations` unlocks parallel `.d.ts` emit

`isolatedDeclarations` (TS 5.5+) requires explicit type annotations on all exported symbols so each file's `.d.ts` can be generated independently, without a whole-program type-checker pass — letting tools (Oxc, esbuild, swc) emit declarations in parallel and removing the monorepo serialization bottleneck. Requires `declaration` or `composite`. Tradeoff: explicit return types on exported functions become mandatory; it pays off most when you already enforce `typescript/explicit-function-return-type` in oxlint, and it is what lets tsdown emit `.d.ts` per file instead of running a whole-program pass.

```typescript
// isolatedDeclarations: true (with declaration: true)
export function computeTotal(items: Item[]): number {
  return items.reduce((sum, i) => sum + i.price, 0);
}
```

### Currency

#### Import attributes: `with { type: 'json' }`, not `assert`

Import assertions using the withdrawn `assert` keyword were superseded by import attributes using `with` (ES2025). Under `--module nodenext`, TS 5.8 makes `assert` a hard error (matching Node.js 22+), and TS 5.7 already required `with` for validated JSON imports under nodenext. Migrate all `assert { type: 'json' }` to `with { type: 'json' }`.

```typescript
import config from "./config.json" with { type: "json" };
```

#### Compiler migrations: inspect the installed release notes

Compiler defaults and deprecated module-resolution modes change across TypeScript releases. Before changing `target`, `moduleResolution`, `baseUrl`, or path aliases, inspect the release notes for the **installed and target** compiler, then test the emitted package from a consumer. Prefer standards-based package `exports`/`imports` maps for new Node aliases, but do not mechanically replace an established bundler alias without confirming runtime and test-runner support.

```json
{ "imports": { "#utils/*": "./src/utils/*.js", "#models/*": "./src/models/*.js" } }
```

## Checklist: type-safety review before done

- [ ] No `any` in changed code (search it); external inputs typed `unknown` and narrowed
- [ ] Runtime boundaries (HTTP/JSON/env/DB) validated with a schema, not `as` — parse inputs typed `z.input`, results `z.infer`
- [ ] Every `switch` over a union ends in `assertNever(x)`; discriminants are string-literal unions, not numeric enums
- [ ] `catch` variables guarded with `instanceof Error` (not `error as Error`)
- [ ] No non-null `!` on lookups that can miss; optional chaining / explicit throw instead
- [ ] Public API types use `interface extends` (not `&`); no `const enum` in shipped `.d.ts`
- [ ] Type guards prefer inferred predicates; explicit `x is T` guards are unit-tested
- [ ] `tsc --noEmit` clean under `strict` + `noUncheckedIndexedAccess` + `exactOptionalPropertyTypes`
- [ ] `oxlint --deny-warnings` and `oxfmt --check` clean; no ESLint/Prettier/esbuild/Babel added to a project on the Bun + Vite + Oxc stack
