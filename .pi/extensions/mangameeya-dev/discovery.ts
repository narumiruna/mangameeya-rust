import { constants, lstatSync, openSync, closeSync, fstatSync, readSync, opendirSync, type Dir, type Stats } from "node:fs";
import { homedir } from "node:os";
import { join, dirname } from "node:path";
import { Bridge, BridgeError, object, text } from "./bridge.ts";

export type Instance = { v: 1; protocol_version: 1; instance_id: string; port: number; app_version: string };
function safe(s: Stats) {
  return process.platform === "win32" ||
    (s.uid === process.getuid!() && (s.mode & 0o022) === 0);
}
export class DiscoveryRecords {
  private dir?: Dir;
  private identity?: Stats;
  constructor(private directory = join(homedir(), ".mangameeya", "developer")) {}
  close() {
    const dir = this.dir;
    this.dir = undefined;
    this.identity = undefined;
    dir?.closeSync();
  }
  // Resume raw enumeration across attempts. Read only when a probe can start.
  *read(): Generator<Instance> {
    try {
      const parent = lstatSync(dirname(this.directory)), current = lstatSync(this.directory);
      if ([parent, current].some(s => !s.isDirectory() || !safe(s))) {
        this.close();
        return;
      }
      if (current.ino !== this.identity?.ino || current.dev !== this.identity?.dev) this.close();
      this.dir ??= opendirSync(this.directory, { bufferSize: 16 });
      this.identity = current;
      let found = 0;
      for (let n = 0; n < 256 && found < 32; n++) {
        const entry = this.dir.readSync();
        if (!entry) { this.close(); break; }
        if (!/^[0-9a-f]{64}\.json$/.test(entry.name) || !entry.isFile()) continue;
        let fd: number | undefined;
        let candidate: Instance | undefined;
        try {
          const path = join(this.directory, entry.name), before = lstatSync(path);
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
          candidate = x as Instance;
        } catch { /* Invalid or replaced records are not discovery candidates. */ }
        finally { if (fd !== undefined) closeSync(fd); }
        if (candidate) { found++; yield candidate; }
      }
    } catch { this.close(); /* Missing or unsafe directories require manual pairing. */ }
  }
}
export async function discover(signal: AbortSignal, check: () => void, candidates: Iterable<Instance>): Promise<Instance[]> {
  const available: Instance[] = [];
  const iterator = candidates[Symbol.iterator]();
  const deadline = Date.now() + 6000;
  try {
    await Promise.all(Array.from({ length: 4 }, async () => {
      while (Date.now() < deadline) {
        check();
        signal.throwIfAborted();
        const next = iterator.next();
        if (next.done) break;
        const candidate = next.value;
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
  } finally { iterator.return?.(); }
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
