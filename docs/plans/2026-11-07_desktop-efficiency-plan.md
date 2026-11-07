# MangaMeeya Rust — 桌面效率型實作計畫

- 日期：2026-11-07
- 狀態：**已核准；PR #13 已建立，必要平台 CI 尚待驗證**
- Objective：桌面效率型｜建議起點
- 草擬時的基準 commit：`d41f28b`
- 設計參考：`docs/ui-concepts/01-desktop.html`、`docs/ui-concepts/01-desktop.png`
- 比較說明：`docs/ui-concepts/README.md`

## 1. 核准與執行規則

1. 草擬階段只新增計畫；核准後依本計畫實作，不擴大到未核准範圍。
2. 使用者明確核准本計畫後，才開始執行第 6 節。
3. 執行前重新檢查工作目錄與原始碼基準，不覆寫使用者或其他工作產生的變更。
4. 若範圍、架構、主要風險或驗收條件有重大變更：
   - 先更新本文件，說明差異與原因。
   - 再取得明確核准，才繼續受影響的工作。
   - 不以降低驗收標準處理失敗的測試。
5. 只有第 6 節所有必要項目、第 7 節驗收條件與第 8 節必要檢查都有證據時，才回報 `DONE`。
   - 缺少平台、工具或測試證據時，回報 blocker 或未驗證項目。
   - 歷史通過記錄與 HTML 原型測試不能代替新實作的原生驗證。
6. 使用者於 2026-11-07 的後續指示已授權建立專用分支、commit、push 與建立 PR；正常 PR CI 可提供平台證據。不發布套件、不建立版本標籤、不手動觸發 release workflow。

## 2. 目標與目前狀態

目標是在既有 Rust / GPUI Kit 介面上落實方案 1，改善資訊層級與操作一致性，不重寫閱讀器。

已確認的現況：

1. `src/ui.rs` 已有選單、工具列、縮圖、影像調整與底部狀態列。
2. `ReadingMode` 已支援 Single、Spread、Continuous；`FitMode` 已支援 Window、Width、Height、Original、Custom。
3. `render_zoom_control` 已整合縮放模式、實際百分比與 Popover。必須沿用其數值、焦點與輸入草稿保護。
4. 左側開關與導覽類別已儲存；右側開關目前只在 `ReaderView` 記憶體中，兩側寬度固定。
5. `Preferences` 與 `AppSettings` 使用 `serde(default, deny_unknown_fields)`。新增設定可讓新版讀取舊檔，但舊版不會自動接受新欄位。
6. GPUI Kit 0.9.0 已提供 `h_resizable`、`ResizableState`、`resizable_panel`、`fixed`、`size_range` 與拖曳完成的 `on_resize`，不需要新增套件。
7. 已有 GPUI 互動、縮放、三語系及原生 rendering 測試。正式視窗最小尺寸為 800 × 540 logical px，不提高此限制。

## 3. 範圍與設計界線

### 3.1 本次納入

1. 工具列分組與一致的 Icon Button、模式選擇、縮放入口。
2. 明確且互斥的「單頁／雙頁／連續」選中狀態。
3. 左側清楚的「檔案／縮圖／最近開啟」分頁、固定容器縮圖、獨立頁碼與完整檔名 Tooltip。
4. 右側「影像／閱讀／播放」分頁，拆開不同責任的設定。
5. 左右面板獨立收合、拖曳調整寬度，並儲存開關與偏好寬度。
6. 精簡狀態列；頁碼導覽只放底部，完整檔名、尺寸與檔案大小移入 Information Popover。
7. 保留全部既有功能入口、快捷鍵、三語系、錯誤回饋及設定損壞復原功能。
8. 補上設定、互動、版面與回歸測試，以及使用與相容性文件。

### 3.2 不納入

1. 方案 2 的懸浮控制列、自動隱藏、邊緣叫回工具及新增專注模式。
2. 方案 3 的底部橫向縮圖、工具覆蓋抽屜與 Comfortable Width。
3. 新增 Light / Dark Theme 切換、Fluent 圖示套件、動畫或新的視覺框架。
4. WebView、Tauri、HTML 內嵌、Node 相依套件或 GUI 框架升級。
5. 改寫解碼、封存檔、GPU texture、快取、虛擬化、IPC 或檔案匯出流程。
6. 安裝程式、簽署、版本號、發布流程或完整效能測試專案。

### 3.3 HTML 原型轉為正式介面的界線

1. 以方案 1 的結構與操作入口為準，不把瀏覽器外框、方案標題或示範漫畫放入應用程式。
2. 全螢幕按鈕沿用現有 `Command::Fullscreen`、F11 與 macOS 快捷鍵，不把它改成原型的應用程式內專注模式。
3. 保留 `ResetFilters` 既有語意：只重設亮度、對比與灰階，不重設旋轉或偏好；正式標籤明確使用「重設濾鏡」。
4. 模式切換不自動覆寫目前 FitMode 或自訂縮放。原型展示的模式預設縮放不直接移植；每種模式獨立記憶縮放不在本次範圍。
5. 保留既有書籤模型與設定，不移植原型的多頁書籤集合或重新載入即消失的資料模型。
6. 使用 GPUI ActiveTheme 與既有 Lucide icons；不強制套用原型的淺色背景或特定藍色。

## 4. 目標介面與操作規則

### 4.1 頂部

1. 保留平台原生選單與既有視窗內選單入口。Windows 與 macOS 都能找到全部命令。
2. 工具列分為以下群組：
   - 開啟檔案／資料夾入口與左側面板開關。
   - Single／Spread／Continuous 三個互斥選項。
   - 唯一的縮放模式與百分比入口，以及有清楚語意的閱讀方向控制。
   - 書籤、右側閱讀工具、全螢幕與次要工具入口。
3. 上一頁／下一頁移至底部，不在工具列重複顯示。鍵盤與閱讀區邊緣翻頁保持不變。
4. 模式按鈕直接選擇指定模式；重複點選目前模式不切換到其他模式。
   - S 與 C 保留既有 toggle 快捷鍵語意。
   - 共用設定模式的 UI 路徑，避免工具列、選單與快捷鍵狀態不同步。
5. 「包含子資料夾」放到檔案選單的開啟選項。
   - 沿用 `recursive_folder`，只影響下次開啟，不重新載入目前漫畫。
6. 偏好設定從工具選單與平台原生 Preferences 入口開啟，直接顯示右側「閱讀」分頁。
7. 較窄視窗可縮減按鈕文字，但不可失去必要入口、選中狀態、Tooltip 或可及名稱，也不可與縮放控制重疊。

### 4.2 左側內容導覽

1. 使用一致的 Tab 樣式，保留 Files、Thumbnails、Recent 功能。
   - 三語系完整名稱不得跨出面板；必要時讓 Tab 列分行，而不是覆蓋閱讀區。
2. 縮圖容器以 110 × 156 logical px 為目標，等比例縮放、不裁切、不變形。
   - 使用一致的項目容器，避免原圖比例造成不規則的大幅高度變化。
   - 頁碼與檔名分開；頁碼不截斷，檔名單行省略，Tooltip 顯示完整名稱。
3. 選中狀態採淡色背景與細色條，並提供可及選中狀態。
4. 沿用 `ListState` 虛擬化與可見縮圖追蹤；不得一次解碼整本漫畫。
5. 跳頁後目前縮圖保持可見。縮圖捲動不改變中央閱讀捲動位置。

### 4.3 右側閱讀工具

1. Image 分頁：亮度、對比、灰階、旋轉與重設濾鏡。
   - Slider 標籤與實際數值對齊。
   - 沿用既有數值範圍、濾鏡延遲處理與背景載圖流程。
2. Reading 分頁：記住閱讀位置、閱讀方向與閱讀背景。
   - Language 與損壞設定復原放在有清楚標題的「一般設定」群組。
   - 不把它們混入 Image 或 Playback。
   - 設定載入／儲存錯誤仍有可見提示與明確復原入口。
3. Playback 分頁：投影片播放／暫停與 1–120 秒間隔。
   - 沿用目前 Enter／Blur 套用與最後一頁停止規則。
4. Tab 切換不重設任何值，不重建 `InputState`、`SliderState` 或遺失未送出的輸入草稿。
5. 目前工具分頁只保留於本次執行，不新增分頁記憶欄位。
6. 關閉面板後，焦點返回合理的既有控制或閱讀區，不能留在不可見輸入框。

### 4.4 中央閱讀區與面板寬度

1. 中央沿用原始圖片比例與既有 Single／Spread／Continuous renderer。
2. 使用 GPUI Kit 的原生 resizable 元件與 theme；不自行實作另一套拖曳框架。
3. 左側預設寬度 224、範圍 216–320 logical px；右側預設 256、範圍 240–360 logical px。
   - 中央閱讀區至少保留 280 logical px，並計入實際 resize handle 寬度。
   - 先限制拖曳可用範圍，不能讓面板把閱讀區擠成零寬度。
4. 視窗縮小時，先將有效右側寬度降到最小，再將左側降到最小；不提高既有 800 × 540 的視窗最小尺寸。
   - 自動限制只改有效顯示寬度，不覆寫使用者儲存的偏好寬度。
   - 放大視窗後恢復偏好寬度。
   - 收合任一面板時，另一側不被重設。
5. 只在使用者完成拖曳且值確實變更時更新儲存寬度；沿用既有 atomic save 與 save ordering。
   - 不在每個 pointer move 直接寫入設定檔。
6. 重新量測要保留目前頁碼、連續閱讀錨點、仍開啟且聚焦的縮放草稿與正確的實際百分比。
   - 真正拖曳浮層外的面板邊界時，沿用既有 Popover 外部點擊取消規則：可關閉浮層，但不得套用草稿、改變實際縮放或重建輸入 Entity。此規則與既有 Escape／外部關閉語意一致。
7. 滾輪只作用於游標所在區域。新 Popover 開啟時不得讓 Ctrl + 滾輪穿透去縮放圖片。

### 4.5 底部與資訊浮層

1. 正常狀態使用單列：必要狀態、上一頁／頁碼／總頁數／下一頁、進度與資訊入口。
2. 不再放置長檔名、圖片尺寸、檔案大小、縮放或面板開關。
3. 閱讀百分比使用 `round((current + 1) / total * 100)`；沒有漫畫時顯示空狀態並停用導覽，不除以零。
4. Information Popover 顯示目前頁的完整檔名、實際尺寸與可取得的來源大小。
   - 資訊尚未載入時顯示未取得，不為顯示資訊解碼整本漫畫。
   - 換頁時更新；關閉或替換漫畫時關閉並清除舊資料。
5. 一般訊息單行省略；錯誤可使用額外的可見訊息列，不能只藏在 Tooltip 或資訊浮層。
   - 保留可及狀態訊息與完整錯誤內容入口。

### 4.6 一致性與可及性

1. 一般標籤使用 13–14 logical px，次要設定說明至少 12 logical px。
2. 相同類型控制採相同尺寸、間距、圓角、Hover、Focus、Selected 與 Disabled 樣式。
3. Icon Button 使用既有 Lucide，提供本地化名稱與 Tooltip，必要時標示既有快捷鍵。
4. 新 Tab、模式選項與面板開關具有正確角色及 selected／pressed 狀態。
5. 保留輸入框與 Popover 的快捷鍵隔離；輸入頁碼、路徑、播放秒數或縮放值時，不觸發閱讀快捷鍵。

## 5. 架構與設定相容性

### 5.1 責任分配

1. `Reader` 繼續負責閱讀狀態與既有版面計算；不建立第二套閱讀模型。
2. `ReaderView` 負責 `InspectorTab`、Popover 生命週期與 resizable view state。
   - 狀態與緊密相依的 render helper 保留在 `src/ui.rs`。
   - 面板寬度限制使用可直接測試的純 helper；不為本次改版拆出通用 UI framework。
3. 使用 `h_resizable` 管理左、中、右三區；左右區使用固定偏好寬度，中央吸收剩餘空間。
   - 面板 identity 在收合與重開後保持穩定，避免 list、輸入框及縮放 Popover 被意外重建。
4. 新資訊浮層沿用 GPUI Popover 的受控生命週期與焦點模式，並與既有縮放浮層協調事件隔離。

### 5.2 設定欄位

在既有 `Preferences` 新增具預設值的 `panel_layout: PanelLayoutPreferences`，不另建設定檔：

| 欄位 | 型別 | 預設 | 規則 |
| --- | --- | --- | --- |
| `sidebar_width` | `u32` | 224 | 驗證時限制至 216–320 |
| `inspector_width` | `u32` | 256 | 驗證時限制至 240–360 |
| `inspector_visible` | `bool` | true | 右側獨立開關 |

1. 左側開關仍使用既有 `preferences.sidebar`，不建立重複欄位。
2. 寬度以 logical px 的整數儲存；GPUI 的 Pixels 僅用於顯示，保留設定型別的 Eq 語意。
3. 缺少 `panel_layout` 或部分子欄位的舊檔使用預設值。
4. 保留 `deny_unknown_fields`、原子寫入、讀取大小限制、未知值錯誤與損壞資料保護。
   - 不把無效設定靜默重設，也不覆寫不支援的檔案。
5. 舊版程式會拒絕新增欄位。README 必須列出降版方式：
   - 先備份 `settings.json`。
   - 只在降版用的工作副本移除 `preferences.panel_layout`。
   - 保留語言、最近開啟、閱讀位置、書籤及其他既有欄位。
6. UI 不同時維護另一個獨立的右側可見性布林值，避免儲存狀態與畫面不同步。

## 6. 實作順序與交付項目

以下項目已獲核准；依實際結果逐項記錄，不預先標記完成。

### P1 — 確立原生基準與回歸保護

- [x] 重新確認 HEAD、使用者變更、工具鏈及測試環境。
- [x] 執行第 8 節可在目前主機進行的基準檢查，記錄執行平台、版本與失敗；不將既有失敗當成本次成功。
- [x] 保存可取得的原生 before 畫面／幾何證據；Windows 不宣稱有 Metal 截圖。
- [x] 在 `src/ui/tests/desktop.rs` 建立本次 UI 行為的測試群組，沿用既有 Fixture 與暫存設定目錄。

### P2 — 設定與可調整三區布局

- [x] 新增 `PanelLayoutPreferences`、預設值、驗證及舊檔讀取測試。
- [x] 加入原生 resizable state、兩側獨立開關與穩定的面板 identity。
- [x] 實作寬度限制、最小視窗配置、拖曳完成儲存及重開恢復。
- [x] 驗證純視窗尺寸變更不覆寫儲存的偏好寬度，拖曳與收合不破壞閱讀錨點。

### P3 — 工具列與內容導覽

- [x] 重組工具列，保留原生及視窗內選單，將次要操作移入適當選單。
- [x] 加入三個明確模式選項，共用既有 Reader 行為，保留快捷鍵語意。
- [x] 保留唯一的縮放 Popover；將頁碼導覽集中底部。
- [x] 更新左側 Tab、縮圖容器、獨立頁碼、檔名 Tooltip 及選中狀態。
- [x] 將子資料夾選項移到檔案開啟選項，測試只影響下次開啟。

### P4 — 閱讀工具與精簡狀態列

- [x] 將右側拆成 Image／Reading／Playback，保留語言與設定復原入口。
- [x] 保留所有輸入 Entity、Slider 訂閱與播放計時，不因換分頁重建。
- [x] 精簡狀態列，新增真實頁面資訊 Popover，保留可見錯誤與可及狀態回饋。
- [x] 處理浮層關閉、焦點返回、書籍替換與 Ctrl + 滾輪事件隔離。
- [x] 保留旋轉、ResetFilters、書籤、全螢幕與安全匯出的既有行為。

### P5 — 三語系、一致性與原生回歸

- [x] 在 `src/i18n.rs` 補齊 English、Japanese、Traditional Chinese 的新文字及可及名稱，不硬編碼新的使用者可見字串。
- [x] 更新 `src/ui/tests.rs`、`src/ui/tests/i18n.rs`、`src/ui/tests/zoom.rs` 中受布局改動影響的真實操作路徑，不刪除既有保護斷言。
- [x] 擴充 `tests/rendering.rs`，涵蓋三語系、三種尺寸、工具分頁、面板收合／拖曳、資訊浮層與錯誤狀態。
- [ ] 完成第 7、8 節檢查；修復本次引入的失敗。

### P6 — 文件、相容性與最終審查

- [x] 更新 `README.md`、`README.tw.md`、`README.ja.md` 的操作入口與降版說明，保留上游三語文件配置。
- [x] 在 `docs/verification.md` 記錄本次實際平台、命令、結果、畫面路徑及未驗證限制，不改寫歷史結果。
- [x] 依 `.changenote/README.md` 新增使用者可見的 minor release note，並以指定版本 CLI 驗證；不手動修改版本號或 Release 狀態檔。
- [x] 審查設定安全、焦點、空資料、長檔名、縮圖虛擬化與原生選單回歸。
- [x] 確認最終 diff 只包含核准範圍，未更動使用者其他檔案，未納入產生的原生截圖或二進位產物。
- [x] 更新本計畫的核准記錄、逐項結果與證據；若仍有必要檢查未驗證，列出 blocker，不回報 `DONE`。

預計受影響檔案：`src/ui.rs`、`src/settings.rs`、`src/i18n.rs`、`src/ui/tests.rs`、`src/ui/tests/desktop.rs`、`src/ui/tests/i18n.rs`、`src/ui/tests/zoom.rs`、`tests/rendering.rs`、`README.md`、`README.tw.md`、`README.ja.md`、`docs/verification.md`、本計畫與一份 `.changenote/` note。原型檔案只作設計參考，不解除其 ignore、不納入正式執行相依。

## 7. 可驗證的驗收條件

| ID | 必須成立的條件 | 必要證據 |
| --- | --- | --- |
| A1 | 三語系在 1280×800、960×640、800×540 下無面板、Tab、工具列或頁碼重疊；中央至少 280 logical px | 原生 bounds 斷言；macOS 畫面 |
| A2 | 三個模式互斥且直接可選；重複點選冪等；S／C／D、RTL／LTR 與換頁順序保持既有語意 | GPUI 真實按鍵與點擊測試 |
| A3 | 唯一縮放入口保留模式、實際比例、10%–800% 手動限制、未取整步進、草稿及關閉規則 | 既有 zoom suite 全過，外加面板拖曳時的草稿／比例測試 |
| A4 | 兩側獨立收合、拖曳、重開恢復；視窗自動限寬不改偏好值 | 指標事件測試、暫存設定 round trip 與重新建立 view |
| A5 | 舊設定缺少新欄位仍可讀；異常寬度限制有效；未知／損壞檔不被覆寫；閱讀資料不變 | Settings 正反向測試與原始檔 bytes 比對 |
| A6 | 右側每次只顯示一類工具；數值與草稿跨 Tab 保留；ResetFilters 不改旋轉或閱讀偏好 | GPUI 控制項狀態與模型斷言 |
| A7 | 子資料夾選項只在檔案開啟選項出現，且不重新載入目前書籍 | 真實選單操作與 book revision／下次開啟結果斷言 |
| A8 | 縮圖保留比例、頁碼獨立、名稱可完整取得；選中項目可見；仍只追蹤可見範圍 | 版面、縮圖 cache／eviction 既有測試及新斷言 |
| A9 | 頁碼只在 Enter 套用；進度正確；空書、第一頁、最後頁及單頁書不產生錯誤 | 真實輸入與邊界測試 |
| A10 | 資訊浮層顯示真實目前頁資料；換頁更新、關書清除；未知資訊不偽造 | 換頁、未載入、錯誤與替換書籍測試 |
| A11 | Input／Popover 內快捷鍵不穿透，Ctrl + 滾輪不穿透新浮層，關閉或隱藏後焦點合理 | 真實鍵盤、滾輪、Focus 斷言 |
| A12 | 語言即時切換與原生選單正常；播放間隔、播放停止、錯誤可見與備份復原未遺失 | i18n、播放、設定損壞與 rendering suite |
| A13 | 檔案開啟、原圖安全匯出、IPC、長頁、動畫、連續錨點與快取沒有本次引入的回歸 | 完整既有 suite 與必要新增回歸 |
| A14 | 無新增相依套件、無讀圖框架改寫、無來源圖片變更；文件與 note 符合實際結果 | 最終 diff、文件與 changenote 檢查 |

## 8. 必要檢查、證據與環境限制

### 8.1 核准後執行的命令

```sh
cargo fmt --all -- --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test rendering
changenote status
git diff --check
```

1. 使用 repository 固定的 Rust 1.99.1、Cargo.lock 與 changenote 0.10.0 指定 revision。
2. Windows 必須使用原生 MSVC 工具鏈、Windows SDK 與 Windows 執行環境，不將 WSL 編譯結果當成 Windows runtime 驗證。
3. macOS 執行前依 README 設定完整 Xcode、Metal compiler 與 `DEVELOPER_DIR`；不自動接受授權或開啟互動提示。
4. 測試只使用既有 fixture 或暫存書籍／設定，不改使用者漫畫與真實閱讀資料。

### 8.2 本次完成所需的平台證據

| 平台／檢查 | 本次要求 |
| --- | --- |
| Windows x64 | 必須取得新實作的 fmt、完整測試、Clippy 與 production rendering 互動／幾何檢查結果 |
| macOS arm64 | 必須取得新實作的完整測試、Clippy 與 production Metal rendering；保存必要 before／after 證據 |
| macOS x86_64 | 必須取得新實作的 all-target compile／Clippy 結果；可由既有 CI 原生 job 提供更完整證據 |
| 三語系、三種尺寸 | 必須完成 A1 所列矩陣；原生截圖放 `target/ui-smoke/desktop-*`，不以 HTML 圖片替代 |
| Release note 與 diff | 必須通過指定 CLI 與 diff 檢查 |

1. Windows 基準與新實作檢查已執行；結果見第 11 節與 `docs/verification.md` 的本次紀錄，不使用歷史通過結果替代。
2. 本機若缺少 macOS，可使用使用者提供的相同實作 revision CI 結果或另一台原生主機的結果。
   - 使用者後續指示已授權 push 與建立 PR；可讀取正常 PR CI，不手動觸發 release workflow。
   - 取得證據前列為未驗證／外部 blocker，不能標記完成。
3. Windows 的現有 rendering test 不產生 Metal 像素截圖；不得宣稱其結果是 Windows 實體桌面截圖。
4. 實體 Windows 125%／150% DPI、IME、Intel Mac 實機操作與主觀閱讀舒適度列為額外平台 review，不是本次必要 coding gate。
   - 未執行時仍要清楚列出，不宣稱已驗證。
5. 安裝、簽署、公證與發布不屬本次必要項目；既有 workflow 若一併執行，另外記錄結果，不據此擴大本計畫。

## 9. 風險與處理方式

| 風險 | 處理方式 |
| --- | --- |
| 新面板欄位使舊版拒絕設定 | 保留嚴格驗證，補舊檔讀取測試與明確備份／降版步驟，不宣稱雙向相容 |
| Resizable state 隨條件 render 重建 | 穩定 entity 與 panel identity；測試收合、重開、拖曳與大小變更 |
| 新寬度擠壓三語系控制項 | 固定最小閱讀寬度與面板界線；覆蓋三語系 × 三尺寸 |
| 拖曳觸發大量儲存或回復舊快照 | 只在拖曳完成提交偏好；沿用 save lock／epoch 與既有競態測試 |
| 分頁卸載造成草稿／Blur 行為改變 | 保留 Input Entity 與訂閱；測試未送出輸入與 interval Blur |
| 新資訊浮層穿透快捷鍵／滾輪 | 延伸現有受控 Popover 與 viewer 事件保護，使用真實事件測試 |
| 改版讓語言、錯誤復原或次要功能消失 | 保留選單／Preferences 命令，逐項測試原有功能入口 |
| 無可用平台或 CI 證據 | 提供明確所需平台、命令與 revision；保持未完成，不降低 gate |

## 10. 完成與回復

1. 完成時逐項記錄 P1–P6、A1–A14 與平台 gate 的結果和證據位置。
2. 提供變更摘要、驗證命令、原生畫面位置與仍屬額外 review 的限制。
3. 若需要回復實作，只回復本次核准的程式、文件與 release note，不回復使用者其他修改。
4. 回到舊版前依第 5.2 節保留設定備份並移除新增欄位；不刪除閱讀歷史或來源圖片。

## 11. 執行紀錄與剩餘 blocker

1. 本機已完成 P1–P4 的程式與 Windows 行為驗證；P5/P6 的必要平台完成條件尚未全數成立。
2. 最終 Windows 全套測試為預設並行 166 passed / 2 failed（既有 IPC ConnectionReset）；序列執行 168 passed / 0 failed，無 ignored。較早的序列執行曾失敗，最終已全過；仍需正常並行 CI 結果。queue test 在基準 `d41f28b` 也重現，原因未定位；未改 IPC 或刪除測試掩蓋失敗。
3. A1 的 Windows bounds／事件矩陣、A2–A12 的本機互動／設定檢查通過；UI 共 60 項，設定共 24 項。A13 的全套序列測試通過；A1 的 Metal 證據、必要 macOS gate 與正常並行 CI 尚未完成。
4. macOS arm64 tests／Clippy／Metal 與 before/after、Intel macOS compile／Clippy 尚未執行；不以 HTML 原型圖片替代。
5. [PR #13](https://github.com/narumiruna/mangameeya-rust/pull/13) 已建立。WSL GitHub CLI 授權可用，先前 Windows CLI 的登入 blocker 已解除。
   - 2026-11-07 合併上游 `main` 的 `986512f`，保留已發布的 `0.3.0` metadata、已消耗的 release notes 與三語 README 配置。
   - README 衝突以保留英文主頁並將桌面 UI 操作、面板欄位與安全降版說明同步至三語文件解決；Rust 程式、測試與 dependencies 未改動。
   - 合併後 fmt、Clippy、原生 rendering 與版本整合測試通過；序列測試 168 項通過，預設並行為 167 passed / 1 failed（既有 IPC queue ConnectionReset）。正常 CI 與 macOS 證據仍待確認。
6. 本機證據、已修復的 review findings、21 項 release checks 的 Linux／Windows 分工與限制見 `docs/verification.md`。
7. 目前不得回報 `DONE`。保留必要驗證與交付 blocker，不降低既定驗收標準。

### 核准記錄

- 初版草案：2026-11-07。
- 使用者核准：2026-11-07，指定本計畫並授權端到端實作及 PR 交付。
- 實作開始：2026-11-07；分支 `narumi/feat/desktop-efficiency-ui`，基準與 `origin/main` 同為 `d41f28b`。
- 基準檢查：Windows MSVC `cargo test --locked --all-features` 為 139 passed / 3 failed；兩個 IPC rejection tests 出現既有 ConnectionReset，一個設定備份測試出現暫時 AccessDenied。設定測試單獨重跑通過，IPC queue 測試單獨仍失敗；保留紀錄，不視為本次成功。基準 production rendering 通過。
- 必要檢查：Windows UI 60 項、設定 24 項與 production rendering 通過；fmt、Clippy、changenote 與跨環境合計 21 項 release tests 通過。完整套件序列執行 168 項全過，預設並行仍有基準也重現的 IPC 失敗。PR #13 已建立，macOS／正常 CI 證據仍未取得；詳見第 11 節。
