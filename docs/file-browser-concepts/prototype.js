/* Offline demonstration only. No filesystem, network, or persistent storage access. */
(() => {
  "use strict";

  const $ = (selector) => document.querySelector(selector);
  const $$ = (selector) => [...document.querySelectorAll(selector)];
  const variant = document.body.dataset.variant;
  const app = $("#app");
  const browser = $("#browser");
  const reader = $("#reader");
  const scroller = $("#browser-scroll");
  const natural = new Intl.Collator("zh-Hant", { numeric: true, sensitivity: "base" });
  const literal = new Intl.Collator("zh-Hant", { numeric: false, sensitivity: "base" });
  const escape = (value) => String(value).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
  const paths = {
    search: '<circle cx="10" cy="10" r="6"/><path d="m15 15 5 5"/>',
    folder: '<path d="M3 6h7l2 3h9v11H3z"/><path d="M3 9V5h7l2 3"/>',
    file: '<path d="M5 3h9l5 5v13H5zM14 3v6h5M8 13h8M8 17h6"/>',
    image: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8" cy="8" r="1"/><path d="m3 17 5-5 4 4 4-6 5 7"/>',
    back: '<path d="m9 5-7 7 7 7M3 12h18"/>',
    play: '<path d="m8 4 12 8-12 8z"/>',
    left: '<path d="m15 5-7 7 7 7"/>',
    right: '<path d="m9 5 7 7-7 7"/>',
    ascending: '<path d="M6 20V4m-4 4 4-4 4 4M14 6h7M14 12h5M14 18h3"/>',
    descending: '<path d="M6 4v16m-4-4 4 4 4-4M14 6h3M14 12h5M14 18h7"/>',
    shuffle: '<path d="M3 6h3c5 0 7 12 12 12h3m-4-4 4 4-4 4M3 18h3c2 0 4-3 5-6m3-5c1-1 3-1 4-1h3m-4-4 4 4-4 4"/>',
    grid: '<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>',
    sidebar: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>',
    tools: '<path d="M4 6h16M4 12h16M4 18h16M8 3v6m8 0v6m-6 0v6"/>',
  };
  function icon(name, extraClass = "") {
    return `<svg class="icon ${extraClass}" viewBox="0 0 24 24" aria-hidden="true">${paths[name] || paths.file}</svg>`;
  }
  $$('[data-icon]').forEach((node) => { node.innerHTML = icon(node.dataset.icon); });

  const day = 86400000;
  const epoch = Date.UTC(2026, 10, 1);
  const folders = {
    root: { name: "Comics", parent: null },
    short: { name: "短篇集", parent: "root" },
    scans: { name: "掃描原稿", parent: "root" },
    empty: { name: "待整理", parent: "root" },
  };
  const titles = [
    "雨後的城市 1", "雨後的城市 2", "雨後的城市 10", "遠山通信 01", "海邊的星期日",
    "午夜電車 01", "植物觀察日記", "月球散步指南", "街角咖啡館 03", "旅途的風景",
    "遠山通信 02", "午夜電車 02", "玻璃溫室", "夏日慢車", "屋頂上的星圖",
    "沿海公路", "山間小屋", "城市的另一面", "一封沒有寄出的信", "雨聲採集簿",
    "星期六的書店", "空白的旅行手帳",
  ];
  const entries = Object.keys(folders).filter((id) => id !== "root").map((id, i) => ({
    id, directory: "root", name: folders[id].name, kind: "folder", type: "資料夾",
    size: null, pages: null, current: null, created: epoch - (i + 8) * day,
    modified: epoch + i * day, lastRead: null, art: i,
  }));
  titles.forEach((title, i) => {
    const type = ["CBZ", "CBZ", "CBZ", "CBR", "ZIP"][i % 5];
    const pages = i === 15 ? null : 32 + (i * 11) % 105;
    const total = pages || 32;
    entries.push({
      id: `book-${i + 1}`, directory: "root", name: `${title}.${type.toLowerCase()}`,
      title, kind: "archive", type, pages,
      size: i === 12 ? null : Math.round((18 + ((i * 47) % 190) + i / 10) * 1048576),
      current: i === 1 ? 18 : i % 5 === 0 ? total : i % 3 === 0 ? Math.floor(total * .37) : 0,
      modified: epoch + ((i * 7) % 20 - 14) * day,
      created: i === 19 ? null : epoch - (22 + (i * 13) % 180) * day,
      lastRead: i === 1 || i % 5 === 0 || i % 3 === 0 ? epoch + ((i * 3) % 8) * day : null,
      art: i,
    });
  });
  for (let i = 0; i < 6; i++) {
    entries.push({ ...entries.find((item) => item.id === `book-${i + 1}`),
      id: `short-${i + 1}`, directory: "short", name: `旅途短篇 ${i + 1}.cbz`,
      title: `旅途短篇 ${i + 1}`, type: "CBZ", art: i + 4,
    });
  }
  for (let i = 0; i < 4; i++) {
    entries.push({ id: `scan-${i}`, directory: "scans", name: `原稿 ${i + 1}.jpg`,
      title: `原稿 ${i + 1}`, kind: "image", type: "JPG", size: (i + 2) * 1048576,
      pages: 1, current: 0, modified: epoch + i * day, created: epoch - i * day,
      lastRead: null, art: i + 8,
    });
  }

  if (variant === "integrated") {
    for (const id of ["book-8", "book-18", "book-21"]) {
      const item = entries.find((entry) => entry.id === id);
      item.name = `[原創示範 (MangaMeeya Studio)] ${item.title} — 旅途特別篇 [繁體中文].${item.type.toLowerCase()}`;
    }
  }

  const state = {
    mode: "browser", folder: "root", selectedId: "book-2", query: "", collection: "all",
    sort: "natural", descending: false, foldersFirst: true, scrollTop: 0, shuffleSeed: 41,
  };
  const directoryStates = new Map();
  let visible = [];
  let currentBook = entries.find((item) => item.id === "book-2");
  let page = currentBook.current;
  let toastTimer;
  const artCache = new Map();
  const progress = (item) => item.kind !== "folder" && item.pages ? item.current / item.pages : null;
  const totalPages = (item) => item.pages || 32;
  const formatDate = (value) => value == null ? "—" : new Date(value).toISOString().slice(0, 10).replaceAll("-", "/");
  const formatSize = (value) => value == null ? "—" : `${(value / 1048576).toFixed(1)} MB`;
  const percentage = (item) => progress(item) == null ? "—" : `${Math.round(progress(item) * 100)}%`;
  const childCount = (item) => entries.filter((entry) => entry.directory === item.id).length;
  const normalize = (value) => value.normalize("NFKC").toLocaleLowerCase("zh-TW");
  const dataUri = (svg) => `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;

  // Original vector covers avoid external images and preserve portrait/landscape ratios.
  function cover(item) {
    if (artCache.has(item.id)) return artCache.get(item.id);
    const palettes = [
      ["#d9d9c5", "#637b72", "#344f4d", "#e1a976"],
      ["#c6d6da", "#7c9d9e", "#375665", "#efdbab"],
      ["#e9d8bb", "#ba8969", "#6b5c4f", "#c76247"],
      ["#233748", "#536878", "#b5c8be", "#e1c688"],
      ["#d5d8bd", "#82936c", "#3e604c", "#d4b278"],
      ["#d8c7b8", "#9c7974", "#5c535b", "#efe3c6"],
    ];
    const [bg, mid, ink, sun] = palettes[item.art % palettes.length];
    const scene = item.art % 6;
    const illustrations = [
      `<circle cx="235" cy="151" r="44" fill="${sun}"/><path d="M0 287V228h38v-57h38v83h36V146h48v141h30V207h36v80h28V185h46v102" fill="${ink}" opacity=".85"/><path d="M0 296h300M0 324h300M0 351h300M55 270v116m72-116v116m72-116v116m72-116v116" stroke="${mid}" stroke-width="2"/><path d="M135 389 194 284h24l-35 105" fill="${sun}" opacity=".45"/>`,
      `<circle cx="228" cy="167" r="35" fill="${sun}"/><path d="m-30 300 116-156 92 135 39-66 112 127H-30" fill="${mid}"/><path d="m-20 343 107-118 67 91 51-37 125 79H-20" fill="${ink}"/><path d="M144 287c-43 42 44 30 12 63s-65 23-67 50" fill="none" stroke="${bg}" stroke-width="10" opacity=".6"/>`,
      `<circle cx="75" cy="176" r="41" fill="${sun}"/><path d="M0 237q78-19 150 0t150 0v163H0" fill="${mid}"/><g stroke="${bg}" fill="none" opacity=".6"><path d="M0 270q78-19 150 0t150 0M0 302q78-19 150 0t150 0M0 334q78-19 150 0t150 0"/></g><path d="m182 257 61-14-5 16-45 10z" fill="${ink}"/><path d="M213 174v74m-3-68-39 58h39z" stroke="${ink}" fill="${bg}" stroke-width="3"/>`,
      `<circle cx="238" cy="142" r="26" fill="${sun}"/><g fill="${sun}" opacity=".8"><circle cx="43" cy="132" r="2"/><circle cx="136" cy="171" r="2"/><circle cx="201" cy="211" r="2"/><circle cx="76" cy="208" r="2"/></g><path d="M0 291h300v109H0" fill="${mid}"/><path d="m25 400 84-157h86l80 157M150 243v157M36 365h227M54 331h190M72 299h153M88 270h123" fill="none" stroke="${ink}" stroke-width="3"/><rect x="112" y="235" width="78" height="68" rx="9" fill="${sun}"/><rect x="121" y="245" width="60" height="29" fill="${bg}"/>`,
      `<path d="M148 360V146m0 130c-81 3-99-57-99-57 64-13 97 25 99 57zm0-46c70-3 100-68 100-68-73-6-100 36-100 68zm0 93c77-3 109-66 109-66-73-9-106 35-109 66z" fill="${mid}" stroke="${ink}" stroke-width="3"/><path d="m97 342 13 58h79l14-58z" fill="${ink}"/><circle cx="76" cy="153" r="20" fill="${sun}"/>`,
      `<g stroke="${ink}" fill="none" opacity=".7"><ellipse cx="151" cy="254" rx="115" ry="48" transform="rotate(-35 151 254)"/><ellipse cx="151" cy="254" rx="115" ry="48" transform="rotate(35 151 254)"/><circle cx="151" cy="254" r="77"/></g><circle cx="151" cy="254" r="45" fill="${sun}"/><circle cx="63" cy="200" r="13" fill="${ink}"/><circle cx="225" cy="312" r="9" fill="${mid}"/>`,
    ];
    const shortTitle = (item.title || item.name).replace(/\s*\d+$/, "");
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="300" height="400" viewBox="0 0 300 400">
      <rect width="300" height="400" fill="${bg}"/><rect x="13" y="13" width="274" height="374" fill="none" stroke="${ink}" opacity=".25"/>
      <text x="26" y="41" font-family="sans-serif" font-size="8" letter-spacing="3" fill="${ink}">STORIES / ${String(item.art + 1).padStart(2, "0")}</text>
      <text x="25" y="83" font-family="sans-serif" font-size="23" fill="${ink}">${escape(shortTitle)}</text>
      <path d="M26 101h36" stroke="${ink}" stroke-width="2"/>
      ${illustrations[scene]}<rect x="0" y="371" width="300" height="29" fill="${bg}"/>
      <text x="25" y="389" font-family="sans-serif" font-size="7" letter-spacing="3" fill="${ink}">MANGAMEEYA / ORIGINAL DEMO</text></svg>`;
    let result = dataUri(svg);
    if (item.art % 11 === 9) {
      result = dataUri(`<svg xmlns="http://www.w3.org/2000/svg" width="480" height="300" viewBox="0 0 480 300"><rect width="480" height="300" fill="${bg}"/><circle cx="365" cy="85" r="42" fill="${sun}"/><path d="m0 280 140-190 110 150 80-100 150 160H0" fill="${mid}"/><path d="m0 300 150-110 130 110" fill="${ink}"/><text x="28" y="48" font-family="sans-serif" font-size="22" fill="${ink}">${escape(shortTitle)}</text><text x="28" y="70" font-family="sans-serif" font-size="8" letter-spacing="2" fill="${ink}">ORIGINAL LANDSCAPE DEMO</text></svg>`);
    }
    artCache.set(item.id, result);
    return result;
  }

  function pageArt(pageNumber = page) {
    const shift = (pageNumber % 3) * 18;
    return dataUri(`<svg xmlns="http://www.w3.org/2000/svg" width="660" height="900" viewBox="0 0 660 900">
      <defs><pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse"><path d="M0 6 6 0" stroke="#d9d9d1" stroke-width=".8"/></pattern></defs>
      <rect width="660" height="900" fill="#f6f5ee"/>
      <text x="35" y="34" font-size="10" font-family="sans-serif" fill="#74766d" letter-spacing="3">AFTER THE RAIN / ORIGINAL COMIC STUDY</text>
      <g stroke="#313d36" stroke-width="2" fill="none">
        <rect x="32" y="54" width="596" height="309"/>
        <path d="M33 311h594M33 331h594M80 363l115-142m370 142L449 221M195 221h254"/>
        <path d="M40 309V158h78v151m0-35V110h88v164m0-19V157h65v98m0-19V95h97v141m0 8V127h61v117m0 33V165h97v112m0 33V125h88v185" fill="url(#hatch)"/>
        <path d="M137 131h49m-49 20h49m-49 20h49m-49 20h49m105-69h55m-55 21h55m-55 21h55m-55 21h55M59 183h40m-40 21h40m-40 21h40m345-36h62m-62 21h62m-62 21h62" stroke-width="1"/>
        <rect x="32" y="381" width="284" height="226"/><rect x="334" y="381" width="294" height="226"/>
        <path d="M33 579 216 395m-120 212 206-206M334 558l123-166m-86 215 142-215m-70 215 126-180" stroke="#a7aca1"/>
        <path d="M76 574h90l-9-68H89zM95 504v-43q26-26 51 0v43" fill="#e6e6da"/>
        <path d="m386 580 122-118m-80 142 114-111M507 460l36 27-14 28-49-36z" fill="#d2d6c9"/>
        <rect x="32" y="625" width="596" height="226"/>
        <path d="M33 805h593M33 820h593M312 850V674M37 730h570M72 850V704m105 146V686m270 164V693m100 157V714"/>
        <path d="M80 802V698h58v104m83 0V665h62v137m114 0V688h60v114m80 0V670h62v132" fill="url(#hatch)"/>
        <path d="M${280 + shift} 806v-57q-20-22-40 0v57m4-66q16-43 33-1" fill="#a3ad9e"/>
      </g>
      <g font-family="sans-serif" font-size="15" fill="#364038"><rect x="451" y="76" width="146" height="52" rx="25" fill="#fffef7" stroke="#364038"/><text x="479" y="108">雨停了。</text><rect x="347" y="399" width="216" height="46" rx="22" fill="#fffef7" stroke="#364038"/><text x="366" y="428">下一站，去看海。</text></g>
      <text x="330" y="878" text-anchor="middle" font-family="serif" font-size="12" fill="#67705f">${String(pageNumber).padStart(2, "0")}</text></svg>`);
  }

  function shuffleValue(item) {
    let hash = state.shuffleSeed;
    for (const char of item.id) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619);
    hash ^= hash >>> 16;
    hash = Math.imul(hash, 0x45d9f3b);
    return (hash ^ (hash >>> 16)) >>> 0;
  }
  function sortValue(item) {
    switch (state.sort) {
      case "natural": case "name": return item.name;
      case "progress": return progress(item);
      case "random": return shuffleValue(item);
      default: return item[state.sort];
    }
  }
  function compare(left, right) {
    if (state.foldersFirst && (left.kind === "folder") !== (right.kind === "folder")) {
      return left.kind === "folder" ? -1 : 1;
    }
    const a = sortValue(left);
    const b = sortValue(right);
    // Missing metadata remains last within its group, including descending order.
    if (a == null && b != null) return 1;
    if (b == null && a != null) return -1;
    let order = 0;
    if (a != null && b != null) {
      order = typeof a === "number" ? a - b : (state.sort === "name" ? literal : natural).compare(a, b);
    }
    return (state.descending ? -order : order) || natural.compare(left.name, right.name) || left.id.localeCompare(right.id);
  }
  function filteredEntries() {
    const query = normalize(state.query.trim());
    return entries.filter((item) => item.directory === state.folder)
      .filter((item) => !query || normalize(item.name).includes(query))
      .filter((item) => state.collection === "all" || (item.kind !== "folder" &&
        (state.collection === "reading" ? progress(item) > 0 && progress(item) < 1 : item.current === 0)))
      .sort(compare);
  }
  function progressBar(item) {
    return `<div class="progress-track" aria-hidden="true"><span style="width:${Math.round((progress(item) || 0) * 100)}%"></span></div>`;
  }
  function coverItem(item) {
    const folder = item.kind === "folder";
    const meta = folder ? `${childCount(item)} 個項目` : `${item.type} · ${item.pages == null ? "頁數未知" : `${item.pages} 頁`}`;
    return `<div class="item" role="option" data-id="${item.id}" aria-selected="${item.id === state.selectedId}" tabindex="${item.id === state.selectedId ? 0 : -1}" aria-label="${escape(item.name)}，${escape(meta)}" title="${escape(item.name)}">
      <div class="cover-well ${folder ? "is-folder" : ""}">${folder ? icon("folder", "folder-art") : `<img src="${cover(item)}" alt="" draggable="false"><span class="cover-type">${item.type}</span>`}</div>
      <h3 class="cover-title">${escape(item.name)}</h3><div class="cover-meta"><span>${meta}</span><span>${folder ? "" : percentage(item)}</span></div>${folder ? "" : progressBar(item)}</div>`;
  }
  function tableItem(item) {
    const folder = item.kind === "folder";
    return `<tr class="item" role="row" data-id="${item.id}" aria-selected="${item.id === state.selectedId}" tabindex="${item.id === state.selectedId ? 0 : -1}" title="${escape(item.name)}">
      <td><div class="table-name">${icon(folder ? "folder" : item.kind === "image" ? "image" : "file")}<span>${escape(item.name)}</span></div></td>
      <td>${formatDate(item.modified)}</td><td>${formatSize(item.size)}</td><td>${item.pages ?? "—"}</td>
      <td><div class="table-progress">${folder ? "—" : `${progressBar(item)}<span>${percentage(item)}</span>`}</div></td></tr>`;
  }
  function emptyState() {
    const filtered = state.query || state.collection !== "all";
    const html = `<div class="empty-state"><strong>${filtered ? "找不到符合條件的檔案" : "這個資料夾還是空的"}</strong>${filtered ? "請變更搜尋條件，或清除篩選。" : "這是示範空資料夾。可按 Backspace 返回上一層。"}<br><button class="action" data-action="${filtered ? "clear" : "parent"}">${filtered ? "清除搜尋與篩選" : "回到 Comics"}</button></div>`;
    return variant === "explorer" ? `<tr><td colspan="5">${html}</td></tr>` : html;
  }
  function updateChrome() {
    $("#sort").value = state.sort;
    $("#folders-first").checked = state.foldersFirst;
    $("#search").value = state.query;
    const reverse = $('[data-action="reverse"]');
    reverse.innerHTML = icon(state.descending ? "descending" : "ascending");
    reverse.setAttribute("aria-label", `目前為${state.descending ? "降冪" : "升冪"}；切換為${state.descending ? "升冪" : "降冪"}`);
    reverse.title = reverse.getAttribute("aria-label");
    $('[data-action="shuffle"]').hidden = state.sort !== "random";
    $$('[data-sort-key]').forEach((button) => {
      const active = button.dataset.sortKey === state.sort || (button.dataset.sortKey === "natural" && state.sort === "name");
      button.parentElement.removeAttribute("aria-sort");
      if (active) button.parentElement.setAttribute("aria-sort", state.descending ? "descending" : "ascending");
    });
    $$('[data-folder]').forEach((button) => {
      button.removeAttribute("aria-current");
      if (button.dataset.folder === state.folder) button.setAttribute("aria-current", "location");
    });
    $$('[data-collection]').forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.collection === state.collection)));
    const parent = folders[state.folder].parent;
    $(".breadcrumbs").innerHTML = `<span class="path-prefix">D:</span><span>/</span><button data-folder="root">Comics</button>${parent ? `<span>/</span><button data-folder="${state.folder}" aria-current="location">${escape(folders[state.folder].name)}</button>` : ""}`;
    const count = entries.filter((item) => item.directory === state.folder).length;
    $('[data-results]').innerHTML = `<strong>${visible.length}</strong> / ${count} 個項目${state.foldersFirst ? " · 資料夾優先" : " · 混合排序"}`;
    $$('[data-current-book]').forEach((node) => { node.textContent = currentBook.name; });
    $$('[data-current-progress]').forEach((node) => { node.textContent = `第 ${page} / ${totalPages(currentBook)} 頁`; });
  }
  function updateSelection() {
    $$('#entries .item').forEach((node) => {
      const selected = node.dataset.id === state.selectedId;
      node.setAttribute("aria-selected", String(selected));
      node.tabIndex = selected ? 0 : -1;
    });
    const item = visible.find((entry) => entry.id === state.selectedId);
    $('[data-selection-status]').textContent = item ? `${item.name} ${item.kind === "folder" ? ` · ${childCount(item)} 個項目` : ` · ${formatSize(item.size)}`}` : "沒有選取項目";
    $('[data-action="open"]').disabled = !item;
    if (!$("#preview")) return;
    if (!item) { $("#preview").innerHTML = '<p class="preview-note">選取檔案後，在此查看封面與資訊。</p>'; return; }
    const folder = item.kind === "folder";
    $("#preview").innerHTML = `<div class="preview-cover">${folder ? icon("folder", "folder-art") : `<img src="${cover(item)}" alt="選取項目的原創示意封面">`}</div>
      <h3>${escape(item.name)}</h3><span class="preview-type">${folder ? "示範資料夾" : `${item.type} · 示範${item.kind === "image" ? "圖片" : "封存檔"}`}</span>
      <dl class="preview-data"><div><dt>大小</dt><dd>${formatSize(item.size)}</dd></div><div><dt>頁數</dt><dd>${item.pages ?? "—"}</dd></div><div><dt>修改日期</dt><dd>${formatDate(item.modified)}</dd></div><div><dt>建立日期</dt><dd>${formatDate(item.created)}</dd></div><div><dt>閱讀進度</dt><dd>${percentage(item)}</dd></div><div><dt>最近閱讀</dt><dd>${formatDate(item.lastRead)}</dd></div></dl>
      <button class="action primary" data-action="open">${icon(folder ? "folder" : "play")}${folder ? "進入資料夾" : "開始閱讀"}<kbd>Enter</kbd></button><p class="preview-note">封面與中繼資料均為示範。<br>「—」表示不適用或資料未知。</p>`;
  }
  function selectedNode() { return $(`#entries .item[data-id="${state.selectedId}"]`); }
  function renderBrowser(focus = false) {
    visible = filteredEntries();
    if (!visible.some((item) => item.id === state.selectedId)) state.selectedId = visible[0]?.id || null;
    $("#entries").innerHTML = visible.length ? visible.map(variant === "explorer" ? tableItem : coverItem).join("") : emptyState();
    updateChrome();
    updateSelection();
    scroller.scrollTop = state.scrollTop;
    if (focus) (selectedNode() || browser).focus({ preventScroll: true });
  }
  function select(id, focus = false) {
    if (!visible.some((item) => item.id === id)) return;
    state.selectedId = id;
    updateSelection();
    if (focus) selectedNode()?.focus({ preventScroll: true });
  }
  function saveFolderState() {
    directoryStates.set(state.folder, {
      selectedId: state.selectedId, query: state.query, collection: state.collection, scrollTop: scroller.scrollTop,
    });
  }
  function goFolder(id) {
    if (!folders[id] || id === state.folder) return;
    saveFolderState();
    state.folder = id;
    Object.assign(state, directoryStates.get(id) || { selectedId: null, query: "", collection: "all", scrollTop: 0 });
    renderBrowser(true);
  }
  function parentFolder() {
    const parent = folders[state.folder].parent;
    if (parent) goFolder(parent);
    else toast("已在示範根目錄，不會再往上，也不會返回瀏覽器上一頁。");
  }
  function setMode(mode, focus = true) {
    state.mode = mode;
    app.classList.toggle("mode-browser", mode === "browser");
    browser.hidden = mode !== "browser";
    browser.inert = mode !== "browser";
    reader.hidden = mode === "browser" && variant !== "switcher";
    reader.inert = mode === "browser";
    reader.setAttribute("aria-hidden", String(mode === "browser"));
    if ($('[data-overlay-context]')) $('[data-overlay-context]').hidden = mode !== "browser";
    if (mode === "browser") renderBrowser(focus);
    else if (focus) reader.focus({ preventScroll: true });
    if (variant === "integrated") {
      $(".native-workspace").classList.remove("mobile-navigation", "mobile-inspector");
      updateIntegratedFrame();
    }
  }
  function openSelected() {
    const item = visible.find((entry) => entry.id === state.selectedId);
    if (!item) return;
    if (item.kind === "folder") { goFolder(item.id); return; }
    state.scrollTop = scroller.scrollTop;
    currentBook = item;
    page = Math.max(1, item.current || 1);
    item.current = page;
    item.lastRead = Date.now();
    renderReader();
    setMode("reader");
  }
  function renderReader() {
    $('[data-reader-title]').textContent = currentBook.name;
    $('[data-page-count]').textContent = `${page} / ${totalPages(currentBook)} 頁`;
    $("#reading-page").src = pageArt();
    $("#reading-page").alt = `原創示意漫畫，第 ${page} 頁`;
    $('[data-action="previous"]').disabled = page <= 1;
    $('[data-action="next"]').disabled = page >= totalPages(currentBook);
    updateIntegratedFrame();
  }
  function turnPage(delta) {
    page = Math.max(1, Math.min(totalPages(currentBook), page + delta));
    currentBook.current = page;
    currentBook.lastRead = Date.now();
    renderReader();
  }
  // These controls belong to the retained shell, not either central view.
  function updateIntegratedFrame() {
    if (variant !== "integrated") return;
    const browsing = state.mode === "browser";
    const total = totalPages(currentBook);
    $$('[data-mode-button]').forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.modeButton === state.mode)));
    $('[data-native-total]').textContent = `${total} 頁`;
    $('[data-native-total-inline]').textContent = `/ ${total}`;
    $('[data-native-view]').textContent = browsing ? "檔案瀏覽 · Enter 開啟 / Esc 返回閱讀" : "閱讀模式 · Backspace 檔案瀏覽";
    $('[data-progress-label]').textContent = `${browsing ? "保留閱讀位置" : "閱讀進度"} ${Math.round(page / total * 100)}%`;
    $('[data-current-book]').textContent = currentBook.name;
    for (const control of [$("#native-page"), $("#native-progress")]) {
      control.max = total;
      control.value = page;
      control.disabled = browsing;
    }
    $('[data-action="previous"]').disabled = browsing || page <= 1;
    $('[data-action="next"]').disabled = browsing || page >= total;
    const recent = entries.filter((item) => item.kind === "archive" && item.lastRead != null)
      .sort((a, b) => b.lastRead - a.lastRead || natural.compare(a.name, b.name)).slice(0, 10);
    $("#nav-recent").innerHTML = recent.map((item) => `<button class="native-recent-entry" data-open-entry="${item.id}" title="${escape(item.name)}"${item.id === currentBook.id ? ' aria-current="true"' : ""}><span>${escape(item.name)}</span><small>${item.current || 0} / ${totalPages(item)} 頁</small></button>`).join("");
    const first = Math.max(1, Math.min(page - 2, total - 5));
    $("#nav-thumbnails").innerHTML = Array.from({ length: Math.min(6, total) }, (_, i) => first + i)
      .map((number) => `<button data-jump-page="${number}"${number === page ? ' aria-current="page"' : ""}><img src="${pageArt(number)}" alt="">第 ${number} 頁</button>`).join("");
    updatePanelButtons();
  }
  function updatePanelButtons() {
    if (variant !== "integrated") return;
    $$('[data-toggle-panel]').forEach((button) => {
      const panel = $(`#${button.dataset.togglePanel}`);
      button.setAttribute("aria-expanded", String(panel.getClientRects().length > 0));
    });
  }
  function togglePanel(name) {
    const panel = $(`#${name}`);
    const workspace = $(".native-workspace");
    if (matchMedia("(max-width:650px)").matches) {
      const open = !workspace.classList.contains(`mobile-${name}`);
      workspace.classList.remove("mobile-navigation", "mobile-inspector");
      panel.hidden = false;
      if (open) workspace.classList.add(`mobile-${name}`);
    } else panel.hidden = !panel.hidden;
    updatePanelButtons();
  }
  function activateTab(button, selector) {
    $$(selector).forEach((tab) => {
      const active = tab === button;
      tab.setAttribute("aria-selected", String(active));
      tab.tabIndex = active ? 0 : -1;
      $(`#${tab.getAttribute("aria-controls")}`).hidden = !active;
    });
    button.focus({ preventScroll: true });
  }
  function jumpToPage(value) {
    if (!Number.isFinite(value)) { updateIntegratedFrame(); return; }
    page = Math.max(1, Math.min(totalPages(currentBook), Math.trunc(value)));
    currentBook.current = page;
    currentBook.lastRead = Date.now();
    renderReader();
    if (state.mode !== "reader") setMode("reader");
  }
  function chooseSort(key) {
    if (state.sort === key) state.descending = !state.descending;
    else {
      state.sort = key;
      state.descending = ["modified", "created", "size", "pages", "progress", "lastRead"].includes(key);
    }
    state.scrollTop = 0;
    renderBrowser();
  }
  function clearFilters() {
    state.query = "";
    state.collection = "all";
    state.scrollTop = 0;
    renderBrowser();
    $("#search").focus();
  }
  function toast(message) {
    clearTimeout(toastTimer);
    $("#toast").textContent = message;
    $("#toast").hidden = false;
    toastTimer = setTimeout(() => { $("#toast").hidden = true; }, 3200);
  }

  document.addEventListener("click", (event) => {
    const action = event.target.closest("[data-action]");
    if (action && !action.disabled) {
      switch (action.dataset.action) {
        case "open": openSelected(); break;
        case "browse": setMode("browser"); break;
        case "resume": if (state.mode === "browser") state.scrollTop = scroller.scrollTop; setMode("reader"); break;
        case "parent": parentFolder(); break;
        case "clear": clearFilters(); break;
        case "previous": turnPage(-1); break;
        case "next": turnPage(1); break;
        case "reverse": state.descending = !state.descending; state.scrollTop = 0; renderBrowser(); break;
        case "shuffle": state.shuffleSeed++; state.scrollTop = 0; renderBrowser(); break;
      }
      return;
    }
    const panelToggle = event.target.closest("[data-toggle-panel]");
    if (panelToggle) { togglePanel(panelToggle.dataset.togglePanel); return; }
    const nativeTab = event.target.closest("[data-native-tab], [data-tool-tab]");
    if (nativeTab) {
      activateTab(nativeTab, nativeTab.hasAttribute("data-native-tab") ? "[data-native-tab]" : "[data-tool-tab]");
      return;
    }
    const recentEntry = event.target.closest("[data-open-entry]");
    if (recentEntry) {
      const item = entries.find((entry) => entry.id === recentEntry.dataset.openEntry);
      goFolder(item.directory);
      Object.assign(state, { selectedId: item.id, query: "", collection: "all" });
      renderBrowser();
      openSelected();
      return;
    }
    const thumbnail = event.target.closest("[data-jump-page]");
    if (thumbnail) { jumpToPage(Number(thumbnail.dataset.jumpPage)); return; }
    const background = event.target.closest("[data-background]");
    if (background) {
      $("#reader-scroll").style.background = background.dataset.background;
      $$('[data-background]').forEach((button) => button.setAttribute("aria-pressed", String(button === background)));
      return;
    }
    const folder = event.target.closest("[data-folder]");
    if (folder) {
      if (variant === "integrated" && state.mode !== "browser") setMode("browser", false);
      goFolder(folder.dataset.folder);
      return;
    }
    const sort = event.target.closest("[data-sort-key]");
    if (sort) { chooseSort(sort.dataset.sortKey); return; }
    const collection = event.target.closest("[data-collection]");
    if (collection) {
      state.collection = collection.dataset.collection;
      state.scrollTop = 0;
      renderBrowser();
      return;
    }
    const item = event.target.closest(".item[data-id]");
    if (item && state.mode === "browser") select(item.dataset.id, true);
  });
  $("#entries").addEventListener("dblclick", (event) => {
    const item = event.target.closest(".item[data-id]");
    if (!item) return;
    select(item.dataset.id);
    openSelected();
  });
  $("#search").addEventListener("input", (event) => {
    state.query = event.target.value;
    state.scrollTop = 0;
    renderBrowser();
  });
  $("#sort").addEventListener("change", (event) => { chooseSort(event.target.value); });
  $("#folders-first").addEventListener("change", (event) => {
    state.foldersFirst = event.target.checked;
    state.scrollTop = 0;
    renderBrowser();
  });
  if ($("#tile-size")) $("#tile-size").addEventListener("input", (event) => {
    $("#entries").style.setProperty("--tile-width", `${event.target.value}px`);
    if ($("#tile-size-value")) $("#tile-size-value").textContent = `${event.target.value} px`;
  });
  if (variant === "integrated") {
    $("#show-filenames").addEventListener("change", (event) => app.classList.toggle("hide-filenames", !event.target.checked));
    $("#show-metadata").addEventListener("change", (event) => app.classList.toggle("show-metadata", event.target.checked));
    $("#filename-lines").addEventListener("change", (event) => $("#entries").style.setProperty("--name-lines", event.target.value));
    $("#native-page").addEventListener("change", (event) => jumpToPage(event.target.valueAsNumber));
    $("#native-progress").addEventListener("input", (event) => jumpToPage(event.target.valueAsNumber));
    window.addEventListener("resize", updatePanelButtons);
  }
  scroller.addEventListener("scroll", () => {
    if (state.mode === "browser") state.scrollTop = scroller.scrollTop;
  }, { passive: true });

  document.addEventListener("keydown", (event) => {
    if (event.defaultPrevented || event.isComposing || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    const target = event.target;
    if (target.closest("input, textarea, select") || target.isContentEditable) return;
    const tab = target.closest("[data-native-tab], [data-tool-tab]");
    if (tab && ["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      const tabs = $$(tab.hasAttribute("data-native-tab") ? "[data-native-tab]" : "[data-tool-tab]");
      const index = tabs.indexOf(tab);
      const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
      tabs[next].click();
      return;
    }
    if (event.key === "Backspace") {
      event.preventDefault();
      if (event.repeat) return;
      if (state.mode === "reader") setMode("browser");
      else parentFolder();
      return;
    }
    if (event.key === "Escape" && state.mode === "browser") {
      state.scrollTop = scroller.scrollTop;
      setMode("reader");
      return;
    }
    if (state.mode === "reader") {
      if (["ArrowRight", "PageDown", "ArrowLeft", "PageUp"].includes(event.key)) {
        event.preventDefault();
        turnPage(["ArrowLeft", "PageUp"].includes(event.key) ? -1 : 1);
      }
      return;
    }
    const itemTarget = target.closest(".item") || target === browser;
    if (!itemTarget) return;
    if (event.key === "Enter") { event.preventDefault(); openSelected(); return; }
    const keys = ["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp", "Home", "End"];
    if (!keys.includes(event.key) || !visible.length) return;
    event.preventDefault();
    const grid = $(".cover-grid");
    const columns = grid ? getComputedStyle(grid).gridTemplateColumns.split(" ").length : 1;
    const offset = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: columns, ArrowUp: -columns }[event.key] || 0;
    const index = visible.findIndex((item) => item.id === state.selectedId);
    const next = event.key === "Home" ? 0 : event.key === "End" ? visible.length - 1 : Math.min(visible.length - 1, Math.max(0, index + offset));
    select(visible[next].id, true);
    selectedNode()?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });
  // The switcher is a dialog. Keep keyboard focus inside it while it is open.
  document.addEventListener("keydown", (event) => {
    if (variant !== "switcher" || state.mode !== "browser" || event.key !== "Tab") return;
    const controls = [...browser.querySelectorAll('button:not(:disabled), input, select, [tabindex="0"]')]
      .filter((node) => node.getClientRects().length && !node.closest("[inert]"));
    const first = controls[0];
    const last = controls.at(-1);
    if (event.shiftKey && (document.activeElement === first || !browser.contains(document.activeElement))) {
      event.preventDefault(); last?.focus();
    } else if (!event.shiftKey && (document.activeElement === last || !browser.contains(document.activeElement))) {
      event.preventDefault(); first?.focus();
    }
  });

  renderReader();
  if (document.body.dataset.initialMode === "reader") renderBrowser();
  setMode(document.body.dataset.initialMode || "browser", false);
})();
