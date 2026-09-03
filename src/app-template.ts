interface TemplatePreferences {
  background: string;
  sidebar: boolean;
}

interface TemplateFilters {
  brightness: number;
  contrast: number;
  grayscale: number;
}

export function appTemplate(preferences: TemplatePreferences, filters: TemplateFilters): string {
  return `
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
        ${range("brightness", "亮度", 50, 150, filters.brightness)}
        ${range("contrast", "對比", 50, 150, filters.contrast)}
        ${range("grayscale", "灰階", 0, 100, filters.grayscale)}
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
}

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
