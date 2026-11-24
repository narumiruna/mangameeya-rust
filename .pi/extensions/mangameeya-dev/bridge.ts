import { createConnection, type Socket } from "node:net";
import { randomUUID } from "node:crypto";

export const MAX_BODY = 65536;
export class BridgeError extends Error {
  constructor(public code: string) {
    super(code);
  }
}
type Obj = Record<string, any>;
const fail = (): never => {
  throw new BridgeError("protocol_mismatch");
};
export function object(
  x: unknown,
  keys: string[],
  optional: string[] = [],
): asserts x is Obj {
  if (!x || typeof x !== "object" || Array.isArray(x)) fail();
  if (
    Object.keys(x).some((k) => !keys.includes(k) && !optional.includes(k)) ||
    keys.some((k) => !(k in x))
  )
    fail();
}
export function text(
  x: unknown,
  max: number,
  ascii = false,
): asserts x is string {
  if (
    typeof x !== "string" ||
    !x.length ||
    Buffer.byteLength(x) > max ||
    (ascii && !/^[A-Za-z0-9_.:-]+$/.test(x))
  )
    fail();
}
function integer(x: unknown) {
  if (!Number.isSafeInteger(x) || (x as number) < 0) fail();
}
function choice(x: unknown, list: string[]) {
  if (!list.includes(x as string)) fail();
}
function hex(x: unknown) {
  if (typeof x !== "string" || !/^[0-9a-f]{64}$/.test(x)) fail();
}
export function query(x: unknown): asserts x is Obj {
  object(x, [], ["after", "level", "target", "contains", "limit"]);
  if (x.after != null) {
    object(x.after, ["capture_id", "sequence"]);
    hex(x.after.capture_id);
    integer(x.after.sequence);
  }
  if (x.level != null) choice(x.level, ["debug", "info", "warn", "error"]);
  if (x.target != null)
    choice(x.target, [
      "app",
      "open",
      "list",
      "decode",
      "navigation",
      "preferences",
      "release",
    ]);
  if (
    x.contains != null &&
    (typeof x.contains !== "string" || Buffer.byteLength(x.contains) > 256)
  )
    fail();
  if (x.limit != null) {
    integer(x.limit);
    if (x.limit < 1 || x.limit > 200) fail();
  }
}
export function feedback(x: unknown): asserts x is Obj {
  object(x, [
    "feedback_id",
    "capture_id",
    "session_id",
    "captured_at_ms",
    "target",
    "state",
    "text",
  ]);
  hex(x.feedback_id);
  hex(x.capture_id);
  text(x.session_id, 512);
  integer(x.captured_at_ms);
  text(x.text, 8192);
  if (!x.text.trim()) fail();
  const t = x.target;
  object(t, [
    "id",
    "role",
    "label",
    "source",
    "window_id",
    "view_revision",
    "target_kind",
    "bounds",
    "click",
    "scale_factor",
  ]);
  text(t.id, 64, true);
  text(t.role, 64, true);
  text(t.label, 256);
  text(t.source, 256);
  text(t.window_id, 64, true);
  integer(t.view_revision);
  choice(t.target_kind, ["element", "region"]);
  object(t.bounds, ["x", "y", "width", "height"]);
  object(t.click, ["x", "y"]);
  for (const n of [
    ...Object.values(t.bounds),
    ...Object.values(t.click),
    t.scale_factor,
  ])
    if (typeof n !== "number" || !Number.isFinite(n) || Math.abs(n) > 1e7)
      fail();
  if (
    t.bounds.width <= 0 ||
    t.bounds.height <= 0 ||
    t.scale_factor < 0.1 ||
    t.scale_factor > 16
  )
    fail();
  const s = x.state;
  object(s, [
    "ui_mode",
    "reading_mode",
    "page_index",
    "fit_mode",
    "app_version",
  ]);
  choice(s.ui_mode, ["reading", "browsing", "preferences"]);
  choice(s.reading_mode, ["single", "spread", "continuous"]);
  choice(s.fit_mode, ["window", "width", "height", "original", "custom"]);
  if (s.page_index !== null) integer(s.page_index);
  text(s.app_version, 64);
}
const codes = [
  "disconnected",
  "timeout",
  "protocol_mismatch",
  "canceled",
  "busy",
  "invalid_request",
  "capture_changed",
];
export function validate(x: unknown): Obj {
  object(
    x,
    ["v", "id", "type"],
    [
      "capture_id",
      "status",
      "events",
      "after",
      "oldest_sequence",
      "dropped",
      "truncated",
      "code",
      "feedback",
    ],
  );
  if (x.v !== 1) fail();
  text(x.id, 64, true);
  const base = ["v", "id", "type"];
  switch (x.type) {
    case "hello_ok":
      object(x, [...base, "capture_id"]);
      hex(x.capture_id);
      break;
    case "error":
      object(x, [...base, "code"]);
      choice(x.code, codes);
      break;
    case "feedback":
      object(x, [...base, "feedback"]);
      feedback(x.feedback);
      if (x.id !== x.feedback.feedback_id) fail();
      break;
    case "status_result": {
      object(x, [...base, "status"]);
      const s = x.status;
      object(s, [
        "app_version",
        "protocol_version",
        "instance_id",
        "capture_id",
        "session_id",
        "capture_started_ms",
        "ui_mode",
      ]);
      text(s.app_version, 64);
      if (s.protocol_version !== 1) fail();
      hex(s.instance_id);
      hex(s.capture_id);
      text(s.session_id, 512);
      integer(s.capture_started_ms);
      choice(s.ui_mode, ["reading", "browsing", "preferences"]);
      break;
    }
    case "logs_result":
      object(x, [
        ...base,
        "capture_id",
        "events",
        "after",
        "oldest_sequence",
        "dropped",
        "truncated",
      ]);
      hex(x.capture_id);
      object(x.after, ["capture_id", "sequence"]);
      hex(x.after.capture_id);
      integer(x.after.sequence);
      if (x.after.capture_id !== x.capture_id) fail();
      integer(x.oldest_sequence);
      integer(x.dropped);
      if (
        typeof x.truncated !== "boolean" ||
        !Array.isArray(x.events) ||
        x.events.length > 200
      )
        fail();
      for (const e of x.events) {
        object(e, [
          "sequence",
          "timestamp_ms",
          "level",
          "target",
          "code",
          "operation_id",
          "duration_ms",
          "fields",
        ]);
        integer(e.sequence);
        integer(e.timestamp_ms);
        query({ level: e.level, target: e.target });
        choice(e.code, ["started", "completed", "failed", "changed"]);
        integer(e.operation_id);
        integer(e.duration_ms);
        if (
          !e.fields ||
          typeof e.fields !== "object" ||
          Array.isArray(e.fields) ||
          Object.keys(e.fields).length > 32
        )
          fail();
        for (const [k, v] of Object.entries(e.fields)) {
          choice(k, [
            "count",
            "page_index",
            "success",
            "error_code",
            "width",
            "height",
          ]);
          if (
            typeof v !== "boolean" &&
            !(typeof v === "number" && Number.isSafeInteger(v) && v >= 0)
          )
            fail();
        }
        if (Buffer.byteLength(JSON.stringify(e)) > 8192) fail();
      }
      break;
    default:
      fail();
  }
  return x;
}
export function encode(x: unknown) {
  const b = Buffer.from(JSON.stringify(x));
  if (!b.length || b.length > MAX_BODY) fail();
  const h = Buffer.alloc(4);
  h.writeUInt32BE(b.length);
  return Buffer.concat([h, b]);
}
export class Decoder {
  private header = Buffer.alloc(4);
  private used = 0;
  private body?: Buffer;
  private at = 0;
  get incomplete() {
    return this.used > 0;
  }
  push(data: Buffer): Obj[] {
    const result: Obj[] = [];
    let p = 0;
    while (p < data.length) {
      if (this.used < 4) {
        const n = Math.min(4 - this.used, data.length - p);
        data.copy(this.header, this.used, p, p + n);
        this.used += n;
        p += n;
        if (this.used < 4) break;
        const size = this.header.readUInt32BE();
        if (!size || size > MAX_BODY) fail();
        this.body = Buffer.alloc(size);
      }
      const n = Math.min(this.body!.length - this.at, data.length - p);
      data.copy(this.body!, this.at, p, p + n);
      p += n;
      this.at += n;
      if (this.at === this.body!.length) {
        let value;
        try {
          value = JSON.parse(
            new TextDecoder("utf-8", { fatal: true }).decode(this.body),
          );
        } catch {
          fail();
        }
        result.push(validate(value));
        this.used = 0;
        this.at = 0;
        this.body = undefined;
      }
    }
    return result;
  }
}
export class Bridge {
  private socket?: Socket;
  private decoder = new Decoder();
  private frameTimer?: ReturnType<typeof setTimeout>;
  private writes = 0;
  private retired = new Set<string>();
  private pending = new Map<
    string,
    { type: string; resolve: (x: Obj) => void; reject: (e: Error) => void }
  >();
  capture = "";
  closed = false;
  constructor(
    private onFeedback: (x: Obj) => void,
    private onClose: () => void = () => {},
  ) {}
  async connect(
    port: number,
    token: string,
    session: string,
    workspace: string,
  ) {
    this.socket = createConnection({ host: "127.0.0.1", port });
    this.socket.setNoDelay(true);
    this.socket.on("error", () => this.close());
    this.socket.on("close", () => this.close());
    this.socket.on("data", (b) => {
      try {
        for (const x of this.decoder.push(b)) {
          if (x.type === "feedback") {
            if (!this.capture || x.feedback.capture_id !== this.capture) fail();
            this.onFeedback(x);
          } else {
            const p = this.pending.get(x.id);
            if (!p && this.retired.delete(x.id)) continue;
            if (!p || (x.type !== p.type && x.type !== "error")) fail();
            if (x.type === "hello_ok") this.capture = x.capture_id;
            this.pending.delete(x.id);
            x.type === "error"
              ? p.reject(new BridgeError(x.code))
              : p.resolve(x);
          }
        }
        if (!this.decoder.incomplete) {
          clearTimeout(this.frameTimer);
          this.frameTimer = undefined;
        } else if (!this.frameTimer)
          this.frameTimer = setTimeout(() => this.close("timeout"), 2000);
      } catch {
        this.close("protocol_mismatch");
      }
    });
    const result = await this.request(
      "hello",
      { token, session_id: session, workspace },
      undefined,
      2000,
    );
    this.capture = result.capture_id;
  }
  send(x: Obj) {
    if (this.closed || !this.socket) throw new BridgeError("disconnected");
    if (this.writes >= 32) throw new BridgeError("busy");
    const b = encode(x);
    this.writes++;
    const timer = setTimeout(() => this.close("timeout"), 2000);
    this.socket.write(b, () => {
      clearTimeout(timer);
      this.writes--;
    });
  }
  request(
    type: string,
    params: Obj = {},
    signal?: AbortSignal,
    timeout = 5000,
  ): Promise<Obj> {
    if (signal?.aborted) return Promise.reject(new BridgeError("canceled"));
    if (this.pending.size >= 32) return Promise.reject(new BridgeError("busy"));
    const id = randomUUID();
    const expected = type === "hello" ? "hello_ok" : type + "_result";
    return new Promise((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timer);
        signal?.removeEventListener("abort", abort);
        this.pending.delete(id);
      };
      const abort = () => {
        cleanup();
        this.retired.add(id);
        if (this.retired.size > 32) this.close("canceled");
        reject(new BridgeError("canceled"));
      };
      const timer = setTimeout(() => {
        cleanup();
        reject(new BridgeError("timeout"));
        this.close("timeout");
      }, timeout);
      this.pending.set(id, {
        type: expected,
        resolve: (x) => {
          cleanup();
          resolve(x);
        },
        reject: (e) => {
          cleanup();
          reject(e);
        },
      });
      signal?.addEventListener("abort", abort, { once: true });
      try {
        this.send({ v: 1, id, type, ...params });
      } catch (e) {
        cleanup();
        reject(e);
      }
    });
  }
  close(code = "disconnected") {
    if (this.closed) return;
    this.closed = true;
    clearTimeout(this.frameTimer);
    this.socket?.destroy();
    for (const p of [...this.pending.values()]) p.reject(new BridgeError(code));
    this.pending.clear();
    this.onClose();
  }
}
