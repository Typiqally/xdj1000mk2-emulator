#!/usr/bin/env python3
"""Summarize the paired four-candidate measurements; retain all raw evidence."""
import argparse
import json
from pathlib import Path
import re
import statistics


def stats(values, minimum=3):
    if len(values) < minimum:
        raise RuntimeError(f'Incomplete measurements: {len(values)} < {minimum}')
    return {'samples': len(values), 'median': statistics.median(values),
            'min': min(values), 'max': max(values), 'values': values}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    args = parser.parse_args()
    root = args.root
    result = {'machine': json.loads((root / 'machine-identity.json').read_text()),
              'invalidation_ns': {}, 'indirect_ns_per_cycle': {}, 'display': {},
              'replay_cpu_seconds': {}, 'replay_checks': {}}
    for variant in ('before', 'after'):
        rows = [json.loads(line) for path in sorted(root.glob(f'invalidation-cpu-{variant}*.jsonl'))
                for line in path.read_text().splitlines()]
        result['invalidation_ns'][variant] = {
            blocks: stats([row['ns'] for row in rows if row['blocks'] == blocks], 45)
            for blocks in (0, 1, 16, 128)}
        rows = [json.loads(line) for path in sorted(root.glob(f'indirect-cpu-{variant}*.jsonl'))
                for line in path.read_text().splitlines()]
        result['indirect_ns_per_cycle'][variant] = stats([row['cpu_ns_per_cycle'] for row in rows], 45)
        for workload in ('full', 'small'):
            frames = [json.loads(path.read_text())
                      for path in sorted(root.glob(f'ui-final-{variant}-{workload}*/summary.json'))]
            result['display'][f'{variant}-{workload}'] = {
                'cpu_percent': stats([frame['cpu_percent'] for frame in frames]),
                'upload_p95_us': stats([frame['metrics']['upload_p95_us'] for frame in frames]),
                'echo_p95_ms': stats([frame['echo_p95_ms'] for frame in frames]),
                'uploaded_frames': stats([frame['metrics']['frames'] for frame in frames]),
                'peak_rss_mib': stats([frame['peak_rss_mib'] for frame in frames])}
    patterns = {'invalidation-baseline': 'replay-invalidation-baseline*',
                'invalidation-after': 'replay-invalidation-invalidation*',
                'busy-baseline': 'replay-variant-baseline*',
                'busy-layout': 'replay-variant-layout*', 'busy-dynamic': 'replay-variant-dynamic*',
                'variants-baseline': 'replay-quiet-baseline*',
                'layout': 'replay-quiet-layout*', 'dynamic': 'replay-quiet-dynamic*',
                'final-baseline': 'replay-final-baseline*', 'combined-loops': 'replay-final-combined*',
                'retained-baseline': 'replay-retained-baseline*', 'retained': 'replay-retained-retained*'}
    for name, pattern in patterns.items():
        paths = sorted(root.glob(pattern + '/run0.metrics.json'))
        result['replay_cpu_seconds'][name] = stats([
            json.loads(path.read_text())['cpu_seconds'] for path in paths])
        checks = []
        for path in paths:
            text = path.with_name('run0.out').read_text()
            if 'verdict: EXACT' not in text:
                raise RuntimeError(f'Replay failed: {path}')
            hashes, mismatches = re.search(r'state hashes (\d+) checked, (\d+) differ', text).groups()
            if int(mismatches):
                raise RuntimeError(f'State mismatch: {path}')
            checks.append({'path': str(path.parent), 'hashes': int(hashes),
                           'final': re.search(r'^final: (.*)$', text, re.M)[1]})
        if len({check['final'] for check in checks}) != 1:
            raise RuntimeError(f'Inconsistent final states: {name}')
        result['replay_checks'][name] = checks
    if len({check['final'] for checks in result['replay_checks'].values() for check in checks}) != 1:
        raise RuntimeError('Variants disagree on final state')
    (root / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({name: value['median'] for name, value in result['replay_cpu_seconds'].items()}, indent=2))


if __name__ == '__main__':
    main()
