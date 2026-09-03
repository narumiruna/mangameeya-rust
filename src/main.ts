import "./styles.css";
import { distantPageIndices, loadOnce, transformedPageSize } from "./continuous-pages";
import { baseName, clamp, escapeAttribute, escapeHtml, formatBytes, readJson } from "./utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";

type FitMode = "window" | "width" | "height" | "original" | "custom";
type SidebarMode = "files" | "thumbs" | "recent";

interface BookInfo {
  title: string;
  path: string;
  pageNames: string[];
  initialPage: number;
}

interface PageData {
  index: number;
  name: string;
  mime: string;
  dataUrl: string;
  byteSize: number;
}

interface DirectoryEntry {
  name: string;
  path: string;
  isDirectory: boolean;
  kind: "folder" | "archive" | "image";
}

interface DirectoryListing {
  path: string;
  parent: string | null;
  entries: DirectoryEntry[];
}

interface Preferences {
  spread: boolean;
  continuous: boolean;
  rtl: boolean;
  fit: FitMode;
  sidebar: boolean;
  sidebarMode: SidebarMode;
  background: "black" | "gray" | "paper";
  recursiveFolder: boolean;
  rememberPosition: boolean;
  interval: number;
}

const defaultPreferences: Preferences = {
  spread: false,
  continuous: false,
  rtl: true,
  fit: "window",
  sidebar: true,
  sidebarMode: "thumbs",
  background: "black",
  recursiveFolder: true,
  rememberPosition: true,
  interval: 5,
};

const preferences: Preferences = {
  ...defaultPreferences,
  ...readJson<Partial<Preferences>>("mmr-preferences", {}),
};

const state = {
  book: null as BookInfo | null,
  current: 0,
  zoom: 1,
  rotation: 0,
  brightness: 100,
  contrast: 100,
  grayscale: 0,
  slideshow: 0 as ReturnType<typeof setInterval> | 0,
  loadingToken: 0,
  bookGeneration: 0,
  cache: new Map<number, PageData>(),
  pageRequests: new Map<number, Promise<PageData>>(),
  listing: null as DirectoryListing | null,
};

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
  <div class="app-shell background-${preferences.background}">
    <header class="chrome">
      <nav class="menubar" aria-label="主選單">
        ${menu("檔案(&F)", [
          ["open-file", "開啟檔案…", "Ctrl+O"],
          ["open-folder", "開啟資料夾…", "Ctrl+Shift+O"],
          ["save-page", "另存目前圖片…", "Ctrl+S"],
          ["separator", "", ""],
          ["close-book", "關閉漫畫", "Ctrl+W"],
          ["quit", "結束", "Alt+F4"],
        ])}
        ${menu("檢視(&V)", [
          ["toggle-sidebar", "顯示側欄", "Tab"],
          ["mode-thumbs", "縮圖模式", "T"],
          ["toggle-spread", "雙頁跨頁", "S"],
          ["toggle-continuous", "連續頁面", "C"],
          ["toggle-rtl", "右向左閱讀", "D"],
          ["separator", "", ""],
          ["fullscreen", "全螢幕", "F11"],
        ])}
        ${menu("縮放(&S)", [
          ["fit-window", "適合視窗", "F"],
          ["fit-width", "適合寬度", "W"],
          ["fit-height", "適合高度", "H"],
          ["fit-original", "原始大小", "1"],
          ["separator", "", ""],
          ["zoom-in", "放大", "+"],
          ["zoom-out", "縮小", "−"],
          ["zoom-reset", "重設縮放", "0"],
        ])}
        ${menu("移動(&M)", [
          ["first", "第一頁", "Home"],
          ["previous", "上一頁", "←"],
          ["next", "下一頁", "→ / Space"],
          ["last", "最後一頁", "End"],
          ["separator", "", ""],
          ["toggle-slideshow", "開始／停止投影片", "P"],
        ])}
        ${menu("書籤(&B)", [
          ["bookmark", "加入／更新書籤", "B"],
          ["resume", "跳至書籤", "Shift+B"],
          ["clear-bookmark", "刪除本書書籤", ""],
        ])}
        ${menu("工具(&T)", [
          ["rotate-left", "向左旋轉", "["],
          ["rotate-right", "向右旋轉", "]"],
          ["reset-filter", "重設影像濾鏡", ""],
          ["separator", "", ""],
          ["settings", "偏好設定…", ""],
        ])}
        ${menu("說明(&H)", [["about", "關於 MangaMeeya Rust", ""]])}
      </nav>
      <div class="toolbar" role="toolbar" aria-label="閱讀工具">
        ${tool("open-file", "📂", "開啟漫畫")}
        ${tool("previous", "◀", "上一頁")}
        ${tool("next", "▶", "下一頁")}
        <span class="tool-separator"></span>
        ${tool("toggle-spread", "▣", "單頁／雙頁")}
        ${tool("toggle-continuous", "▤", "連續頁面")}
        ${tool("toggle-rtl", "R⇄L", "閱讀方向")}
        ${tool("fit-window", "⊡", "適合視窗")}
        ${tool("fit-width", "↔", "適合寬度")}
        ${tool("fit-original", "1:1", "原始大小")}
        ${tool("zoom-out", "−", "縮小")}
        <output id="zoom-output" class="zoom-output">100%</output>
        ${tool("zoom-in", "+", "放大")}
        <span class="tool-separator"></span>
        ${tool("rotate-left", "↶", "向左旋轉")}
        ${tool("rotate-right", "↷", "向右旋轉")}
        ${tool("bookmark", "★", "書籤")}
        ${tool("toggle-slideshow", "▷", "投影片")}
        ${tool("fullscreen", "⛶", "全螢幕")}
      </div>
    </header>

    <main class="workspace">
      <aside id="sidebar" class="sidebar ${preferences.sidebar ? "" : "hidden"}">
        <div class="sidebar-tabs" role="tablist">
          <button data-sidebar="files">檔案</button>
          <button data-sidebar="thumbs">縮圖</button>
          <button data-sidebar="recent">歷史</button>
        </div>
        <section id="sidebar-content" class="sidebar-content"></section>
      </aside>

      <section id="viewer" class="viewer" tabindex="0" aria-label="漫畫閱讀區">
        <div id="empty-state" class="empty-state">
          <img src="/app-icon.svg" alt="" />
          <h1>MangaMeeya Rust</h1>
          <p>將漫畫拖放到這裡，或開啟圖片資料夾與封存檔。</p>
          <div class="empty-actions">
            <button class="primary" data-action="open-file">開啟漫畫</button>
            <button data-action="open-folder">開啟資料夾</button>
          </div>
          <small>JPG · PNG · GIF · BMP · WebP · ZIP/CBZ · RAR/CBR</small>
        </div>
        <div id="loading" class="loading hidden"><span></span><p>正在讀取圖片…</p></div>
        <div id="pages" class="pages hidden" aria-live="polite"></div>
        <button class="page-zone page-zone-left" data-zone="left" aria-label="左側翻頁"></button>
        <button class="page-zone page-zone-right" data-zone="right" aria-label="右側翻頁"></button>
        <div id="drop-overlay" class="drop-overlay hidden"><strong>放開以開啟</strong></div>
      </section>

      <aside class="filter-panel" aria-label="影像調整">
        <h2>影像調整</h2>
        ${range("brightness", "亮度", 50, 150, state.brightness)}
        ${range("contrast", "對比", 50, 150, state.contrast)}
        ${range("grayscale", "灰階", 0, 100, state.grayscale)}
        <button data-action="reset-filter">重設</button>
      </aside>
    </main>

    <footer class="statusbar">
      <span id="status-message">就緒</span>
      <span id="status-name">尚未開啟漫畫</span>
      <label class="page-jump">頁碼 <input id="page-input" type="number" min="1" value="1" disabled /> <span id="page-total">/ 0</span></label>
      <input id="page-slider" class="page-slider" type="range" min="1" max="1" value="1" disabled aria-label="頁面位置" />
      <span id="status-size">—</span>
      <span id="status-mode">單頁 · R→L</span>
    </footer>
  </div>

  <dialog id="settings-dialog">
    <form method="dialog" class="dialog-card">
      <header><h2>偏好設定</h2><button value="cancel" aria-label="關閉">×</button></header>
      <label><input id="pref-remember" type="checkbox" /> 記住每本漫畫的閱讀位置</label>
      <label>投影片間隔 <input id="pref-interval" type="number" min="1" max="120" /> 秒</label>
      <label>閱讀區背景
        <select id="pref-background"><option value="black">黑色</option><option value="gray">深灰</option><option value="paper">紙色</option></select>
      </label>
      <p class="hint">滑鼠滾輪會捲動閱讀區；按住 Ctrl 並滾動可縮放。</p>
      <footer><button value="cancel">取消</button><button id="save-settings" value="default" class="primary">儲存</button></footer>
    </form>
  </dialog>

  <dialog id="about-dialog">
    <form method="dialog" class="dialog-card about-card">
      <img src="/app-icon.svg" alt="" />
      <h2>MangaMeeya Rust</h2>
      <p>以 Rust 與 Tauri 2 重製的 Windows 漫畫閱讀器。</p>
      <p class="hint">快速、離線、鍵盤優先。原始圖片永不被修改。</p>
      <button class="primary">確定</button>
    </form>
  </dialog>

  <div id="toast" class="toast hidden" role="status"></div>
`;

function menu(label: string, items: string[][]): string {
  return `<div class="menu"><button class="menu-label">${label}</button><div class="menu-popup">${items
    .map(([action, text, shortcut]) =>
      action === "separator"
        ? `<hr />`
        : `<button data-action="${action}"><span>${text}</span><kbd>${shortcut}</kbd></button>`,
    )
    .join("")}</div></div>`;
}

function tool(action: string, icon: string, title: string): string {
  return `<button class="tool-button" data-action="${action}" title="${title}" aria-label="${title}">${icon}</button>`;
}

function range(id: string, label: string, min: number, max: number, value: number): string {
  return `<label class="filter-control"><span>${label}<output id="${id}-value">${value}%</output></span><input id="${id}" type="range" min="${min}" max="${max}" value="${value}" /></label>`;
}

const shell = document.querySelector<HTMLDivElement>(".app-shell")!;
const viewer = document.querySelector<HTMLElement>("#viewer")!;
const pagesElement = document.querySelector<HTMLDivElement>("#pages")!;
const emptyState = document.querySelector<HTMLDivElement>("#empty-state")!;
const loading = document.querySelector<HTMLDivElement>("#loading")!;
const sidebar = document.querySelector<HTMLElement>("#sidebar")!;
const sidebarContent = document.querySelector<HTMLElement>("#sidebar-content")!;
const pageInput = document.querySelector<HTMLInputElement>("#page-input")!;
const pageSlider = document.querySelector<HTMLInputElement>("#page-slider")!;
const pageTotal = document.querySelector<HTMLElement>("#page-total")!;
const statusMessage = document.querySelector<HTMLElement>("#status-message")!;
const statusName = document.querySelector<HTMLElement>("#status-name")!;
const statusSize = document.querySelector<HTMLElement>("#status-size")!;
const statusMode = document.querySelector<HTMLElement>("#status-mode")!;
const zoomOutput = document.querySelector<HTMLOutputElement>("#zoom-output")!;
const toast = document.querySelector<HTMLDivElement>("#toast")!;

let toastTimer = 0;
let continuousObserver: IntersectionObserver | null = null;
let continuousScrollFrame = 0;
function notify(message: string, error = false): void {
  window.clearTimeout(toastTimer);
  toast.textContent = message;
  toast.className = `toast ${error ? "error" : ""}`;
  toastTimer = window.setTimeout(() => toast.classList.add("hidden"), 3200);
}

function persistPreferences(): void {
  localStorage.setItem("mmr-preferences", JSON.stringify(preferences));
}

function recentBooks(): string[] {
  return readJson<string[]>("mmr-recent", []);
}

function addRecent(path: string): void {
  const recent = [path, ...recentBooks().filter((item) => item !== path)].slice(0, 15);
  localStorage.setItem("mmr-recent", JSON.stringify(recent));
}

function positions(): Record<string, number> {
  return readJson<Record<string, number>>("mmr-positions", {});
}

function bookmarks(): Record<string, number> {
  return readJson<Record<string, number>>("mmr-bookmarks", {});
}

async function chooseFile(): Promise<void> {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "漫畫與圖片", extensions: ["jpg", "jpeg", "jpe", "png", "gif", "bmp", "webp", "zip", "cbz", "rar", "cbr"] }],
  });
  if (typeof selected === "string") await openPath(selected);
}

async function chooseFolder(): Promise<void> {
  const selected = await open({ multiple: false, directory: true });
  if (typeof selected === "string") await openPath(selected);
}

async function openPath(path: string): Promise<void> {
  stopSlideshow();
  statusMessage.textContent = "正在開啟…";
  loading.classList.remove("hidden");
  emptyState.classList.add("hidden");
  pagesElement.classList.add("hidden");
  try {
    const book = await invoke<BookInfo>("open_book", { path });
    state.bookGeneration += 1;
    state.book = book;
    state.cache.clear();
    state.pageRequests.clear();
    const saved = positions()[book.path];
    state.current = preferences.rememberPosition && saved !== undefined
      ? clamp(saved, 0, book.pageNames.length - 1)
      : book.initialPage;
    state.zoom = 1;
    state.rotation = 0;
    addRecent(book.path);
    await getCurrentWindow().setTitle(`${book.title} — MangaMeeya Rust`);
    updateControls();
    await renderPages();
    renderSidebar();
    notify(`已開啟 ${book.title}（${book.pageNames.length} 頁）`);
  } catch (error) {
    state.book = null;
    emptyState.classList.remove("hidden");
    pagesElement.classList.add("hidden");
    notify(String(error), true);
    statusMessage.textContent = "開啟失敗";
  } finally {
    loading.classList.add("hidden");
  }
}

async function closeBook(): Promise<void> {
  stopSlideshow();
  continuousObserver?.disconnect();
  continuousObserver = null;
  viewer.classList.remove("continuous-mode");
  await invoke("close_book");
  state.bookGeneration += 1;
  state.book = null;
  state.cache.clear();
  state.pageRequests.clear();
  pagesElement.replaceChildren();
  pagesElement.classList.add("hidden");
  emptyState.classList.remove("hidden");
  await getCurrentWindow().setTitle("MangaMeeya Rust");
  updateControls();
  renderSidebar();
}

function visibleIndices(): number[] {
  if (!state.book) return [];
  const indices = [state.current];
  if (!preferences.continuous && preferences.spread && state.current + 1 < state.book.pageNames.length) indices.push(state.current + 1);
  return indices;
}

async function pageData(index: number): Promise<PageData> {
  const cached = state.cache.get(index);
  if (cached) return cached;
  const generation = state.bookGeneration;
  const data = await loadOnce(state.pageRequests, index, () => invoke<PageData>("get_page", { index }));
  if (generation !== state.bookGeneration) return data;
  state.cache.set(index, data);
  if (state.cache.size > 24) {
    const keep = new Set([...visibleIndices(), index]);
    const old = [...state.cache.keys()].find((key) => !keep.has(key));
    if (old !== undefined) state.cache.delete(old);
  }
  return data;
}

function pageTransform(centered = false): string {
  const position = centered ? "translate(-50%, -50%) " : "";
  return `${position}rotate(${state.rotation}deg) scale(${preferences.fit === "custom" ? state.zoom : 1})`;
}

function createPageImage(page: PageData): HTMLImageElement {
  const image = document.createElement("img");
  image.src = page.dataUrl;
  image.alt = `第 ${page.index + 1} 頁：${page.name}`;
  image.draggable = false;
  image.style.transform = pageTransform();
  image.style.filter = `brightness(${state.brightness}%) contrast(${state.contrast}%) grayscale(${state.grayscale}%)`;
  return image;
}

function layoutContinuousPage(frame: HTMLElement, image: HTMLImageElement): void {
  const width = image.offsetWidth;
  const height = image.offsetHeight;
  const scale = preferences.fit === "custom" ? state.zoom : 1;
  const layout = transformedPageSize(width, height, state.rotation, scale);
  const content = document.createElement("div");
  content.className = "continuous-page-content";
  content.style.width = `${layout.width}px`;
  content.style.height = `${layout.height}px`;
  image.classList.add("continuous-page-image");
  image.style.width = `${width}px`;
  image.style.height = `${height}px`;
  image.style.transform = pageTransform(true);
  content.append(image);
  frame.replaceChildren(content);
  frame.style.removeProperty("height");
  frame.classList.remove("page-pending", "page-error");
  frame.classList.add("loaded");
}

function createPagePlaceholder(index: number): HTMLElement {
  const placeholder = document.createElement("span");
  placeholder.className = "page-placeholder";
  placeholder.textContent = String(index + 1);
  return placeholder;
}

async function renderPages(): Promise<void> {
  if (!state.book) return;
  const token = ++state.loadingToken;
  continuousObserver?.disconnect();
  continuousObserver = null;
  loading.classList.remove("hidden");
  pagesElement.classList.add("hidden");
  try {
    let ready = true;
    if (preferences.continuous) ready = await renderContinuousPages(token);
    else await renderPagedPages(token);
    if (token !== state.loadingToken) return;
    if (ready) statusMessage.textContent = "就緒";
    rememberPosition();
    updateControls();
    preloadNearby();
  } catch (error) {
    notify(`無法顯示頁面：${String(error)}`, true);
    statusMessage.textContent = "讀取失敗";
  } finally {
    if (token === state.loadingToken) loading.classList.add("hidden");
  }
}

async function renderPagedPages(token: number): Promise<void> {
  const data = await Promise.all(visibleIndices().map(pageData));
  if (token !== state.loadingToken) return;
  pagesElement.replaceChildren(
    ...data.map((page) => {
      const wrapper = document.createElement("figure");
      wrapper.className = "page-frame";
      wrapper.append(createPageImage(page));
      return wrapper;
    }),
  );
  pagesElement.className = `pages fit-${preferences.fit} ${preferences.spread ? "spread" : "single"} ${preferences.rtl ? "rtl" : "ltr"}`;
  viewer.classList.remove("continuous-mode");
  viewer.scrollTo({ top: 0, left: 0 });
  statusSize.textContent = formatBytes(data.reduce((sum, page) => sum + page.byteSize, 0));
  statusName.textContent = data.map((page) => page.name).join("  ·  ");
}

async function renderContinuousPages(token: number): Promise<boolean> {
  if (!state.book) return false;
  const frames = state.book.pageNames.map((name, index) => {
    const frame = document.createElement("figure");
    frame.className = "page-frame page-pending";
    frame.dataset.continuousPage = String(index);
    frame.setAttribute("aria-label", `第 ${index + 1} 頁：${name}`);
    frame.append(createPagePlaceholder(index));
    return frame;
  });
  pagesElement.replaceChildren(...frames);
  pagesElement.className = `pages continuous fit-${preferences.fit}`;
  viewer.classList.add("continuous-mode");
  viewer.scrollTop = frames[state.current]?.offsetTop ?? 0;
  updateContinuousStatus(state.current);

  continuousObserver = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const frame = entry.target as HTMLElement;
      void loadContinuousPage(frame, Number(frame.dataset.continuousPage), token);
    }
  }, { root: viewer, rootMargin: "100% 0px" });
  frames.forEach((frame) => continuousObserver?.observe(frame));
  return loadContinuousPage(frames[state.current], state.current, token);
}

async function loadContinuousPage(frame: HTMLElement | undefined, index: number, token: number): Promise<boolean> {
  if (!frame) return false;
  if (frame.dataset.loading === "true" || frame.classList.contains("loaded")) return true;
  frame.dataset.loading = "true";
  try {
    const page = await pageData(index);
    if (token !== state.loadingToken || !frame.isConnected) return false;
    const image = createPageImage(page);
    const finishLoading = () => {
      if (token !== state.loadingToken || !frame.isConnected || frame.classList.contains("loaded")) return;
      layoutContinuousPage(frame, image);
      unloadDistantContinuousPages(state.current);
    };
    image.addEventListener("load", finishLoading, { once: true });
    frame.replaceChildren(image);
    if (image.complete) finishLoading();
    if (index === state.current) {
      statusName.textContent = page.name;
      statusSize.textContent = formatBytes(page.byteSize);
      statusMessage.textContent = "就緒";
    }
    return true;
  } catch (error) {
    if (token !== state.loadingToken || !frame.isConnected) return false;
    frame.classList.add("page-error");
    frame.replaceChildren(document.createTextNode(`第 ${index + 1} 頁讀取失敗`));
    if (index === state.current) {
      updateContinuousStatus(index, "讀取失敗");
    }
    notify(`無法顯示第 ${index + 1} 頁：${String(error)}`, true);
    return false;
  } finally {
    delete frame.dataset.loading;
  }
}

function updateContinuousStatus(index: number, message?: string): void {
  if (!state.book) return;
  const cached = state.cache.get(index);
  statusName.textContent = cached?.name ?? state.book.pageNames[index];
  statusSize.textContent = cached ? formatBytes(cached.byteSize) : "—";
  statusMessage.textContent = message ?? (cached ? "就緒" : "正在讀取…");
}

function updateContinuousPosition(): void {
  continuousScrollFrame = 0;
  if (!preferences.continuous || !state.book) return;
  const frames = pagesElement.querySelectorAll<HTMLElement>("[data-continuous-page]");
  if (!frames.length) return;
  const center = viewer.scrollTop + viewer.clientHeight / 2 - pagesElement.offsetTop;
  let low = 0;
  let high = frames.length - 1;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    const frame = frames[middle];
    if (center > frame.offsetTop + frame.offsetHeight) low = middle + 1;
    else high = middle;
  }
  const index = Number(frames[low].dataset.continuousPage);
  unloadDistantContinuousPages(index);
  if (index === state.current) return;
  state.current = index;
  updateContinuousStatus(index);
  rememberPosition();
  updateControls();
  highlightThumbnail();
  preloadNearby();
}

function unloadDistantContinuousPages(current: number): void {
  const frames = new Map(
    [...pagesElement.querySelectorAll<HTMLElement>(".page-frame.loaded")]
      .map((frame) => [Number(frame.dataset.continuousPage), frame] as const),
  );
  for (const index of distantPageIndices(frames.keys(), current, 8)) {
    const frame = frames.get(index)!;
    frame.style.height = `${frame.offsetHeight}px`;
    frame.classList.remove("loaded");
    frame.classList.add("page-pending");
    frame.replaceChildren(createPagePlaceholder(index));
  }
}

function preloadNearby(): void {
  if (!state.book) return;
  for (const index of [state.current - 1, state.current + 1, state.current + 2]) {
    if (index >= 0 && index < state.book.pageNames.length && !state.cache.has(index)) {
      void pageData(index).catch(() => undefined);
    }
  }
}

function rememberPosition(): void {
  if (!state.book || !preferences.rememberPosition) return;
  const saved = positions();
  saved[state.book.path] = state.current;
  localStorage.setItem("mmr-positions", JSON.stringify(saved));
}

function navigate(target: number): void {
  if (!state.book) return;
  const next = clamp(target, 0, state.book.pageNames.length - 1);
  if (next === state.current) return;
  state.current = next;
  if (preferences.continuous) {
    const frame = pagesElement.querySelector<HTMLElement>(`[data-continuous-page="${next}"]`);
    unloadDistantContinuousPages(next);
    updateContinuousStatus(next);
    frame?.scrollIntoView({ block: "start" });
    if (frame) void loadContinuousPage(frame, next, state.loadingToken);
    rememberPosition();
    updateControls();
    preloadNearby();
  } else {
    void renderPages();
  }
  highlightThumbnail();
}

function nextPage(): void {
  navigate(state.current + (preferences.spread ? 2 : 1));
}

function previousPage(): void {
  navigate(state.current - (preferences.spread ? 2 : 1));
}

function setFit(fit: FitMode): void {
  preferences.fit = fit;
  if (fit !== "custom") state.zoom = 1;
  persistPreferences();
  void renderPages();
}

function setZoom(factor: number): void {
  preferences.fit = "custom";
  state.zoom = clamp(Math.round(state.zoom * factor * 100) / 100, 0.1, 8);
  persistPreferences();
  void renderPages();
}

function updateControls(): void {
  const count = state.book?.pageNames.length ?? 0;
  pageInput.disabled = count === 0;
  pageSlider.disabled = count === 0;
  pageInput.max = String(Math.max(1, count));
  pageSlider.max = String(Math.max(1, count));
  pageInput.value = String(state.current + 1);
  pageSlider.value = String(state.current + 1);
  pageTotal.textContent = `/ ${count}`;
  zoomOutput.textContent = `${Math.round(state.zoom * 100)}%`;
  statusMode.textContent = `${preferences.continuous ? "連續" : preferences.spread ? "雙頁" : "單頁"} · ${preferences.rtl ? "R→L" : "L→R"}`;
  document.querySelectorAll<HTMLElement>("[data-action='toggle-spread']").forEach((element) => element.classList.toggle("active", preferences.spread));
  document.querySelectorAll<HTMLElement>("[data-action='toggle-continuous']").forEach((element) => element.classList.toggle("active", preferences.continuous));
  document.querySelectorAll<HTMLElement>("[data-action='toggle-rtl']").forEach((element) => element.classList.toggle("active", preferences.rtl));
  document.querySelectorAll<HTMLElement>("[data-action='toggle-slideshow']").forEach((element) => element.classList.toggle("active", Boolean(state.slideshow)));
  document.querySelectorAll<HTMLElement>("[data-sidebar]").forEach((element) => element.classList.toggle("active", element.dataset.sidebar === preferences.sidebarMode));
}

function renderSidebar(): void {
  updateControls();
  if (preferences.sidebarMode === "thumbs") renderThumbnails();
  else if (preferences.sidebarMode === "recent") renderRecent();
  else void renderFiles(state.listing?.path);
}

function renderThumbnails(): void {
  if (!state.book) {
    sidebarContent.innerHTML = `<div class="sidebar-empty">開啟漫畫後顯示縮圖</div>`;
    return;
  }
  sidebarContent.innerHTML = `<div class="thumbnail-list">${state.book.pageNames
    .map((name, index) => `<button class="thumbnail ${index === state.current ? "current" : ""}" data-page="${index}" title="${escapeHtml(name)}"><span class="thumb-image" data-thumb="${index}"><i>${index + 1}</i></span><span>${index + 1}. ${escapeHtml(baseName(name))}</span></button>`)
    .join("")}</div>`;
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const target = entry.target as HTMLElement;
      const index = Number(target.dataset.thumb);
      observer.unobserve(target);
      void pageData(index).then((page) => {
        if (!target.isConnected) return;
        const image = document.createElement("img");
        image.src = page.dataUrl;
        image.alt = "";
        target.replaceChildren(image);
      });
    }
  }, { root: sidebarContent, rootMargin: "240px" });
  document.querySelectorAll<HTMLElement>("[data-thumb]").forEach((element) => observer.observe(element));
  highlightThumbnail();
}

function highlightThumbnail(): void {
  document.querySelectorAll<HTMLElement>("[data-page]").forEach((element) => {
    const current = Number(element.dataset.page) === state.current;
    element.classList.toggle("current", current);
    if (current) element.scrollIntoView({ block: "nearest" });
  });
}

function renderRecent(): void {
  const books = recentBooks();
  sidebarContent.innerHTML = books.length
    ? `<div class="file-list recent-list">${books.map((path) => `<button data-open-path="${escapeAttribute(path)}"><span class="entry-icon">◷</span><span><strong>${escapeHtml(baseName(path))}</strong><small>${escapeHtml(path)}</small></span></button>`).join("")}</div><button class="clear-history" data-action="clear-history">清除歷史</button>`
    : `<div class="sidebar-empty">尚無開啟歷史</div>`;
}

async function renderFiles(path?: string): Promise<void> {
  sidebarContent.innerHTML = `<div class="sidebar-loading">正在讀取資料夾…</div>`;
  try {
    const listing = await invoke<DirectoryListing>("browse_directory", { path: path ?? null });
    state.listing = listing;
    sidebarContent.innerHTML = `<div class="pathbar"><button ${listing.parent ? "" : "disabled"} data-browse-path="${escapeAttribute(listing.parent ?? "")}">↑</button><input value="${escapeAttribute(listing.path)}" aria-label="目前資料夾" /></div><div class="file-list">${listing.entries.map((entry) => `<button ${entry.isDirectory ? `data-browse-path="${escapeAttribute(entry.path)}"` : `data-open-path="${escapeAttribute(entry.path)}"`}><span class="entry-icon">${entry.kind === "folder" ? "📁" : entry.kind === "archive" ? "▰" : "▧"}</span><span><strong>${escapeHtml(entry.name)}</strong><small>${entry.kind === "folder" ? "資料夾" : entry.kind === "archive" ? "漫畫封存檔" : "圖片"}</small></span></button>`).join("")}</div>`;
    const pathInput = sidebarContent.querySelector<HTMLInputElement>(".pathbar input");
    pathInput?.addEventListener("keydown", (event) => {
      if (event.key === "Enter") void renderFiles(pathInput.value);
    });
  } catch (error) {
    sidebarContent.innerHTML = `<div class="sidebar-empty error-text">${escapeHtml(String(error))}</div>`;
  }
}

async function saveCurrentPage(): Promise<void> {
  if (!state.book) return;
  const name = baseName(state.book.pageNames[state.current]);
  const destination = await save({ defaultPath: name });
  if (destination) {
    await invoke("save_page", { index: state.current, destination });
    notify(`已儲存 ${baseName(destination)}`);
  }
}

function toggleBookmark(): void {
  if (!state.book) return;
  const saved = bookmarks();
  saved[state.book.path] = state.current;
  localStorage.setItem("mmr-bookmarks", JSON.stringify(saved));
  notify(`已將書籤設在第 ${state.current + 1} 頁`);
}

function resumeBookmark(): void {
  if (!state.book) return;
  const page = bookmarks()[state.book.path];
  if (page === undefined) notify("本書尚無書籤", true);
  else navigate(page);
}

function clearBookmark(): void {
  if (!state.book) return;
  const saved = bookmarks();
  delete saved[state.book.path];
  localStorage.setItem("mmr-bookmarks", JSON.stringify(saved));
  notify("已刪除本書書籤");
}

function toggleSlideshow(): void {
  if (state.slideshow) {
    stopSlideshow();
    notify("已停止投影片");
    return;
  }
  if (!state.book) return;
  state.slideshow = window.setInterval(() => {
    if (state.book && state.current < state.book.pageNames.length - 1) nextPage();
    else stopSlideshow();
  }, preferences.interval * 1000);
  updateControls();
  notify(`每 ${preferences.interval} 秒自動翻頁`);
}

function stopSlideshow(): void {
  if (state.slideshow) window.clearInterval(state.slideshow);
  state.slideshow = 0;
  updateControls();
}

async function action(name: string): Promise<void> {
  closeMenus();
  switch (name) {
    case "open-file": await chooseFile(); break;
    case "open-folder": await chooseFolder(); break;
    case "save-page": await saveCurrentPage(); break;
    case "close-book": await closeBook(); break;
    case "quit": await getCurrentWindow().close(); break;
    case "toggle-sidebar":
      preferences.sidebar = !preferences.sidebar;
      sidebar.classList.toggle("hidden", !preferences.sidebar);
      persistPreferences();
      break;
    case "mode-thumbs": preferences.sidebarMode = "thumbs"; preferences.sidebar = true; sidebar.classList.remove("hidden"); persistPreferences(); renderSidebar(); break;
    case "toggle-spread":
      preferences.spread = !preferences.spread;
      if (preferences.spread) preferences.continuous = false;
      persistPreferences();
      await renderPages();
      break;
    case "toggle-continuous":
      preferences.continuous = !preferences.continuous;
      if (preferences.continuous) preferences.spread = false;
      persistPreferences();
      await renderPages();
      break;
    case "toggle-rtl": preferences.rtl = !preferences.rtl; persistPreferences(); await renderPages(); break;
    case "fullscreen": await getCurrentWindow().setFullscreen(!(await getCurrentWindow().isFullscreen())); break;
    case "fit-window": setFit("window"); break;
    case "fit-width": setFit("width"); break;
    case "fit-height": setFit("height"); break;
    case "fit-original": setFit("original"); break;
    case "zoom-in": setZoom(1.15); break;
    case "zoom-out": setZoom(1 / 1.15); break;
    case "zoom-reset": state.zoom = 1; setFit("custom"); break;
    case "first": navigate(0); break;
    case "previous": previousPage(); break;
    case "next": nextPage(); break;
    case "last": if (state.book) navigate(state.book.pageNames.length - 1); break;
    case "toggle-slideshow": toggleSlideshow(); break;
    case "bookmark": toggleBookmark(); break;
    case "resume": resumeBookmark(); break;
    case "clear-bookmark": clearBookmark(); break;
    case "rotate-left": state.rotation = (state.rotation - 90) % 360; await renderPages(); break;
    case "rotate-right": state.rotation = (state.rotation + 90) % 360; await renderPages(); break;
    case "reset-filter": resetFilters(); break;
    case "settings": showSettings(); break;
    case "about": document.querySelector<HTMLDialogElement>("#about-dialog")!.showModal(); break;
    case "clear-history": localStorage.removeItem("mmr-recent"); renderRecent(); break;
  }
}

function resetFilters(): void {
  state.brightness = 100;
  state.contrast = 100;
  state.grayscale = 0;
  for (const id of ["brightness", "contrast", "grayscale"] as const) {
    const input = document.querySelector<HTMLInputElement>(`#${id}`)!;
    input.value = String(state[id]);
    document.querySelector<HTMLOutputElement>(`#${id}-value`)!.textContent = `${state[id]}%`;
  }
  void renderPages();
}

function showSettings(): void {
  const dialog = document.querySelector<HTMLDialogElement>("#settings-dialog")!;
  document.querySelector<HTMLInputElement>("#pref-remember")!.checked = preferences.rememberPosition;
  document.querySelector<HTMLInputElement>("#pref-interval")!.value = String(preferences.interval);
  document.querySelector<HTMLSelectElement>("#pref-background")!.value = preferences.background;
  dialog.showModal();
}

function closeMenus(): void {
  document.querySelectorAll(".menu.open").forEach((element) => element.classList.remove("open"));
}

app.addEventListener("click", (event) => {
  const target = event.target as HTMLElement;
  const menuLabel = target.closest<HTMLElement>(".menu-label");
  if (menuLabel) {
    const parent = menuLabel.parentElement!;
    const open = parent.classList.contains("open");
    closeMenus();
    parent.classList.toggle("open", !open);
    return;
  }
  const actionElement = target.closest<HTMLElement>("[data-action]");
  if (actionElement) void action(actionElement.dataset.action!);
  const sidebarTab = target.closest<HTMLElement>("[data-sidebar]");
  if (sidebarTab) {
    preferences.sidebarMode = sidebarTab.dataset.sidebar as SidebarMode;
    persistPreferences();
    renderSidebar();
  }
  const page = target.closest<HTMLElement>("[data-page]");
  if (page) navigate(Number(page.dataset.page));
  const openElement = target.closest<HTMLElement>("[data-open-path]");
  if (openElement) void openPath(openElement.dataset.openPath!);
  const browse = target.closest<HTMLElement>("[data-browse-path]");
  if (browse && !browse.hasAttribute("disabled")) void renderFiles(browse.dataset.browsePath);
  const zone = target.closest<HTMLElement>("[data-zone]");
  if (zone) {
    const left = zone.dataset.zone === "left";
    if (left === preferences.rtl) nextPage(); else previousPage();
  }
  if (!target.closest(".menu")) closeMenus();
});

for (const id of ["brightness", "contrast", "grayscale"] as const) {
  document.querySelector<HTMLInputElement>(`#${id}`)!.addEventListener("input", (event) => {
    state[id] = Number((event.target as HTMLInputElement).value);
    document.querySelector<HTMLOutputElement>(`#${id}-value`)!.textContent = `${state[id]}%`;
    const images = pagesElement.querySelectorAll<HTMLImageElement>("img");
    images.forEach((image) => image.style.filter = `brightness(${state.brightness}%) contrast(${state.contrast}%) grayscale(${state.grayscale}%)`);
  });
}

pageInput.addEventListener("change", () => navigate(Number(pageInput.value) - 1));
pageSlider.addEventListener("input", () => navigate(Number(pageSlider.value) - 1));
viewer.addEventListener("wheel", (event) => {
  if (!event.ctrlKey) return;
  event.preventDefault();
  setZoom(event.deltaY < 0 ? 1.1 : 1 / 1.1);
}, { passive: false });
viewer.addEventListener("scroll", () => {
  if (!preferences.continuous || continuousScrollFrame) return;
  continuousScrollFrame = window.requestAnimationFrame(updateContinuousPosition);
});

document.querySelector<HTMLButtonElement>("#save-settings")!.addEventListener("click", () => {
  preferences.rememberPosition = document.querySelector<HTMLInputElement>("#pref-remember")!.checked;
  preferences.interval = clamp(Number(document.querySelector<HTMLInputElement>("#pref-interval")!.value) || 5, 1, 120);
  preferences.background = document.querySelector<HTMLSelectElement>("#pref-background")!.value as Preferences["background"];
  shell.className = `app-shell background-${preferences.background}`;
  persistPreferences();
  if (state.slideshow) { stopSlideshow(); toggleSlideshow(); }
});

document.addEventListener("keydown", (event) => {
  const target = event.target as HTMLElement;
  if (target.matches("input, select") || (target.matches("button") && [" ", "Enter"].includes(event.key))) return;
  const key = event.key.toLowerCase();
  if (event.ctrlKey && event.shiftKey && key === "o") { event.preventDefault(); void chooseFolder(); return; }
  if (event.ctrlKey && key === "o") { event.preventDefault(); void chooseFile(); return; }
  if (event.ctrlKey && key === "s") { event.preventDefault(); void saveCurrentPage(); return; }
  if (event.ctrlKey && key === "w") { event.preventDefault(); void closeBook(); return; }
  if (event.key === "ArrowRight") preferences.rtl ? previousPage() : nextPage();
  else if (event.key === "ArrowLeft") preferences.rtl ? nextPage() : previousPage();
  else if (event.key === "PageDown" || event.key === " ") { event.preventDefault(); nextPage(); }
  else if (event.key === "PageUp") previousPage();
  else if (event.key === "Home") navigate(0);
  else if (event.key === "End" && state.book) navigate(state.book.pageNames.length - 1);
  else if (event.key === "F11") { event.preventDefault(); void action("fullscreen"); }
  else if (event.key === "Tab") { event.preventDefault(); void action("toggle-sidebar"); }
  else if (key === "s") void action("toggle-spread");
  else if (key === "c") void action("toggle-continuous");
  else if (key === "d") void action("toggle-rtl");
  else if (key === "f") void action("fit-window");
  else if (key === "w") void action("fit-width");
  else if (key === "h") void action("fit-height");
  else if (key === "1") void action("fit-original");
  else if (key === "+" || key === "=") void action("zoom-in");
  else if (key === "-") void action("zoom-out");
  else if (key === "0") void action("zoom-reset");
  else if (key === "[") void action("rotate-left");
  else if (key === "]") void action("rotate-right");
  else if (key === "p") void action("toggle-slideshow");
  else if (key === "b" && event.shiftKey) resumeBookmark();
  else if (key === "b") toggleBookmark();
  else if (event.key === "Escape") { closeMenus(); void getCurrentWindow().setFullscreen(false); }
});

void getCurrentWebview().onDragDropEvent((event) => {
  const overlay = document.querySelector<HTMLElement>("#drop-overlay")!;
  if (event.payload.type === "over") overlay.classList.remove("hidden");
  else if (event.payload.type === "drop") {
    overlay.classList.add("hidden");
    const path = event.payload.paths[0];
    if (path) void openPath(path);
  } else overlay.classList.add("hidden");
});

void listen<string[]>("open-args", (event) => {
  const path = event.payload.slice(1).find((candidate) => candidate && !candidate.startsWith("-"));
  if (path) void openPath(path);
});

window.addEventListener("unhandledrejection", (event) => {
  notify(String(event.reason), true);
  event.preventDefault();
});

preferences.sidebarMode = preferences.sidebarMode ?? "thumbs";
shell.className = `app-shell background-${preferences.background}`;
renderSidebar();
updateControls();
void invoke<string | null>("launch_path").then((path) => { if (path) void openPath(path); });
