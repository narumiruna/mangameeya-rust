export interface PageSize {
  width: number;
  height: number;
}

export interface PageLayout extends PageSize {
  scale: number;
}

export function transformedPageSize(width: number, height: number, rotation: number, scale: number): PageSize {
  const quarterTurn = Math.abs(rotation % 180) === 90;
  return {
    width: (quarterTurn ? height : width) * scale,
    height: (quarterTurn ? width : height) * scale,
  };
}

export function fittedPageLayout(
  naturalWidth: number,
  naturalHeight: number,
  rotation: number,
  fit: string,
  zoom: number,
  availableWidth: number,
  availableHeight: number,
): PageLayout {
  const rotated = transformedPageSize(naturalWidth, naturalHeight, rotation, 1);
  let scale = 1;
  if (fit === "window") {
    const widthScale = availableWidth > 0 ? availableWidth / rotated.width : 1;
    const heightScale = availableHeight > 0 ? availableHeight / rotated.height : 1;
    scale = Math.min(1, widthScale, heightScale);
  } else if (fit === "width" && availableWidth > 0) {
    scale = availableWidth / rotated.width;
  } else if (fit === "height" && availableHeight > 0) {
    scale = availableHeight / rotated.height;
  } else if (fit === "custom") {
    scale = zoom;
  }
  return { ...transformedPageSize(naturalWidth, naturalHeight, rotation, scale), scale };
}

export function pageScrollProgress(scrollTop: number, pageTop: number, pageHeight: number): number {
  if (pageHeight <= 0) return 0;
  return Math.min(1, Math.max(0, (scrollTop - pageTop) / pageHeight));
}

export function pageScrollTop(pageTop: number, pageHeight: number, progress: number): number {
  return pageTop + pageHeight * Math.min(1, Math.max(0, progress));
}

export function usesResponsivePageSize(fit: string): boolean {
  return fit === "window" || fit === "width" || fit === "height";
}

export function isPageNavigationKey(key: string): boolean {
  return ["ArrowRight", "ArrowLeft", "PageDown", "PageUp", " ", "Home", "End"].includes(key);
}

export function isNativeScrollKey(key: string): boolean {
  return key === "ArrowUp" || key === "ArrowDown";
}

export function continuousPageLoadState(failed: boolean, cached: boolean): "error" | "ready" | "loading" {
  if (failed) return "error";
  return cached ? "ready" : "loading";
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
