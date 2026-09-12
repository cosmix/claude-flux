import { useCallback, useSyncExternalStore } from "react";

function getServerSnapshot() {
  return false;
}

function matches(query: string): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia(query).matches;
}

/// Tracks a CSS media query in React state; the phone layout is a `useMediaQuery`
/// call, not a component split, so both renderings share every hook above it.
/// The subscribe function is memoized per `query` string — `useSyncExternalStore`
/// resubscribes whenever its identity changes, so an inline closure here would
/// tear down and rebuild the `MediaQueryList` listener on every render.
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
        return () => {};
      }
      const mql = window.matchMedia(query);
      mql.addEventListener("change", onChange);
      return () => mql.removeEventListener("change", onChange);
    },
    [query],
  );
  return useSyncExternalStore(subscribe, () => matches(query), getServerSnapshot);
}
