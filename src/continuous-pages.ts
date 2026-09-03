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
