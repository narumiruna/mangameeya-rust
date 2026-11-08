/* Shared mock library and interactions for the three sidebar studies. No disk access. */
(() => {
  "use strict";
  const variant = document.body.dataset.variant;
  const paths = {
    folder: '<path d="M2 5h5l2 2h13v13H2z"/>',
    book: '<rect x="4" y="3" width="16" height="18" rx="2"/><path d="M8 3v18M12 8h5M12 12h5"/>',
    image: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="8" r="2"/><path d="m3 18 6-6 4 4 3-3 5 5"/>',
    left: '<path d="m15 5-7 7 7 7"/>',
    right: '<path d="m9 5 7 7-7 7"/>',
    up: '<path d="m5 12 7-7 7 7M12 5v15"/>',
    locate: '<circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="2"/><path d="M12 2v3m0 14v3M2 12h3m14 0h3"/>',
    search: '<circle cx="10" cy="10" r="6"/><path d="m15 15 6 6"/>',
    edit: '<path d="m4 16 12-12 4 4L8 20H4zM13 7l4 4"/>',
    eye: '<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
    play: '<path d="m8 4 12 8-12 8z"/>',
    panel: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>',
    bookmark: '<path d="M6 3h12v18l-6-4-6 4z"/>',
    sliders: '<path d="M3 6h18M3 12h18M3 18h18M8 3v6m8 0v6m-6 0v6"/>',
    check: '<path d="m4 12 5 5L20 6"/>',
    close: '<path d="m6 6 12 12M6 18 18 6"/>'
  };
  const icon = (name) => `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true">${paths[name] || paths.book}</svg>`;
  const escape = (value) => String(value).replace(/[&<>"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[char]));
  const folders = {
    root: { name: "Comics", parent: null, path: "D:\\Comics", entries: ["night", "short", "scans", "empty", "hidden"] },
    night: { name: "夜行列車", parent: "root", path: "D:\\Comics\\夜行列車", entries: ["extras", "n01", "n02", "n03", "n04", "n05"] },
    extras: { name: "特典", parent: "night", path: "D:\\Comics\\夜行列車\\特典", entries: ["bonus"] },
    short: { name: "短篇集", parent: "root", path: "D:\\Comics\\短篇集", entries: ["s01", "s02"] },
    scans: { name: "原稿", parent: "root", path: "D:\\Comics\\原稿", entries: ["p01", "p02", "p03"] },
    empty: { name: "待整理", parent: "root", path: "D:\\Comics\\待整理", entries: [] },
    hidden: { name: ".metadata", parent: "root", path: "D:\\Comics\\.metadata", entries: [], hidden: true }
  };
  const books = {};
  for (let volume = 1; volume <= 5; volume++) {
    const number = String(volume).padStart(2, "0");
    const id = `n${number}`;
    books[id] = { id, name: `夜行列車 第 ${number} 卷.cbz`, title: `夜行列車 · 第 ${number} 卷`, parent: "night", pages: 48, size: "24.8 MB", saved: volume < 3 ? 48 : volume === 3 ? 18 : 1, type: "book" };
  }
  [
    ["s01", "雨後的書店.cbz", "short", 32, "16.2 MB"],
    ["s02", "星期日的散步.zip", "short", 28, "14.6 MB"],
    ["bonus", "夜行列車 特典 — 作者訪談與分鏡草稿（完整收藏版）.cbz", "extras", 24, "12.4 MB"],
    ["p01", "原稿 001.png", "scans", 1, "3.1 MB"],
    ["p02", "原稿 002.png", "scans", 1, "3.4 MB"],
    ["p03", "原稿 003.png", "scans", 1, "2.9 MB"]
  ].forEach(([id, name, parent, pages, size]) => {
    books[id] = { id, name, title: name.replace(/\.(cbz|zip|png)$/i, ""), parent, pages, size, saved: 1, type: id.startsWith("p") ? "image" : "book" };
  });
  const entries = { ...books };
  Object.entries(folders).forEach(([id, folder]) => { entries[id] = { ...folder, id, type: "folder" }; });
  const state = {
    folder: "night", selected: "n03", current: books.n03, page: 18,
    query: "", showHidden: false, expanded: new Set(["root", "night"]),
    back: [], snapshots: new Map(), recent: ["n03", "n02", "s01"],
    tab: "files", pathEditing: false, pathDraft: "", pathError: "", nextScroll: null
  };
  const panel = document.querySelector("#panel-files");
  const get = (selector) => document.querySelector(selector);
  const say = (message) => { get("#status-message").textContent = message; };
  const pathOf = (entry) => entry.type === "folder" ? entry.path : `${folders[entry.parent].path}\\${entry.name}`;
  const matches = (entry) => (state.showHidden || !entry.hidden) && entry.name.toLocaleLowerCase().includes(state.query.trim().toLocaleLowerCase());
  const children = (id) => folders[id].entries.map((key) => entries[key]).filter((entry) => state.showHidden || !entry.hidden);
  const filtered = (id) => {
    const items = children(id).filter(matches);
    if (variant !== "reading" || id !== state.current.parent) return items;
    const files = items.filter((entry) => entry.type !== "folder");
    const index = files.findIndex((entry) => entry.id === state.current.id);
    return index < 0 ? items : [...files.slice(index), ...files.slice(0, index), ...items.filter((entry) => entry.type === "folder")];
  };
  const subtreeMatches = (id) => {
    const entry = entries[id];
    return (state.showHidden || !entry.hidden) && (matches(entry) || (entry.type === "folder" && children(id).some((child) => subtreeMatches(child.id))));
  };
  const visibleIds = () => variant === "tree" ? [...panel.querySelectorAll(".entry-button")].filter((button) => !button.closest("[hidden]")).map((button) => button.dataset.entry) : filtered(state.folder).map((entry) => entry.id);
  const canOpenSelected = () => Boolean(books[state.selected] && visibleIds().includes(state.selected));
  const iconButton = (action, name, label, disabled = false) => `<button type="button" class="icon-button" data-action="${action}" id="action-${action}" title="${label}" aria-label="${label}" ${disabled ? "disabled" : ""}>${icon(name)}</button>`;

  function controls() {
    const folder = folders[state.folder];
    const location = state.pathEditing
      ? `<form class="path-form" id="path-form"><input id="path-editor" aria-label="資料夾完整路徑" aria-describedby="path-help${state.pathError ? " path-error" : ""}" aria-invalid="${Boolean(state.pathError)}" value="${escape(state.pathDraft)}" autocomplete="off" spellcheck="false"><span id="path-help" class="sr-only">輸入示範資料夾路徑後按 Enter；按 Escape 取消。</span><button class="icon-button" type="submit" aria-label="前往此路徑">${icon("check")}</button>${iconButton("cancel-path", "close", "取消路徑編輯")}</form>`
      : `<button class="location-button" id="location-button" data-action="edit-path" title="${escape(folder.path)} · 點擊編輯路徑">${icon("folder")}<span class="location-text">${folder.parent ? `${escape(folders[folder.parent].name)} <span aria-hidden="true"> / </span> ` : "D: / "}<strong>${escape(folder.name)}</strong></span>${icon("edit")}<span class="sr-only">，編輯資料夾路徑</span></button>`;
    return `<div class="folder-controls"><div class="${variant === "reading" ? "reading-location-row" : ""}"><div class="navigation-actions">${iconButton("back", "left", "返回上一個瀏覽位置", !state.back.length)}${iconButton("parent", "up", "上一層", !folder.parent)}${variant === "reading" ? "" : `<span class="navigation-divider"></span><button class="locate-button" id="locate-current" data-action="locate" title="定位目前閱讀的書籍">${icon("locate")}定位書籍</button>`}</div>${variant === "reading" ? location : ""}</div>${variant === "reading" ? "" : location}${state.pathError ? `<p class="path-error" id="path-error" role="alert">${escape(state.pathError)}</p>` : ""}<label class="search-box">${icon("search")}<input id="file-search" type="search" aria-label="${variant === "tree" ? "搜尋所有示範資料夾" : "搜尋目前資料夾"}" placeholder="${variant === "tree" ? "搜尋書籍與資料夾…" : "篩選這個資料夾…"}" value="${escape(state.query)}" autocomplete="off"></label></div>`;
  }

  function row(entry, tree = false) {
    const selected = entry.id === state.selected;
    const current = entry.id === state.current.id;
    const isFolder = entry.type === "folder";
    const metadata = isFolder ? `${children(entry.id).length} 個項目` : `${entry.pages} 頁 · ${entry.size}${entry.saved === entry.pages && entry.pages > 1 ? " · 已讀" : ""}`;
    return `<button class="entry-button${selected ? " is-selected" : ""}${current ? " is-current" : ""}${entry.hidden ? " is-hidden" : ""}${isFolder && entry.id === state.folder && tree ? " is-browsing" : ""}" data-entry="${entry.id}" title="${escape(pathOf(entry))}" ${!isFolder ? `aria-pressed="${selected}"` : ""} ${current ? 'aria-current="true"' : ""}><span class="entry-icon ${isFolder ? "folder" : ""}">${icon(isFolder ? "folder" : entry.type === "image" ? "image" : "book")}</span><span class="entry-text"><span class="entry-name">${escape(entry.name)}</span>${variant === "reading" ? `<span class="entry-meta">${metadata}</span>` : ""}</span>${current ? '<span class="entry-current">閱讀中</span>' : isFolder ? `<span class="folder-count">${children(entry.id).length}</span>` : ""}</button>`;
  }

  function treeBranch(id) {
    const entry = entries[id];
    if (state.query && !subtreeMatches(id)) return "";
    if (entry.type !== "folder") return `<li class="tree-item"><div class="tree-row"><span class="tree-spacer"></span>${row(entry, true)}</div></li>`;
    const expanded = state.expanded.has(id) || Boolean(state.query);
    return `<li class="tree-item"><div class="tree-row"><button class="disclosure" data-expand="${id}" aria-expanded="${expanded}" aria-controls="branch-${id}" aria-label="${expanded ? "收合" : "展開"} ${escape(entry.name)}" ${state.query ? 'disabled title="搜尋時保持結果展開"' : ""}>${icon("right")}</button>${row(entry, true)}</div><ul class="tree-children" id="branch-${id}" ${expanded ? "" : "hidden"}>${children(id).map((child) => treeBranch(child.id)).join("")}</ul></li>`;
  }

  function currentCard() {
    const book = state.current;
    const coverTitle = book.id.startsWith("n") ? "夜行<br>列車" : escape(book.title.slice(0, 4));
    return `<div class="reading-card"><div class="reading-card-label"><span class="current-dot"></span>正在閱讀</div><div class="reading-card-main"><div class="mini-cover" aria-hidden="true"><strong>${coverTitle}</strong><small>READING</small></div><div class="reading-card-copy"><strong title="${escape(book.title)}">${escape(book.title)}</strong><p>第 ${state.page} 頁 / ${book.pages} 頁</p><div class="card-progress"><span style="width:${Math.round(state.page / book.pages * 100)}%"></span></div><button class="card-locate" id="locate-current" data-action="locate">${icon("locate")}顯示所在位置</button></div></div></div><nav class="quick-places" aria-label="常用位置"><span>前往</span><button data-folder="root">Comics</button><button data-folder="short">短篇集</button><button data-folder="scans">原稿</button></nav>`;
  }

  function emptyState() {
    return `<div class="empty-state">${icon(state.query ? "search" : "folder")}<p>${state.query ? "找不到符合的項目。" : "此資料夾沒有可閱讀的檔案。"}</p><button data-action="${state.query ? "clear-search" : "parent"}">${state.query ? "清除搜尋" : "返回上一層"}</button></div>`;
  }

  function selectionFooter() {
    const entry = books[state.selected];
    return `<div class="selection-caption"><span>${entry && canOpenSelected() ? "已選取" : "選取書籍後再開始閱讀"}</span>${entry && canOpenSelected() ? `<span title="${escape(entry.name)}">${escape(entry.name)}</span>` : ""}</div><button class="primary-button" id="read-selected" data-action="read-selected" ${!canOpenSelected() ? "disabled" : ""}>${icon("play")}閱讀選取<kbd>Enter</kbd></button><button class="read-folder-button" data-action="read-folder" ${!children(state.folder).some((item) => item.type !== "folder") ? "disabled" : ""} title="把目前資料夾內的檔案作為一本書閱讀">閱讀此資料夾</button>`;
  }

  function captureFocus() {
    const active = document.activeElement;
    if (!panel.contains(active)) return null;
    const selector = active.id ? `#${CSS.escape(active.id)}` : active.dataset.entry ? `[data-entry="${active.dataset.entry}"]` : active.dataset.expand ? `[data-expand="${active.dataset.expand}"]` : active.dataset.action ? `[data-action="${active.dataset.action}"]` : null;
    return selector ? { selector, start: active.selectionStart, end: active.selectionEnd } : null;
  }

  function render() {
    const focus = captureFocus();
    const scroll = state.nextScroll ?? get("#file-list")?.scrollTop ?? 0;
    state.nextScroll = null;
    const list = filtered(state.folder);
    const count = list.length;
    panel.innerHTML = `${variant === "reading" ? currentCard() : ""}${controls()}${variant === "tree" ? `<p class="tree-hint">三角形展開目錄，名稱選擇位置。</p>${!children(state.folder).length ? '<p class="tree-hint" id="folder-empty">目前資料夾沒有可閱讀的檔案。</p>' : ""}` : ""}<div class="list-heading"><strong>${variant === "tree" ? "資料夾階層" : variant === "reading" && state.folder === state.current.parent ? "目前與接續閱讀" : "目前資料夾"}${variant !== "tree" ? ` · ${count}` : ""}</strong><button class="hidden-toggle" id="hidden-toggle" data-action="hidden" aria-pressed="${state.showHidden}" title="顯示或隱藏 dot 目錄">${icon("eye")}隱藏項目</button></div><div class="file-list ${variant === "tree" ? "tree-list" : ""}" id="file-list" aria-label="${variant === "tree" ? "可展開的資料夾導覽" : "檔案與資料夾"}">${variant === "tree" ? (subtreeMatches("root") ? `<ul class="entry-list">${treeBranch("root")}</ul>` : emptyState()) : (count ? `<ul class="entry-list">${list.map((entry) => `<li class="file-row">${row(entry)}</li>`).join("")}</ul>` : emptyState())}</div><div class="selection-footer" id="selection-footer"></div>`;
    get("#selection-footer").innerHTML = selectionFooter();
    get("#file-list").scrollTop = scroll;
    if (focus) {
      const active = panel.querySelector(focus.selector);
      if (active && !active.disabled && !active.closest("[hidden]")) {
        active.focus({ preventScroll: true });
        if (active instanceof HTMLInputElement && focus.start != null) active.setSelectionRange(focus.start, focus.end);
      } else {
        get("#location-button")?.focus({ preventScroll: true });
      }
    }
    updateReader();
  }

  function updateReader() {
    const book = state.current;
    const progress = Math.round(state.page / book.pages * 100);
    get("#window-title").textContent = `${book.name} — MangaMeeya Rust`;
    get("#canvas-title").textContent = book.title;
    get("#sidebar-reading-title").textContent = `正在閱讀 · ${book.title}`;
    get("#reader-page-label").textContent = `第 ${state.page} 頁 / ${book.pages} 頁`;
    get("#page-number").textContent = state.page;
    get("#page-total").textContent = book.pages;
    get("#reading-progress").style.width = `${progress}%`;
    get("#progress-value").textContent = `${progress}%`;
    get("#previous-page").disabled = state.page <= 1;
    get("#next-page").disabled = state.page >= book.pages;
  }

  function rememberFolder() {
    state.snapshots.set(state.folder, { selected: state.selected, query: state.query, scroll: get("#file-list")?.scrollTop || 0 });
  }

  function expandAncestors(id) {
    for (let current = id; current; current = folders[current].parent) state.expanded.add(current);
  }

  function browse(id, pushHistory = true) {
    if (!folders[id]) return;
    rememberFolder();
    if (pushHistory && state.folder !== id) state.back.push(state.folder);
    state.folder = id;
    const snapshot = state.snapshots.get(id);
    state.selected = snapshot?.selected ?? (state.current.parent === id ? state.current.id : null);
    state.query = snapshot?.query ?? "";
    state.nextScroll = snapshot?.scroll ?? 0;
    state.pathEditing = false;
    state.pathError = "";
    expandAncestors(id);
    render();
    say(`瀏覽 ${folders[id].name} · 閱讀畫面未變更`);
  }

  function select(id) {
    state.selected = id;
    panel.querySelectorAll(".entry-button").forEach((button) => {
      const selected = button.dataset.entry === id;
      button.classList.toggle("is-selected", selected);
      if (button.hasAttribute("aria-pressed")) button.setAttribute("aria-pressed", String(selected));
    });
    get("#selection-footer").innerHTML = selectionFooter();
    say(`已選取 ${books[id].name} · 雙擊或按 Enter 閱讀`);
  }

  function read(id) {
    const book = books[id];
    if (!book) return;
    state.current.saved = state.page;
    state.current = book;
    state.selected = id;
    state.page = book.saved;
    state.recent = [id, ...state.recent.filter((key) => key !== id)].slice(0, 8);
    render();
    renderAlternatePanels();
    say(`正在閱讀 ${book.name}`);
  }

  function locate() {
    setTab("files", false);
    browse(state.current.parent);
    state.query = "";
    state.selected = books[state.current.id] ? state.current.id : null;
    expandAncestors(state.current.parent);
    render();
    const button = panel.querySelector(`[data-entry="${state.current.id}"]`);
    button?.scrollIntoView({ block: "nearest" });
    button?.focus({ preventScroll: true });
    say(`已定位 ${state.current.name}`);
  }

  function turnPage(delta) {
    state.page = Math.max(1, Math.min(state.current.pages, state.page + delta));
    state.current.saved = state.page;
    if (variant === "reading") render(); else updateReader();
    renderAlternatePanels();
    say(`第 ${state.page} 頁，共 ${state.current.pages} 頁`);
  }

  function renderAlternatePanels() {
    const pages = Array.from(new Set([1, Math.max(1, state.page - 1), state.page, Math.min(state.current.pages, state.page + 1), state.current.pages])).sort((a, b) => a - b);
    get("#panel-thumbnails").innerHTML = `<div class="alt-panel"><h2>${state.current.pages} 頁</h2><p>示範首頁、末頁與目前頁附近的縮圖；所有圖片使用同一張原創示意圖。</p>${pages.map((page) => `<button class="thumbnail-button" data-page="${page}" ${page === state.page ? 'aria-current="page"' : ""}><img src="reader-art.svg" alt=""><span>第 ${page} 頁<small>${page === state.page ? "目前閱讀位置" : "點擊跳頁"}</small></span></button>`).join("")}</div>`;
    get("#panel-recent").innerHTML = `<div class="alt-panel"><h2>最近閱讀</h2><p>點擊書籍，繼續上次的閱讀位置。</p>${state.recent.map((id) => `<button class="recent-button" data-recent="${id}">${icon("book")}<span><strong>${escape(books[id].name)}</strong><small>${escape(folders[books[id].parent].path)} · 第 ${id === state.current.id ? state.page : books[id].saved} 頁</small></span></button>`).join("")}</div>`;
  }

  function setTab(tab, focus = true) {
    state.tab = tab;
    document.querySelectorAll("[data-tab]").forEach((button) => {
      const selected = button.dataset.tab === tab;
      button.setAttribute("aria-selected", String(selected));
      button.tabIndex = selected ? 0 : -1;
      get(`#panel-${button.dataset.tab}`).hidden = !selected;
    });
    renderAlternatePanels();
    if (focus) get(`#tab-${tab}`).focus();
  }

  const actions = {
    back: () => { const id = state.back.pop(); if (id) browse(id, false); },
    parent: () => { const id = folders[state.folder].parent; if (id) browse(id); },
    locate,
    hidden: () => { state.showHidden = !state.showHidden; render(); say(state.showHidden ? "已顯示隱藏項目" : "已隱藏 dot 目錄"); },
    "clear-search": () => { state.query = ""; render(); get("#file-search").focus(); },
    "edit-path": () => { state.pathEditing = true; state.pathDraft = folders[state.folder].path; render(); get("#path-editor").focus(); get("#path-editor").select(); },
    "cancel-path": () => { state.pathEditing = false; state.pathError = ""; render(); get("#location-button").focus(); },
    "read-selected": () => { if (canOpenSelected()) read(state.selected); },
    "read-folder": () => {
      const files = children(state.folder).filter((entry) => entry.type !== "folder");
      if (!files.length) return;
      state.current.saved = state.page;
      const folder = folders[state.folder];
      state.current = { id: `folder:${state.folder}`, name: folder.name, title: `${folder.name} · 資料夾書籍`, parent: state.folder, pages: files.reduce((total, file) => total + file.pages, 0), saved: 1 };
      state.page = 1;
      state.selected = null;
      render();
      renderAlternatePanels();
      say(`以資料夾書籍閱讀 ${folder.name} · ${state.current.pages} 頁`);
    }
  };

  document.addEventListener("click", (event) => {
    const button = event.target.closest("button");
    if (!button || button.disabled) return;
    if (button.dataset.action) actions[button.dataset.action]?.();
    else if (button.dataset.tab) setTab(button.dataset.tab);
    else if (button.dataset.folder) browse(button.dataset.folder);
    else if (button.dataset.expand) {
      const id = button.dataset.expand;
      if (state.expanded.has(id)) state.expanded.delete(id); else state.expanded.add(id);
      render();
    } else if (button.dataset.entry) {
      const entry = entries[button.dataset.entry];
      if (entry.type === "folder") browse(entry.id); else select(entry.id);
    } else if (button.dataset.recent) {
      const id = button.dataset.recent;
      read(id);
      get(`[data-recent="${id}"]`)?.focus();
    }
    else if (button.dataset.page) {
      state.page = Number(button.dataset.page);
      state.current.saved = state.page;
      render();
      renderAlternatePanels();
      // Keep focus on the chosen thumbnail after its markup is updated.
      get(`[data-page="${state.page}"]`)?.focus();
    }
  });

  document.addEventListener("dblclick", (event) => {
    const button = event.target.closest("[data-entry]");
    if (button && books[button.dataset.entry]) read(button.dataset.entry);
  });
  document.addEventListener("input", (event) => {
    if (event.target.id === "file-search") { state.query = event.target.value; state.nextScroll = 0; render(); }
    if (event.target.id === "path-editor") state.pathDraft = event.target.value;
  });
  document.addEventListener("submit", (event) => {
    if (event.target.id !== "path-form") return;
    event.preventDefault();
    const normalize = (value) => value.trim().replace(/^\\\\\?\\/, "").replace(/\//g, "\\").replace(/\\+$/, "").toLocaleLowerCase();
    const id = Object.keys(folders).find((key) => normalize(folders[key].path) === normalize(state.pathDraft));
    if (id) { browse(id); get("#location-button").focus(); }
    else { state.pathError = "找不到此示範路徑。可使用 D:\\Comics 或其子資料夾。"; render(); get("#path-editor").focus(); }
  });

  document.addEventListener("keydown", (event) => {
    const target = event.target;
    if (target.matches("input, textarea, [contenteditable=true]")) {
      if (target.id === "path-editor" && event.key === "Escape") { event.preventDefault(); actions["cancel-path"](); }
      return;
    }
    const tab = target.closest("[data-tab]");
    if (tab && ["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      const tabs = ["files", "thumbnails", "recent"];
      const index = tabs.indexOf(tab.dataset.tab);
      const next = event.key === "Home" ? 0 : event.key === "End" ? 2 : (index + (event.key === "ArrowRight" ? 1 : 2)) % 3;
      setTab(tabs[next]);
      return;
    }
    if (state.tab !== "files" || !panel.contains(target)) return;
    const button = target.closest("[data-entry]");
    if (event.key === "Enter" && button && books[button.dataset.entry]) { event.preventDefault(); read(button.dataset.entry); return; }
    if (event.key === "Backspace" || (event.altKey && event.key === "ArrowUp")) { event.preventDefault(); actions.parent(); get("#location-button")?.focus(); return; }
    if (!button) return;
    const visible = [...panel.querySelectorAll(".entry-button")].filter((item) => !item.closest("[hidden]"));
    const index = visible.indexOf(button);
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      const next = event.key === "Home" ? 0 : event.key === "End" ? visible.length - 1 : Math.max(0, Math.min(visible.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)));
      visible[next]?.focus();
      return;
    }
    if (variant === "tree" && ["ArrowLeft", "ArrowRight"].includes(event.key)) {
      const id = button.dataset.entry;
      event.preventDefault();
      const expanded = state.expanded.has(id) || Boolean(state.query);
      if (event.key === "ArrowRight") {
        if (!folders[id]) return;
        if (!expanded) { state.expanded.add(id); render(); }
        else panel.querySelector(`#branch-${id} .entry-button`)?.focus();
      } else if (folders[id] && expanded && !state.query) {
        state.expanded.delete(id);
        render();
      } else panel.querySelector(`[data-entry="${entries[id].parent}"]`)?.focus();
    }
  });
  get("#sidebar-toggle").addEventListener("click", () => {
    const sidebar = get("#sidebar");
    sidebar.hidden = !sidebar.hidden;
    get("#workspace").classList.toggle("sidebar-hidden", sidebar.hidden);
    get("#sidebar-toggle").setAttribute("aria-expanded", String(!sidebar.hidden));
  });
  get("#previous-page").addEventListener("click", () => turnPage(-1));
  get("#next-page").addEventListener("click", () => turnPage(1));
  document.querySelectorAll("[data-icon]").forEach((node) => { node.innerHTML = icon(node.dataset.icon); });
  render();
  renderAlternatePanels();
  panel.querySelector(`[data-entry="${state.current.id}"]`)?.scrollIntoView({ block: "nearest" });
})();
