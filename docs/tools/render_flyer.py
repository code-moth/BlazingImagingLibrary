# SPDX-License-Identifier: LGPL-2.1-or-later
"""Render the top-level flyer.html to flyer.pdf and docs/flyer/flyer-1.png and flyer-2.png.

The flyer uses IBM Plex, under the SIL Open Font License. The fonts are not kept in this
repository; clone github.com/IBM/plex and pass the clone's folder:

    python3 docs/tools/render_flyer.py PATH/TO/plex

Needs Playwright for Python with Chromium. Google Fonts is blocked during rendering, so the PDF
and the images use the local fonts and render the same with or without a network.
"""

import pathlib
import sys

from playwright.sync_api import sync_playwright

REPO = pathlib.Path(__file__).resolve().parents[2]
FACES = [
    ("IBM Plex Sans", "plex-sans", ["Regular:400", "Medium:500", "SemiBold:600"]),
    ("IBM Plex Mono", "plex-mono", ["Regular:400", "Medium:500"]),
    ("IBM Plex Sans Condensed", "plex-sans-condensed", ["SemiBold:600"]),
]


def font_faces(plex: pathlib.Path) -> str:
    rules = []
    for family, folder, weights in FACES:
        stem = family.replace(" ", "")
        for item in weights:
            name, weight = item.split(":")
            path = plex / "packages" / folder / "fonts/complete/woff2" / f"{stem}-{name}.woff2"
            if not path.exists():
                sys.exit(f"missing font: {path}")
            rules.append(
                f"@font-face{{font-family:'{family}';font-weight:{weight};"
                f"src:url('{path.as_uri()}') format('woff2');}}"
            )
    return "\n".join(rules)


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    faces = font_faces(pathlib.Path(sys.argv[1]).resolve())
    (REPO / "docs/flyer").mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 842, "height": 1200}, device_scale_factor=2)
        page.route("https://fonts.googleapis.com/**", lambda route: route.abort())
        page.goto((REPO / "flyer.html").as_uri())
        page.add_style_tag(content=faces)
        page.evaluate("document.fonts.ready")
        page.wait_for_timeout(500)
        for i, sheet in enumerate(page.query_selector_all(".page"), start=1):
            sheet.screenshot(path=str(REPO / f"docs/flyer/flyer-{i}.png"))
        page.pdf(
            path=str(REPO / "flyer.pdf"),
            width="794px",
            height="1123px",
            print_background=True,
            prefer_css_page_size=True,
        )
        browser.close()


if __name__ == "__main__":
    main()
