import { ChevronDownIcon } from "lucide-react";
import { useState } from "react";

import type { ConfigKind } from "@/api/config";

export interface ControlProps {
  id: string;
  kind: ConfigKind;
  /// The value the control shows: the in-flight one while a write is
  /// pending, otherwise the server's last known value at this scope.
  value: string;
  pending: boolean;
  invalid: boolean;
  describedBy: string;
  onCommit: (value: string) => void;
}

/// One control per wire `kind`; the key name never decides the widget.
export function ValueControl(props: ControlProps) {
  switch (props.kind.type) {
    case "bool":
      return <BoolSwitch {...props} />;
    case "u32":
      return <NumberField {...props} />;
    case "enum":
      return <EnumSelect {...props} variants={props.kind.variants} />;
  }
}

function BoolSwitch({ id, value, pending, invalid, describedBy, onCommit }: ControlProps) {
  return (
    <input
      id={id}
      type="checkbox"
      role="switch"
      className="settings-switch"
      checked={value === "true"}
      aria-checked={value === "true"}
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
function NumberField({ id, value, pending, invalid, describedBy, onCommit }: ControlProps) {
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
      className="settings-input"
      value={shown}
      data-draft={draft !== null || undefined}
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
        className="settings-input settings-select"
        value={value}
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
