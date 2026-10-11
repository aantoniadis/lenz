# Regenerates cases.rs from IntelliJ Community's word/char/split comparison tests (commit bc34875):
#   T=<intellij-community>/platform/diff-impl/tests/testSrc/com/intellij/diff/comparison
#   python3 convert.py $T/WordComparisonUtilTest.kt $T/CharComparisonUtilTest.kt $T/SplitComparisonUtilTest.kt cases.rs
"""Convert IntelliJ's `lines_inner`/`words`/`chars*`/`splitter` DSL blocks into a Rust golden-test table."""
import re, sys, json

# DiffTestCase.kt's surrogate-pair constants
CONSTS = {'chSmile': '\U0001F602', 'chGun': '\U0001F52B', 'chMan': '\U0001F9D2', 'chFace': '\U0001F92B'}
STR = r'"(?:[^"\\]|\\.)*"|ch(?:Smile|Gun|Man|Face)'
KINDS = {'lines_inner': 'LinesInner', 'words': 'Words', 'chars': 'Chars', 'chars_smart': 'CharsSmart',
         'chars_raw': 'CharsRaw', 'splitter': 'Splitter'}


def kotlin_string(tok):
    if not tok.startswith('"'):
        return CONSTS[tok]
    s = tok[1:-1]
    s = re.sub(r'\$(ch(?:Smile|Gun|Man|Face))', lambda m: CONSTS[m.group(1)], s)
    return re.sub(r'\\(.)', lambda m: {'t': '\t', 'n': '\n'}.get(m.group(1), m.group(1)), s)


def strip_comments(src):
    out = []
    for line in src.split('\n'):
        if line.lstrip().startswith('//'):
            continue
        out.append(re.sub(r'\)\s*//.*$', ')', line))  # trailing `// TODO` after a statement
    return '\n'.join(out)


def pick(d, pol):
    for o in {'ignore': ['ignore', 'trim', 'default'], 'trim': ['trim', 'default'], 'default': ['default']}[pol]:
        if o in d:
            return d[o]
    return None


cases = []
for path in sys.argv[1:-1]:
    suite = re.search(r'(\w+)ComparisonUtilTest', path).group(1)
    src = strip_comments(open(path).read())
    for f in re.split(r'\n  fun ', src)[1:]:
        fname = re.match(r'(`[^`]+`|\w+)\(\)', f).group(1).strip('`')
        blocks = re.findall(r'\n    (lines_inner|words|chars_smart|chars_raw|chars|splitter)(\([^)]*\))? \{(.*?)\n    \}', f, re.S)
        for bi, (kind, args, block) in enumerate(blocks):
            squash = 'squash = true' in (args or '')
            trim = 'trim = true' in (args or '')
            pairs = re.findall(r'\(\s*(' + STR + r')\s*-\s*(' + STR + r')\s*\)((?:\.\w+\(\))*)', block, re.S)
            text = None
            matchings = {}
            for a, b, calls in pairs:
                plain = '.plainSource()' in calls
                if text is None:
                    l, r = kotlin_string(a), kotlin_string(b)
                    text = (l, r) if plain else (l.replace('_', '\n'), r.replace('_', '\n'))
                for pol in re.findall(r'\.(default|trim|ignore)\(\)', calls):
                    matchings[pol] = (kotlin_string(a).replace('.', ''), kotlin_string(b).replace('.', ''))
            changes = {}
            for pol, body in re.findall(r'\n\s*(default|trim|ignore)\((.*)\)\s*(?=\n|$)', block):
                lst = []
                for k, nums in re.findall(r'(mod|ins|del)\(([^)]*)\)', body):
                    n = [int(x) for x in nums.split(',')]
                    a, b, c1, c2 = n if k == 'mod' else (n[0], n[1], n[2], 0) if k == 'del' else (n[0], n[1], 0, n[2])
                    lst.append([a, a + c1, b, b + c2])
                changes[pol] = lst
            runs = ['default', 'trim', 'ignore'] if 'testAll()' in block else []
            runs += [p.lower() for p in ['Default', 'Trim', 'Ignore'] if f'test{p}()' in block]
            for pol in runs:
                ch, mt = pick(changes, pol), pick(matchings, pol)
                if ch is None and mt is None:
                    continue
                cases.append(dict(name=f"{suite}: {fname} #{bi} [{pol}]", suite=suite, kind=KINDS[kind], left=text[0],
                                  right=text[1], policy=pol, changes=ch, matching=mt, squash=squash, trim=trim))

mode = {'default': 'WhitespaceMode::Default', 'trim': 'WhitespaceMode::Trim', 'ignore': 'WhitespaceMode::Ignore'}
out = [
    "// Golden tests ported from IntelliJ IDEA Community Edition: WordComparisonUtilTest.kt, CharComparisonUtilTest.kt,",
    "// SplitComparisonUtilTest.kt (platform/diff-impl/tests/testSrc/com/intellij/diff/comparison, commit bc34875).",
    "// Copyright 2000-2015 JetBrains s.r.o. Licensed under the Apache License, Version 2.0.",
    "// Modified: converted from the Kotlin DSL into a Rust table by convert.py (alongside); `_` in sources is already",
    "// replaced by newline (except `.plainSource()` cases); offsets and matchings are in UTF-16 units, as in IntelliJ.",
    "// GENERATED — do not edit by hand.\n",
    "use lenz_engine::WhitespaceMode;\n",
    "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
    "pub enum Kind { LinesInner, Words, Chars, CharsSmart, CharsRaw, Splitter }\n",
    "pub struct Case {",
    "    pub name: &'static str,",
    "    pub suite: &'static str,",
    "    pub kind: Kind,",
    "    pub left: &'static str,",
    "    pub right: &'static str,",
    "    pub mode: WhitespaceMode,",
    "    pub changes: Option<&'static [[u32; 4]]>,",
    "    pub matching: Option<(&'static str, &'static str)>,",
    "    /// `splitter(squash = ..)`; `splitter(trim = true)` under Ignore is already `IgnoreTrimChunks` in `mode`.",
    "    pub squash: bool,",
    "}\n",
    "pub const CASES: &[Case] = &[",
]
for c in cases:
    m = 'WhitespaceMode::IgnoreTrimChunks' if c['trim'] and c['policy'] == 'ignore' else mode[c['policy']]
    ch = 'None' if c['changes'] is None else "Some(&[" + ", ".join(f"[{a}, {b}, {x}, {y}]" for a, b, x, y in c['changes']) + "])"
    mt = 'None' if c['matching'] is None else f"Some(({json.dumps(c['matching'][0])}, {json.dumps(c['matching'][1])}))"
    out.append(f"    Case {{ name: {json.dumps(c['name'], ensure_ascii=False)}, suite: {json.dumps(c['suite'])}, kind: Kind::{c['kind']}, "
               f"left: {json.dumps(c['left'], ensure_ascii=False)}, right: {json.dumps(c['right'], ensure_ascii=False)}, mode: {m}, "
               f"changes: {ch}, matching: {mt}, squash: {'true' if c['squash'] else 'false'} }},")
out.append("];")
open(sys.argv[-1], 'w').write("\n".join(out) + "\n")
by_suite = {}
for c in cases:
    by_suite[c['suite']] = by_suite.get(c['suite'], 0) + 1
print(len(cases), "cases", by_suite)
