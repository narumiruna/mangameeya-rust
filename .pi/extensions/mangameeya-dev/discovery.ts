import { constants, lstatSync, openSync, closeSync, fstatSync, readSync, opendirSync, type Stats } from "node:fs";
import { homedir } from "node:os";
import { join, dirname } from "node:path";
import { Bridge, BridgeError, object, text } from "./bridge.ts";

export type Instance = { v: 1; protocol_version: 1; instance_id: string; port: number; app_version: string };
function safe(s: Stats) {
  return process.platform === "win32" ||
    (s.uid === process.getuid!() && (s.mode & 0o022) === 0);
}
// Fixed allocations and entry limits also apply to invalid records.
export function records(directory = join(homedir(), ".mangameeya", "developer")): Instance[] {
  const result: Instance[] = [];
  try {
    for (const path of [dirname(directory), directory]) {
      const s = lstatSync(path);
      if (!s.isDirectory() || !safe(s)) return [];
    }
    const dir = opendirSync(directory, { bufferSize: 16 });
    try {
      for (let n = 0; n < 256 && result.length < 32; n++) {
        const entry = dir.readSync();
        if (!entry) break;
        if (!/^[0-9a-f]{64}\.json$/.test(entry.name) || !entry.isFile()) continue;
        let fd: number | undefined;
        try {
          const path = join(directory, entry.name), before = lstatSync(path);
          if (!before.isFile() || !safe(before) || before.size > 2048) continue;
          fd = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0));
          const s = fstatSync(fd);
          if (!s.isFile() || !safe(s) || s.size > 2048 || s.ino !== before.ino || s.dev !== before.dev) continue;
          const b = Buffer.alloc(2049);
          let used = 0, count = 0;
          while (used < b.length && (count = readSync(fd, b, used, b.length - used, null)) > 0) used += count;
          if (used > 2048) continue;
          const x = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(b.subarray(0, used)));
          object(x, ["v", "protocol_version", "instance_id", "port", "app_version"]);
          if (x.v !== 1 || x.protocol_version !== 1 || x.instance_id !== entry.name.slice(0, -5) ||
              !Number.isInteger(x.port) || x.port < 1 || x.port > 65535) continue;
          text(x.app_version, 64);
          result.push(x as Instance);
        } catch { /* Invalid or replaced records are not discovery candidates. */ }
        finally { if (fd !== undefined) closeSync(fd); }
      }
    } finally { dir.closeSync(); }
  } catch { /* Missing or unsafe directories require manual pairing. */ }
  return result;
}
export async function discover(signal: AbortSignal, check: () => void, candidates = records()): Promise<Instance[]> {
  const available: Instance[] = [];
  let next = 0;
  const deadline = Date.now() + 6000;
  await Promise.all(Array.from({ length: 4 }, async () => {
    while (next < candidates.length && Date.now() < deadline) {
      check();
      signal.throwIfAborted();
      const candidate = candidates[next++];
      const client = new Bridge(() => {});
      const abort = () => client.close("canceled");
      signal.addEventListener("abort", abort, { once: true });
      try {
        const x = await client.probe(candidate.port, candidate.instance_id, signal);
        check();
        signal.throwIfAborted();
        if (x.available && x.app_version === candidate.app_version) available.push(candidate);
      } catch { check(); signal.throwIfAborted(); }
      finally { signal.removeEventListener("abort", abort); client.close(); }
    }
  }));
  check();
  return available.sort((a, b) => a.instance_id.localeCompare(b.instance_id));
}
export async function selectInstance(instances: Instance[], ctx: any, signal: AbortSignal, check: () => void): Promise<Instance> {
  check();
  signal.throwIfAborted();
  if (!instances.length) throw new BridgeError("disconnected");
  if (instances.length === 1) return instances[0];
  if (!ctx.hasUI) throw new BridgeError("invalid_request");
  const labels = instances.map((x, i) => `${i + 1}. Mangameeya ${x.app_version.replace(/[^A-Za-z0-9._+-]/g, "?").slice(0, 32)} (port ${x.port})`);
  const choice = await ctx.ui.select("Select Mangameeya App", labels, { signal, timeout: 60000 });
  check();
  signal.throwIfAborted();
  const index = labels.indexOf(choice);
  if (index < 0) throw new BridgeError("canceled");
  return instances[index];
}
