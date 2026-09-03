export interface PageSize {
  width: number;
  height: number;
}

export function transformedPageSize(width: number, height: number, rotation: number, scale: number): PageSize {
  const quarterTurn = Math.abs(rotation % 180) === 90;
  return {
    width: (quarterTurn ? height : width) * scale,
    height: (quarterTurn ? width : height) * scale,
  };
}

export function usesResponsivePageSize(fit: string): boolean {
  return fit === "window" || fit === "width" || fit === "height";
}

export function isPageNavigationKey(key: string): boolean {
  return ["ArrowRight", "ArrowLeft", "PageDown", "PageUp", " ", "Home", "End"].includes(key);
}

export function waitForImageLoad(image: HTMLImageElement): Promise<boolean> {
  if (image.complete) return Promise.resolve(image.naturalWidth > 0);
  return new Promise((resolve) => {
    const finish = (loaded: boolean) => {
      image.removeEventListener("load", loadedImage);
      image.removeEventListener("error", failedImage);
      resolve(loaded);
    };
    const loadedImage = () => finish(image.naturalWidth > 0);
    const failedImage = () => finish(false);
    image.addEventListener("load", loadedImage, { once: true });
    image.addEventListener("error", failedImage, { once: true });
  });
}

export function distantPageIndices(indices: Iterable<number>, current: number, retention: number): number[] {
  return [...indices].filter((index) => Math.abs(index - current) > retention);
}

export function loadOnce<K, V>(inFlight: Map<K, Promise<V>>, key: K, loader: () => Promise<V>): Promise<V> {
  const pending = inFlight.get(key);
  if (pending) return pending;

  const request = Promise.resolve().then(loader);
  inFlight.set(key, request);
  const clear = () => {
    if (inFlight.get(key) === request) inFlight.delete(key);
  };
  void request.then(clear, clear);
  return request;
}
