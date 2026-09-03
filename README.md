# MangaMeeya Rust

MangaMeeya Rust 是以 Rust 與 Tauri 2 為 Windows 重製的離線漫畫閱讀器。
介面與操作參考 `data/MangaMeeyaCE/` 中的 MangaMeeyaCE 2.4 Beta，但程式碼為全新實作。

## 功能

- 開啟圖片、圖片資料夾、ZIP/CBZ 與 RAR/CBR 漫畫。
- 自然排序頁名，例如 `2.jpg` 會排在 `10.jpg` 前面。
- 單頁、雙頁跨頁、垂直連續頁面，以及由右向左或由左向右閱讀。
- 適合視窗、適合寬度、適合高度、原始尺寸與 10%–800% 自訂縮放。
- 90 度旋轉、亮度、對比與灰階調整。
- 縮圖列、檔案瀏覽器、最近開啟、每本書閱讀位置與書籤。
- 鍵盤、滑鼠滾輪、畫面兩側點擊、拖放及 Windows 檔案關聯。
- 全螢幕、定時投影片及另存目前原始圖片。
- 第二次開啟程式時會沿用既有視窗並載入新的漫畫。

圖片資料只會在需要顯示時由 Rust 端讀取，並有每頁 128 MiB 的安全限制。
閱讀位置、書籤及偏好保存在 WebView 的 local storage，原始漫畫不會被修改。
加密 RAR 目前會顯示明確錯誤，不會要求或保存密碼。

## 主要操作

| 操作 | 按鍵 |
| --- | --- |
| 開啟漫畫 | `Ctrl+O` |
| 開啟資料夾 | `Ctrl+Shift+O` |
| 下一頁／上一頁 | `Space`、`PageDown`／`PageUp` |
| 依閱讀方向翻頁 | `←`、`→` |
| 第一頁／最後一頁 | `Home`／`End` |
| 單頁／雙頁 | `S` |
| 切換連續頁面 | `C` |
| 切換閱讀方向 | `D` |
| 適合視窗／寬度／高度／原尺寸 | `F`／`W`／`H`／`1` |
| 放大／縮小／重設 | `+`／`-`／`0` |
| 旋轉 | `[`／`]` |
| 新增書籤／跳至書籤 | `B`／`Shift+B` |
| 投影片 | `P` |
| 顯示側欄 | `Tab` |
| 全螢幕 | `F11` |

一般滾輪會捲動閱讀區，不會直接翻至上／下一頁；按住 `Ctrl` 使用滾輪則會縮放。

## Windows 開發

### 必要工具

- Windows 10 1803 以上或 Windows 11。
- Microsoft C++ Build Tools 的「Desktop development with C++」工作負載。
- Rust stable MSVC toolchain。
- Node.js 22 以上。
- WebView2 Runtime，Windows 11 已內建。

### 執行

```powershell
npm ci
npm run tauri dev
```

### 檢查

```powershell
npm run check
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

`npm ci` 會透過 Husky 安裝 Git hooks。Commit 前會執行 Biome、Rust 格式檢查與 Clippy，push 前會執行 Rust 測試。若要自動格式化前端檔案，可執行 `npm run format`。

### 建立安裝檔

```powershell
npm run tauri build -- --bundles nsis
```

NSIS 安裝檔會出現在 `src-tauri/target/release/bundle/nsis/`。
安裝後 JPG、PNG、GIF、BMP、WebP、ZIP/CBZ 及 RAR/CBR 可透過 Windows 的「開啟方式」交給 MangaMeeya Rust。

## 自動建置

`.github/workflows/windows.yml` 會在 `windows-latest` 執行前端建置、Rust 測試、Clippy 及 NSIS 打包。
成功的工作流程會上傳 `mangameeya-rust-windows` artifact。

## 專案結構

- `src/`：TypeScript 使用者介面與閱讀操作。`src/main.ts` 的 DOM 狀態、Tauri 呼叫與事件生命週期緊密相依，因此目前集中管理，待形成穩定的責任邊界後再拆分。
- `src-tauri/src/lib.rs`：圖片、資料夾與封存檔讀取，以及 Tauri commands。
- `src-tauri/tauri.conf.json`：Windows 視窗、安裝程式及檔案關聯。
- `data/MangaMeeyaCE/`：僅供本機行為與外觀參考，不納入 Git。

## 授權

本專案依 [GNU Affero General Public License v3.0](LICENSE) 授權。
MangaMeeya 名稱及原始應用程式相關權利屬各自權利人所有。
