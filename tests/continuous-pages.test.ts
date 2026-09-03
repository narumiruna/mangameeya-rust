import assert from "node:assert/strict";
import test from "node:test";
import { distantPageIndices, isPageNavigationKey, loadOnce, transformedPageSize, usesResponsivePageSize, waitForImageLoad } from "../src/continuous-pages.ts";

test("transformedPageSize reserves scaled and rotated layout space", () => {
  assert.deepEqual(transformedPageSize(800, 1200, 0, 1.5), { width: 1200, height: 1800 });
  assert.deepEqual(transformedPageSize(800, 1200, 90, 1), { width: 1200, height: 800 });
  assert.deepEqual(transformedPageSize(800, 1200, -270, 2), { width: 2400, height: 1600 });
});

test("responsive fit modes are relaid out when the viewer changes size", () => {
  assert.equal(usesResponsivePageSize("window"), true);
  assert.equal(usesResponsivePageSize("width"), true);
  assert.equal(usesResponsivePageSize("height"), true);
  assert.equal(usesResponsivePageSize("original"), false);
  assert.equal(usesResponsivePageSize("custom"), false);
});

test("page navigation keys suppress native viewer scrolling", () => {
  for (const key of ["ArrowRight", "ArrowLeft", "PageDown", "PageUp", " ", "Home", "End"]) {
    assert.equal(isPageNavigationKey(key), true);
  }
  assert.equal(isPageNavigationKey("c"), false);
});

test("waitForImageLoad distinguishes decoded and failed images", async () => {
  class MockImage extends EventTarget {
    complete = false;
    naturalWidth = 0;
  }
  const loadedImage = new MockImage();
  const loaded = waitForImageLoad(loadedImage as unknown as HTMLImageElement);
  loadedImage.naturalWidth = 800;
  loadedImage.dispatchEvent(new Event("load"));
  assert.equal(await loaded, true);

  const failedImage = new MockImage();
  const failed = waitForImageLoad(failedImage as unknown as HTMLImageElement);
  failedImage.dispatchEvent(new Event("error"));
  assert.equal(await failed, false);

  const corruptCompletedImage = new MockImage();
  corruptCompletedImage.complete = true;
  assert.equal(await waitForImageLoad(corruptCompletedImage as unknown as HTMLImageElement), false);
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
