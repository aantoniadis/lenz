#!/usr/bin/env python3
"""Replay matching disk-boundary scenarios in a private Xvfb GUI and owned TUI PTY.

Reuses the existing probes. Requires Linux, Xvfb, ImageMagick and Tesseract.
Screenshots/cell dumps are retained for visual review; they are not pixel oracles.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from perf_support import isolate_user_state


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def wait_bytes(path, expected):
    deadline = time.monotonic() + 10
    while path.read_bytes() != expected:
        if time.monotonic() >= deadline:
            raise AssertionError(f'{path.name}: save did not produce {expected!r}')
        time.sleep(.01)


def label_bounds(tsv, label, max_y=None):
    """Find one complete painted phrase; refuse missing or ambiguous controls."""
    lines = {}
    for row in tsv.splitlines()[1:]:
        cells = row.split('\t')
        if len(cells) == 12 and cells[0] == '5' and cells[-1].strip():
            x, y, width, height = map(int, cells[6:10])
            if max_y is None or y < max_y:
                lines.setdefault(tuple(cells[1:5]), []).append((cells[-1], x, y, width, height))
    hits = []
    words = label.split()
    for line in lines.values():
        for i in range(len(line) - len(words) + 1):
            group = line[i:i + len(words)]
            if [part[0].casefold() for part in group] == [word.casefold() for word in words]:
                left, top = min(p[1] for p in group), min(p[2] for p in group)
                hits.append((left, top, max(p[1] + p[3] for p in group) - left,
                             max(p[2] + p[4] for p in group) - top))
    assert len(hits) == 1, f'painted {label!r}: expected one control, found {len(hits)}'
    return hits[0]


def click_label(probe, label, phase, max_y=None, bounds=None, button=1):
    """Click the actual painted control on this probe's private display."""
    # Move off the window before OCR: a tooltip can cover the next toolbar label.
    probe.t.XTestFakeMotionEvent(probe.display, 0, 10, 200, 0)
    probe.x.XFlush(probe.display)
    probe.pause(.15)
    probe.screenshot(phase)
    if bounds is None:
        rows = subprocess.check_output(['tesseract', str(probe.output / (phase + '.png')),
                                        'stdout', '--psm', '11', 'tsv'], stderr=subprocess.DEVNULL, text=True)
        bounds = label_bounds(rows, label, max_y)
    x, y, width, height = bounds
    c = probe.C
    translate = probe.x.XTranslateCoordinates
    translate.restype = c.c_int
    translate.argtypes = [c.c_void_p, c.c_ulong, c.c_ulong, c.c_int, c.c_int,
                         c.POINTER(c.c_int), c.POINTER(c.c_int), c.POINTER(c.c_ulong)]
    screen_x, screen_y, child = c.c_int(), c.c_int(), c.c_ulong()
    assert translate(probe.display, probe.win, probe.x.XDefaultRootWindow(probe.display),
                     x + width // 2, y + height // 2, c.byref(screen_x), c.byref(screen_y), c.byref(child))
    probe.t.XTestFakeMotionEvent(probe.display, 0, screen_x.value, screen_y.value, 0)
    for down in [1, 0]:
        probe.t.XTestFakeButtonEvent(probe.display, button, down, 0)
    probe.x.XFlush(probe.display)
    probe.pause(.1)
    return bounds


def gui(binary, root, output, xvfb, rounds, original, applied):
    probe = load('flow_gui', 'markdown-bench.py').NativeGui(
        binary, ['--no-index', 'left.txt', 'right.txt'], root, output, xvfb)
    try:
        for _ in range(rounds):
            probe.action('Right', ('Alt_L', 'Shift_L'))
            assert (root / 'right.txt').read_bytes() == original
            probe.action('f', ('Control_L',))
            for character in 'needle':
                probe.key(character)
            probe.pause(.1)
            probe.key('s', ('Control_L',))
            probe.pause(.1)
            assert (root / 'right.txt').read_bytes() == original, 'GUI Find leaked Save'
            probe.action('Escape')
            probe.action('s', ('Control_L',))
            wait_bytes(root / 'right.txt', applied)
            probe.action('z', ('Control_L',))
            assert (root / 'right.txt').read_bytes() == applied
            probe.action('s', ('Control_L',))
            wait_bytes(root / 'right.txt', original)
        probe.action('Right', ('Alt_L', 'Shift_L'))
        external = b'external writer\n'
        (root / 'right.txt').write_bytes(external)
        probe.action('s', ('Control_L',))
        assert (root / 'right.txt').read_bytes() == external
        probe.screenshot('external-save-refusal')
        probe.action('F10')
        for character in 'discard unsaved':
            probe.key('space' if character == ' ' else character)
        probe.pause(.1)
        click_label(probe, 'Discard unsaved changes', 'discard-draft-control')
        assert (root / 'right.txt').read_bytes() == external
        # Apply/Undo/Save after recovery restores the external version exactly.
        probe.action('Right', ('Alt_L', 'Shift_L'))
        probe.action('s', ('Control_L',))
        wait_bytes(root / 'right.txt', applied)
        probe.action('z', ('Control_L',))
        probe.action('s', ('Control_L',))
        wait_bytes(root / 'right.txt', external)
        for width, height in [(900, 650), (640, 480), (400, 240)]:
            probe.x.XResizeWindow(probe.display, probe.win, width, height)
            probe.x.XSync(probe.display, 0)
            probe.pause(.15)
            probe.screenshot(f'{width}x{height}-comparison')
            probe.action('F10')
            probe.action('End')
            probe.action('Up')
            probe.screenshot(f'{width}x{height}-commands')
            probe.action('Escape')
            probe.action('slash', ('Shift_L',))
            gui_visible(probe, f'{width}x{height}-help', 'Key bindings')
            if width == 400:
                probe.action('End')
                probe.action('Up')
                gui_visible(probe, '400x240-help-end-up', 'This help')
                probe.action('s', ('Control_L',))
                assert (root / 'right.txt').read_bytes() == external, 'Help leaked Save'
            probe.action('Escape')
            gui_visible(probe, f'{width}x{height}-help-dismissed', absent=('Key bindings',))
            assert (root / 'right.txt').read_bytes() == external
        return {'rounds': rounds, 'disk_boundaries': True, 'external_save_recovery': True, 'settled': probe.sample('settled')}
    finally:
        probe.close()


def tui(binary, root, output, rounds, original, applied):
    module = load('flow_tui', 'tui-workflow-bench.py')
    probe = module.Workflow(str(binary.resolve()), root, ['--no-index', 'left.txt', 'right.txt'])
    output.mkdir(parents=True, exist_ok=True)
    try:
        probe.wait('new needle')
        for _ in range(rounds):
            probe.key(b'\x1b[1;4C', 'Unsaved changes')
            assert (root / 'right.txt').read_bytes() == original
            probe.key(b'\x06', 'Find:')
            probe.key(b'needle', 'Find: needle')
            probe.key(b'\x13')
            assert (root / 'right.txt').read_bytes() == original, 'TUI Find leaked Save'
            probe.close_overlay()
            probe.key(b'\x13', 'Changes saved')
            wait_bytes(root / 'right.txt', applied)
            probe.key(b'\x1a', 'Undone')
            assert (root / 'right.txt').read_bytes() == applied
            probe.key(b'\x13', 'Changes saved')
            wait_bytes(root / 'right.txt', original)
        probe.key(b'\x1b[1;4C', 'Unsaved changes')
        external = b'external writer\n'
        (root / 'right.txt').write_bytes(external)
        probe.key(b'\x13', 'file changed on disk')
        assert (root / 'right.txt').read_bytes() == external
        probe.screen.update(probe.probe)
        (output / 'external-save-refusal.txt').write_text(probe.screen.text())
        resize_tui(probe, 48, 16)
        probe.key(b'\x1bOQ', 'Details')
        tui_snapshot(probe, output, '48x16-refusal-details')
        body = ''.join(''.join(row[2:46]) for row in probe.screen.cells[2:13])
        assert 'file changed on disk' in body and 'nothing written' in body and 'Discard unsaved changes' in body, body
        probe.key(b'\x1b[F')
        probe.key(b'\x1b[A')
        tui_snapshot(probe, output, '48x16-refusal-details-end-up')
        probe.key(b'\x13')
        assert (root / 'right.txt').read_bytes() == external, 'Details leaked Save'
        probe.close_overlay()
        probe.wait(absent=('Details',))
        resize_tui(probe, 120, 36)
        probe.menu('Discard unsaved changes')
        probe.key(b'\x1b[1;4C', 'Unsaved changes')
        probe.key(b'\x13', 'Changes saved')
        wait_bytes(root / 'right.txt', applied)
        probe.key(b'\x1a', 'Undone')
        probe.key(b'\x13', 'Changes saved')
        wait_bytes(root / 'right.txt', external)
        for width, height in [(120, 36), (80, 24), (48, 16)]:
            resize_tui(probe, width, height)
            for name, opening in [('comparison', None), ('commands', b'\x1b[21~'), ('help', b'\x1bOP')]:
                if opening:
                    probe.key(opening)
                    probe.key(b'\x1b[F')
                    probe.key(b'\x1b[A')
                    if name == 'help':
                        probe.wait('Keyboard shortcuts', 'Close current view')
                probe.probe.quiet(.1)
                probe.screen.update(probe.probe)
                (output / f'{width}x{height}-{name}.txt').write_text(probe.screen.text())
                if opening:
                    probe.close_overlay()
                    probe.wait(absent=('Keyboard shortcuts',) if name == 'help' else ('Commands:',))
                assert (root / 'right.txt').read_bytes() == external
        result = {'rounds': rounds, 'disk_boundaries': True, 'external_save_recovery': True, 'settled': probe.probe.sample()}
        probe.probe.finish(key=b'\x11')
        result['terminal_restored'] = True
        return result
    finally:
        (output / 'terminal.ansi').write_bytes(probe.probe.output)
        probe.probe.close()


def content_fixture(root):
    """Small offline fixtures; merge inputs live outside Git to avoid resume rules."""
    workflow = load('flow_fixtures', 'tui-workflow-bench.py')
    repo = root / 'repository'
    repo.mkdir()
    # Pin commit times as well as author/config through the existing fixture helper.
    previous = {key: os.environ.get(key) for key in ('GIT_AUTHOR_DATE', 'GIT_COMMITTER_DATE')}
    try:
        os.environ.update(GIT_AUTHOR_DATE='2026-10-06T00:00:00Z', GIT_COMMITTER_DATE='2026-10-06T00:00:00Z')
        workflow.git(repo, 'init', '-q', '-b', 'main')
        for version, subject in [(1, 'Workflow first commit'), (2, 'Workflow second commit')]:
            for filename, label in [('file.rs', 'FILE'), ('other.rs', 'OTHER')]:
                (repo / filename).write_text(f'const LABEL: &str = "WORKFLOW_{version}_{label} needle";\n' +
                    ''.join(f'const ITEM_{i}: &str = "needle café";\n' for i in range(20)))
            workflow.git(repo, 'add', '.')
            workflow.git(repo, 'commit', '-qm', subject)
        workflow.git(repo, 'branch', 'topic')
    finally:
        for key, value in previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
    for folder, values in [('merge', [b'common\nbase\nend\n', b'common\nours\nend\n', b'common\ntheirs\nend\n', b'initial result\n']),
                           ('versions', [b'base\0\xff\r\n', b'ours\0\xff\r\n', b'theirs\0\xff\r\n', b'initial\0result\n'])]:
        directory = root / folder
        directory.mkdir()
        for name, value in zip(('base', 'ours', 'theirs', 'result'), values):
            (directory / name).write_bytes(value)
    for side in ('left', 'right'):
        (root / (side + '.md')).write_text(''.join(
            f'## Section {i}\n\nParagraph {i} with **strong**, *emphasis*, `code`.\n\n'
            for i in range(24)) + f'# {side.upper()}_CHANGE\n\n# MARKDOWN_TAIL_END\n')
        def chunk(kind, value):
            return struct.pack('>I', len(value)) + kind + value + struct.pack('>I', zlib.crc32(kind + value))
        pixels = b''.join(b'\0' + bytes((220, 40 if side == 'left' else 180, 70, 255)) * 96 for _ in range(64))
        (root / (side + '.png')).write_bytes(b'\x89PNG\r\n\x1a\n' +
            chunk(b'IHDR', struct.pack('>IIBBBBB', 96, 64, 8, 6, 0, 0, 0)) +
            chunk(b'IDAT', zlib.compress(pixels)) + chunk(b'IEND', b''))


def visible_ready(visible, expected, absent=()):
    visible = visible.casefold()
    return all(text.casefold() in visible for text in expected) and all(text.casefold() not in visible for text in absent)


def gui_visible(probe, phase, *expected, pane_only=False, absent=()):
    deadline = time.monotonic() + 10
    while True:
        visible = probe.screenshot(phase)
        if pane_only or not visible_ready(visible, expected, absent):
            # Whole-window segmentation drops colored insertion blocks. OCR a
            # separately retained enlarged right-pane crop, never old screenshots.
            path = probe.output / (phase + '.png')
            width, height = map(int, subprocess.check_output(['identify', '-format', '%w %h', str(path)], text=True).split())
            crop_width = width // (3 if pane_only else 2)
            # No single preparation reads every pane: the 60% threshold reads highlighted code, plain grayscale the
            # dimmer pane header, a 50% threshold at 300% a fully tinted inserted line. Read the pane each way.
            passes = ([('', ['-colorspace', 'Gray', '-threshold', '60%'], '200%'), ('-gray', ['-colorspace', 'Gray'], '200%'),
                       ('-t50', ['-colorspace', 'Gray', '-threshold', '50%'], '300%')] if pane_only else [('', [], '200%')])
            text = ''
            for suffix, preprocessing, scale in passes:
                crop = probe.output / (phase + f'-right{suffix}.png')
                subprocess.run(['convert', str(path), '-crop', f'{crop_width}x{height}+{width - crop_width}+0',
                                *preprocessing, '-resize', scale, str(crop)], check=True, stderr=subprocess.DEVNULL)
                text += subprocess.check_output(['tesseract', str(crop), 'stdout', '--psm', '6'], stderr=subprocess.DEVNULL, text=True) + '\n'
            (probe.output / (phase + '-right.txt')).write_text(text)
            visible = text if pane_only else visible + '\n' + text
        if visible_ready(visible, expected, absent):
            return visible
        if probe.proc.poll() is not None or time.monotonic() > deadline:
            raise AssertionError(f'GUI expected {expected}: {visible}')
        probe.pause(.1)


def tui_snapshot(probe, output, phase):
    probe.probe.quiet(.1)
    probe.screen.update(probe.probe)
    visible = probe.screen.text()
    (output / (phase + '.txt')).write_text(visible)
    return visible


def tui_repository_ready(visible, filename, marker, absent=()):
    """Only the current comparison title and its focused right-pane cells count."""
    rows = visible.splitlines()
    for y, title in enumerate(rows[1:-3], 1):
        if not re.search(r'\b[1-9]\d*/[1-9]\d*\b', rows[y - 1]):
            continue
        right_x = title.find('▶ ' + filename)
        if right_x < 0:
            continue
        if title[right_x:].split(' · ', 1)[0].strip() != '▶ ' + filename:
            return False
        # Repository replays retain Right focus. In wide layouts this starts
        # after the left pane; in narrow layouts the right pane is shown alone.
        current = '\n'.join(line[right_x:] for line in rows[y + 1:-2])
        return visible_ready(current, (marker,), absent)
    return False


def tui_repository_visible(probe, output, phase, filename, marker, absent=()):
    deadline = time.monotonic() + 15
    while True:
        probe.screen.update(probe.probe)
        visible = probe.screen.text()
        if tui_repository_ready(visible, filename, marker, absent):
            (output / (phase + '.txt')).write_text(visible)
            return visible
        if probe.probe.proc.poll() is not None or time.monotonic() >= deadline:
            raise AssertionError(f'current right pane must show {filename} / {marker}, absent {absent}:\n{visible}')
        probe.probe.read(.01)


def resize_tui(probe, width, height):
    probe.probe.quiet(.05)
    probe.screen.update(probe.probe)
    # Retain stream offset/decoder: replaying old ANSI into a new-size screen can
    # make a previous frame satisfy the current readiness assertion.
    probe.screen.width, probe.screen.height = width, height
    probe.screen.cells = [[' '] * width for _ in range(height)]
    probe.screen.x = probe.screen.y = 0
    probe.probe.resize(width, height)
    probe.probe.quiet(.1)
    probe.screen.update(probe.probe)


def content_flow(frontend, scenario, binary, root, output, xvfb, rounds):
    markdown = load('flow_markdown', 'markdown-bench.py')
    workflow = markdown.workflow
    cwd = root / scenario if scenario in ('merge', 'versions') else root / 'repository' if scenario == 'repository' else root
    args = ['log'] if scenario == 'repository' else ['merge', 'base', 'ours', 'theirs', 'result'] if scenario in ('merge', 'versions') else ['--no-index', 'left.' + ('md' if scenario == 'markdown' else 'png'), 'right.' + ('md' if scenario == 'markdown' else 'png')]
    output.mkdir(parents=True, exist_ok=True)
    inputs = {p: p.read_bytes() for p in cwd.iterdir() if p.is_file() and p.name != 'result'}
    if scenario in ('merge', 'versions'):
        assert subprocess.run(['git', 'rev-parse', '--show-toplevel'], cwd=cwd, capture_output=True).returncode != 0, 'four-file fixtures must be outside Git'
    is_gui = frontend == 'gui'
    probe = markdown.NativeGui(binary, args, cwd, output, xvfb) if is_gui else workflow.Workflow(str(binary.resolve()), cwd, args)
    started = time.monotonic()
    samples = []
    result = {'scenario': scenario, 'rounds': rounds, 'argv': [str(binary.resolve()), *args], 'cwd': str(cwd)}
    try:
        if scenario == 'repository':
            if is_gui:
                gui_visible(probe, 'repository-ready', 'Workflow second commit')
                gui_visible(probe, 'repository-source-ready', 'WORKFLOW_2_FILE', 'file.rs', pane_only=True, absent=('WORKFLOW_1_FILE',))
                for cycle in range(rounds):
                    click_label(probe, 'Workflow first commit', f'first-commit-{cycle}')
                    gui_visible(probe, f'first-comparison-{cycle}', 'WORKFLOW_1_FILE', 'file.rs', pane_only=True, absent=('WORKFLOW_2_FILE',))
                    click_label(probe, 'other.rs', f'other-file-{cycle}')
                    gui_visible(probe, f'other-comparison-{cycle}', 'WORKFLOW_1_OTHER', 'other.rs', pane_only=True, absent=('WORKFLOW_2_OTHER',))
                    click_label(probe, 'Workflow second commit', f'second-commit-{cycle}')
                    gui_visible(probe, f'second-comparison-{cycle}', 'WORKFLOW_2_FILE', 'file.rs', pane_only=True, absent=('WORKFLOW_1_FILE',))
                    samples.append(probe.sample('repository-cycle'))
            else:
                probe.wait('Workflow second commit', 'WORKFLOW_2_FILE')
                tui_repository_visible(probe, output, 'repository-source-ready', 'file.rs', 'WORKFLOW_2_FILE', ('WORKFLOW_1_FILE',))
                for cycle in range(rounds):
                    probe.key(b'Workflow first commit\r')
                    tui_repository_visible(probe, output, f'first-comparison-{cycle}', 'file.rs', 'WORKFLOW_1_FILE', ('WORKFLOW_2_FILE',))
                    probe.close_overlay()
                    probe.key(b'\t', 'Files')
                    probe.key(b'other.rs\r')
                    tui_repository_visible(probe, output, f'first-other-{cycle}', 'other.rs', 'WORKFLOW_1_OTHER', ('WORKFLOW_2_OTHER',))
                    probe.menu('Overview', ' Focus ')
                    probe.screen.update(probe.probe)
                    hits=[(row.find('Workflow second commit'),y) for y,row in enumerate(probe.screen.text().splitlines()) if 'Workflow second commit' in row]
                    assert len(hits)==1, 'second commit must be visible once in the log'
                    x,y=hits[0]
                    probe.key(f'\x1b[<0;{x+1};{y+1}M\x1b[<0;{x+1};{y+1}m'.encode())
                    tui_repository_visible(probe, output, f'second-comparison-{cycle}', 'file.rs', 'WORKFLOW_2_FILE', ('WORKFLOW_1_FILE',))
                    probe.close_overlay()
                    samples.append(probe.probe.sample())
                probe.history_filter('first', 'Workflow first commit', absent=('Workflow second commit',))
                tui_snapshot(probe, output, 'filtered-history')
                probe.close_overlay()
                probe.wait('Workflow second commit')
            (cwd / 'new-file').write_bytes(b'new local file\n')
            workflow.git(cwd, 'add', 'new-file')
            workflow.git(cwd, 'commit', '-qm', 'Workflow refreshed commit')
            workflow.git(cwd, 'tag', 'workflow-tag')
            if is_gui:
                probe.action('F10')
                for character in 'refresh repository':
                    probe.key('space' if character == ' ' else character)
                click_label(probe, 'Refresh repository', 'refresh-control')
                gui_visible(probe, 'refreshed-history', 'Workflow refreshed', 'workflow-tag')
                gui_visible(probe, 'refresh-preserved-comparison', 'WORKFLOW_2_FILE', 'file.rs', pane_only=True, absent=('WORKFLOW_1_FILE',))
                click_label(probe, 'Workflow refreshed', 'open-refreshed-commit')
                gui_visible(probe, 'refreshed-comparison', 'new-file', 'new local file', pane_only=True)
            else:
                probe.menu('refresh repository', 'Workflow refreshed commit', repository=True)
                tui_repository_visible(probe, output, 'refreshed-history', 'file.rs', 'WORKFLOW_2_FILE', ('WORKFLOW_1_FILE',))
            assert (cwd / 'new-file').read_bytes() == b'new local file\n'
            result['external_commit_refresh'] = True
        elif scenario in ('merge', 'versions'):
            target = cwd / 'result'
            original = target.read_bytes()
            if scenario == 'versions':
                # Undo restores the previous chosen version. Undoing the initial
                # choice restores "no choice", which intentionally writes nothing.
                if is_gui:
                    probe.action('1')
                    assert target.read_bytes() == original
                    probe.action('s', ('Control_L',))
                else:
                    probe.wait('binary', 'Ours', 'Theirs')
                    probe.key(b'1')
                    assert target.read_bytes() == original
                    probe.key(b'\x13')
                original = inputs[cwd / 'ours']
                wait_bytes(target, original)
            if scenario == 'merge':
                # The native merge starts with generated conflict markers, independent
                # of the supplied result. Save that unresolved state as the undo oracle.
                if is_gui:
                    gui_visible(probe, 'merge-ready', 'Result', 'Ours', 'Theirs')
                    probe.action('s', ('Control_L',))
                else:
                    probe.wait('Result', 'Ours', 'Theirs')
                    probe.key(b'\x13')
                deadline = time.monotonic() + 10
                while b'<<<<<<<' not in target.read_bytes():
                    if time.monotonic() > deadline:
                        raise AssertionError('merge did not save unresolved markers')
                    time.sleep(.01)
                original = target.read_bytes()
            for cycle in range(rounds):
                for choice, expected in ([('2', inputs[cwd / 'theirs'])] if scenario == 'versions' else
                                         [('1', inputs[cwd / 'ours']), ('2', inputs[cwd / 'theirs'])]):
                    if is_gui:
                        probe.action('Right' if choice == '1' else 'Left', ('Alt_L', 'Shift_L')) if scenario == 'merge' else probe.action(choice)
                        assert target.read_bytes() == original, 'choice wrote before Save'
                        probe.screenshot(f'chosen-{cycle}-{choice}')
                        probe.action('s', ('Control_L',))
                    else:
                        probe.key(choice.encode())
                        assert target.read_bytes() == original, 'choice wrote before Save'
                        tui_snapshot(probe, output, f'chosen-{cycle}-{choice}')
                        probe.key(b'\x13')
                    wait_bytes(target, expected)
                    if is_gui:
                        probe.action('z', ('Control_L',))
                        assert target.read_bytes() == expected, 'Undo wrote before Save'
                        probe.action('s', ('Control_L',))
                    else:
                        probe.key(b'\x1a')
                        assert target.read_bytes() == expected, 'Undo wrote before Save'
                        probe.key(b'\x13')
                    wait_bytes(target, original)
                samples.append(probe.sample('merge-cycle') if is_gui else probe.probe.sample())
            result['exact_choice_save_undo'] = True
        elif scenario == 'markdown':
            for cycle in range(rounds):
                if is_gui:
                    click_label(probe, 'Inline', f'rendered-control-{cycle}', max_y=100)
                    probe.action('End', pause=.25)
                    visible = probe.screenshot(f'rendered-tail-{cycle}')
                    assert markdown.gui_rendered_tail(visible), f'rendered prose/tail absent: {visible}'
                    click_label(probe, 'Source', f'source-control-{cycle}', max_y=100)
                    gui_visible(probe, f'source-tail-{cycle}', '## Section')
                else:
                    probe.menu('View: Inline', 'Rendered Markdown')
                    probe.key(b'\x1b[F')
                    probe.wait('MARKDOWN_TAIL_END', 'Rendered Markdown', absent=('Preparing rendered Markdown',))
                    visible = tui_snapshot(probe, output, f'rendered-tail-{cycle}')
                    assert markdown.terminal_completed(visible) and '**strong**' not in visible, 'completed rendered prose/tail missing'
                    probe.menu('View: Source')
                    probe.key(b'\x1b[A', '**strong**')
                    tui_snapshot(probe, output, f'source-tail-{cycle}')
                samples.append(probe.sample('markdown-cycle') if is_gui else probe.probe.sample())
            result['rendered_readiness_oracle'] = 2
        elif scenario == 'images':
            for cycle in range(rounds):
                for mode in ('Swipe', 'Onion skin', 'Difference', '2-up'):
                    if is_gui:
                        probe.action('m', pause=.15)
                        gui_visible(probe, f'{cycle}-{mode}', mode)
                    else:
                        probe.key(b'm', mode)
                        assert '▀' in tui_snapshot(probe, output, f'{cycle}-{mode}'), 'image raster missing'
                        if mode in ('Swipe', 'Onion skin'):
                            control = 'split' if mode == 'Swipe' else 'opacity'
                            probe.key(b'.', control + ' 60%')
                            tui_snapshot(probe, output, f'{cycle}-{mode}-adjusted')
                            probe.key(b',', control + ' 50%')
                if is_gui:
                    probe.action('1')
                    gui_visible(probe, f'image-actual-size-{cycle}', '100 %')
                    for key in ('equal', '0', 'period', 'comma'):
                        probe.action(key)
                    probe.screenshot(f'image-fit-{cycle}')
                else:
                    probe.key(b'1', '100%')
                    tui_snapshot(probe, output, f'image-actual-size-{cycle}')
                    for key in (b'+', b'0', b'.', b','):
                        probe.key(key)
                    tui_snapshot(probe, output, f'image-fit-{cycle}')
                samples.append(probe.sample('image-cycle') if is_gui else probe.probe.sample())
        for path, expected in inputs.items():
            assert path.read_bytes() == expected, f'input modified: {path.name}'
        result.update(inputs_unchanged=True, samples=samples, elapsed_seconds=time.monotonic() - started)
        if not is_gui:
            tui_snapshot(probe, output, 'completed')
            if scenario in ('merge', 'versions'):
                probe.key(b'q')  # Four-file mergetool asks before canceling.
                tui_snapshot(probe, output, 'cancel-confirmation')
                probe.probe.finish(key=b'\x1b', expected=1)
                result['canceled_exit'] = 1
            else:
                probe.probe.finish(key=b'\x11')
            result['terminal_restored'] = True
        return result
    finally:
        if is_gui:
            if probe.proc.poll() is None:
                probe.screenshot('final')
            probe.close()
        else:
            (output / 'terminal.ansi').write_bytes(probe.probe.output)
            (output / 'final.txt').write_text(probe.screen.text())
            probe.probe.close()


def main():
    isolate_user_state()  # never measure with the user's own settings or saved positions
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gui-app', type=Path, required=True)
    parser.add_argument('--tui-app', type=Path, required=True)
    parser.add_argument('--xvfb', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--frontends', nargs='+', choices=['gui', 'tui'], default=['gui', 'tui'])
    parser.add_argument('--scenarios', nargs='+', choices=['comparison', 'repository', 'merge', 'versions', 'markdown', 'images'],
                        default=['comparison', 'repository', 'merge', 'versions', 'markdown', 'images'])
    parser.add_argument('--output', type=Path, default=Path('target/user-flow-audit/native'))
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error('rounds must be positive')
    os.environ.update(GIT_AUTHOR_DATE='2026-10-06T00:00:00Z', GIT_COMMITTER_DATE='2026-10-06T00:00:00Z')
    args.output.mkdir(parents=True, exist_ok=True)
    original = b'new needle\n' + b'unchanged\n' * 10
    applied = b'old needle\n' + b'unchanged\n' * 10
    results = {'workload_version': 5, 'rounds': args.rounds, 'scenarios': args.scenarios,
               'command': sys.argv, 'platform': platform.platform(),
               'environment': {key: os.environ.get(key) for key in ('LENZ_THEME', 'LENZ_APPEARANCE', 'LENZ_ICONS')},
               'frontends': args.frontends, 'binaries': {str(p.resolve()): hashlib.sha256(p.read_bytes()).hexdigest()
                            for p in [args.gui_app, args.tui_app]},
               'oracle_limits': 'Screenshots/cells and fixture-specific OCR checks; no complete visual oracle or physical-display latency.',
               'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()}
    results['source_files'] = {str(path): hashlib.sha256(path.read_bytes()).hexdigest()
                              for directory in ('crates', 'scripts') for path in sorted(Path(directory).rglob('*'))
                              if path.is_file() and path.suffix in ('.rs', '.py', '.toml')}
    results['source_files'].update({name: hashlib.sha256(Path(name).read_bytes()).hexdigest() for name in ('Cargo.toml', 'Cargo.lock')})
    with tempfile.TemporaryDirectory(prefix='.lenz-native-flow-', dir=Path.cwd()) as directory:
        root = Path(directory)
        for frontend in args.frontends if 'comparison' in args.scenarios else []:
            (root / 'left.txt').write_bytes(applied)
            (root / 'right.txt').write_bytes(original)
            try:
                if frontend == 'gui':
                    results[frontend] = gui(args.gui_app, root, args.output / frontend, args.xvfb, args.rounds, original, applied)
                else:
                    results[frontend] = tui(args.tui_app, root, args.output / frontend, args.rounds, original, applied)
            except Exception as error:
                results[frontend] = {'failed': True, 'error': str(error)}
            (args.output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
            assert (root / 'left.txt').read_bytes() == applied
    # Four-file merges must be outside Git, unlike writable two-way comparisons.
    with tempfile.TemporaryDirectory(prefix='lenz-native-content-') as directory:
        root = Path(directory)
        content_scenarios = [scenario for scenario in args.scenarios if scenario != 'comparison']
        for frontend, binary in [('gui', args.gui_app), ('tui', args.tui_app)]:
            if frontend not in args.frontends or not content_scenarios:
                continue
            frontend_root = root / frontend
            frontend_root.mkdir()
            content_fixture(frontend_root)
            (args.output / (frontend + '-fixtures.json')).write_text(json.dumps(
                {str(p.relative_to(frontend_root)): {'sha256': hashlib.sha256(p.read_bytes()).hexdigest(), 'bytes': p.stat().st_size}
                 for p in frontend_root.rglob('*') if p.is_file() and '.git' not in p.parts}, indent=2) + '\n')
            results.setdefault(frontend, {})['content'] = {}
            for scenario in content_scenarios:
                try:
                    results[frontend]['content'][scenario] = content_flow(frontend, scenario, binary, frontend_root,
                        args.output / frontend / scenario, args.xvfb, args.rounds)
                except Exception as error:
                    results[frontend]['content'][scenario] = {'failed': True, 'error': str(error)}
                (args.output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
    (args.output / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, indent=2))
    if any(results.get(frontend, {}).get('failed') or any(case.get('failed') for case in results.get(frontend, {}).get('content', {}).values())
           for frontend in args.frontends):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
