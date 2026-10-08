#!/usr/bin/env python3
"""Rebuild frozen profile inputs and replay; never touches live player state."""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CORE = ROOT / 'native/qemu/hw/cdj/c6x'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def generate(args):
    args.out.mkdir(parents=True, exist_ok=True)
    source = args.core or CORE
    frozen = args.out / 'source'
    shutil.copytree(source, frozen, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    abi = re.search(r'#define C66X_JIT_ABI (\d+)',
                    (frozen / 'c66x_core_int.h').read_text())[1]
    def batch(src):
        dst = args.out / src.name
        dst.mkdir(exist_ok=True)
        inputs = {}
        for ram in src.glob('ram_*.bin'):
            link = dst / ram.name
            if not link.exists():
                link.symlink_to(ram.resolve())
            inputs[ram.name] = sha(ram)
        profile = (src / 'profile.txt').read_text()
        inputs['profile.txt'] = sha(src / 'profile.txt')
        # These are offline compiler inputs: regenerate against our core's
        # layout, preserving the recorded instruction bytes/counts. This is
        # not a cache identity/readiness conversion or a player-state import.
        profile = re.sub(r'^abi \d+$', 'abi ' + abi, profile, flags=re.M)
        (dst / 'profile.txt').write_text(profile)
        command = [sys.executable, str(frozen / 'tools/c14_jitgen.py'), str(dst),
                   str(dst / 'm'), '--lib', str(args.library.resolve()),
                   '--min', '20000', '--nodes', '1200', '--loops', '32',
                   '--cold', '0', '--fn-nodes', '1500', '--roots', '400',
                   '--kmin', '1000000', '--qroots', '128', '--qmin', '20000',
                   '--jobs', '2', *args.extra]
        env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
        env['C66X_JIT_CC'] = 'cc'
        with (dst / 'gen.log').open('w') as log:
            subprocess.run(command, env=env, stdout=log, stderr=log, check=True)
        result = {'inputs': inputs, 'generated_profile_sha256': sha(dst / 'profile.txt'),
                  'module_sha256': sha(dst / 'm.so'), 'argv': command}
        (dst / 'inputs.json').write_text(json.dumps(result, indent=2))
        print(src.name + ' built', flush=True)
        return str((dst / 'm.so').resolve())
    sources = sorted(args.profiles.glob('batch*'), key=lambda p: int(p.name[5:]))
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        modules = list(pool.map(batch, sources))
    (args.out / 'modules.txt').write_text(':'.join(modules))
    (args.out / 'identity.json').write_text(json.dumps({
        'core_sha256': sha(args.library), 'generator_sha256': sha(frozen / 'tools/c14_jitgen.py'),
        'header_sha256': sha(frozen / 'c66x_core_int.h'), 'abi': int(abi),
        'profiles': str(args.profiles.resolve()), 'extra': args.extra}, indent=2))


def replay(args):
    args.out.mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
    env.update(C66X_JIT=args.modules.read_text().strip(), REPLAY_WINDOW='451584000')
    identity = {'replay_sha256': sha(args.binary), 'record_sha256': sha(args.record),
                'modules': args.modules.read_text().strip(), 'cpu_workload': 'recorded DSP; no host audio/display'}
    (args.out / 'identity.json').write_text(json.dumps(identity, indent=2))
    for n in range(args.runs):
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        load = os.getloadavg()
        with (args.out / f'run{n}.out').open('w') as out, (args.out / f'run{n}.err').open('w') as err:
            subprocess.run([str(args.binary.resolve()), str(args.record.resolve())],
                           env=env, stdout=out, stderr=err, check=True)
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        (args.out / f'run{n}.metrics.json').write_text(json.dumps({
            'cpu_seconds': after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime,
            'load_average': load}, indent=2))
        text = (args.out / f'run{n}.out').read_text()
        if 'verdict: EXACT' not in text:
            raise RuntimeError('Replay did not establish exactness: ' + text[-1000:])
        print(f'run{n}: ' + text.splitlines()[0] + '; EXACT', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    g = commands.add_parser('generate')
    g.add_argument('--profiles', type=Path, required=True)
    g.add_argument('--out', type=Path, required=True)
    g.add_argument('--library', type=Path, required=True)
    g.add_argument('--core', type=Path, help='maintained source to freeze for this variant')
    g.add_argument('extra', nargs=argparse.REMAINDER)
    r = commands.add_parser('replay')
    r.add_argument('--binary', type=Path, required=True)
    r.add_argument('--modules', type=Path, required=True)
    r.add_argument('--record', type=Path, required=True)
    r.add_argument('--out', type=Path, required=True)
    r.add_argument('--runs', type=int, default=3)
    args = parser.parse_args()
    if args.command == 'generate':
        if args.extra[:1] == ['--']:
            args.extra = args.extra[1:]
        generate(args)
    else:
        replay(args)
