import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useMediaQuery } from "@/lib/use-media-query";

/// A minimal `MediaQueryList` stand-in: `matches` is mutated directly by a
/// test, then the stored "change" listener is invoked to simulate the
/// browser firing the event.
function fakeMediaQueryList(initial: boolean) {
  let listener: (() => void) | null = null;
  const mql = {
    matches: initial,
    addEventListener: vi.fn((type: string, cb: () => void) => {
      if (type === "change") listener = cb;
    }),
    removeEventListener: vi.fn(),
  };
  return {
    mql: mql as unknown as MediaQueryList,
    setMatches(value: boolean) {
      mql.matches = value;
    },
    fireChange() {
      listener?.();
    },
    getListener: () => listener,
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("useMediaQuery", () => {
  it("returns the initial matches value", () => {
    const { mql } = fakeMediaQueryList(true);
    vi.spyOn(window, "matchMedia").mockReturnValue(mql);

    const { result } = renderHook(() => useMediaQuery("(max-width: 699px)"));

    expect(result.current).toBe(true);
  });

  it("updates when the stored change listener fires", () => {
    const { mql, setMatches, fireChange } = fakeMediaQueryList(false);
    vi.spyOn(window, "matchMedia").mockReturnValue(mql);

    const { result } = renderHook(() => useMediaQuery("(max-width: 699px)"));
    expect(result.current).toBe(false);

    act(() => {
      setMatches(true);
      fireChange();
    });

    expect(result.current).toBe(true);
  });

  it("removes the same listener it added on unmount", () => {
    const { mql, getListener } = fakeMediaQueryList(false);
    vi.spyOn(window, "matchMedia").mockReturnValue(mql);

    const { unmount } = renderHook(() => useMediaQuery("(max-width: 699px)"));
    const listener = getListener();
    unmount();

    expect(mql.removeEventListener).toHaveBeenCalledWith("change", listener);
  });
});
