"""Build the BIL specification page from the exported Markdown."""
import re
import sys
import markdown

src, out, version = sys.argv[1], sys.argv[2], int(sys.argv[3])
md_text = open(src, encoding="utf-8").read()

# Drop the document title and byline; the page header replaces them.
lines = md_text.split("\n")
assert lines[0].startswith("# ")
body_lines = lines[1:]
while body_lines and not body_lines[0].startswith("## "):
    body_lines.pop(0)
md_text = "\n".join(body_lines)

open_count = len(re.findall(r"^- \[ \] ", md_text, flags=re.M))
done_count = len(re.findall(r"^- \[x\] ", md_text, flags=re.M))

md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"])
html = md.convert(md_text)

# ---- diagram -------------------------------------------------------------
W, H, yA, yB, c1, c2, c3 = 212, 72, 100, 228, 130, 380, 630


def box(cx, cy, main=False):
    cls = "d-box d-main" if main else "d-box"
    return f'<rect class="{cls}" x="{cx - W / 2}" y="{cy - H / 2}" width="{W}" height="{H}" rx="8"/>'


def label(cx, cy, name, a, b, main=False):
    q = "d-ink" if main else "d-quiet"
    return (
        f'<text class="d-name" x="{cx}" y="{cy - 12}" text-anchor="middle">{name}</text>'
        f'<text class="d-line {q}" x="{cx}" y="{cy + 4}" text-anchor="middle">{a}</text>'
        f'<text class="d-line {q}" x="{cx}" y="{cy + 20}" text-anchor="middle">{b}</text>'
    )


paths = [
    f"M{c1 + W / 2} {yA}H{c2 - W / 2}",
    f"M{c2 + W / 2} {yA}H{c3 - W / 2}",
    f"M{c3} {yA + H / 2}V164H{c1}V{yB - H / 2}",
    f"M{c1 + W / 2} {yB}H{c2 - W / 2}",
    f"M{c2 + W / 2} {yB}H{c3 - W / 2}",
    f"M{c1} {yB + H / 2}V312",
    f"M{c3} {yB + H / 2}V312",
]
diagram = f"""
<figure class="diagram">
  <div class="diagram-scroll">
  <svg viewBox="0 0 760 392" role="img" aria-label="BIL delivers pixels two ways: native Fragments and float32 Slices">
    <defs><marker id="bil-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path class="d-head" d="M0 0L10 5L0 10z"/></marker></defs>
    <text class="d-title" x="24" y="34">BIL delivers pixels two ways: native Fragments and float32 Slices</text>
    <g class="d-edges">{''.join(f'<path d="{p}" marker-end="url(#bil-arrow)"/>' for p in paths)}</g>
    {box(c1, yA)}{label(c1, yA, 'Files and streams', 'NITF, GeoTIFF, ENVI, HDF5,', 'raw camera, video')}
    {box(c2, yA)}{label(c2, yA, 'Driver', 'One per format family; wraps', 'an open source library')}
    {box(c3, yA)}{label(c3, yA, 'Source and its Images', 'Each Image is one 2D grid', 'with a single pixel type')}
    {box(c1, yB, True)}{label(c1, yB, 'Fragment', 'Native pixels of a Window;', 'zero-copy where possible', True)}
    {box(c2, yB)}{label(c2, yB, 'Slice function', 'Named mapping to one', 'or more named Slices')}
    {box(c3, yB, True)}{label(c3, yB, 'Slice', '2D float32 array, at most', 'one pass over the Window', True)}
    <rect class="d-box" x="24" y="312" width="712" height="56" rx="8"/>
    <text class="d-name" x="380" y="336" text-anchor="middle">Image processing code</text>
    <text class="d-line d-quiet" x="380" y="352" text-anchor="middle">Works on 2D arrays and takes native Fragments or float32 Slices</text>
    <text class="d-line d-quiet" x="{c2}" y="156" text-anchor="middle">read_fragment</text>
    <text class="d-line d-quiet" x="{c1 + 10}" y="292">native pixels</text>
    <text class="d-line d-quiet" x="{c3 + 10}" y="292">2D float32</text>
  </svg>
  </div>
  <figcaption>Pixel path, from file to Fragment to Slice.</figcaption>
</figure>
"""
html, n = re.subn(r"<p>(?:&#91;|\[)embedded content:[^<]*</p>", lambda m: diagram, html)
assert n == 1, f"diagram placeholder replaced {n} times"

# ---- post-processing -----------------------------------------------------
html = re.sub(r"<table>", '<div class="table-wrap"><table>', html)
html = html.replace("</table>", "</table></div>")
html = html.replace("<li>[x] ", '<li class="task done"><span class="sr">Decided: </span>')
html = html.replace("<li>[ ] ", '<li class="task open"><span class="sr">Open: </span>')
html = re.sub(r"<ul>\s*(?=<li class=\"task)", '<ul class="tasks">', html)
html = re.sub(r'<a href="(https?://[^"]+)"', r'<a href="\1" target="_blank" rel="noopener"', html)
assert "[ ] " not in html and "[x] " not in html

# ---- table of contents ---------------------------------------------------
def toc_html():
    items = []
    for t in md.toc_tokens:
        sub = ""
        if t["children"]:
            sub = "<ol>" + "".join(
                f'<li><a href="#{c["id"]}">{c["name"]}</a></li>' for c in t["children"]
            ) + "</ol>"
        items.append(f'<li><a href="#{t["id"]}">{t["name"]}</a>{sub}</li>')
    return "<ol>" + "".join(items) + "</ol>"


toc = toc_html()

# ---- header motif: an Image grid with one Window picked out --------------
cells = []
cols, rows, s, g = 14, 8, 9, 3
win = (4, 2, 6, 4)  # col, row, width, height
for r in range(rows):
    for c in range(cols):
        inside = win[0] <= c < win[0] + win[2] and win[1] <= r < win[1] + win[3]
        cells.append(
            f'<rect class="{"px in" if inside else "px"}" x="{c * (s + g)}" y="{r * (s + g)}" width="{s}" height="{s}" rx="1.5"/>'
        )
motif = (
    f'<svg class="motif" viewBox="0 0 {cols * (s + g) - g} {rows * (s + g) - g}" aria-hidden="true">'
    + "".join(cells)
    + "</svg>"
)

template = open(__file__.replace("build.py", "template.html"), encoding="utf-8").read()
page = (
    template.replace("{{BODY}}", html)
    .replace("{{TOC}}", toc)
    .replace("{{MOTIF}}", motif)
    .replace("{{OPEN}}", str(open_count))
    .replace("{{DONE}}", str(done_count))
    .replace("{{VERSION}}", str(version))
)
assert page.count(f">{version}</dd>") == 1, "version missing from the header"
assert "{{" not in page
open(out, "w", encoding="utf-8").write(page)
print(f"wrote {out}: {len(page)} bytes, {open_count} open, {done_count} decided, {len(md.toc_tokens)} sections")
