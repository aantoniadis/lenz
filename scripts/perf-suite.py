#!/usr/bin/env python3
"""Repeatable Linux/macOS performance suite. All generated data stays in a new output directory."""
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from perf_support import compare, metrics, run, tail_growth, cargo_executable, isolate_user_state

ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = ROOT / 'scripts'
SCHEMA = 1
WORKLOAD_VERSION = 6


def command_text(args, env):
    try:
        return subprocess.check_output(args, cwd=ROOT, env=env, stderr=subprocess.DEVNULL, text=True).strip()
    except (OSError, subprocess.SubprocessError):
        return None


def machine(env):
    cpu = platform.processor()
    memory = None
    if platform.system() == 'Linux':
        for line in Path('/proc/cpuinfo').read_text().splitlines():
            if line.startswith('model name'):
                cpu = line.split(':', 1)[1].strip()
                break
        for line in Path('/proc/meminfo').read_text().splitlines():
            if line.startswith('MemTotal:'):
                memory = int(line.split()[1]) * 1024
    elif platform.system() == 'Darwin':
        cpu = command_text(['sysctl','-n','machdep.cpu.brand_string'], env)
        value = command_text(['sysctl','-n','hw.memsize'], env)
        memory = int(value) if value else None
    return {'os':platform.system(), 'os_version':platform.release(), 'arch':platform.machine(),
            'cpu':cpu, 'logical_cpus':os.cpu_count(), 'memory_bytes':memory}


def main():
    isolate_user_state()  # never measure with the user's own settings or saved positions
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--profile', choices=['quick','full','stress'], default='full')
    cli.add_argument('--output', type=Path, help='New or empty directory (default perf-results/TIMESTAMP)')
    cli.add_argument('--runs', type=int, help='Measured repetitions: quick 1, full/stress 3')
    cli.add_argument('--stress-seconds', type=int, help='Mixed model workload in one process: full 10, stress 900')
    cli.add_argument('--ui', choices=['auto','off','native'], default='auto', help='auto: Linux private Xvfb; native: opt-in macOS desktop windows')
    cli.add_argument('--xvfb', default=shutil.which('Xvfb'))
    cli.add_argument('--renderer-label', default='unspecified', help='Record renderer/device/driver details for reproducible GUI comparisons')
    cli.add_argument('--require-ui', action='store_true', help='Fail if startup or automated interactions are unavailable')
    cli.add_argument('--max-growth-mib', type=float, default=512, help='Memory-growth ceiling for UI browsing and the latter half of model stress')
    cli.add_argument('--check', action='store_true', help='Enforce the original reference-machine timing budgets')
    cli.add_argument('--compare', type=Path, help='Previous result directory or results.json')
    cli.add_argument('--fail-regression', type=float, metavar='PERCENT', help='Fail above this median increase; requires comparable baseline')
    cli.add_argument('--allow-machine-mismatch', action='store_true', help='Allow informational cross-machine comparison; regression gate remains disabled')
    cli.add_argument('--timeout', type=float, default=300, help='Per-case timeout in seconds, excluding builds and configured stress duration')
    opt = cli.parse_args()
    opt.runs = opt.runs if opt.runs is not None else (1 if opt.profile == 'quick' else 3)
    opt.stress_seconds = opt.stress_seconds if opt.stress_seconds is not None else (900 if opt.profile == 'stress' else 10)
    if opt.runs < 1 or opt.stress_seconds < 1 or opt.timeout <= 0 or not math.isfinite(opt.timeout) or not math.isfinite(opt.max_growth_mib) or opt.max_growth_mib < 0:
        cli.error('runs, stress-seconds and timeout must be positive')
    if opt.fail_regression is not None and (not opt.compare or opt.fail_regression < 0 or not math.isfinite(opt.fail_regression)):
        cli.error('--fail-regression requires --compare and a nonnegative percentage')
    if opt.ui == 'native' and platform.system() != 'Darwin':
        cli.error('--ui native is macOS only; Linux always uses a private display')
    if platform.system() not in ('Linux', 'Darwin'):
        cli.error('supported hosts are Linux and macOS')
    out = (opt.output or ROOT/'perf-results'/datetime.datetime.now().strftime('%Y%m%d-%H%M%S')).resolve()
    if out.exists() and any(out.iterdir()):
        cli.error('output directory must be new or empty')
    if opt.compare:
        baseline_path = opt.compare/'results.json' if opt.compare.is_dir() else opt.compare
        try:
            baseline = json.loads(baseline_path.read_text())
        except (OSError, ValueError) as exc:
            cli.error(f'cannot read baseline: {exc}')
        if baseline.get('schema') != SCHEMA or baseline.get('workload_version') != WORKLOAD_VERSION:
            cli.error('baseline schema/workload version differs; collect a baseline with the current suite')
    out.mkdir(parents=True, exist_ok=True)
    (out/'logs').mkdir()
    env = dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1', LENZ_PERF_JSON='1',
               LENZ_LOG_DIR=str(out/'diagnostics'), LENZ_STRESS_SECONDS=str(opt.stress_seconds), PYTHONDONTWRITEBYTECODE='1', LC_ALL='C')
    if opt.check:
        env.pop('LENZ_PERF_REPORT_ONLY', None)
    else:
        env['LENZ_PERF_REPORT_ONLY'] = '1'
    env.pop('LENZ_PERF_FILE', None)
    ui = 'native-macos' if opt.ui == 'native' else 'private-xvfb' if opt.ui != 'off' and platform.system() == 'Linux' and opt.xvfb else 'off'
    if opt.profile == 'quick':
        ui = 'off'
    result = {'schema':SCHEMA, 'workload_version':WORKLOAD_VERSION, 'started_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'machine':machine(env), 'revision':command_text(['git','rev-parse','HEAD'], env),
              'dirty':bool(command_text(['git','status','--porcelain'], env)),
              'rustc':command_text(['rustc','-Vv'], env), 'git':command_text(['git','--version'], env),
              'python':platform.python_version(), 'environment':{key:env.get(key) for key in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_BUILD_TARGET','MALLOC_ARENA_MAX','GLIBC_TUNABLES')}, 'cargo_lock_sha256':hashlib.sha256((ROOT/'Cargo.lock').read_bytes()).hexdigest(),
              'settings':{'profile':opt.profile, 'runs':opt.runs, 'stress_seconds':opt.stress_seconds, 'ui':ui,
                          'build_profile':'release', 'renderer_label':opt.renderer_label, 'timing_budgets':opt.check, 'max_growth_mib':opt.max_growth_mib},
              'cases':[], 'metrics':{}, 'skipped':[], 'status':'running'}

    def save():
        temporary = out/'results.tmp'
        temporary.write_text(json.dumps(result, indent=2, allow_nan=False)+'\n')
        temporary.replace(out/'results.json')

    def collect(name, command, trial=1, timeout=None, cwd=ROOT, parse=True, case_env=env):
        print(f'[{name} {trial}]', flush=True)
        log = out/'logs'/f'{name}-{trial}.log'
        row = {'name':name, 'trial':trial, **run(command,log,case_env,timeout or opt.timeout,cwd)}
        if parse:
            try:
                measured = metrics(log.read_text())
                if not measured:
                    raise ValueError('no benchmark measurements emitted (wrong test filter or executable)')
                for item in measured:
                    key = item['name']
                    existing = result['metrics'].setdefault(key, {'unit':item['unit'],'samples':[]})
                    if existing['unit'] != item['unit']:
                        raise ValueError(f'changed units for {key}')
                    existing['samples'].extend(item['samples'])
            except ValueError as exc:
                row.update(status='failed', error=str(exc))
        result['cases'].append(row)
        save()
        if row['status'] != 'passed':
            raise RuntimeError(f'{name} failed; see {row["log"]}')
        return row, log.read_text()

    def skip(name, reason):
        result['skipped'].append({'name':name,'reason':reason})
        save()

    def add(name, unit, values):
        result['metrics'].setdefault(name, {'unit':unit,'samples':[]})['samples'].extend(values)

    try:
        artifacts = {}
        for label, command in [
            ('build-tests',['cargo','test','--locked','--release','-p','lenz','--no-run','--message-format=json']),
            ('build-content-tests',['cargo','test','--locked','--release','-p','lenz-content','--no-run','--message-format=json']),
            ('build-engine',['cargo','bench','--locked','-p','lenz-engine','--bench','engine','--no-run','--message-format=json']),
            ('build-app',['cargo','build','--locked','--release','-p','lenz','--message-format=json']),
            ('build-tui',['cargo','build','--locked','--release','-p','lenz-tui','--message-format=json']),
            ('build-tui-tests',['cargo','test','--locked','--release','-p','lenz-tui','--no-run','--message-format=json']),
        ]:
            row, text = collect(label,command,timeout=3600,parse=False)
            if row['status'] != 'passed':
                raise RuntimeError(f'{label} failed; see {row["log"]}')
            kind,name,test = {
                'build-tests':('lib','lenz',True),
                'build-content-tests':('lib','lenz_content',True),
                'build-engine':('bench','engine',True),
                'build-app':('bin','lenz',False),
                'build-tui':('bin','lenz-tui',False),
                'build-tui-tests':('bin','lenz-tui',True),
            }[label]
            artifacts[label]=cargo_executable(text,kind,name,test)
        result['executables'] = {k:{'path':v, 'sha256':hashlib.sha256(Path(v).read_bytes()).hexdigest()} for k,v in artifacts.items()}
        source_paths = sorted([ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT/'assets/lenz-icon.png',ROOT/'assets/lenz-about.png',ROOT/'assets/lenz-about-terminal.rgba',ROOT/'packaging/lenz.desktop',ROOT/'packaging/package.sh',ROOT/'packaging/aur/PKGBUILD',*[p for base in ('crates','scripts','benches') for p in (ROOT/base).rglob('*') if p.is_file() and p.suffix in ('.rs','.py','.toml')]])
        sources = {str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in source_paths}
        (out/'sources.json').write_text(json.dumps(sources,indent=2)+'\n')
        result['source_manifest_sha256'] = hashlib.sha256(json.dumps(sources,sort_keys=True).encode()).hexdigest()
        tests = [
            ('view','view::tests::view_model_perf_on_a_100k_line_pair',True,'build-tests'),
            ('refine-poll','view::tests::refinement_polling_perf_5000_blocks',True,'build-tests'),
            ('merge','ui::merge_view::tests::merge_view_perf',True,'build-tests'),
            ('syntax','highlight::tests::perf',True,'build-content-tests'),
            ('search','find::tests::search_perf',True,'build-tests'),
            ('search-session','search_session::scheduler_perf::search_session_perf_stream_cancel_and_retained_rows',True,'build-tests'),
            ('images','images::tests::image_perf',True,'build-tests'),
            ('history','history::tests::layout_a_million_synthetic_commits',True,'build-tests'),
            ('decode','encoding::tests::timings',False,'build-content-tests'),
        ]
        if opt.profile != 'quick':
            tests.append(('blame','blame::tests::blame_timing_on_a_long_history',True,'build-tests'))
            tests.append(('blame-fragmented','annotation_session::tests::annotation_perf_fragmented_polling',True,'build-tests'))
        # Warm a fresh process for each component, excluded from results. Builds are never timed as workloads.
        for trial in range(0, opt.runs+1):
            prefix = 'warmup-' if trial == 0 else ''
            engine_cmd = [artifacts['build-engine'], *(['--quick'] if opt.profile == 'quick' else []), *(['--check'] if opt.check else [])]
            terminal_cases = [
                ('tui-render',[artifacts['build-tui-tests'],'app::tests::terminal_perf','--exact','--ignored','--nocapture']),
                ('tui-about',[artifacts['build-tui-tests'],'app::chrome_tests::terminal_perf_about','--exact','--ignored','--nocapture']),
                ('tui-chrome',[artifacts['build-tui-tests'],'app::chrome_tests::terminal_perf_chrome','--exact','--ignored','--nocapture']),
                ('tui-images-syntax',[artifacts['build-tui-tests'],'visual::tests::terminal_perf_images_syntax','--exact','--ignored','--nocapture']),
                ('tui-visual-lifecycle',[artifacts['build-tui-tests'],'visual::tests::terminal_perf_visual_lifecycle','--exact','--ignored','--nocapture']),
                ('tui-merge',[artifacts['build-tui-tests'],'merge::tests::terminal_perf_merge','--exact','--ignored','--nocapture']),
                ('tui-independent-views',[artifacts['build-tui-tests'],'tab_perf_tests::terminal_perf_independent_views','--exact','--ignored','--nocapture']),
                ('tui-workflows',[sys.executable,str(SCRIPTS/'tui-workflow-bench.py'),'--app',artifacts['build-tui'],
                                  '--lines','300' if opt.profile == 'quick' else '20000' if opt.profile == 'stress' else '2000',
                                  '--rounds','111' if opt.profile == 'stress' else '3' if opt.profile == 'quick' else '11']),
                ('tui-pty',[sys.executable,str(SCRIPTS/'tui-bench.py'),'--app',artifacts['build-tui'],
                            '--lines','1000' if opt.profile == 'quick' else '100000',
                            '--rounds','500' if opt.profile == 'stress' else '30']),
            ]
            for name, command in [('engine',engine_cmd), *[(name,[artifacts[binary],test,'--exact','--nocapture',*(['--ignored'] if ignored else [])]) for name,test,ignored,binary in tests], *terminal_cases]:
                row, text = collect(prefix+name,command,trial=trial,parse=trial>0)
                if name == 'engine' and 'skipped:' in text and trial == 1:
                    skip('optional-tokio-corpus',next(line for line in text.splitlines() if 'skipped:' in line))
        if opt.profile != 'quick':
            row,_ = collect('mixed-stress',[artifacts['build-tests'],'perf_tests::workload_stress','--exact','--ignored','--nocapture'],timeout=opt.stress_seconds+opt.timeout)
            growth = tail_growth(result['metrics'].get('stress/rss-timeline',{}).get('samples',[]))
            row['tail_growth_mib'] = growth
            if growth is None:
                skip('stress-memory-growth', 'fewer than four RSS samples; increase --stress-seconds')
            elif growth > opt.max_growth_mib:
                row.update(status='failed',error=f'sustained memory growth {growth:.1f} MiB exceeds limit')
            save()
        else:
            skip('mixed-stress','quick profile; use full or stress')

        if opt.profile != 'quick':
            fixture = out/'fixtures'
            row,_ = collect('fixtures',[sys.executable,str(SCRIPTS/'perf-fixtures.py'),'--engine',artifacts['build-engine'],str(fixture)],parse=False)
            if row['status'] != 'passed':
                raise RuntimeError('fixture generation failed')
            manifest = {str(p.relative_to(fixture)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(fixture.rglob('*')) if p.is_file() and '.git' not in p.parts}
            (out/'fixtures.json').write_text(json.dumps(manifest,indent=2)+'\n')
            result['fixture_manifest_sha256'] = hashlib.sha256(json.dumps(manifest,sort_keys=True).encode()).hexdigest()
            if ui != 'off':
                display_args = ['--native'] if ui == 'native-macos' else ['--xvfb',opt.xvfb]
                cases = [(name,[str(fixture/name/('left.rs' if '100k' in name else 'left.png' if name == 'images-4k' else 'left.txt')),
                               str(fixture/name/('right.rs' if '100k' in name else 'right.png' if name == 'images-4k' else 'right.txt'))],ROOT)
                         for name in ['small','generated-100k','different-100k','long','unicode','crlf','windows1252','binary','oversized','images-4k']]
                merge_args = ['merge',*[str(fixture/'merge'/f'{side}.rs') for side in ['base','ours','theirs','merged']]]
                cases += [('about-standalone',['--about'],Path(tempfile.gettempdir())),('merge',merge_args,ROOT),('repository',['log'],fixture/'history'),('many-small',[str(fixture/'many-small'/'left'),str(fixture/'many-small'/'right')],ROOT)]
                for cache in ['warm', *(['fresh-app-cache'] if ui == 'private-xvfb' else [])]:
                    for name,args,cwd in cases:
                        if cache != 'warm' and name not in ('small','merge','images-4k'):
                            continue
                        command = [sys.executable,str(SCRIPTS/'startup-bench.py'),'--binary',artifacts['build-app'],*display_args,'--runs',str(opt.runs),*(['--warmups','1'] if cache == 'warm' else ['--fresh-cache']),'--',*args]
                        row,text = collect(f'startup-{name}-{cache}',command,parse=False,cwd=cwd,timeout=max(opt.timeout,30*(opt.runs+1)))
                        if row['status'] == 'passed':
                            records = [json.loads(line) for line in text.splitlines() if line.startswith('{')]
                            if len(records) != opt.runs:
                                row.update(status='failed',error='missing startup samples')
                            for field,unit in [('first_paint_ms','ms'),('load_ms','ms'),('diff_ms','ms'),('rss_mib','MiB'),('peak_rss_mib','MiB'),('cpu_seconds','s'),('last_second_cpu_percent','percent')]:
                                values = [r[field] for r in records if r.get(field) is not None]
                                if values: add(f'startup/{cache}/{name}/{field}',unit,values)
                        save()
                if ui == 'private-xvfb':
                    scenarios = [('about','small','files'),('diff','generated-100k','files'),('diff','long','files'),('merge','merge','merge'),('images','images-4k','files'),('files','files','dirs'),('files','image-files','dirs'),('history','history','repo')]
                    for scenario,name,kind in scenarios:
                        args = merge_args if kind == 'merge' else ['log'] if kind == 'repo' else [str(fixture/name/side) for side in ('left','right')] if kind == 'dirs' else next(a for n,a,_ in cases if n==name)
                        extra = ['--files','6','--file-pause','1'] if name == 'image-files' else []
                        rounds = 50 if opt.profile == 'stress' else 3
                        if scenario == 'files': extra += ['--rounds',str(rounds),'--max-growth-mib',str(opt.max_growth_mib)]
                        for trial in range(1,opt.runs+1):
                            command = [sys.executable,str(SCRIPTS/'interaction-bench.py'),'--binary',artifacts['build-app'],'--xvfb',opt.xvfb,'--scenario',scenario,'--check',*extra,'--',*args]
                            row,text = collect('ui-'+name,command,trial=trial,parse=False,cwd=fixture/'history' if kind=='repo' else ROOT,timeout=max(opt.timeout,rounds*15))
                            records = [json.loads(line) for line in text.splitlines() if line.startswith('{')]
                            if row['status'] == 'passed' and not any(r['phase']=='settled' for r in records):
                                row.update(status='failed',error='missing settled sample')
                            for record in records:
                                phase=record['phase']
                                values=record.get('first_damage_samples_ms',[])
                                if values and all(v is not None for v in values): add(f'ui/{name}/{phase}/first-damage','ms',values)
                                if 'rss_mib' in record: add(f'ui/{name}/{phase}/rss','MiB',[record['rss_mib']])
                            save()
                else:
                    skip('ui-interactions','XTest/XDamage are Linux-only; use the native macOS manual walkthrough for input/rendering')
                    skip('fresh-app-cache','macOS cache isolation is not implemented; OS cold boot is never simulated')
            else:
                skip('ui-startup','no private Xvfb, --ui off, or macOS without explicit --ui native')
                skip('ui-interactions','no private Xvfb; native interactions remain a manual walkthrough')
        else:
            skip('ui-startup','quick profile excludes windows and fixture generation')
            skip('ui-interactions','quick profile excludes windows and fixture generation')
        if opt.require_ui and any(s['name'] in ('ui-startup','ui-interactions') for s in result['skipped']):
            raise RuntimeError('required UI coverage is unavailable')
        if opt.compare:
            if baseline.get('schema') != SCHEMA or baseline.get('workload_version') != result['workload_version']:
                raise ValueError('baseline schema/workload version differs')
            if baseline.get('status') != 'passed':
                raise ValueError('baseline is not a passing completed run')
            comparable = baseline['machine'] == result['machine'] and baseline['settings'] == result['settings'] and baseline.get('fixture_manifest_sha256') == result.get('fixture_manifest_sha256') and baseline.get('rustc') == result['rustc'] and baseline.get('environment') == result['environment']
            if not comparable and (not opt.allow_machine_mismatch or opt.fail_regression is not None):
                raise ValueError('machine/workload settings differ; --allow-machine-mismatch permits informational comparison only')
            selected = lambda d: {k:v for k,v in d.items() if not k.endswith('/rss-time')}
            comparison = compare(selected(baseline['metrics']),selected(result['metrics']),opt.fail_regression if opt.fail_regression is not None else 15)
            comparison['comparable'] = comparable
            result['comparison'] = comparison
            if opt.fail_regression is not None and (comparison['regressions'] or comparison['missing'] or comparison['uncomparable']):
                raise RuntimeError('performance regression or missing baseline measurements; see comparison')
        result['status'] = 'failed' if any(c['status']!='passed' for c in result['cases']) else 'passed'
    except (Exception, KeyboardInterrupt) as exc:
        result['status'] = 'interrupted' if isinstance(exc,KeyboardInterrupt) else 'failed'
        result['error'] = str(exc) or 'interrupted'
        print(f'ERROR: {result["error"]}',file=sys.stderr)
    finally:
        result['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save()
        report = ['# Lenz benchmark results','',f'Status: **{result["status"]}**', '',f'Machine: `{result["machine"]}`',f'Revision: `{result["revision"]}` (dirty: {result["dirty"]})',f'Settings: `{result["settings"]}`','',
                  'Raw samples, commands, status and metadata: [results.json](results.json). Builds and untimed warmups are excluded from the metrics. RSS is process residency, not GPU memory. UI damage is early feedback, not completed loading or physical presentation.','',
                  '| Metric | Median | p95 | Samples |','| --- | ---: | ---: | ---: |']
        for name,item in sorted(result['metrics'].items()):
            values=sorted(item['samples'])
            p95=values[min(len(values)-1, max(0, math.ceil(.95*len(values))-1))]
            report.append(f'| {name} | {statistics.median(values):.4g} {item["unit"]} | {p95:.4g} | {len(values)} |')
        report += ['', '## Skipped coverage',''] + [f'- {s["name"]}: {s["reason"]}' for s in result['skipped']]
        report += ['', '## Failed cases',''] + [f'- {c["name"]}: {c["status"]}; [log](logs/{Path(c["log"]).name})' for c in result['cases'] if c['status']!='passed']
        if result.get('error'): report += ['',result['error']]
        if 'comparison' in result:
            report += ['', '## Baseline comparison','',f'Comparable: {result["comparison"]["comparable"]}', '', '| Metric | Median change |','| --- | ---: |']
            report += [f'| {k} | {v:+.1f}% |' for k,v in sorted(result['comparison']['changes'].items())]
            report += ['',f'Missing: {result["comparison"]["missing"]}',f'Added: {result["comparison"]["added"]}',f'Cannot compute percentage from zero baseline: {result["comparison"]["uncomparable"]}']
        (out/'summary.md').write_text('\n'.join(report)+'\n')
        print(f'{result["status"].upper()}: {out}/summary.md',flush=True)
    return 0 if result['status']=='passed' else 1


if __name__ == '__main__':
    sys.exit(main())
