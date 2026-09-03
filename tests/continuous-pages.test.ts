import assert from "node:assert/strict";
import test from "node:test";
import { distantPageIndices, loadOnce, transformedPageSize } from "../src/continuous-pages.ts";

test("transformedPageSize reserves scaled and rotated layout space", () => {
  assert.deepEqual(transformedPageSize(800, 1200, 0, 1.5), { width: 1200, height: 1800 });
  assert.deepEqual(transformedPageSize(800, 1200, 90, 1), { width: 1200, height: 800 });
  assert.deepEqual(transformedPageSize(800, 1200, -270, 2), { width: 2400, height: 1600 });
});

test("distantPageIndices keeps only pages outside the retention window", () => {
  assert.deepEqual(distantPageIndices([0, 1, 8, 9, 10, 17, 18], 9, 8), [0, 18]);
});

test("loadOnce shares an in-flight request and clears it after success", async () => {
  const inFlight = new Map<number, Promise<string>>();
  let calls = 0;
  let resolveRequest!: (value: string) => void;
  const loader = () => {
    calls += 1;
    return new Promise<string>((resolve) => { resolveRequest = resolve; });
  };

  const first = loadOnce(inFlight, 4, loader);
  const second = loadOnce(inFlight, 4, loader);
  assert.equal(first, second);
  assert.equal(calls, 0);

  await Promise.resolve();
  assert.equal(calls, 1);
  resolveRequest("page");
  assert.equal(await first, "page");
  await Promise.resolve();
  assert.equal(inFlight.size, 0);
});

test("loadOnce clears failed requests so they can be retried", async () => {
  const inFlight = new Map<number, Promise<string>>();
  let calls = 0;
  const loader = async () => {
    calls += 1;
    if (calls === 1) throw new Error("failed");
    return "retried";
  };

  await assert.rejects(loadOnce(inFlight, 2, loader), /failed/);
  await Promise.resolve();
  assert.equal(await loadOnce(inFlight, 2, loader), "retried");
  assert.equal(calls, 2);
});
