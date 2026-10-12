"""Build a stand-alone BIL document page from Markdown, reusing the specification's page shell."""
import json, os, re, sys
import markdown

src, out, cfg_path = sys.argv[1], sys.argv[2], sys.argv[3]
cfg = json.load(open(cfg_path, encoding="utf-8"))
md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"])
html = md.convert(open(src, encoding="utf-8").read())
html = html.replace("<table>", '<div class="table-wrap"><table>').replace("</table>", "</table></div>")

def toc_items(tokens):
    lis = []
    for t in tokens:
        sub = f"<ol>{toc_items(t['children'])}</ol>" if t["children"] and t["level"] < 3 else ""
        lis.append(f'<li><a href="#{t["id"]}">{t["name"]}</a>{sub}</li>')
    return "".join(lis)
toc = "<ol>" + toc_items(md.toc_tokens) + "</ol>"

t = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "template.html"), encoding="utf-8").read()
def swap(old, new):
    global t
    assert t.count(old) == 1, old[:60]
    t = t.replace(old, new)
swap("<title>BIL Specification</title>", f"<title>{cfg['title']}</title>")
swap('<p class="eyebrow">Specification · Draft · Version {{VERSION}}</p>', f'<p class="eyebrow">{cfg["eyebrow"]}</p>')
swap("<h1>Blazing Imaging Library</h1>", f"<h1>{cfg['h1']}</h1>")
swap('<p class="lede">Extracts a processable subset of imaging sensor data, efficiently, from any format: as native Fragments or 2D float32 Slices.</p>', f'<p class="lede">{cfg["lede"]}</p>')
start, end = t.index('<dl class="control">'), t.index("</dl>") + len("</dl>")
items = "".join(f"<div><dt>{k}</dt><dd>{v}</dd></div>" for k, v in cfg["control"])
t = t[:start] + f'<dl class="control">{items}</dl>' + t[end:]
start, end = t.index("<footer>"), t.index("</footer>") + len("</footer>")
t = t[:start] + f"<footer>{cfg['footer']}</footer>" + t[end:]
page = t.replace("{{BODY}}", html).replace("{{TOC}}", toc).replace("{{MOTIF}}", cfg.get("motif", ""))
page = page.replace("{{VERSION}}", str(cfg.get("version", 1)))
assert "{{" not in page, re.findall(r"\{\{[A-Z]+\}\}", page)
open(out, "w", encoding="utf-8").write(page)
print(f"wrote {out}: {len(page)} bytes")
