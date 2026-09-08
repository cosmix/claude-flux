import "@testing-library/react";

// jsdom lacks the layout APIs React Flow (and the shadcn ScrollArea) read on
// mount; these stubs let the graph and panels render in tests.
if (!("ResizeObserver" in window)) {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: ResizeObserverStub,
  });
}
if (typeof window.matchMedia !== "function") {
  // The theme atoms read the OS colour scheme at import; motion hooks read
  // the reduced-motion preference.
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      dispatchEvent: () => false,
    }),
  });
}
if (!("DOMMatrixReadOnly" in window)) {
  class DOMMatrixReadOnlyStub {
    m22: number;
    constructor(transform?: string) {
      const scale = transform?.match(/scale\(([\d.]+)\)/)?.[1];
      this.m22 = scale === undefined ? 1 : Number(scale);
    }
  }
  Object.defineProperty(window, "DOMMatrixReadOnly", { value: DOMMatrixReadOnlyStub });
}
Object.defineProperties(window.HTMLElement.prototype, {
  offsetHeight: {
    configurable: true,
    get() {
      return Number.parseFloat((this as HTMLElement).style.height) || 1;
    },
  },
  offsetWidth: {
    configurable: true,
    get() {
      return Number.parseFloat((this as HTMLElement).style.width) || 1;
    },
  },
});
Object.defineProperty(window.SVGElement.prototype, "getBBox", {
  configurable: true,
  value: () => ({ x: 0, y: 0, width: 0, height: 0 }),
});
