//! Reader interface languages and complete, compile-time translation catalog.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "zh-Hant")]
    TraditionalChinese,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::English, Self::Japanese, Self::TraditionalChinese];

    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Japanese => "ja",
            Self::TraditionalChinese => "zh-Hant",
        }
    }

    pub const fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Japanese => "日本語",
            Self::TraditionalChinese => "正體中文",
        }
    }

    /// GPUI uses a region tag for Traditional Chinese. Its missing Japanese
    /// component strings fall back to English; reader text uses our catalog.
    pub const fn component_locale(self) -> &'static str {
        match self {
            Self::TraditionalChinese => "zh-TW",
            _ => self.code(),
        }
    }

    /// Substitute named values once. Inserted paths and diagnostic text are
    /// never parsed as templates, even when they contain braces.
    pub fn format(self, key: Text, args: &[(&str, String)]) -> String {
        let mut rest = self.text(key);
        let mut result = String::new();
        while let Some(start) = rest.find('{') {
            result.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}').map(|end| start + end) else {
                break;
            };
            let name = &rest[start + 1..end];
            if let Some((_, value)) = args.iter().find(|(key, _)| *key == name) {
                result.push_str(value);
            } else {
                result.push_str(&rest[start..=end]);
            }
            rest = &rest[end + 1..];
        }
        result.push_str(rest);
        result
    }
}

// One row requires all three languages. Adding a key cannot silently fall back
// to English because of a missing catalog entry.
macro_rules! translations {
    ($($key:ident => ($en:literal, $ja:literal, $zh:literal),)*) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum Text { $($key,)* }

        impl Language {
            pub const fn text(self, key: Text) -> &'static str {
                match (self, key) {
                    $((Self::English, Text::$key) => $en,
                      (Self::Japanese, Text::$key) => $ja,
                      (Self::TraditionalChinese, Text::$key) => $zh,)*
                }
            }
        }

        #[cfg(test)]
        const ALL_TEXT: &[Text] = &[$(Text::$key,)*];
    };
}

translations! {
    BrowseFiles => ("Browse files", "ファイルを参照", "瀏覽檔案"),
    ContinueReading => ("Continue reading", "読書に戻る", "繼續閱讀"),
    BrowserTab => ("Browser", "閲覧", "瀏覽"),
    BrowserPageCount => ("Pages: {count}", "{count}ページ", "{count} 頁"),
    BrowserArea => ("File browser", "ファイルブラウザー", "檔案瀏覽區"),
    BrowserSearch => ("Search names", "名前を検索", "搜尋名稱"),
    BrowserSort => ("Sort", "並べ替え", "排序"),
    BrowserReverse => ("Descending", "降順", "降冪"),
    BrowserFoldersFirst => ("Folders first", "フォルダーを先頭に", "資料夾優先"),
    BrowserRefresh => ("Refresh", "更新", "重新整理"),
    BrowserOpen => ("Open selected", "選択項目を開く", "開啟選取項目"),
    BrowserShuffle => ("Shuffle", "シャッフル", "重新洗牌"),
    BrowserEmpty => ("No supported files in this folder.", "このフォルダーに対応ファイルはありません。", "此資料夾沒有支援的檔案。"),
    BrowserNoMatches => ("No matching names.", "一致する名前はありません。", "沒有符合的名稱。"),
    BrowserTileWidth => ("Cover width", "表紙の幅", "封面寬度"),
    BrowserShowNames => ("Show names", "名前を表示", "顯示名稱"),
    BrowserNameLines => ("Name lines", "名前の行数", "名稱行數"),
    BrowserMetadata => ("Show information", "情報を表示", "顯示資訊"),
    BrowserKnownHint => ("Pages and progress use known data only. Apply sorting again after covers load to include new data.", "ページ数と進捗には既知のデータのみを使用します。表紙の読み込み後に並べ替えを再適用すると、新しいデータが反映されます。", "頁數與進度僅使用已知資料。封面載入後，重新套用排序以納入新資料。"),
    BrowserSortNatural => ("Name: natural", "名前：自然順", "名稱：自然"),
    BrowserSortName => ("Name: text", "名前：文字順", "名稱：文字"),
    BrowserSortModified => ("Modified", "更新日時", "修改日期"),
    BrowserSortCreated => ("Created", "作成日時", "建立日期"),
    BrowserSortSize => ("File size", "ファイルサイズ", "檔案大小"),
    BrowserSortType => ("Type", "種類", "類型"),
    BrowserSortPages => ("Pages: known", "ページ数：既知", "頁數：已知"),
    BrowserSortProgress => ("Progress: known", "進捗：既知", "進度：已知"),
    BrowserSortRecent => ("Recently opened", "最近開いた順", "最近開啟"),
    BrowserSortRandom => ("Random", "ランダム", "隨機"),
    FitWindow => ("Fit window", "ウィンドウに合わせる", "適合視窗"),
    FitWidth => ("Fit width", "幅に合わせる", "適合寬度"),
    FitHeight => ("Fit height", "高さに合わせる", "適合高度"),
    OriginalSize => ("Original size", "原寸表示", "原始尺寸"),
    CustomZoom => ("Custom zoom", "カスタム倍率", "自訂縮放"),
    FolderPath => ("Folder path", "フォルダーのパス", "資料夾路徑"),
    InvalidPage => ("Page number must be a positive integer.", "ページ番号には正の整数を入力してください。", "頁碼必須是正整數。"),
    ZoomApplied => ("Zoom applied.", "表示倍率を適用しました。", "已套用縮放比例。"),
    InvalidZoom => ("Zoom must be 10–800%.", "表示倍率は 10–800% で指定してください。", "縮放必須是 10–800%。"),
    InvalidInterval => ("Slideshow interval must be 1–120 seconds.", "スライドショーの間隔は 1–120 秒で指定してください。", "投影片間隔必須是 1–120 秒。"),
    Ready => ("Ready", "準備完了", "就緒"),
    SlideshowEnded => ("Slideshow ended.", "スライドショーが終了しました。", "投影片已結束。"),
    BookOpening => ("Opening {path}…", "{path} を開いています…", "正在開啟 {path}…"),
    BookOpened => ("Opened {title} ({count} pages)", "{title} を開きました（{count} ページ）", "已開啟 {title}（{count} 頁）"),
    OpenImageFolder => ("Open image folder", "画像フォルダーを開く", "開啟圖片資料夾"),
    OpenComicOrImage => ("Open comic or image", "漫画または画像を開く", "開啟漫畫或圖片"),
    OpenDialogFailed => ("Cannot open file dialog: {error}", "ファイル選択画面を開けません：{error}", "無法開啟選擇視窗：{error}"),
    SaveDialogFailed => ("Cannot select save location: {error}", "保存先を選択できません：{error}", "無法選擇儲存位置：{error}"),
    ImageSaved => ("Original image saved.", "元の画像を保存しました。", "已儲存原始圖片。"),
    BookClosed => ("Comic closed.", "漫画を閉じました。", "已關閉漫畫。"),
    BookmarkSet => ("Bookmark: page {page}", "しおり：{page} ページ", "書籤：第 {page} 頁"),
    NoBookmark => ("This comic has no bookmark.", "この漫画にしおりはありません。", "本書尚無書籤。"),
    BookmarkUnavailable => ("The bookmarked image is not in this book's current image list.", "しおりの画像は現在の画像一覧にありません。", "書籤所指的圖片不在目前的圖片清單中。"),
    BookmarkDeleted => ("Bookmark deleted.", "しおりを削除しました。", "已刪除本書書籤。"),
    AboutTitle => ("About MangaMeeya Rust", "MangaMeeya Rust について", "關於 MangaMeeya Rust"),
    AboutDescription => ("Read image folders, ZIP/CBZ, and RAR/CBR offline. Source comics are not modified.", "画像フォルダー、ZIP/CBZ、RAR/CBR をオフラインで読めます。元の漫画は変更しません。", "離線閱讀圖片資料夾、ZIP/CBZ 與 RAR/CBR。原始漫畫不會被修改。"),
    PageLabel => ("Page {page}", "{page} ページ", "第 {page} 頁"),
    PageTitle => ("Page {page}: {name}", "{page} ページ：{name}", "第 {page} 頁：{name}"),
    Retry => ("Retry", "再試行", "重試"),
    PageLoading => ("Loading page {page}…", "{page} ページを読み込んでいます…", "正在讀取第 {page} 頁…"),
    OpenComic => ("Open comic…", "漫画を開く…", "開啟漫畫…"),
    OpenFolder => ("Open folder…", "フォルダーを開く…", "開啟資料夾…"),
    OpenOptions => ("Open options", "開くときのオプション", "開啟選項"),
    SaveImage => ("Save original image…", "元の画像を保存…", "另存原始圖片…"),
    CloseComic => ("Close comic", "漫画を閉じる", "關閉漫畫"),
    Quit => ("Quit", "終了", "結束"),
    ShowSidebar => ("Show sidebar", "サイドバーを表示", "顯示側欄"),
    HideInspector => ("Hide reading tools", "閲覧ツールを閉じる", "關閉閱讀工具"),
    ShowInspector => ("Show reading tools", "読書ツールを表示", "顯示閱讀工具"),
    Thumbnails => ("Thumbnails", "サムネイル", "縮圖"),
    ContentNavigation => ("Contents", "コンテンツ", "內容導覽"),
    PageCount => ("{count} pages", "{count} ページ", "{count} 頁"),
    DirectionRtl => ("Right → left", "右 → 左", "右 → 左"),
    DirectionLtr => ("Left → right", "左 → 右", "左 → 右"),
    Rotation => ("Rotation", "回転", "旋轉"),
    FiltersScope => ("Resets brightness, contrast, and grayscale only. Rotation and reading preferences stay unchanged.", "明るさ・コントラスト・グレースケールのみをリセットします。回転と閲覧設定は変更しません。", "只重設亮度、對比與灰階，不變更旋轉或閱讀偏好。"),
    SourceUnchanged => ("Display adjustments do not change source images.", "表示の調整は元の画像を変更しません。", "顯示調整不會修改來源圖片。"),
    ReadingMode => ("Reading mode", "閲覧モード", "閱讀模式"),
    SinglePage => ("Single page", "単ページ", "單頁"),
    Spread => ("Two-page spread", "見開き", "雙頁跨頁"),
    ContinuousPages => ("Continuous pages", "連続表示", "連續頁面"),
    RightToLeft => ("Read right to left", "右から左に読む", "右向左閱讀"),
    LeftToRight => ("Read left to right", "左から右に読む", "左向右閱讀"),
    Fullscreen => ("Fullscreen", "全画面表示", "全螢幕"),
    ZoomIn => ("Zoom in", "拡大", "放大"),
    ZoomOut => ("Zoom out", "縮小", "縮小"),
    FirstPage => ("First page", "最初のページ", "第一頁"),
    PreviousPage => ("Previous page", "前のページ", "上一頁"),
    NextPage => ("Next page", "次のページ", "下一頁"),
    LastPage => ("Last page", "最後のページ", "最後一頁"),
    Slideshow => ("Slideshow", "スライドショー", "投影片"),
    SetBookmark => ("Set or update bookmark", "しおりを追加・更新", "加入／更新書籤"),
    GoToBookmark => ("Go to bookmark", "しおりに移動", "跳至書籤"),
    DeleteBookmark => ("Delete bookmark", "しおりを削除", "刪除本書書籤"),
    RotateLeft => ("Rotate left", "左に回転", "向左旋轉"),
    RotateRight => ("Rotate right", "右に回転", "向右旋轉"),
    ResetFilters => ("Reset filters", "フィルターをリセット", "重設濾鏡"),
    Preferences => ("Preferences", "設定", "偏好設定"),
    PreferencesOpenFailed => ("Cannot open Preferences: {error}", "設定ウィンドウを開けません：{error}", "無法開啟偏好設定：{error}"),
    RememberPositionDescription => ("Continue from your last page when you reopen a comic.", "漫画を開き直したときに、最後に読んだページから再開します。", "重新開啟漫畫時，從上次閱讀的頁面繼續。"),
    ReadingDirectionDescription => ("Set the page order for reading.", "ページを読む順序を設定します。", "設定閱讀時的頁面順序。"),
    ReadingBackgroundDescription => ("Change the reading canvas, not the window theme.", "閲覧領域の背景だけを変更します。ウィンドウのテーマは変わりません。", "只變更閱讀區背景，不變更視窗主題。"),
    LanguageDescription => ("Change the language in both windows.", "両方のウィンドウの表示言語を変更します。", "變更兩個視窗的顯示語言。"),
    PreferencesAutosave => ("Changes apply immediately. Settings save automatically.", "変更はすぐに反映されます。設定は自動保存されます。", "變更會立即套用，設定會自動儲存。"),
    AboutMenu => ("About MangaMeeya Rust…", "MangaMeeya Rust について…", "關於 MangaMeeya Rust…"),
    Zoom => ("Zoom", "表示倍率", "縮放"),
    ZoomSummary => ("Zoom settings, {mode}, {percent}", "表示倍率の設定、{mode}、{percent}", "縮放設定，{mode}，{percent}"),
    ZoomTip => ("Select a fit mode or enter a zoom percentage. For spreads, the percentage refers to the current page.", "表示方法または倍率を指定します。見開きの倍率は現在のページを基準にします。", "選擇適應方式或輸入縮放比例。雙頁時，百分比以目前頁面為準。"),
    ZoomLevel => ("Zoom level", "表示倍率", "縮放比例"),
    ZoomPercentage => ("Zoom percentage", "表示倍率（%）", "縮放百分比"),
    FitMode => ("Fit mode", "表示方法", "適應方式"),
    FitWindowDescription => ("Show the whole page", "ページ全体を表示", "完整顯示整頁"),
    FitWidthDescription => ("Fit to the reading area's width", "閲覧領域の幅に合わせる", "依閱讀區寬度縮放"),
    FitHeightDescription => ("Fit to the reading area's height", "閲覧領域の高さに合わせる", "依閱讀區高度縮放"),
    OriginalDescription => ("Show the image at its original size", "画像を原寸で表示", "依圖片原尺寸顯示"),
    ZoomSettings => ("Zoom settings", "表示倍率の設定", "縮放設定"),
    File => ("File", "ファイル", "檔案"),
    View => ("View", "表示", "檢視"),
    Navigate => ("Navigate", "移動", "移動"),
    Bookmarks => ("Bookmarks", "しおり", "書籤"),
    Bookmark => ("Bookmark", "しおり", "書籤"),
    Tools => ("Tools", "ツール", "工具"),
    Help => ("Help", "ヘルプ", "說明"),
    Open => ("Open…", "開く…", "開啟…"),
    TwoPages => ("Two pages", "見開き", "雙頁"),
    Continuous => ("Continuous", "連続", "連續"),
    Stop => ("Stop", "停止", "停止"),
    Recent => ("Recent", "最近開いた項目", "最近開啟"),
    NoBook => ("No comic open", "漫画が開かれていません", "尚未開啟漫畫"),
    NoHistory => ("No recent comics", "最近開いた漫画はありません", "尚無開啟歷史"),
    ClearHistory => ("Clear history", "履歴を消去", "清除歷史"),
    ReadFailed => ("Cannot load", "読み込めません", "無法讀取"),
    ParentFolder => ("Parent", "上の階層", "上層"),
    OpenCurrentFolder => ("Open this folder", "このフォルダーを開く", "開啟此資料夾"),
    FilesTree => ("Files tree", "ファイルツリー", "檔案樹"),
    FilesBack => ("Back to previous sidebar location", "前の参照先に戻る", "返回上一個側欄位置"),
    FilesLocate => ("Locate current book", "読書中の本を表示", "定位目前書籍"),
    FilesSearchLoaded => ("Search loaded folders", "読込済みフォルダーを検索", "搜尋已載入資料夾"),
    FilesSearchHint => ("Loaded folders only", "読込済みフォルダーのみ", "僅限已載入資料夾"),
    FilesHidden => ("Show hidden items", "隠し項目を表示", "顯示隱藏項目"),
    FilesReadSelected => ("Read selection", "選択項目を読む", "閱讀選取項目"),
    FilesReadFolder => ("Read this folder", "このフォルダーを読む", "閱讀此資料夾"),
    FilesExpand => ("Expand folder", "フォルダーを展開", "展開資料夾"),
    FilesCollapse => ("Collapse folder", "フォルダーを折りたたむ", "收合資料夾"),
    FilesCycle => ("Folder link points to an ancestor", "リンク先は上位フォルダーです", "資料夾連結指向上層資料夾"),
    FilesReading => ("Reading", "読書中", "閱讀中"),
    FilesHiddenRevealed => ("Hidden items shown to locate the current book", "読書中の本を表示するため隠し項目を表示しました", "已顯示隱藏項目以定位目前書籍"),
    FilesPathTip => ("Enter a folder path; Escape cancels changes", "フォルダーパスを入力；Escape で変更を取消", "輸入資料夾路徑；Escape 取消變更"),
    FolderLoading => ("Loading folder…", "フォルダーを読み込んでいます…", "正在讀取資料夾…"),
    ReadingTools => ("Reading tools", "読書ツール", "閱讀工具"),
    ImageTab => ("Image", "画像", "影像"),
    ReadingTab => ("Reading", "読書", "閱讀"),
    PlaybackTab => ("Playback", "再生", "播放"),
    GeneralSettings => ("General settings", "一般設定", "一般設定"),
    ImageAndPreferences => ("Image and preferences", "画像と設定", "影像與偏好設定"),
    ImageAdjustments => ("Image adjustments", "画像調整", "影像調整"),
    HideImageAdjustments => ("Hide image adjustments", "画像調整を閉じる", "隱藏影像調整"),
    Brightness => ("Brightness", "明るさ", "亮度"),
    Contrast => ("Contrast", "コントラスト", "對比"),
    Grayscale => ("Grayscale", "グレースケール", "灰階"),
    RotateLeftTip => ("Rotate left ([)", "左に回転（[）", "向左旋轉（[）"),
    RotateRightTip => ("Rotate right (])", "右に回転（]）", "向右旋轉（]）"),
    Reset => ("Reset", "リセット", "重設"),
    RememberPosition => ("Remember reading position", "読書位置を記憶する", "記住閱讀位置"),
    IncludeSubfolders => ("Include subfolders", "サブフォルダーを含める", "包含子資料夾"),
    SubfoldersTip => ("Subfolder changes apply the next time you open a comic.", "サブフォルダーの設定は次に漫画を開くときに適用します。", "包含子資料夾的變更在下次開啟時生效。"),
    IntervalLabel => ("Slideshow interval (1–120 seconds)", "スライドショーの間隔（1–120 秒）", "投影片間隔（1–120 秒）"),
    SlideshowInterval => ("Slideshow interval", "スライドショーの間隔", "投影片間隔"),
    ReadingBackground => ("Reading background", "閲覧背景", "閱讀背景"),
    Black => ("Black", "黒", "黑色"),
    Gray => ("Dark gray", "濃い灰色", "深灰"),
    Paper => ("Paper", "紙色", "紙色"),
    SettingsTip => ("Wheel to scroll; Ctrl+wheel to zoom. Settings save automatically.", "ホイールでスクロール、Ctrl＋ホイールで拡大・縮小。設定は自動保存します。", "滾輪捲動；Ctrl＋滾輪縮放。所有設定自動儲存。"),
    BackupReset => ("Back up and reset settings", "設定をバックアップしてリセット", "備份並重設設定"),
    SettingsReset => ("Settings reset. Original backup: {path}", "設定をリセットしました。元の設定のバックアップ：{path}", "設定已重設；原始檔備份：{path}"),
    EmptyHint => ("Drop a comic, or open an image folder or archive", "漫画をドロップするか、画像フォルダーまたは書庫を開いてください", "拖放漫畫，或開啟圖片資料夾與封存檔"),
    ReadingArea => ("Comic reading area", "漫画の閲覧領域", "漫畫閱讀區"),
    PageNumber => ("Page number", "ページ番号", "頁碼"),
    PagePosition => ("Page position", "ページ位置", "頁面位置"),
    ReadProgress => ("Read {percent}%", "読書の進捗 {percent}%", "閱讀進度 {percent}%"),
    PageInformation => ("Page information", "ページ情報", "頁面資訊"),
    Dimensions => ("Dimensions", "画像サイズ", "圖片尺寸"),
    FileSize => ("File size", "ファイルサイズ", "檔案大小"),
    NotAvailable => ("Not available", "未取得", "未取得"),
    Sidebar => ("Sidebar", "サイドバー", "側欄"),
    DropHint => ("Release to open comic", "ドロップして漫画を開く", "放開以開啟漫畫"),
    ReadingDirection => ("Reading direction", "読む方向", "閱讀方向"),
    Language => ("Language", "言語", "語言"),
    ErrorDetails => ("Error: {error}", "エラー：{error}", "錯誤：{error}"),
    SettingsError => ("Cannot save or load settings: {error}", "設定を保存または読み込みできません：{error}", "無法儲存或載入設定：{error}"),
    FolderError => ("Cannot read folder: {error}", "フォルダーを読み込めません：{error}", "無法讀取資料夾：{error}"),
    PageError => ("Cannot load page: {error}", "ページを読み込めません：{error}", "無法讀取頁面：{error}"),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .map(|part| part.split_once('}').expect("closed placeholder").0)
            .collect()
    }

    #[test]
    fn every_translation_is_nonempty_and_has_the_same_placeholders() {
        for &key in ALL_TEXT {
            let expected = placeholders(Language::English.text(key));
            for language in Language::ALL {
                let text = language.text(key);
                assert!(!text.trim().is_empty(), "{language:?} {key:?}");
                assert_eq!(placeholders(text), expected, "{language:?} {key:?}");
            }
        }
    }

    #[test]
    fn formatting_preserves_unicode_and_does_not_interpolate_values() {
        for language in Language::ALL {
            let title = "日本語/正體中文/{count}.cbz";
            let result = language.format(
                Text::BookOpened,
                &[("title", title.into()), ("count", "12".into())],
            );
            assert!(result.contains(title));
            assert!(result.contains("12"));
            assert!(!result.contains("{title}"));
        }
        assert_eq!(Language::English.format(Text::Ready, &[]), "Ready");
    }

    #[test]
    fn language_codes_and_names_are_stable() {
        for language in Language::ALL {
            let json = serde_json::to_string(&language).unwrap();
            assert_eq!(json, format!("{:?}", language.code()));
            assert_eq!(serde_json::from_str::<Language>(&json).unwrap(), language);
            assert!(!language.native_name().is_empty());
        }
        assert_eq!(Language::default(), Language::English);
        assert_eq!(Language::TraditionalChinese.component_locale(), "zh-TW");
    }
}
