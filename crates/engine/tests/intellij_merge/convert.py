# Regenerates cases.rs from IntelliJ Community's three-way comparison tests (commit bc34875):
#   T=<intellij-community>/platform/diff-impl/tests/testSrc/com/intellij/diff/comparison
#   python3 convert.py $T/CharComparisonMergeUtilTest.kt $T/LineComparisonMergeUtilTest.kt \
#     $T/WordMergeComparisonUtilTest.kt $T/MergeResolveUtilTest.kt cases.rs
"""Convert IntelliJ's `chars`/`lines_merge`/`lines_diff`/`words` three-way DSL blocks and MergeResolveUtilTest's
`test`/`testSimple` calls (the simple resolver only; `testGreedy` is not ported) into a Rust golden-test table."""
import re, sys, json

STR = r'"(?:[^"\\]|\\.)*"'
TRIPLE = r'\(\s*(' + STR + r')\s*-\s*(' + STR + r')\s*-\s*(' + STR + r')\s*\)((?:\.\w+\(\))*)'
KINDS = {'chars': 'Chars', 'lines_merge': 'LinesMerge', 'lines_diff': 'LinesDiff', 'words': 'Words'}
MODE = {'default': 'WhitespaceMode::Default', 'trim': 'WhitespaceMode::Trim', 'ignore': 'WhitespaceMode::Ignore'}


def kotlin_string(tok):
    if tok.startswith('"""'):
        return tok[3:-3]
    return re.sub(r'\\(.)', lambda m: {'t': '\t', 'n': '\n'}.get(m.group(1), m.group(1)), tok[1:-1])


def pick(d, pol):
    for o in {'ignore': ['ignore', 'trim', 'default'], 'trim': ['trim', 'default'], 'default': ['default']}[pol]:
        if o in d:
            return d[o]
    return None


cases, resolve = [], []
for path in sys.argv[1:-1]:
    src = open(path).read()
    suite = re.search(r'(\w+)Test\.kt', path).group(1)
    if suite == 'MergeResolveUtil':
        body = src.split('private fun testSimple')[0]
        for f in re.split(r'\n  fun ', body)[1:]:
            fname = re.match(r'(\w+)\(\)', f).group(1)
            for call, args in re.findall(r'\n    (test|testSimple|testGreedy)\((.*?)\n    \)', f, re.S):
                if call == 'testGreedy':
                    continue
                toks = re.findall(r'"""[\s\S]*?"""|' + STR + r'|null', args)
                vals = [None if t == 'null' else kotlin_string(t) for t in toks]
                base, left, right, expected = vals[:4]  # (base, left, right, expectedSimple[, expectedGreedy])
                resolve.append(dict(name=f"{fname} #{len([r for r in resolve if r['f'] == fname])}", f=fname,
                                    base=base, left=left, right=right, expected=expected))
        continue
    for f in re.split(r'\n  fun ', src)[1:]:
        fname = re.match(r'(\w+)\(\)', f).group(1)
        blocks = re.findall(r'\n    (chars|lines_merge|lines_diff|words) \{(.*?)\n    \}', f, re.S)
        for bi, (kind, block) in enumerate(blocks):
            block = re.sub(r'//.*', '', block)
            triples = re.findall(TRIPLE, block)
            texts = [kotlin_string(t).replace('_', '\n') for t in triples[0][:3]]
            matchings = {}
            for a, b, c, calls in triples:
                for pol in re.findall(r'\.(matching|default|trim|ignore)\(\)', calls):
                    matchings['default' if pol == 'matching' else pol] = [kotlin_string(x).replace('.', '') for x in (a, b, c)]
            changes = None
            m = re.search(r'changes\(((?:[^()]|\([^()]*\))*)\)', block)
            if m:
                changes = []
                for nums in re.findall(r'mod\(([^)]*)\)', m.group(1)):
                    l, b, r, cl, cb, cr = [int(x) for x in nums.split(',')]
                    changes.append([l, l + cl, b, b + cb, r, r + cr])
            if kind == 'words':
                runs = ['default', 'trim', 'ignore'] if 'testAll()' in block else []
            else:
                pm = re.search(r'test\((?:ComparisonPolicy\.(\w+))?\)', block)
                runs = [{'DEFAULT': 'default', 'IGNORE_WHITESPACES': 'ignore', 'TRIM_WHITESPACES': 'trim', None: 'default'}[pm.group(1)]]
            for pol in runs:
                mt = pick(matchings, pol) if kind == 'words' else matchings.get('default')
                cases.append(dict(name=f"{suite}: {fname} #{bi} [{pol}]", kind=KINDS[kind], texts=texts, policy=pol,
                                  changes=changes, matching=mt))


def rs(s):
    return 'None' if s is None else json.dumps(s, ensure_ascii=False)


out = [
    "// Golden tests ported from IntelliJ IDEA Community Edition: CharComparisonMergeUtilTest.kt,",
    "// LineComparisonMergeUtilTest.kt, WordMergeComparisonUtilTest.kt, MergeResolveUtilTest.kt",
    "// (platform/diff-impl/tests/testSrc/com/intellij/diff/comparison, commit bc34875).",
    "// Copyright 2000-2018 JetBrains s.r.o. Licensed under the Apache License, Version 2.0.",
    "// Modified: converted from the Kotlin DSL into a Rust table by convert.py (alongside); `_` in sources is already",
    "// replaced by newline; texts are ASCII, so IntelliJ's UTF-16 offsets are byte offsets. Greedy-resolver cases dropped.",
    "// GENERATED — do not edit by hand.\n",
    "use lenz_engine::WhitespaceMode;\n",
    "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
    "pub enum Kind { Chars, LinesMerge, LinesDiff, Words }\n",
    "pub struct Case {",
    "    pub name: &'static str,",
    "    pub kind: Kind,",
    "    /// Left, base, right.",
    "    pub texts: [&'static str; 3],",
    "    pub mode: WhitespaceMode,",
    "    /// `mod(..)` as [left start, left end, base start, base end, right start, right end].",
    "    pub changes: Option<&'static [[usize; 6]]>,",
    "    /// Left, base, right; lines for `lines_*`, characters otherwise.",
    "    pub matching: Option<[&'static str; 3]>,",
    "}\n",
    "pub const CASES: &[Case] = &[",
]
for c in cases:
    ch = 'None' if c['changes'] is None else "Some(&[" + ", ".join("[" + ", ".join(map(str, x)) + "]" for x in c['changes']) + "])"
    mt = 'None' if c['matching'] is None else "Some([" + ", ".join(json.dumps(x) for x in c['matching']) + "])"
    out.append(f"    Case {{ name: {json.dumps(c['name'])}, kind: Kind::{c['kind']}, texts: [{', '.join(json.dumps(t) for t in c['texts'])}], "
               f"mode: {MODE[c['policy']]}, changes: {ch}, matching: {mt} }},")
out.append("];\n")
out.append("/// MergeResolveUtilTest: `tryResolve(left, base, right)`; `None` = can't resolve.")
out.append("pub struct Resolve { pub name: &'static str, pub base: &'static str, pub left: &'static str, pub right: &'static str, pub expected: Option<&'static str> }\n")
out.append("pub const RESOLVE: &[Resolve] = &[")
for r in resolve:
    out.append(f"    Resolve {{ name: {json.dumps(r['name'])}, base: {rs(r['base'])}, left: {rs(r['left'])}, right: {rs(r['right'])}, "
               f"expected: {'None' if r['expected'] is None else 'Some(' + rs(r['expected']) + ')'} }},")
out.append("];")
open(sys.argv[-1], 'w').write("\n".join(out) + "\n")
by_kind = {}
for c in cases:
    by_kind[c['kind']] = by_kind.get(c['kind'], 0) + 1
print(len(cases), "cases", by_kind, len(resolve), "resolve")
