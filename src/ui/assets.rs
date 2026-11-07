//! Component assets plus the Lucide icons used by the reader.

use gpui_kit::{AssetSource, SharedString};
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(
    ReaderIcons,
    [
        ArrowLeftRight,
        Bookmark,
        Check,
        ChevronDown,
        ChevronLeft,
        ChevronRight,
        Columns2,
        Ellipsis,
        FileText,
        FolderOpen,
        Info,
        Maximize,
        PanelLeft,
        RotateCcw,
        RotateCw,
        Rows3,
        SlidersHorizontal,
        X,
    ]
);

/// The default GPUI Kit bundle does not contain every reader icon.
pub struct ReaderAssets;

impl AssetSource for ReaderAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = ReaderIcons.load(path)? {
            return Ok(Some(icon));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ReaderIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::assets::IconName;

    #[test]
    fn reader_and_component_icons_are_embedded_and_listed() {
        let paths = ReaderAssets.list("icons/").unwrap();
        for icon in [
            IconName::ArrowLeftRight,
            IconName::Bookmark,
            IconName::Check,
            IconName::ChevronDown,
            IconName::ChevronLeft,
            IconName::ChevronRight,
            IconName::Columns2,
            IconName::Ellipsis,
            IconName::FileText,
            IconName::FolderOpen,
            IconName::Info,
            IconName::Maximize,
            IconName::PanelLeft,
            IconName::RotateCcw,
            IconName::RotateCw,
            IconName::Rows3,
            IconName::SlidersHorizontal,
            IconName::X,
            // Component controls also need their default bundle.
            IconName::LoaderCircle,
        ] {
            let path = icon.path();
            let bytes = ReaderAssets.load(&path).unwrap().unwrap();
            assert!(bytes.starts_with(b"<svg"), "{path} must contain SVG data");
            assert!(paths.contains(&path), "{path} must be listed");
        }
        assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
