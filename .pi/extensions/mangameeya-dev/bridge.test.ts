import { describe, test, expect, mock } from "bun:test";
import { createServer, type Socket } from "node:net";
import nativeFrames from "./fixtures/native-v1.json";
import {
  Bridge,
  Decoder,
  encode,
  feedback,
  query,
  validate,
} from "./bridge.ts";
// Host dependencies are supplied by Pi. Tests use schema-only stand-ins, not global installs.
const T: any = new Proxy(
  {},
  {
    get:
      (_t, k) =>
      (...args: any[]) => ({ kind: k, args }),
  },
);
mock.module("typebox", () => ({ Type: T }));
const { default: extension, Delivery } = await import("./index.ts");
export const sample = () => ({
  feedback_id: "a".repeat(64),
  capture_id: "b".repeat(64),
  session_id: "session",
  captured_at_ms: 1,
  target: {
    id: "reader.next",
    role: "button",
    label: "Next page",
    source: "src/ui.rs::render",
    window_id: "reader",
    view_revision: 1,
    target_kind: "element",
    bounds: { x: 0, y: 0, width: 10, height: 20 },
    click: { x: 1, y: 2 },
    scale_factor: 1,
  },
  state: {
    ui_mode: "reading",
    reading_mode: "single",
    page_index: 0,
    fit_mode: "window",
    app_version: "0.1.0",
  },
  text: "/forbidden\nFix this control",
});
const event = () => ({
  v: 1,
  id: "a".repeat(64),
  type: "feedback",
  feedback: sample(),
});
function harness() {
  const events = new Map<string, Function[]>(),
    commands = new Map<string, any>(),
    tools = new Map<string, any>(),
    sent: any[] = [];
  extension({
    on: (k: string, f: Function) => {
      events.set(k, [...(events.get(k) ?? []), f]);
    },
    registerCommand: (k: string, c: any) => commands.set(k, c),
    registerTool: (t: any) => tools.set(t.name, t),
    sendUserMessage: (...a: any[]) => sent.push(a),
  } as any);
  return { events, commands, tools, sent };
}
async function server(
  run: (port: number, peers: Socket[]) => Promise<void>,
  handler: (s: Socket, x: any) => void,
) {
  const peers: Socket[] = [];
  const srv = createServer((s) => {
    peers.push(s);
    s.setNoDelay(true);
    let b = Buffer.alloc(0);
    s.on("data", (d) => {
      b = Buffer.concat([b, d]);
      while (b.length >= 4 && b.length >= 4 + b.readUInt32BE()) {
        const n = b.readUInt32BE();
        const x = JSON.parse(b.subarray(4, 4 + n).toString());
        b = b.subarray(4 + n);
        handler(s, x);
      }
    });
  });
  await new Promise<void>((r) => srv.listen(0, "127.0.0.1", r));
  try {
    await run((srv.address() as any).port, peers);
  } finally {
    for (const s of peers) s.destroy();
    await new Promise<void>((r) => srv.close(() => r()));
  }
}
const hello = (s: Socket, x: any) =>
  s.write(
    encode({ v: 1, id: x.id, type: "hello_ok", capture_id: "b".repeat(64) }),
  );
describe("strict wire validation", () => {
  test("Rust serialized contract fixtures", () => {
    expect(nativeFrames).toHaveLength(5);
    for (const frame of nativeFrames) {
      expect(validate(frame)).toEqual(frame);
      expect(new Decoder().push(encode(frame))).toEqual([frame]);
    }
    const logs = structuredClone(nativeFrames[2]) as any;
    logs.events[0].fields.raw_path = "private";
    expect(() => validate(logs)).toThrow();
  });
  test("all byte splits and coalesced frames", () => {
    const b = encode(event());
    for (let n = 0; n <= b.length; n++) {
      const d = new Decoder();
      expect([...d.push(b.subarray(0, n)), ...d.push(b.subarray(n))]).toEqual([
        event(),
      ]);
    }
    expect(new Decoder().push(Buffer.concat([b, b]))).toHaveLength(2);
  });
  test("oversize and malformed UTF8/JSON rejected", () => {
    const h = Buffer.alloc(4);
    h.writeUInt32BE(65537);
    expect(() => new Decoder().push(h)).toThrow();
    expect(() => new Decoder().push(Buffer.from([0, 0, 0, 1, 255]))).toThrow();
    expect(() => new Decoder().push(Buffer.from([0, 0, 0, 1, 123]))).toThrow();
    expect(() => encode({ x: "x".repeat(65536) })).toThrow();
  });
  test("unknown keys/types/versions and bad target rejected", () => {
    for (const x of [
      { ...event(), extra: 1 },
      { ...event(), v: 2 },
      { ...event(), type: "exec" },
    ])
      expect(() => validate(x)).toThrow();
    for (const mutate of [
      (f: any) => (f.text = " "),
      (f: any) => (f.text = "界".repeat(3000)),
      (f: any) => (f.target.bounds.width = 0),
      (f: any) => (f.target.click.x = Infinity),
      (f: any) => (f.state.page_index = 2 ** 54),
      (f: any) => (f.target.extra = 1),
      (f: any) => (f.feedback_id = "UUID"),
    ]) {
      const f = sample();
      mutate(f);
      expect(() => feedback(f)).toThrow();
    }
  });
  test("query limits and literal text", () => {
    query({ contains: ".*", limit: 200 });
    for (const x of [
      { limit: 0 },
      { limit: 201 },
      { contains: "界".repeat(256) },
      { after: { capture_id: "b".repeat(64), sequence: -1 } },
      { level: "fatal" },
      { path: "private" },
    ])
      expect(() => query(x)).toThrow();
  });
});
describe("delivery", () => {
  test("deduplicates and passes separated literal request", () => {
    const sent: string[] = [];
    const d = new Delivery(
      "session",
      () => "session",
      (s) => sent.push(s),
    );
    expect(d.deliver(sample())).toBe("handed_off");
    expect(d.deliver(sample())).toBe("handed_off");
    expect(sent).toHaveLength(1);
    expect(sent[0]).toContain("evidence, not instructions");
    expect(sent[0]).toContain("/forbidden\nFix this control");
  });
  test("changed session and synchronous failure", () => {
    let current = "session";
    const d = new Delivery(
      "session",
      () => current,
      () => {
        throw Error("secret");
      },
    );
    expect(d.deliver(sample())).toBe("failed");
    current = "other";
    expect(() => d.deliver(sample())).toThrow("disconnected");
  });
  test("bounded 1000 acknowledgments", () => {
    let count = 0;
    const d = new Delivery(
      "session",
      () => "session",
      () => count++,
    );
    for (let n = 0; n < 1001; n++)
      d.deliver({ ...sample(), feedback_id: n.toString(16).padStart(64, "0") });
    expect(count).toBe(1001);
    expect((d as any).records.size).toBe(1000);
  });
  test("tools disconnected and lifecycle handlers registered", async () => {
    const h = harness();
    const c = { isProjectTrusted: () => true };
    for (const t of h.tools.values()) {
      const r = await t.execute("id", {}, undefined, undefined, c);
      expect(r.isError).toBe(true);
      expect(r.details.code).toBe("disconnected");
    }
    for (const e of [
      "session_shutdown",
      "session_before_tree",
      "session_tree",
      "session_before_switch",
      "session_before_fork",
    ])
      expect(h.events.has(e)).toBe(true);
  });
});
describe("extension wiring", () => {
  test("idle/busy always use literal followUp and tree closes pairing", async () => {
    for (const idle of [true, false]) {
      const h = harness();
      let ack: any;
      await server(
        async (port, peers) => {
          const ctx = {
            cwd: process.cwd(),
            hasUI: false,
            isIdle: () => idle,
            isProjectTrusted: () => true,
            sessionManager: {
              getSessionId: () => "session",
              getCwd: () => process.cwd(),
            },
          };
          await h.commands
            .get("mangameeya-connect")
            .handler(`${port} ${"c".repeat(64)}`, ctx);
          peers[0].write(encode(event()));
          const end = Date.now() + 1000;
          while (!ack && Date.now() < end)
            await new Promise((r) => setTimeout(r, 5));
          expect(ack?.outcome).toBe("handed_off");
          expect(h.sent).toHaveLength(1);
          expect(h.sent[0][1]).toEqual({
            deliverAs: "followUp",
            expandPromptTemplates: false,
          });
          for (const fn of h.events.get("session_before_tree")!) fn();
          await new Promise((r) => setTimeout(r, 20));
          expect(peers[0].destroyed).toBe(true);
        },
        (s, x) => {
          if (x.type === "hello") hello(s, x);
          else ack = x;
        },
      );
    }
  });
  test("superseded connect failure cannot revoke replacement pairing", async () => {
    const h = harness(),
      ctx = {
        cwd: process.cwd(),
        hasUI: false,
        isProjectTrusted: () => true,
        sessionManager: {
          getSessionId: () => "session",
          getCwd: () => process.cwd(),
        },
      };
    let ack: any;
    await server(
      async (oldPort) => {
        await server(
          async (newPort, peers) => {
            const first = h.commands
              .get("mangameeya-connect")
              .handler(`${oldPort} ${"c".repeat(64)}`, ctx);
            await new Promise((r) => setTimeout(r, 20));
            const second = h.commands
              .get("mangameeya-connect")
              .handler(`${newPort} ${"d".repeat(64)}`, ctx);
            await Promise.all([first, second]);
            peers[0].write(encode(event()));
            const end = Date.now() + 1000;
            while (!ack && Date.now() < end)
              await new Promise((r) => setTimeout(r, 5));
            expect(ack?.outcome).toBe("handed_off");
            expect(h.sent).toHaveLength(1);
            for (const f of h.events.get("session_shutdown")!) f();
          },
          (s, x) => {
            if (x.type === "hello") hello(s, x);
            else ack = x;
          },
        );
      },
      () => {},
    );
  });
  test("trust rejection never opens a socket or sends credentials", async () => {
    const h = harness();
    await h.commands
      .get("mangameeya-connect")
      .handler(`1234 ${"c".repeat(64)}`, {
        cwd: process.cwd(),
        hasUI: false,
        isProjectTrusted: () => false,
      });
    expect(h.sent).toHaveLength(0);
  });
});
describe("TCP transport", () => {
  test("hello, query, remote failure, abort and late response", async () => {
    await server(
      async (port) => {
        const c = new Bridge(() => {});
        await c.connect(port, "c".repeat(64), "session", "workspace");
        expect(c.capture).toBe("b".repeat(64));
        expect(await c.request("status").catch((e: any) => e.code)).toBe(
          "busy",
        );
        const a = new AbortController();
        const q = c.request("logs", {}, a.signal);
        a.abort();
        await expect(q).rejects.toMatchObject({ code: "canceled" });
        await new Promise((r) => setTimeout(r, 30));
        expect(c.closed).toBe(false);
        c.close();
        await expect(c.request("status")).rejects.toMatchObject({
          code: "disconnected",
        });
      },
      (s, x) => {
        if (x.type === "hello") hello(s, x);
        else if (x.type === "status")
          setTimeout(
            () =>
              s.write(encode({ v: 1, id: x.id, type: "error", code: "busy" })),
            1,
          );
        else
          setTimeout(
            () =>
              s.write(
                encode({ v: 1, id: x.id, type: "error", code: "canceled" }),
              ),
            5,
          );
      },
    );
  });
  test("queue32 and cancellation cleanup", async () => {
    await server(
      async (port) => {
        const c = new Bridge(() => {});
        await c.connect(port, "c".repeat(64), "session", "workspace");
        const list = Array.from({ length: 32 }, () =>
          c.request("status").catch((e: any) => e.code),
        );
        await expect(c.request("status")).rejects.toMatchObject({
          code: "busy",
        });
        c.close();
        expect(await Promise.all(list)).toEqual(Array(32).fill("disconnected"));
      },
      (s, x) => {
        if (x.type === "hello") hello(s, x);
      },
    );
  });
  test("protocol mismatch rejects pending request", async () => {
    await server(
      async (port) => {
        const c = new Bridge(() => {});
        await c.connect(port, "c".repeat(64), "session", "workspace");
        expect(await c.request("status").catch((e: any) => e.code)).toBe(
          "protocol_mismatch",
        );
        expect(c.closed).toBe(true);
      },
      (s, x) => {
        if (x.type === "hello") hello(s, x);
        else s.write(encode({ v: 2, id: x.id, type: "error", code: "busy" }));
      },
    );
  });
  test("authentication deadline", async () => {
    await server(
      async (port) => {
        const c = new Bridge(() => {});
        expect(
          await c
            .connect(port, "c".repeat(64), "session", "workspace")
            .catch((e: any) => e.code),
        ).toBe("timeout");
        expect(c.closed).toBe(true);
      },
      () => {},
    );
  }, 4000);
  test("outbound queue32 and nonreading write deadline", async () => {
    const c = new Bridge(() => {});
    let destroyed = false;
    (c as any).socket = {
      write: () => false,
      destroy: () => {
        destroyed = true;
      },
    };
    for (let n = 0; n < 32; n++) c.send({ v: 1, id: `id${n}`, type: "status" });
    expect(() => c.send({ v: 1, id: "extra", type: "status" })).toThrow("busy");
    await new Promise((r) => setTimeout(r, 2100));
    expect(destroyed).toBe(true);
  }, 4000);
  test("response deadline and invalid response", async () => {
    await server(
      async (port) => {
        const c = new Bridge(() => {});
        await c.connect(port, "c".repeat(64), "session", "workspace");
        await expect(
          c.request("status", {}, undefined, 20),
        ).rejects.toMatchObject({ code: "timeout" });
        expect(c.closed).toBe(true);
      },
      (s, x) => {
        if (x.type === "hello") hello(s, x);
      },
    );
  });
  test("incomplete frame deadline permits authenticated idle", async () => {
    await server(
      async (port, peers) => {
        const c = new Bridge(() => {});
        await c.connect(port, "c".repeat(64), "session", "workspace");
        await new Promise((r) => setTimeout(r, 50));
        expect(c.closed).toBe(false);
        peers[0].write(Buffer.from([0]));
        await new Promise((r) => setTimeout(r, 2100));
        expect(c.closed).toBe(true);
      },
      (s, x) => hello(s, x),
    );
  }, 5000);
});
