"""Build the BIL red team findings page from redteam.md, reusing the specification's page shell."""
import re
import sys
import markdown

src, out, version, spec_version = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
md_text = open(src, encoding="utf-8").read()

# ---- collect findings for the summary table ------------------------------
severity = None
findings = []
for line in md_text.split("\n"):
    m = re.match(r"## (Critical|Serious|Lesser) findings", line)
    if m:
        severity = m.group(1)
    if line.startswith("## Symmetry and consistency"):
        severity = "Consistency"
    m = re.match(r"### ([UR]\d+)\. (.+)", line)
    if m:
        findings.append({"id": m.group(1), "title": m.group(2), "severity": severity})
    m = re.match(r"\*\*Status:\*\* ([A-Za-z0-9 ]+?) · \*\*Where:\*\* (.+)", line)
    if m:
        findings[-1]["status"] = m.group(1)
        findings[-1]["where"] = m.group(2)
assert all("status" in f for f in findings), [f["id"] for f in findings if "status" not in f]
us=[f["id"] for f in findings if f["id"][0]=="U"]; rs=[f["id"] for f in findings if f["id"][0]=="R"]
assert us == [f"U{i}" for i in range(1, len(us) + 1)], us
assert rs == [f"R{i}" for i in range(26, 26 + len(rs))], rs

rows = ["| Finding | Severity | Status |", "| --- | --- | --- |"]
for f in findings:
    rows.append(f"| [{f['id']}. {f['title']}](#{f['id'].lower()}) | {f['severity']} | {f['status']} |")
md_text = md_text.replace("{{SUMMARY}}", "\n".join(rows))

md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"])
html = md.convert(md_text)

# Finding headings get short, stable anchors: #r1, #r2 ...
html, n = re.subn(r'<h3 id="[^"]*">(([UR])(\d+))\. ', lambda m: f'<h3 id="{m.group(1).lower()}"><span class="fid">{m.group(1)}</span> ', html)
assert n == len(findings), n

def chip(status):
    return f'<span class="chip {status.lower().split()[0]}">{status}</span>'

html, n = re.subn(
    r"<p><strong>Status:</strong> ([A-Za-z0-9 ]+?) · <strong>Where:</strong> (.+?)</p>",
    lambda m: f'<p class="meta">{chip(m.group(1))}<span class="where">{m.group(2)[0].upper() + m.group(2)[1:]}</span></p>',
    html,
)
assert n == len(findings), n
html = re.sub(r"<td>(Open|Accepted|Rejected|Deferred|Resolved[^<]*)</td>", lambda m: f"<td>{chip(m.group(1))}</td>", html)
html = html.replace("<table>", '<div class="table-wrap"><table>').replace("</table>", "</table></div>")

# ---- table of contents ---------------------------------------------------
items = []
for t in md.toc_tokens:
    sub = ""
    kids = [c for c in t["children"]]
    if kids:
        lis = []
        for c in kids:
            m = re.match(r"(([UR])(\d+))\. (.+)", c["name"])
            lis.append(f'<li><a href="#{m.group(1).lower()}">{m.group(1)} {m.group(4)}</a></li>')
        sub = "<ol>" + "".join(lis) + "</ol>"
    items.append(f'<li><a href="#{t["id"]}">{t["name"]}</a>{sub}</li>')
toc = "<ol>" + "".join(items) + "</ol>"

# ---- header motif: one cell per finding, filled while it is open ---------
s, g, per_row = 14, 5, 9
cells = []
for i, f in enumerate(findings):
    r, c = divmod(i, per_row)
    cls = "px in" if f["status"] == "Open" else "px"
    cells.append(f'<rect class="{cls}" x="{c * (s + g)}" y="{r * (s + g)}" width="{s}" height="{s}" rx="2"/>')
nrows = (len(findings) + per_row - 1) // per_row
motif = (
    f'<svg class="motif" viewBox="0 0 {per_row * (s + g) - g} {nrows * (s + g) - g}" role="img" '
    f'aria-label="{sum(f["status"] == "Open" for f in findings)} of {len(findings)} findings open">'
    + "".join(cells) + "</svg>"
)

# ---- page shell: the specification's template with its header swapped -----
import os
t = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "template.html"), encoding="utf-8").read()

def swap(old, new):
    global t
    assert t.count(old) == 1, old[:60]
    t = t.replace(old, new)

n_open = sum(f["status"] == "Open" for f in findings)
n_done = sum(f["status"].startswith("Resolved") for f in findings)
counts = {k: sum(f["severity"] == k for f in findings) for k in ("Consistency", "Critical", "Serious", "Lesser")}

swap("<title>BIL Specification</title>", "<title>BIL Red Team: Pass 2</title>")
swap('<p class="eyebrow">Specification · Draft · Version {{VERSION}}</p>', f'<p class="eyebrow">Red team review, pass 2 · Version {version}</p>')
swap("<h1>Blazing Imaging Library</h1>", "<h1>BIL Red Team: Pass 2</h1>")
swap(
    '<p class="lede">Extracts a processable subset of imaging sensor data, efficiently, from any format: as native Fragments or 2D float32 Slices.</p>',
    f'<p class="lede">A second review of version {spec_version} of the Blazing Imaging Library specification: {counts["Consistency"]} symmetry and consistency findings and {len(findings) - counts["Consistency"]} red team findings, each with a proposed resolution and the decision it needs.</p>',
)
start, end = t.index('<dl class="control">'), t.index("</dl>") + len("</dl>")
t = t[:start] + f"""<dl class="control">
    <div><dt>Review version</dt><dd>{version}</dd></div>
    <div><dt>As of</dt><dd>12 October 2026</dd></div>
    <div><dt>Specification reviewed</dt><dd>Version {spec_version}</dd></div>
    <div><dt>Findings</dt><dd>{counts['Consistency']} consistency; {counts['Critical']} critical, {counts['Serious']} serious, {counts['Lesser']} lesser</dd></div>
    <div><dt>Status</dt><dd>{n_open} open, {n_done} resolved</dd></div>
    <div><dt>Method</dt><dd>Paper review; nothing was run</dd></div>
  </dl>""" + t[end:]
start, end = t.index("<footer>"), t.index("</footer>") + len("</footer>")
t = t[:start] + f"<footer>Red team findings, pass 2, for the Blazing Imaging Library (BIL) specification, review version {version}, against specification version {spec_version}, as of 12 October 2026. Pass 1 ended at R25.</footer>" + t[end:]
swap(
    "footer { margin-top: 64px;",
    """.fid { font-family: var(--font-mono); font-weight: 500; font-size: 0.8em; color: var(--accent); margin-right: 0.35em; }
.meta { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 12px; margin: 0 0 14px; color: var(--muted); font-size: 0.9rem; }
.chip { display: inline-block; font-family: var(--font-mono); font-size: 0.7rem; letter-spacing: 0.07em; text-transform: uppercase; padding: 2px 8px; border-radius: 3px; border: 1px solid var(--rule-strong); color: var(--muted); white-space: nowrap; }
.chip.open { border-color: var(--accent); color: var(--accent); background: var(--accent-soft); }
.chip.resolved, .chip.accepted { border-color: var(--fg); color: var(--fg); }
td a { color: var(--fg); text-decoration: none; }
td a:hover { color: var(--accent); text-decoration: underline; }
main h3 { margin-top: 44px; padding-top: 22px; border-top: 1px solid var(--rule); }
main h2 + p + h3 { border-top: 0; padding-top: 0; margin-top: 28px; }
main li > ul, main li > ol { margin-top: 6px; }
footer { margin-top: 64px;""",
)

page = (
    t.replace("{{BODY}}", html).replace("{{TOC}}", toc).replace("{{MOTIF}}", motif)
)
assert "{{" not in page, re.findall(r"\{\{[A-Z]+\}\}", page)
open(out, "w", encoding="utf-8").write(page)
print(f"wrote {out}: {len(page)} bytes, {len(findings)} findings, {n_open} open, {counts}")
