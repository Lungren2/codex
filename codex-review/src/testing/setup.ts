class ObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

Object.defineProperty(globalThis, "ResizeObserver", { value: ObserverStub, configurable: true });
Object.defineProperty(globalThis, "IntersectionObserver", { value: ObserverStub, configurable: true });
