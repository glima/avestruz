#!/usr/bin/env python3
"""Take the picture the page shows where the scene is not drawn.

Usage: still.py SITE_DIR OUT.jpg [SUBJECT]

Serves the assembled site (`just site`), opens it with the scene on in a
headless Chromium, lets it settle and saves what the first screen shows,
without the words over it. With SUBJECT, the picture is of the scene gone up
to that subject (`keys`, `zaps`…), words and all: for looking, not for the
page.
"""

import asyncio
import functools
import http.server
import sys
import threading

from playwright.async_api import async_playwright

SIZE = {"width": 1600, "height": 900}
# The page hides its own words for the picture it keeps.
BARE = ".nav, .hero-text, .tour-strip, .stage-label { visibility: hidden !important; }"


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args) -> None:
        pass


async def shoot(url: str, out: str, subject: str | None) -> None:
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(args=["--use-angle=vulkan", "--enable-gpu", "--ignore-gpu-blocklist", "--enable-unsafe-swiftshader"])
        page = await browser.new_page(viewport=SIZE)
        page.on("console", lambda message: message.type in ("error", "warning") and print(f"[{message.type}] {message.text[:300]}"))
        page.on("pageerror", lambda error: print(f"[page error] {error}"))
        page.on("requestfailed", lambda request: print(f"[failed] {request.url}"))
        await page.goto(f"{url}?mode=hifi" + (f"&see={subject}" if subject else ""))
        await page.wait_for_selector("html.scene-ready", timeout=60_000)
        if not subject:
            await page.add_style_tag(content=BARE)
        await page.wait_for_timeout(2500)
        await page.locator(".hero").screenshot(path=out, type="jpeg", quality=82)
        await browser.close()


def main() -> None:
    site, out = sys.argv[1], sys.argv[2]
    subject = sys.argv[3] if len(sys.argv) > 3 else None
    handler = functools.partial(Quiet, directory=site)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        asyncio.run(shoot(f"http://127.0.0.1:{server.server_address[1]}/", out, subject))
    finally:
        server.shutdown()
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
