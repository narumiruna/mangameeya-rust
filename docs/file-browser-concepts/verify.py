"""Verify the local HTML prototypes, not the native Rust reader.

Run: uv run --no-project --with playwright python docs/file-browser-concepts/verify.py
If Chromium is unavailable: uv run --no-project --with playwright playwright install chromium
"""

from pathlib import Path
from tempfile import mkdtemp

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent
ARTIFACTS = Path(mkdtemp(prefix="mangameeya-browser-concepts-"))
PROPOSALS = ["01-grid", "02-explorer", "03-switcher", "04-integrated"]
checks = 0


def check(condition, message):
    global checks
    assert condition, message
    checks += 1


def ids(page):
    return page.locator("#entries .item").evaluate_all("nodes => nodes.map(n => n.dataset.id)")


def selected(page):
    return page.locator('#entries .item[aria-selected="true"]').get_attribute("data-id")


def browsing(page):
    return page.locator("#app").evaluate("node => node.classList.contains('mode-browser')")


def choose_sort(page, key, descending):
    page.locator("#sort").select_option(key)
    label = page.locator('[data-action="reverse"]').get_attribute("aria-label")
    if ("目前為降冪" in label) != descending:
        page.locator('[data-action="reverse"]').click()


def numeric_value(key, i):
    pages = None if i == 15 else 32 + (i * 11) % 105
    total = pages or 32
    current = 18 if i == 1 else total if i % 5 == 0 else int(total * .37) if i % 3 == 0 else 0
    return {
        "modified": ((i * 7) % 20 - 14),
        "created": None if i == 19 else -(22 + (i * 13) % 180),
        "size": None if i == 12 else 18 + ((i * 47) % 190) + i / 10,
        "pages": pages,
        "progress": current / pages if pages else None,
        "lastRead": ((i * 3) % 8) if i == 1 or i % 5 == 0 or i % 3 == 0 else None,
    }[key]


def check_numeric_order(page, key, descending):
    file_ids = [item for item in ids(page) if item.startswith("book-")]
    values = [numeric_value(key, int(item.split("-")[1]) - 1) for item in file_ids]
    known = [value for value in values if value is not None]
    check(known == sorted(known, reverse=descending), f"{key}: wrong numeric order")
    check(values == known + [None] * (len(values) - len(known)), f"{key}: missing values must be last")


def verify_proposal(browser, name):
    context = browser.new_context(viewport={"width": 1600, "height": 1000}, offline=True)
    page = context.new_page()
    errors, network = [], []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.on("console", lambda message: errors.append(message.text) if message.type == "error" else None)
    page.on("request", lambda request: network.append(request.url) if request.url.startswith(("http:", "https:")) else None)
    url = (ROOT / f"{name}.html").as_uri()
    page.goto(url)
    check(len(ids(page)) == 25, f"{name}: initial entries")
    if name == "04-integrated":
        check(not browsing(page), "integrated starts in reading mode")
        shell = [".native-toolbar", "#navigation", "#inspector", ".native-statusbar", "#content-slot"]
        bounds = {selector: page.locator(selector).bounding_box() for selector in shell}
        page.screenshot(path=str(ARTIFACTS / "04-integrated-reading-1600.png"), full_page=True)
        page.keyboard.press("Backspace")
        check(browsing(page), "integrated Backspace switches the center")
        for selector in shell:
            check(page.locator(selector).bounding_box() == bounds[selector], f"retained shell does not move: {selector}")
        check(page.locator('#browser').get_attribute('role') != 'dialog', "integrated is not a modal dialog")
        check(page.locator('#browser').evaluate("node => getComputedStyle(node).position") != "absolute", "integrated browser occupies the central slot, not an overlay")
        check(page.locator('#reader').is_hidden(), "reading canvas is replaced rather than exposed behind a modal")
    else:
        check(browsing(page), f"{name}: starts in browser mode")
    check(page.locator("#entries img").evaluate_all("nodes => nodes.every(n => n.complete && n.naturalWidth > 0)"), "covers load offline")
    page.screenshot(path=str(ARTIFACTS / f"{name}-1600.png"), full_page=True)

    # Numeric names make the difference between natural and literal sorting visible.
    page.locator("#search").fill("雨後的城市")
    choose_sort(page, "natural", False)
    check(ids(page) == ["book-1", "book-2", "book-3"], "natural names: 1, 2, 10")
    choose_sort(page, "name", False)
    check(ids(page) == ["book-1", "book-3", "book-2"], "literal names: 1, 10, 2")
    page.locator("#search").fill("")

    for key in ["modified", "created", "size", "pages", "progress", "lastRead"]:
        for descending in [False, True]:
            choose_sort(page, key, descending)
            check(set(ids(page)[:3]) == {"short", "scans", "empty"}, "folders first in both directions")
            check_numeric_order(page, key, descending)
    choose_sort(page, "type", False)
    types = [["CBZ", "CBZ", "CBZ", "CBR", "ZIP"][(int(item.split("-")[1]) - 1) % 5] for item in ids(page)[3:]]
    check(types == sorted(types), "file type sort")
    choose_sort(page, "modified", True)
    page.locator("#folders-first").uncheck()
    check(any(item not in {"short", "scans", "empty"} for item in ids(page)[:3]), "folder grouping can be disabled")
    page.locator("#folders-first").check()

    choose_sort(page, "random", False)
    first_shuffle = ids(page)
    choose_sort(page, "natural", False)
    choose_sort(page, "random", False)
    check(ids(page) == first_shuffle, "random order remains stable on redraw")
    page.locator('[data-action="shuffle"]').click()
    check(ids(page) != first_shuffle, "shuffle changes the order")

    # Text editing, no results, empty folders, parent boundary, and child books.
    page.locator("#search").fill("xyz")
    page.locator("#search").press("Backspace")
    check(page.locator("#search").input_value() == "xy" and browsing(page), "Backspace edits search")
    check(page.locator(".empty-state").is_visible(), "no-result message")
    page.locator('[data-action="clear"]').click()
    page.locator('#entries [data-id="empty"]').dblclick()
    check(page.locator(".empty-state").is_visible(), "empty directory state")
    page.locator("#browser").press("Backspace")
    check(len(ids(page)) == 25, "Backspace returns to parent")
    page.locator("#browser").press("Backspace")
    check(page.url == url and len(ids(page)) == 25, "root Backspace does not use browser history")
    page.locator('#entries [data-id="short"]').dblclick()
    check(len(ids(page)) == 6, "child directory navigation")
    page.locator('#entries [data-id="short-2"]').click()
    page.keyboard.press("Enter")
    check(not browsing(page), "Enter opens selected child book")
    before = page.locator('[data-page-count]').inner_text()
    page.keyboard.press("ArrowRight")
    after = page.locator('[data-page-count]').inner_text()
    check(after != before, "reader page navigation")
    page.keyboard.press("Backspace")
    check(browsing(page) and len(ids(page)) == 6 and selected(page) == "short-2", "reader returns to original child directory")
    page.keyboard.press("Enter")
    check(page.locator('[data-page-count]').inner_text() == after, "reading progress retained")
    page.keyboard.press("Backspace")
    page.keyboard.press("Backspace")

    # Restore non-default sort, filter, grouping, selection and scroll on mode changes.
    choose_sort(page, "modified", True)
    page.locator("#folders-first").uncheck()
    page.locator("#search").fill("cb")
    target = ids(page)[-2]
    page.locator(f'#entries [data-id="{target}"]').click()
    page.locator("#browser-scroll").evaluate("node => { node.scrollTop = Math.min(180, node.scrollHeight - node.clientHeight); }")
    page.wait_for_timeout(50)
    scroll = page.locator("#browser-scroll").evaluate("node => node.scrollTop")
    check(scroll > 0, "restoration check includes actual scroll")
    page.locator('[data-action="open"]').first.click()
    check(not browsing(page), "open button enters reader")
    reader_image = page.locator("#reading-page").evaluate("node => node.src")
    page.keyboard.press("Backspace")
    check(browsing(page), "Backspace opens browser")
    check(page.locator("#search").input_value() == "cb", "search retained")
    check(page.locator("#sort").input_value() == "modified", "sort retained")
    check(not page.locator("#folders-first").is_checked(), "grouping retained")
    check(selected(page) == target, "selection retained")
    restored = page.locator("#browser-scroll").evaluate("node => node.scrollTop")
    check(abs(scroll - restored) <= 1, f"scroll retained: {scroll} vs {restored}")
    page.locator('[data-action="resume"]').first.click()
    check(page.locator("#reading-page").evaluate("node => node.src") == reader_image, "resume retains reading page")
    page.keyboard.press("Backspace")

    if name == "01-grid":
        page.locator("#tile-size").fill("200")
        check(page.locator("#entries").evaluate("node => node.style.getPropertyValue('--tile-width')") == "200px", "cover size changes")
    if name == "02-explorer":
        page.locator("#search").fill("")
        page.locator('[data-sort-key="size"]').click()
        check(page.locator("#sort").input_value() == "size", "column header sorts")
        check(page.locator("th[aria-sort]").inner_text().strip() == "大小", "column sort indicator")
        check(page.locator("#preview h3").count() == 1, "selection preview")
    if name == "03-switcher":
        page.locator("#search").fill("")
        page.locator('[data-collection="reading"]').click()
        check(0 < len(ids(page)) < 22, "reading filter")
        page.locator('[data-collection="unread"]').click()
        check(0 < len(ids(page)) < 22, "unread filter")
        page.locator('[data-collection="all"]').click()
        check(page.locator("#reader").evaluate("node => node.inert && !node.hidden"), "reader remains mounted but inert behind dialog")
        page.locator('[data-action="open"]').first.focus()
        for _ in range(30):
            page.keyboard.press("Tab")
            check(page.locator("#browser").evaluate("node => node.contains(document.activeElement)"), "dialog contains Tab focus")
        check(page.locator("#reader-scroll").evaluate("node => getComputedStyle(node).justifyContent") == "flex-end", "background page visible beside overlay")

    if name == "04-integrated":
        page.locator("#search").fill("")
        page.locator('[data-tool-tab="browser"]').click()
        check(page.locator('#tool-browser').is_visible(), "integrated browser settings tab")
        page.locator('#tile-size').fill('180')
        check(page.locator('#entries').evaluate("node => node.style.getPropertyValue('--tile-width')") == '180px', "integrated cover sizing")
        check(page.locator('#tile-size-value').inner_text() == '180 px', "cover size output")
        page.locator('#show-filenames').uncheck()
        check(not page.locator('#entries .cover-title').first.is_visible(), "filenames can be hidden")
        page.locator('#show-filenames').check()
        page.locator('#filename-lines').select_option('3')
        check(page.locator('#entries .cover-title').first.evaluate("node => getComputedStyle(node).webkitLineClamp") == '3', "filename line limit")
        page.locator('#show-metadata').check()
        check(page.locator('#entries .cover-meta').first.is_visible(), "metadata can be shown")
        page.locator('#show-metadata').uncheck()
        check(page.locator('#entries .cover-meta').first.is_hidden(), "metadata can be hidden")
        check(page.locator('#native-page').is_disabled() and page.locator('#native-progress').is_disabled(), "reader controls are retained but inactive while browsing")
        check(page.locator('#browser').evaluate("node => getComputedStyle(node).backgroundColor") == 'rgb(8, 9, 11)', "browser canvas is black")

        for panel in ['navigation', 'inspector']:
            toggle = page.locator(f'.native-toolbar [data-toggle-panel="{panel}"]')
            toggle.click()
            check(page.locator(f'#{panel}').is_hidden(), f'{panel} can close independently')
            check(toggle.get_attribute('aria-expanded') == 'false', f'{panel} toggle state')
            toggle.click()
            check(page.locator(f'#{panel}').is_visible(), f'{panel} can reopen')
        page.locator('[data-tool-tab="reading"]').click()
        page.locator('[data-background="#eee8da"]').click()
        page.locator('[data-action="resume"]').first.click()
        check(page.locator('#native-page').is_enabled(), "reader page input enabled in reading mode")
        check(page.locator('#reader-scroll').evaluate("node => getComputedStyle(node).backgroundColor") == 'rgb(238, 232, 218)', "reading background changes")
        page.locator('#native-page').fill('12')
        page.locator('#native-page').press('Backspace')
        check(not browsing(page) and page.locator('#native-page').input_value() == '1', "Backspace edits page input instead of navigating")
        page.locator('#native-page').fill('12')
        page.locator('#native-page').press('Tab')
        check(page.locator('[data-page-count]').inner_text().startswith('12 /'), "page number jump")
        page.locator('#native-progress').fill('2')
        check(page.locator('[data-page-count]').inner_text().startswith('2 /'), "progress slider updates reading page")
        page.locator('#native-page').fill('9999')
        page.locator('#native-page').press('Tab')
        check(page.locator('#native-page').input_value() == page.locator('#native-page').get_attribute('max'), "page input clamps to book end")

        page.locator('[data-native-tab="thumbnails"]').click()
        check(page.locator('#nav-thumbnails img').count() == 6, "nearby page thumbnails")
        count = page.locator('[data-page-count]').inner_text()
        page.locator('[data-native-tab="thumbnails"]').press('ArrowRight')
        check(page.locator('[data-native-tab="recent"]').get_attribute('aria-selected') == 'true', "sidebar tab keyboard navigation")
        check(page.locator('[data-page-count]').inner_text() == count, "tab arrows do not turn comic pages")
        recent = page.locator('#nav-recent [data-open-entry]').nth(1)
        recent_id = recent.get_attribute('data-open-entry')
        recent.click()
        check(selected(page) == recent_id and not browsing(page), "recent entry opens in same reading canvas")
        page.locator('[data-native-tab="thumbnails"]').click()
        thumb = page.locator('#nav-thumbnails [data-jump-page]').first
        number = thumb.get_attribute('data-jump-page')
        thumb.click()
        check(page.locator('#native-page').input_value() == number, "thumbnail jumps to its page")
        page.locator('[data-native-tab="files"]').click()
        page.locator('#nav-files [data-folder="short"]').click()
        check(browsing(page) and len(ids(page)) == 6, "folder sidebar switches central slot to browser")
        page.locator('#nav-files [data-folder="root"]').click()
        check(len(ids(page)) == 25, "folder sidebar returns to root")

    # Responsive checks include very narrow review sizes, not native mobile support.
    for width in [1280, 900, 600, 390]:
        page.set_viewport_size({"width": width, "height": 1000})
        check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), f"{name}: document overflow at {width}")
        check(page.locator("#browser").evaluate("node => node.scrollWidth <= node.clientWidth + 1"), f"{name}: browser overflow at {width}")
        if name == '04-integrated' and width == 390:
            toggle = page.locator('.native-toolbar [data-toggle-panel="inspector"]')
            toggle.click()
            check(page.locator('#inspector').is_visible(), "narrow viewport can still open inspector")
            toggle.click()
            check(page.locator('#browser').is_visible(), "narrow inspector can return to central canvas")
        if width == 390:
            page.screenshot(path=str(ARTIFACTS / f"{name}-390.png"), full_page=True)
    page.reload()
    check(page.locator("#search").input_value() == "" and page.locator("#sort").input_value() == "natural", "reload resets memory-only state")
    check(not network, f"external requests: {network}")
    check(not errors, f"browser errors: {errors}")
    print(f"PASS {name}")
    context.close()


def verify_index(browser):
    page = browser.new_page(viewport={"width": 1600, "height": 1000}, offline=True)
    page.goto((ROOT / "index.html").as_uri())
    for link in page.locator("a[href]").all():
        href = link.get_attribute("href")
        check(not href.startswith(("http:", "https:")) and (ROOT / href).is_file(), f"index link: {href}")
    for width in [1600, 1280, 900, 600, 390]:
        page.set_viewport_size({"width": width, "height": 1000})
        check(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), f"index overflow at {width}")
    print("PASS index")
    page.close()


if __name__ == "__main__":
    with sync_playwright() as playwright:
        chromium = playwright.chromium.launch(headless=True)
        try:
            for proposal in PROPOSALS:
                verify_proposal(chromium, proposal)
            verify_index(chromium)
        finally:
            chromium.close()
    print(f"PASS {checks} checks. Screenshots: {ARTIFACTS}")
