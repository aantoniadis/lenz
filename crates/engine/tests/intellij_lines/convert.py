# Regenerates cases.rs from IntelliJ Community's LineComparisonUtilTest.kt (commit bc34875):
#   python3 convert.py <intellij-community>/platform/diff-impl/tests/testSrc/com/intellij/diff/comparison/LineComparisonUtilTest.kt cases.rs
"""Convert IntelliJ's LineComparisonUtilTest.kt `lines { }` blocks into a Rust golden-test table."""
import re, sys, json

src = open(sys.argv[1]).read()

# split into test functions to keep names
funcs = re.split(r'\n  fun ', src)
cases = []
for f in funcs[1:]:
    m = re.match(r'(`[^`]+`|\w+)\(\)', f)
    fname = m.group(1).strip('`') if m else 'unknown'
    for bi, block in enumerate(re.findall(r'lines \{(.*?)\n    \}', f, re.S)):
        # string literals with Kotlin '-' operator between them, possibly spanning lines
        pairs = re.findall(r'\(\s*"((?:[^"\\]|\\.)*)"\s*-\s*"((?:[^"\\]|\\.)*)"\s*\)(\.(default|trim|ignore)\(\))?', block, re.S)
        if not pairs:
            continue
        unesc = lambda t: t.replace('\\t', '\t').replace('\\n', '\n').replace('\\"', '"').replace('\\\\', '\\')
        pairs = [(unesc(a), unesc(b), c, d) for a, b, c, d in pairs]
        left, right = pairs[0][0], pairs[0][1]
        matchings = {}
        for l, r, _, pol in pairs[1:]:
            matchings[pol or 'default'] = (l, r)
        if pairs[0][3]:  # ("a" - "b").default() as the text itself is not used
            pass
        changes = {}
        for pol, args in re.findall(r'\n\s*(default|trim|ignore)\(([^()]*(?:\([^()]*\)[^()]*)*)\)', block):
            items = re.findall(r'(mod|ins|del)\(([^)]*)\)', args)
            lst = []
            for kind, nums in items:
                n = [int(x) for x in nums.split(',')]
                if kind == 'mod': a, b, c1, c2 = n
                elif kind == 'del': a, b, c1 = n; c2 = 0
                else: a, b, c2 = n; c1 = 0
                lst.append([a, a + c1, b, b + c2])
            changes[pol] = lst
        runs = []
        if 'testAll()' in block: runs = ['default', 'trim', 'ignore']
        for p in ['Default', 'Trim', 'Ignore']:
            if f'test{p}()' in block: runs.append(p.lower())
        if not runs:
            continue
        def pick(d, pol):
            order = {'ignore': ['ignore', 'trim', 'default'], 'trim': ['trim', 'default'], 'default': ['default']}[pol]
            for o in order:
                if o in d: return d[o]
            return None
        for pol in runs:
            ch = pick(changes, pol)
            mt = pick(matchings, pol)
            if ch is None and mt is None:
                continue
            cases.append(dict(name=f"{fname} #{bi}", left=left, right=right, policy=pol, changes=ch, matching=mt))

def rs(s):
    return json.dumps(s.replace('_', '\n'), ensure_ascii=False)

out = []
out.append("// Golden tests ported from IntelliJ IDEA Community Edition, LineComparisonUtilTest.kt")
out.append("// (platform/diff-impl/tests/testSrc/com/intellij/diff/comparison, commit bc34875).")
out.append("// Copyright 2000-2015 JetBrains s.r.o. Licensed under the Apache License, Version 2.0.")
out.append("// Modified: converted from the Kotlin DSL into a Rust table by convert.py (alongside); `_` in sources means newline.")
out.append("// GENERATED — do not edit by hand.\n")
out.append("use lenz_engine::{Options, WhitespaceMode};\n")
out.append("pub enum Expect { Changes(&'static [[u32; 4]]), Matching(&'static str, &'static str) }\n")
out.append("pub struct Case { pub name: &'static str, pub left: &'static str, pub right: &'static str, pub mode: WhitespaceMode, pub expect: Expect }\n")
out.append("pub fn options(mode: WhitespaceMode) -> Options { Options { whitespace: mode, ..Options::default() } }\n")
out.append("pub const CASES: &[Case] = &[")
mode = {'default': 'WhitespaceMode::Default', 'trim': 'WhitespaceMode::Trim', 'ignore': 'WhitespaceMode::Ignore'}
for c in cases:
    if c['changes'] is not None:
        exp = "Expect::Changes(&[" + ", ".join(f"[{a}, {b}, {cc}, {d}]" for a, b, cc, d in c['changes']) + "])"
    else:
        l, r = c['matching']
        exp = f"Expect::Matching({json.dumps(l)}, {json.dumps(r)})"
    out.append(f"    Case {{ name: {json.dumps(c['name'] + ' [' + c['policy'] + ']')}, left: {rs(c['left'])}, right: {rs(c['right'])}, mode: {mode[c['policy']]}, expect: {exp} }},")
out.append("];")
open(sys.argv[2], 'w').write("\n".join(out) + "\n")
print(len(cases), "cases")
