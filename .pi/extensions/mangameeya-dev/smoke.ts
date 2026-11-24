// Run: bun.exe .pi/extensions/mangameeya-dev/smoke.ts <installed pi-coding-agent directory>
// Uses in-memory credentials/settings/sessions, trusted project discovery, and an in-process stub.
import assert from "node:assert/strict";
import { createServer, type Socket } from "node:net";
import { resolve, join } from "node:path";
import { pathToFileURL } from "node:url";
import { encode } from "./bridge.ts";
const install = resolve(process.argv[2] ?? "");
if (!process.argv[2])
  throw Error("Supply the installed pi-coding-agent directory");
const p = await import(pathToFileURL(join(install, "dist/index.js")).href);
const ai = await import(
  pathToFileURL(join(install, "../pi-ai/dist/index.js")).href
);
const cwd = process.cwd(),
  agentDir = join(cwd, ".pi/nonexistent-smoke-agent");
const settings = p.SettingsManager.inMemory({
  compaction: { enabled: false },
  retry: { enabled: false },
});
settings.setProjectTrusted(true);
const loaderOptions = {
  cwd,
  agentDir,
  settingsManager: settings,
  noSkills: true,
  noPromptTemplates: true,
  noThemes: true,
  noContextFiles: true,
};
const untrusted = p.SettingsManager.inMemory();
untrusted.setProjectTrusted(false);
const untrustedLoader = new p.DefaultResourceLoader({
  ...loaderOptions,
  settingsManager: untrusted,
});
await untrustedLoader.reload();
assert(
  !untrustedLoader
    .getExtensions()
    .extensions.some((e: any) =>
      e.path.replaceAll("\\", "/").endsWith("/mangameeya-dev/index.ts"),
    ),
);
const loader = new p.DefaultResourceLoader(loaderOptions);
await loader.reload();
assert.deepEqual(loader.getExtensions().errors, []);
assert(
  loader
    .getExtensions()
    .extensions.some((e: any) =>
      e.path.replaceAll("\\", "/").endsWith("/mangameeya-dev/index.ts"),
    ),
  "trusted project-local extension discovered",
);
const models = await p.ModelRuntime.create({
  credentials: new ai.InMemoryCredentialStore(),
  modelsPath: null,
  refreshOnCreate: false,
});
let calls = 0,
  release: (() => void) | undefined,
  issueLogTool = false,
  toolIssued = false;
const inputs: any[] = [],
  acks: any[] = [];
models.registerProvider("bridge-stub", {
  api: "openai-completions",
  apiKey: "local-stub",
  baseUrl: "http://127.0.0.1:1",
  models: [
    {
      id: "stub",
      name: "stub",
      reasoning: false,
      input: ["text"],
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: 65536,
      maxTokens: 256,
    },
  ],
  streamSimple(model: any, context: any) {
    calls++;
    inputs.push(context.messages);
    const stream = ai.createAssistantMessageEventStream();
    const finish = () => {
      const useTool = issueLogTool && !toolIssued;
      if (useTool) toolIssued = true;
      const message = {
        role: "assistant",
        api: model.api,
        provider: model.provider,
        model: model.id,
        timestamp: Date.now(),
        content: useTool
          ? [
              {
                type: "toolCall",
                id: "logs-call",
                name: "mangameeya_logs",
                arguments: { level: "info", limit: 1 },
              },
            ]
          : [{ type: "text", text: "Local stub assessment" }],
        stopReason: useTool ? "toolUse" : "stop",
        usage: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 0,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
        },
      };
      stream.push({ type: "start", partial: message });
      stream.push({ type: "done", reason: message.stopReason, message });
      stream.end();
    };
    if (calls === 1) release = finish;
    else queueMicrotask(finish);
    return stream;
  },
});
let peer: Socket | undefined,
  sessionId = "",
  queries = 0;
const capture = "b".repeat(64),
  token = "c".repeat(64);
const sockets: Socket[] = [];
const server = createServer((s) => {
  peer = s;
  sockets.push(s);
  let bytes = Buffer.alloc(0);
  s.on("error", () => {});
  s.on("data", (d) => {
    bytes = Buffer.concat([bytes, d]);
    while (bytes.length >= 4 && bytes.length >= 4 + bytes.readUInt32BE()) {
      const n = bytes.readUInt32BE(),
        x = JSON.parse(bytes.subarray(4, 4 + n).toString());
      bytes = bytes.subarray(4 + n);
      if (x.type === "hello") {
        assert.equal(x.token, token);
        assert.equal(x.workspace, cwd);
        sessionId = x.session_id;
        s.write(
          encode({ v: 1, id: x.id, type: "hello_ok", capture_id: capture }),
        );
      } else if (x.type === "logs") {
        queries++;
        assert.equal(x.limit, 1);
        s.write(
          encode({
            v: 1,
            id: x.id,
            type: "logs_result",
            capture_id: capture,
            events: [],
            after: { capture_id: capture, sequence: 0 },
            oldest_sequence: 0,
            dropped: 0,
            truncated: false,
          }),
        );
      } else if (x.type === "ack") acks.push(x);
      else throw Error("Unexpected operation");
    }
  });
});
await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
const { session } = await p.createAgentSession({
  cwd,
  agentDir,
  modelRuntime: models,
  model: models.getModel("bridge-stub", "stub"),
  settingsManager: settings,
  resourceLoader: loader,
  sessionManager: p.SessionManager.inMemory(cwd),
  tools: ["mangameeya_logs", "mangameeya_status"],
});
const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until(f: () => boolean) {
  const deadline = Date.now() + 5000;
  while (!f()) {
    if (Date.now() > deadline) throw Error("Smoke deadline");
    await delay(5);
  }
}
const feedback = (id: string) => ({
  v: 1,
  id,
  type: "feedback",
  feedback: {
    feedback_id: id,
    capture_id: capture,
    session_id: sessionId,
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
    text: "/forbidden\nAssess this control",
  },
});
try {
  await session.bindExtensions({ mode: "print" });
  await session.prompt(
    `/mangameeya-connect ${(server.address() as any).port} ${token}`,
  );
  assert.equal(calls, 0);
  peer!.write(encode(feedback("a".repeat(64))));
  await until(() => !!release);
  await until(() => acks.length === 1);
  peer!.write(encode(feedback("d".repeat(64))));
  await until(() => acks.length === 2);
  assert.equal(calls, 1, "busy follow-up did not interrupt");
  peer!.write(encode(feedback("a".repeat(64))));
  await until(() => acks.length === 3);
  assert.equal(calls, 1, "deduplicated");
  release!();
  await until(() => calls === 2);
  await session.waitForIdle();
  assert(acks.every((a) => a.outcome === "handed_off"));
  const users = session.messages.filter((m: any) => m.role === "user");
  assert.equal(users.length, 2);
  assert(JSON.stringify(users).includes("/forbidden"));
  assert(
    !JSON.stringify(inputs).includes(token),
    "credentials never reach model",
  );
  issueLogTool = true;
  await session.prompt("Read one safe diagnostic event");
  assert.equal(queries, 1);
  assert(
    session.messages.some(
      (m: any) =>
        m.role === "toolResult" &&
        m.toolName === "mangameeya_logs" &&
        !m.isError,
    ),
  );
  const target = session.sessionManager
    .getEntries()
    .find((e: any) => e.type === "message" && e.message.role === "user").id;
  await session.navigateTree(target, { summarize: false });
  await until(() => peer!.destroyed);
  await session.extensionRunner.emit({
    type: "session_shutdown",
    reason: "quit",
  });
  console.log(
    JSON.stringify({
      pi: "1.6.0",
      runtime: process.versions.bun
        ? `Bun ${process.versions.bun}`
        : process.version,
      projectTrust: true,
      projectLocalDiscovery: true,
      idleBusyFollowUp: true,
      dedupe: true,
      literalSlashText: true,
      readOnlyLogTool: true,
      treeRevocation: true,
      shutdown: true,
      paidCalls: 0,
      userFileWrites: 0,
    }),
  );
} finally {
  await session.extensionRunner.emit({
    type: "session_shutdown",
    reason: "quit",
  });
  session.dispose();
  for (const s of sockets) s.destroy();
  await new Promise<void>((r) => server.close(() => r()));
}
