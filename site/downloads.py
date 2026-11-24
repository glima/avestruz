#!/usr/bin/env python3
"""Fill the site's download table from the latest GitHub release.

Usage: downloads.py INDEX_HTML < release.json

release.json is what `gh release view --json tagName,url,assets` prints. The
table goes where INDEX_HTML has its `<!-- downloads -->` marker. Assets are
matched by name, so the table lists what the release workflow published and
nothing else (checksums and provenance are linked beside it, not in it).
With nothing to read (no release yet, no `gh`, no network), the marker
becomes a card that says where releases are.
"""

import html
import json
import re
import sys

REPO = "https://github.com/glima/avestruz"
MARKER = "<!-- downloads -->"
PACKAGE = "cosmic-applet-avestruz"
ARCHES = ["x86_64", "aarch64"]
DEB_ARCH = {"amd64": "x86_64", "arm64": "aarch64"}

# (row label, file kind, pattern giving the architecture)
FORMATS = [
    ("Debian, Ubuntu, Pop!_OS", ".deb", re.compile(rf"^{PACKAGE}_[^_]+_(?P<arch>amd64|arm64)\.deb$")),
    ("Fedora and other RPM distributions", ".rpm", re.compile(rf"^{PACKAGE}-[0-9][^/]*\.(?P<arch>x86_64|aarch64)\.rpm$")),
    ("Any distribution", ".tar.gz", re.compile(rf"^{PACKAGE}-(?P<arch>x86_64|aarch64)\.tar\.gz$")),
]


def fallback() -> str:
    return (
        '<article class="card install-card downloads">\n'
        "  <h3>Packages</h3>\n"
        "  <p>Each release carries a <code>.deb</code>, an <code>.rpm</code> and a tarball, for x86_64 and aarch64, "
        f'with their checksums: see the <a href="{REPO}/releases">releases</a>.</p>\n'
        "</article>"
    )


def table(release: dict) -> str:
    found = {}
    for asset in release.get("assets", []):
        for _, kind, pattern in FORMATS:
            match = pattern.match(asset["name"])
            if match:
                found[(kind, DEB_ARCH.get(match["arch"], match["arch"]))] = asset
    if not found:
        return fallback()

    tag = html.escape(release["tagName"])
    url = html.escape(release["url"])
    rows = []
    for label, kind, _ in FORMATS:
        links = []
        for arch in ARCHES:
            asset = found.get((kind, arch))
            if asset:
                name = html.escape(asset["name"])
                links.append(f'<a href="{html.escape(asset["url"])}" title="{name}" aria-label="{name}">{arch}</a>')
        rows.append(f'<tr><th scope="row">{label} <small>{kind}</small></th><td>{" ".join(links) or "—"}</td></tr>')

    sums = next((a for a in release.get("assets", []) if a["name"] == "SHA256SUMS"), None)
    verify = (
        f'Check what you downloaded against <a href="{html.escape(sums["url"])}">SHA256SUMS</a>.'
        if sums
        else "Every release lists the checksums of its files."
    )
    return (
        '<article class="card install-card downloads">\n'
        "  <h3>Packages</h3>\n"
        f'  <p>From the latest release, <a href="{url}">{tag}</a>. They name what the applet needs to run.</p>\n'
        '  <div class="table-wrap"><table>\n'
        "    <tbody>\n      " + "\n      ".join(rows) + "\n    </tbody>\n"
        "  </table></div>\n"
        f'  <p class="note">{verify}</p>\n'
        "</article>"
    )


def main() -> None:
    index = sys.argv[1]
    raw = sys.stdin.read().strip()
    try:
        fragment = table(json.loads(raw)) if raw else fallback()
    except (ValueError, KeyError, TypeError):
        fragment = fallback()
    with open(index, encoding="utf-8") as f:
        page = f.read()
    if MARKER not in page:
        sys.exit(f"{index}: no {MARKER} marker")
    with open(index, "w", encoding="utf-8") as f:
        f.write(page.replace(MARKER, fragment))


if __name__ == "__main__":
    main()
