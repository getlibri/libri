"""Write tool descriptions (JSON: tool name -> description) into tools.rs,
replacing the literal in each tool's #[tool(description = ...)] attribute."""
import json, re, sys

src, over = sys.argv[1], json.load(open(sys.argv[2]))
s = open(src).read()
for name, text in over.items():
    pat = re.compile(r'(#\[tool\(\s*description\s*=\s*)"(?:[^"\\]|\\.)*"(\s*\)\]\s*(?:pub\s+)?(?:async\s+)?fn\s+'
                     + re.escape(name) + r'\()', re.S)
    lit = json.dumps(text, ensure_ascii=False)
    s, n = pat.subn(lambda m: m.group(1) + lit + m.group(2), s)
    if n != 1:
        sys.exit(f"{name}: {n} matches")
open(src, "w").write(s)
