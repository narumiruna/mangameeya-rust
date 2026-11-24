import { test, expect, spyOn } from "bun:test";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, symlinkSync, chmodSync, opendirSync, renameSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer, type Socket } from "node:net";
import { DiscoveryRecords, discover, selectInstance, type Instance } from "./discovery.ts";
import { Bridge, encode, validate } from "./bridge.ts";
const instance = (n = 1): Instance => ({ v: 1, protocol_version: 1, instance_id: n.toString(16).padStart(64, "0"), port: 1234, app_version: "1.0" });
const check = () => {};
test("records reject extra credentials, IDs, versions, size, nonfiles, symlinks and permissions", () => {
  const root = mkdtempSync(join(tmpdir(), "mangameeya-")), dir = join(root, "developer");
  mkdirSync(dir);
  const scanner = new DiscoveryRecords(dir);
  const put = (n: number, value: any) => writeFileSync(join(dir, `${instance(n).instance_id}.json`), typeof value === "string" ? value : JSON.stringify(value));
  try {
    put(1, instance());
    put(2, { ...instance(2), token: "never" });
    put(3, { ...instance(3), v: 2 });
    put(4, { ...instance(4), protocol_version: 2 });
    put(5, instance(6));
    put(7, " ".repeat(2049));
    put(8, "{");
    mkdirSync(join(dir, `${instance(9).instance_id}.json`));
    try { symlinkSync(join(dir, `${instance(1).instance_id}.json`), join(dir, `${instance(10).instance_id}.json`)); } catch (e: any) { if (e.code !== "EPERM") throw e; }
    expect([...scanner.read()]).toEqual([instance()]);
    if (process.platform !== "win32") {
      chmodSync(join(dir, `${instance(1).instance_id}.json`), 0o666);
      expect([...scanner.read()]).toEqual([]);
      chmodSync(join(dir, `${instance(1).instance_id}.json`), 0o600);
      expect([...scanner.read()]).toEqual([instance()]);
      chmodSync(dir, 0o777);
      expect([...scanner.read()]).toEqual([]);
    }
  } finally { scanner.close(); rmSync(root, { recursive: true, force: true }); }
});
function names(directory: string) {
  const dir = opendirSync(directory, { bufferSize: 16 }), result: string[] = [];
  try { for (let entry; (entry = dir.readSync());) result.push(entry.name); }
  finally { dir.closeSync(); }
  return result;
}
test("record batches stay bounded, progress, wrap at EOF and reset on close", () => {
  const root = mkdtempSync(join(tmpdir(), "mangameeya-")), dir = join(root, "developer");
  mkdirSync(dir);
  const scanner = new DiscoveryRecords(dir);
  try {
    for (let n = 1; n < 100; n++) writeFileSync(join(dir, `${instance(n).instance_id}.json`), JSON.stringify(instance(n)));
    const batches = Array.from({ length: 4 }, () => [...scanner.read()]);
    expect(batches.map(x => x.length)).toEqual([32, 32, 32, 3]);
    expect(new Set(batches.flat().map(x => x.instance_id)).size).toBe(99);
    expect([...scanner.read()]).toEqual(batches[0]);
    scanner.close();
    scanner.close();
    expect([...scanner.read()]).toEqual(batches[0]);
  } finally { scanner.close(); rmSync(root, { recursive: true, force: true }); }
});
test("raw entry budget resumes beyond invalid records", () => {
  const root = mkdtempSync(join(tmpdir(), "mangameeya-")), dir = join(root, "developer");
  mkdirSync(dir);
  const scanner = new DiscoveryRecords(dir);
  try {
    for (let n = 1; n <= 301; n++) writeFileSync(join(dir, `${instance(n).instance_id}.json`), "{}");
    const last = names(dir).at(-1)!;
    const live = { ...instance(), instance_id: last.slice(0, -5) };
    writeFileSync(join(dir, last), JSON.stringify(live));
    expect([...scanner.read()]).toEqual([]);
    expect([...scanner.read()]).toEqual([live]);
    expect(names(dir)).toHaveLength(301);
  } finally { scanner.close(); rmSync(root, { recursive: true, force: true }); }
});
test("partial iteration keeps unprobed records for the next attempt", () => {
  const root = mkdtempSync(join(tmpdir(), "mangameeya-")), dir = join(root, "developer");
  mkdirSync(dir);
  const scanner = new DiscoveryRecords(dir);
  try {
    for (let n = 1; n <= 6; n++) writeFileSync(join(dir, `${instance(n).instance_id}.json`), JSON.stringify(instance(n)));
    const batch = scanner.read(), first = batch.next().value!;
    batch.return(undefined);
    const rest = [...scanner.read()];
    expect(rest).toHaveLength(5);
    expect(rest.some(x => x.instance_id === first.instance_id)).toBe(false);
    expect([...scanner.read()]).toHaveLength(6);
  } finally { scanner.close(); rmSync(root, { recursive: true, force: true }); }
});
test("missing, unsafe and replaced directories discard the old cursor", () => {
  const root = mkdtempSync(join(tmpdir(), "mangameeya-")), dir = join(root, "developer");
  mkdirSync(dir);
  const scanner = new DiscoveryRecords(dir);
  try {
    for (let n = 1; n <= 33; n++) writeFileSync(join(dir, `${instance(n).instance_id}.json`), JSON.stringify(instance(n)));
    expect([...scanner.read()]).toHaveLength(32);
    renameSync(dir, dir + "-old");
    mkdirSync(dir);
    writeFileSync(join(dir, `${instance(100).instance_id}.json`), JSON.stringify(instance(100)));
    expect([...scanner.read()]).toEqual([instance(100)]);
    const batch = scanner.read();
    expect(batch.next().value).toEqual(instance(100));
    batch.return(undefined);
    if (process.platform !== "win32") {
      chmodSync(dir, 0o777);
      expect([...scanner.read()]).toEqual([]);
      chmodSync(dir, 0o700);
      expect([...scanner.read()]).toEqual([instance(100)]);
    }
    rmSync(dir, { recursive: true, force: true });
    expect([...scanner.read()]).toEqual([]);
    mkdirSync(dir);
    writeFileSync(join(dir, `${instance(200).instance_id}.json`), JSON.stringify(instance(200)));
    expect([...scanner.read()]).toEqual([instance(200)]);
  } finally { scanner.close(); rmSync(root, { recursive: true, force: true }); }
});
async function endpoint(handler: (s: Socket, x: any) => void, run: (port: number, peers: Socket[]) => Promise<void>) {
  const peers: Socket[] = [];
  const srv = createServer(s => {
    peers.push(s); s.on("error", () => {});
    let bytes = Buffer.alloc(0);
    s.on("data", b => {
      bytes = Buffer.concat([bytes, b]);
      while (bytes.length >= 4 && bytes.length >= 4 + bytes.readUInt32BE()) {
        const size = bytes.readUInt32BE();
        const x = JSON.parse(bytes.subarray(4, 4 + size).toString());
        bytes = bytes.subarray(4 + size); handler(s, x);
      }
    });
  });
  await new Promise<void>(r => srv.listen(0, "127.0.0.1", r));
  try { await run((srv.address() as any).port, peers); }
  finally { peers.forEach(s => s.destroy()); await new Promise<void>(r => srv.close(() => r())); }
}
test("retry discovers a live instance beyond 32 stale records without deleting records", async () => {
  const root = mkdtempSync(join(tmpdir(), "mangameeya-")), dir = join(root, "developer");
  mkdirSync(dir);
  const scanner = new DiscoveryRecords(dir);
  let live = "";
  try {
    await endpoint((s, x) => s.write(encode({ v: 1, id: x.id, type: "discovery_result", instance_id: x.instance_id, app_version: "1.0", available: x.instance_id === live })), async port => {
      for (let n = 1; n <= 33; n++) writeFileSync(join(dir, `${instance(n).instance_id}.json`), JSON.stringify({ ...instance(n), port }));
      live = names(dir).at(-1)!.slice(0, -5);
      expect(await discover(new AbortController().signal, check, scanner.read())).toEqual([]);
      const result = await discover(new AbortController().signal, check, scanner.read());
      expect(result.map(x => x.instance_id)).toEqual([live]);
      expect(names(dir)).toHaveLength(33);
    });
  } finally { scanner.close(); rmSync(root, { recursive: true, force: true }); }
});
test("discovery deadline does not consume candidates that cannot be probed", async () => {
  const root = mkdtempSync(join(tmpdir(), "mangameeya-")), dir = join(root, "developer");
  mkdirSync(dir);
  const scanner = new DiscoveryRecords(dir);
  let now = 0;
  const clock = spyOn(Date, "now").mockImplementation(() => now);
  const probe = spyOn(Bridge.prototype, "probe").mockImplementation(async (_port, id) => {
    now += 6001;
    return { v: 1, id: "probe", type: "discovery_result", instance_id: id, app_version: "1.0", available: true };
  });
  try {
    for (let n = 1; n <= 6; n++) writeFileSync(join(dir, `${instance(n).instance_id}.json`), JSON.stringify(instance(n)));
    const found: string[] = [];
    for (let n = 0; n < 6; n++) {
      const result = await discover(new AbortController().signal, check, scanner.read());
      expect(result).toHaveLength(1);
      found.push(result[0].instance_id);
    }
    expect(new Set(found).size).toBe(6);
    expect(probe).toHaveBeenCalledTimes(6);
  } finally { probe.mockRestore(); clock.mockRestore(); scanner.close(); rmSync(root, { recursive: true, force: true }); }
});
test("discovery strict schema", () => {
  const x = { v: 1, id: "x", type: "discovery_result", instance_id: instance().instance_id, app_version: "1", available: true };
  expect(validate(x)).toEqual(x);
  for (const bad of [{ ...x, token: "no" }, { ...x, available: 1 }, { ...x, instance_id: "no" }, { ...x, v: 2 }]) expect(() => validate(bad)).toThrow();
});
test("live probes ignore wrong instance, stale, busy and wrong version; concurrency bounded", async () => {
  let live = 0, peak = 0;
  await endpoint((s, x) => {
    expect(x.type).toBe("discover"); expect(Object.keys(x).sort()).toEqual(["id", "instance_id", "type", "v"]);
    peak = Math.max(peak, ++live);
    const n = parseInt(x.instance_id, 16);
    setTimeout(() => {
      live--;
      if (n === 5) return;
      s.write(encode({ v: 1, id: x.id, type: "discovery_result", instance_id: n === 2 ? instance(99).instance_id : x.instance_id, app_version: n === 3 ? "2" : "1.0", available: n !== 4 }));
    }, 10);
  }, async port => {
    const candidates = Array.from({ length: 8 }, (_, i) => ({ ...instance(i + 1), port }));
    const result = await discover(new AbortController().signal, check, candidates);
    expect(result.map(x => parseInt(x.instance_id, 16))).toEqual([1, 6, 7, 8]);
    expect(peak).toBeLessThanOrEqual(4);
  });
});
test("one auto-selects; multiple use unique safe labels; no UI and cancel fail closed", async () => {
  const a = instance(), b = { ...instance(2), app_version: "\u001b[31m\n" };
  const signal = new AbortController().signal;
  expect(await selectInstance([a], {}, signal, check)).toEqual(a);
  let labels: string[] = [];
  expect(await selectInstance([a, b], { hasUI: true, ui: { select: async (_: any, values: string[]) => { labels = values; return values[1]; } } }, signal, check)).toEqual(b);
  expect(labels[1]).not.toMatch(/[\u0000-\u001f]/);
  expect(new Set(labels).size).toBe(2);
  await expect(selectInstance([a, b], { hasUI: false }, signal, check)).rejects.toMatchObject({ code: "invalid_request" });
  await expect(selectInstance([a, b], { hasUI: true, ui: { select: async () => undefined } }, signal, check)).rejects.toMatchObject({ code: "canceled" });
});
test("selection and probes abort on lifecycle revocation", async () => {
  const a = new AbortController();
  let current = true;
  const guard = () => { if (!current) throw Error("obsolete"); };
  const selecting = selectInstance([instance(), instance(2)], { hasUI: true, ui: { select: async (_: any, labels: string[], opts: any) => {
    expect(opts.signal).toBe(a.signal); current = false; a.abort(); return labels[0];
  } } }, a.signal, guard);
  await expect(selecting).rejects.toThrow("obsolete");
  await endpoint(() => {}, async port => {
    const abort = new AbortController();
    const pending = discover(abort.signal, check, [{ ...instance(), port }]);
    abort.abort();
    await expect(pending).rejects.toThrow();
  });
});
test("approval uses the 65-second client deadline", async () => {
  await endpoint(() => {}, async port => {
    const c = new Bridge(() => {}), abort = new AbortController();
    const request = spyOn(c, "request").mockResolvedValue({ v: 1, id: "id", type: "hello_ok", capture_id: "b".repeat(64) });
    try {
      await c.pair(port, instance().instance_id, "session", "workspace", abort.signal);
      expect(request).toHaveBeenCalledWith("pair_request", { instance_id: instance().instance_id, session_id: "session", workspace: "workspace" }, abort.signal, 65000);
    } finally { request.mockRestore(); c.close(); }
  });
});
test("App approval waits beyond manual deadline; denial and lifecycle cancel never authenticate", async () => {
  for (const outcome of ["approve", "canceled", "timeout", "busy", "abort"]) {
    await endpoint((s, x) => {
      expect(x.type).toBe("pair_request");
      expect(x.instance_id).toBe(instance().instance_id);
      expect(x.token).toBeUndefined();
      if (outcome === "abort") return;
      setTimeout(() => s.write(encode(outcome === "approve"
        ? { v: 1, id: x.id, type: "hello_ok", capture_id: instance().instance_id }
        : { v: 1, id: x.id, type: "error", code: outcome })), outcome === "approve" ? 2100 : 5);
    }, async port => {
      const c = new Bridge(() => {}), abort = new AbortController();
      try {
        const p = c.pair(port, instance().instance_id, "session", "workspace", abort.signal);
        expect(c.capture).toBe("");
        if (outcome === "abort") { abort.abort(); await expect(p).rejects.toMatchObject({ code: "canceled" }); expect(c.closed).toBe(true); }
        else if (outcome === "approve") { await p; expect(c.capture).toBe(instance().instance_id); }
        else { await expect(p).rejects.toMatchObject({ code: outcome }); expect(c.capture).toBe(""); }
      } finally { c.close(); }
    });
  }
}, 5000);
