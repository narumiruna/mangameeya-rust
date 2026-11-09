"""Check the local sidebar prototypes with Playwright. No application dependencies."""

import argparse
import json
import tempfile
from html.parser import HTMLParser
from pathlib import Path

from playwright.sync_api import expect, sync_playwright

ROOT = Path(__file__).resolve().parent
VARIANTS = ["01-compact.html", "02-tree.html", "03-reading-first.html"]
checks = 0


def check(condition, message):
    global checks
    assert condition, message
    checks += 1


class LocalLinks(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links = []

    def handle_starttag(self, tag, attrs):
        for key, value in attrs:
            if key in ("href", "src") and value:
                self.links.append(value)


def verify_links():
    for filename in ["index.html", *VARIANTS]:
        parser = LocalLinks()
        parser.feed((ROOT / filename).read_text(encoding="utf-8"))
        for link in parser.links:
            if link.startswith(("data:", "#")):
                continue
            check("://" not in link, f"External resource in {filename}: {link}")
            check((ROOT / link.split("#")[0]).exists(), f"Missing local link: {link}")


def assert_no_overflow(page, label):
    check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), f"Document overflow: {label}")
    check(page.evaluate("document.querySelector('.sidebar')?.scrollWidth <= document.querySelector('.sidebar')?.clientWidth"), f"Sidebar overflow: {label}")


def verify_variant(browser, filename, output):
    global checks
    before = checks
    context = browser.new_context(viewport={"width": 1600, "height": 1050}, offline=True, reduced_motion="reduce")
    page = context.new_page()
    errors, failed, external = [], [], []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.on("console", lambda message: errors.append(message.text) if message.type == "error" else None)
    page.on("requestfailed", lambda request: failed.append(f"{request.url}: {request.failure}"))
    page.on("request", lambda request: external.append(request.url) if request.url.startswith(("http:", "https:")) else None)
    url = (ROOT / filename).as_uri()
    is_tree = filename.startswith("02")
    prefix = filename.removesuffix(".html")

    def reset():
        page.goto(url)
        expect(page.locator('[data-entry="n03"]')).to_be_visible()

    def path_to(path):
        page.locator("#location-button").click()
        page.locator("#path-editor").fill(path)
        page.locator("#path-editor").press("Enter")
        expect(page.locator("#location-button")).to_be_visible()

    reset()
    expect(page.locator('#tab-files')).to_have_attribute("aria-selected", "true")
    expect(page.locator('[data-entry="n03"]')).to_have_attribute("aria-current", "true")
    check("\\\\?\\" not in page.locator("#location-button").inner_text(), "Raw native path shown")
    check(page.locator('.sidebar-heading').inner_text() == "內容導覽\nLIBRARY", "Book page count leaked into file header")
    xs = page.locator('[data-entry^="n0"] .entry-icon').evaluate_all("nodes => nodes.filter(n => !n.closest('[hidden]')).map(n => n.getBoundingClientRect().x)")
    check(max(xs) - min(xs) < 1, "Sibling file icons must align")
    page.screenshot(path=output / f"{prefix}-desktop.png", full_page=True)

    # Selection does not read. Mouse double-click and Enter explicitly read.
    page.locator('[data-entry="n04"]').click()
    expect(page.locator("#canvas-title")).to_have_text("夜行列車 · 第 03 卷")
    expect(page.locator('[data-entry="n04"]')).to_have_attribute("aria-pressed", "true")
    expect(page.locator('[data-entry="n03"]')).to_have_attribute("aria-current", "true")
    page.screenshot(path=output / f"{prefix}-selection.png", full_page=True)
    page.locator('[data-entry="n04"]').dblclick()
    expect(page.locator("#canvas-title")).to_have_text("夜行列車 · 第 04 卷")
    expect(page.locator("#page-number")).to_have_text("1")
    page.locator("#next-page").click()
    expect(page.locator("#page-number")).to_have_text("2")
    page.locator("#previous-page").click()
    expect(page.locator("#page-number")).to_have_text("1")
    expect(page.locator("#previous-page")).to_be_disabled()
    page.locator('[data-entry="n05"]').focus()
    page.keyboard.press("Enter")
    expect(page.locator("#canvas-title")).to_have_text("夜行列車 · 第 05 卷")
    checks += 10

    # Folder navigation keeps the current reader and locate restores context.
    page.locator("#action-parent").click()
    expect(page.locator("#location-button")).to_contain_text("Comics")
    page.locator('[data-entry="short"]').click()
    expect(page.locator("#location-button")).to_contain_text("短篇集")
    expect(page.locator("#canvas-title")).to_have_text("夜行列車 · 第 05 卷")
    page.locator("#locate-current").click()
    expect(page.locator("#location-button")).to_contain_text("夜行列車")
    expect(page.locator('[data-entry="n05"]')).to_be_focused()
    page.locator("#action-back").click()
    expect(page.locator("#location-button")).to_contain_text("短篇集")
    page.locator("#locate-current").click()
    checks += 6

    # Folder browsing keeps focus and restores the previous query and selection.
    reset()
    page.locator('[data-entry="extras"]').focus()
    page.keyboard.press("Enter")
    expect(page.locator("#location-button")).to_contain_text("特典")
    check(page.evaluate("document.activeElement.tagName !== 'BODY'"), "Folder navigation lost focus")
    expect(page.locator("#canvas-title")).to_have_text("夜行列車 · 第 03 卷")
    reset()
    page.locator('[data-entry="n04"]').click()
    page.locator("#file-search").fill("第 0")
    page.locator("#action-parent").click()
    page.locator('[data-entry="short"]').click()
    page.locator("#action-back").click()
    page.locator("#action-back").click()
    expect(page.locator("#file-search")).to_have_value("第 0")
    expect(page.locator('[data-entry="n04"]')).to_have_attribute("aria-pressed", "true")
    checks += 4

    # Search, no results, native text deletion, and disabled filtered selection.
    reset()
    search = page.locator("#file-search")
    search.fill("第 04")
    expect(page.locator('[data-entry="n04"]')).to_be_visible()
    expect(page.locator('[data-entry="n03"]')).to_have_count(0)
    expect(page.locator("#read-selected")).to_be_disabled()
    expect(search).to_be_focused()
    search.press("Backspace")
    expect(search).to_have_value("第 0")
    expect(page.locator("#location-button")).to_contain_text("夜行列車")
    search.fill("no-matching-book")
    expect(page.locator(".empty-state")).to_contain_text("找不到")
    page.locator('[data-action="clear-search"]').click()
    expect(search).to_have_value("")
    expect(page.locator('[data-entry="n03"]')).to_be_visible()
    checks += 9

    # Hidden directories are opt-in. Parent and Back are separate actions.
    page.locator("#action-parent").click()
    expect(page.locator("#action-parent")).to_be_disabled()
    expect(page.locator('[data-entry="hidden"]')).to_have_count(0)
    page.locator("#hidden-toggle").click()
    expect(page.locator('[data-entry="hidden"]')).to_be_visible()
    expect(page.locator("#hidden-toggle")).to_have_attribute("aria-pressed", "true")
    page.locator("#hidden-toggle").click()
    expect(page.locator('[data-entry="hidden"]')).to_have_count(0)
    checks += 5

    # Invalid paths do not change the reader or the current directory.
    page.locator("#location-button").click()
    expect(page.locator("#path-editor")).to_have_value("D:\\Comics")
    page.locator("#path-editor").fill("Z:\\missing")
    page.locator("#path-editor").press("Enter")
    expect(page.locator("#path-error")).to_be_visible()
    expect(page.locator("#path-editor")).to_have_attribute("aria-invalid", "true")
    expect(page.locator("#canvas-title")).to_have_text("夜行列車 · 第 03 卷")
    page.locator("#path-editor").press("Escape")
    expect(page.locator("#location-button")).to_contain_text("Comics")
    expect(page.locator("#location-button")).to_be_focused()
    path_to("\\\\?\\D:\\Comics\\短篇集\\")
    expect(page.locator("#location-button")).to_contain_text("短篇集")
    # Cancel must not submit the form.
    page.locator("#location-button").click()
    page.locator("#path-editor").fill("Z:\\missing")
    page.locator('[data-action="cancel-path"]').click()
    expect(page.locator("#path-error")).to_have_count(0)
    expect(page.locator("#location-button")).to_contain_text("短篇集")
    checks += 9

    # Empty directories are explicit, and cannot start reading.
    path_to("D:/Comics/待整理/")
    expect(page.locator("#read-selected")).to_be_disabled()
    expect(page.locator('[data-action="read-folder"]')).to_be_disabled()
    expect(page.locator("#folder-empty" if is_tree else ".empty-state")).to_contain_text("沒有可閱讀的檔案")
    checks += 3

    # Folder-as-book has an explicit label and a visible result.
    path_to("D:\\Comics\\原稿")
    page.locator('[data-action="read-folder"]').click()
    expect(page.locator("#canvas-title")).to_have_text("原稿 · 資料夾書籍")
    expect(page.locator("#page-total")).to_have_text("3")
    page.locator("#next-page").click()
    expect(page.locator("#page-number")).to_have_text("2")
    page.locator("#locate-current").click()
    expect(page.locator("#location-button")).to_contain_text("原稿")
    checks += 4

    # Long filenames remain inside the sidebar and keep the full path in a tooltip.
    path_to("D:\\Comics\\夜行列車\\特典")
    expect(page.locator('[data-entry="bonus"]')).to_be_visible()
    check("完整收藏版" in page.locator('[data-entry="bonus"]').get_attribute("title"), "Long-name tooltip missing")
    assert_no_overflow(page, f"{filename} long filename")

    # File-list keys, tab-list keys, thumbnails, and recent books.
    reset()
    page.locator('[data-entry="n03"]').focus()
    page.keyboard.press("ArrowDown")
    expect(page.locator('[data-entry="n04"]')).to_be_focused()
    page.keyboard.press("ArrowUp")
    expect(page.locator('[data-entry="n03"]')).to_be_focused()
    page.locator("#tab-files").focus()
    page.keyboard.press("ArrowRight")
    expect(page.locator("#tab-thumbnails")).to_be_focused()
    expect(page.locator("#panel-thumbnails")).to_be_visible()
    page.locator('[data-page="48"]').click()
    expect(page.locator("#page-number")).to_have_text("48")
    expect(page.locator("#next-page")).to_be_disabled()
    page.locator("#tab-thumbnails").focus()
    page.keyboard.press("End")
    expect(page.locator("#tab-recent")).to_be_focused()
    page.locator('[data-recent="s01"]').click()
    expect(page.locator("#canvas-title")).to_have_text("雨後的書店")
    expect(page.locator('[data-recent="s01"]')).to_be_focused()
    page.locator("#tab-files").click()
    page.locator("#locate-current").click()
    expect(page.locator('[data-entry="s01"]')).to_have_attribute("aria-current", "true")
    checks += 10

    # Tree disclosures expand only. Folder labels navigate. Keyboard mirrors this.
    if is_tree:
        reset()
        page.locator('[data-expand="night"]').click()
        expect(page.locator('[data-entry="n03"]')).not_to_be_visible()
        expect(page.locator("#canvas-title")).to_have_text("夜行列車 · 第 03 卷")
        page.locator('[data-expand="short"]').click()
        expect(page.locator('[data-entry="s01"]')).to_be_visible()
        expect(page.locator("#location-button")).to_contain_text("夜行列車")
        page.locator('[data-entry="night"]').focus()
        page.keyboard.press("ArrowRight")
        expect(page.locator('[data-expand="night"]')).to_have_attribute("aria-expanded", "true")
        page.keyboard.press("ArrowLeft")
        expect(page.locator('[data-expand="night"]')).to_have_attribute("aria-expanded", "false")
        page.locator("#locate-current").click()
        expect(page.locator('[data-entry="n03"]')).to_be_visible()
        expect(page.locator('[data-entry="n03"]')).to_be_focused()
        # Right on a file is a no-op, including search-expanded branches.
        reset()
        page.locator('[data-entry="n03"]').focus()
        page.keyboard.press("ArrowRight")
        expect(page.locator('[data-entry="n03"]')).to_be_focused()
        page.locator("#file-search").fill("雨後")
        expect(page.locator('[data-expand="short"]')).to_have_attribute("aria-expanded", "true")
        expect(page.locator('[data-expand="short"]')).to_be_disabled()
        page.locator('[data-entry="short"]').focus()
        page.keyboard.press("ArrowRight")
        expect(page.locator('[data-entry="s01"]')).to_be_focused()
        page.keyboard.press("ArrowRight")
        expect(page.locator('[data-entry="s01"]')).to_be_focused()
        page.keyboard.press("ArrowLeft")
        expect(page.locator('[data-entry="short"]')).to_be_focused()
        page.keyboard.press("ArrowLeft")
        expect(page.locator('[data-entry="root"]')).to_be_focused()
        expect(page.locator('[data-expand="short"]')).to_have_attribute("aria-expanded", "true")
        page.locator("#file-search").fill("")
        expect(page.locator('[data-expand="short"]')).to_have_attribute("aria-expanded", "false")
        checks += 17

    # Viewport checks keep the sidebar usable; mobile focuses on the sidebar.
    for width, height in [(1600, 1050), (1280, 900), (1280, 850), (900, 1000), (700, 900), (390, 950)]:
        page.set_viewport_size({"width": width, "height": height})
        reset()
        assert_no_overflow(page, f"{filename} {width}")
        expect(page.locator("#read-selected")).to_be_in_viewport()
        expect(page.locator("#location-button")).to_be_in_viewport()
        expect(page.locator('[data-entry="n03"]'), f"Current book visible: {filename} at {width}x{height}").to_be_in_viewport(ratio=0.98)
        if filename.startswith("03"):
            expect(page.locator('[data-entry="n04"]'), f"Next book visible: {filename} at {width}x{height}").to_be_in_viewport(ratio=0.98)
            checks += 1
        checks += 3
        if width == 390:
            page.screenshot(path=output / f"{prefix}-narrow.png", full_page=True)
        if (width, height) == (1280, 850):
            page.screenshot(path=output / f"{prefix}-short-desktop.png", full_page=True)
        page.locator("#sidebar-toggle").click()
        expect(page.locator("#sidebar")).not_to_be_visible()
        expect(page.locator("#sidebar-toggle")).to_have_attribute("aria-expanded", "false")
        expect(page.locator(".reading-area")).to_be_visible()
        page.locator("#sidebar-toggle").click()
        expect(page.locator("#sidebar")).to_be_visible()
        checks += 4

    check(not errors, f"Browser errors in {filename}: {errors}")
    check(not failed, f"Failed resources in {filename}: {failed}")
    check(not external, f"External requests in {filename}: {external}")
    context.close()
    return {"page": filename, "checks": checks - before}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=None, help="Write screenshots and the JSON report here.")
    args = parser.parse_args()
    output = args.output_dir or Path(tempfile.mkdtemp(prefix="mangameeya-files-sidebar-"))
    output.mkdir(parents=True, exist_ok=True)
    verify_links()
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        version = browser.version
        results = [verify_variant(browser, name, output) for name in VARIANTS]
        page = browser.new_page(viewport={"width": 1600, "height": 1050}, offline=True)
        page.goto((ROOT / "index.html").as_uri())
        check(page.locator(".concept-link").count() == 3, "Comparison must link to three concepts")
        for width in (1600, 900, 390):
            page.set_viewport_size({"width": width, "height": 1050})
            check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), f"Comparison overflow at {width}")
            page.screenshot(path=output / f"comparison-{width}.png", full_page=True)
        browser.close()
    report = {"browser": f"Chromium {version}", "checks": checks, "results": results, "screenshots": str(output), "scope": "Local HTML and mock data only; not native GPUI or real filesystem verification."}
    (output / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
