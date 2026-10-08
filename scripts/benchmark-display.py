#!/usr/bin/env python3
"""Owned native GUI benchmark: synthetic full/sparse RFB, no firmware/audio."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time


def percentile(values, p=.95):
    return sorted(values)[round((len(values) - 1) * p)] if values else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--fixture', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--workload', choices=['full', 'small'], required=True)
    parser.add_argument('--seconds', type=float, default=15)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    script = [{'after': 1, 'action': 'button', 'name': 'cue', 'down': True},
              {'after': 1.2, 'action': 'button', 'name': 'cue', 'down': False}]
    script += [{'after': 2 + n * .05, 'action': 'bench-echo', 'id': n} for n in range(100)]
    script += [{'after': args.seconds - 1, 'action': 'snapshot', 'name': 'final'},
               {'after': args.seconds, 'action': 'quit'}]
    script_path = args.out / 'script.json'
    script_path.write_text(json.dumps(script))
    env = dict(os.environ, XDJ_FIXTURE_DAMAGE=args.workload)
    command = [str(args.binary.resolve()), '--backend', str(args.fixture.resolve()),
               '--firmware', 'fixture', '--diagnostics', str(args.out.resolve()),
               '--test-script', str(script_path.resolve())]
    samples = []
    with (args.out / 'launcher.log').open('w') as log:
        child = subprocess.Popen(command, env=env, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + args.seconds + 30
            while child.poll() is None:
                text = subprocess.check_output(['ps', '-p', str(child.pid), '-o', 'time=', '-o', 'rss='], text=True).split()
                if len(text) == 2:
                    cpu = sum(float(v) * 60 ** i for i, v in enumerate(reversed(text[0].split(':'))))
                    samples.append({'cpu_seconds': cpu, 'rss_kib': int(text[1])})
                if time.monotonic() > deadline:
                    raise RuntimeError('GUI exceeded owned benchmark deadline')
                time.sleep(.25)
            if child.returncode:
                raise RuntimeError('GUI failed; see ' + str(args.out / 'launcher.log'))
        finally:
            if child.poll() is None:
                child.terminate()
            child.wait(timeout=10)
    (args.out / 'samples.json').write_text(json.dumps(samples))
    fixture = json.loads((args.out / 'fixture.json').read_text())
    metrics = json.loads((args.out / 'metrics.json').read_text())
    events = [json.loads(line) for line in (args.out / 'app-events.jsonl').read_text().splitlines()]
    echoes = [e for e in events if e['event'] == 'input-ack' and e['command']['action'] == 'bench-echo']
    if len(echoes) != 100 or fixture['frames'] < args.seconds * 55:
        raise RuntimeError('Benchmark did not deliver required echoes/frames')
    held = set()
    for entry in fixture['commands']:
        command = entry['command']
        if command['action'] == 'button':
            if command['down']:
                held.add(command['name'])
            else:
                held.discard(command['name'])
        elif command['action'] == 'release':
            held.clear()
    if held:
        raise RuntimeError('Benchmark left held controls: ' + ', '.join(sorted(held)))
    summary = {'workload': args.workload, 'seconds': fixture['elapsed'], 'peer_frames': fixture['frames'],
               'cpu_seconds': max(s['cpu_seconds'] for s in samples),
               'peak_rss_mib': max(s['rss_kib'] for s in samples) / 1024,
               'echo_p95_ms': percentile([(e['host_received_unix_us'] - e['command']['host_sent_unix_us']) / 1000 for e in echoes]),
               'metrics': metrics, 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               'fixture_sha256': hashlib.sha256(args.fixture.read_bytes()).hexdigest(),
               'argv': command}
    summary['cpu_percent'] = summary['cpu_seconds'] / summary['seconds'] * 100
    (args.out / 'summary.json').write_text(json.dumps(summary, indent=2))
    print(json.dumps({k: v for k, v in summary.items() if k not in ('argv', 'metrics')}), flush=True)


if __name__ == '__main__':
    main()
