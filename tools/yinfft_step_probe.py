"""Numerical controls for the published YINFFT frontend at two fixed cadences.

Usage: python tools/yinfft_step_probe.py --output datasets/yinfft_step_controls
Uses the cached JS/WASM from R1 and the same capture adapter as corpus runs.
The 16 step positions sample a 4096-sample hop; reported bounds are sampled,
not a proof of worst-case latency over every possible alignment.
"""
import argparse
import csv
import json
import math
from pathlib import Path
import struct
import subprocess
import wave


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--reference', type=Path, default=Path('datasets/reference/singing_carrots_2026-10-20'))
    args = parser.parse_args()
    inputs = args.output / 'inputs'
    inputs.mkdir(parents=True, exist_ok=False)
    rate = 44100
    cases = {}
    for name in ('pure', 'missing'):
        cases[name] = (None, 440.)
    for name, target in [('tone', 440 * 2 ** (2 / 12)), ('octave', 880.)]:
        for position in range(16):
            cases[f'{name}_{position:02}'] = (22050 + 256 * position, target)
    for name, (step, target) in cases.items():
        phase = 0.
        samples = bytearray()
        for i in range(2 * rate):
            frequency = 440. if step is None or i < step else target
            value = .3 * (math.sin(2 * phase) + math.sin(3 * phase)) if name == 'missing' else .6 * math.sin(phase)
            samples.extend(struct.pack('<h', round(value * 32767)))
            phase += 2 * math.pi * frequency / rate
        with wave.open(str(inputs / (name + '.wav')), 'wb') as writer:
            writer.setparams((1, 2, rate, 0, 'NONE', 'not compressed'))
            writer.writeframes(samples)
    track_list = args.output / 'tracks.txt'
    track_list.write_text(''.join(str((inputs / (name + '.wav')).resolve()) + '\n' for name in cases))
    results = {}
    for label, settings in [('native_4096', []), ('yinfft_2048', ['2048', '0.016', 'yinfft'])]:
        destination = args.output / label
        subprocess.run(['node', 'tools/singing_carrots_probe.cjs', str(args.reference / 'aubio.js'),
                        str(args.reference / 'aubio.wasm'), str(destination), str(track_list), *settings], check=True)
        checks, delays = {}, {'tone': [], 'octave': []}
        for name, (step, target) in cases.items():
            with (destination / (name + '.csv')).open() as source:
                values = [(float(t), float(f)) for t, f in csv.reader(source)]
            errors = [abs(1200 * math.log2(f / target)) if f > 0 else math.inf for _, f in values]
            assert len(errors) >= 10 and max(errors[-10:]) < 5, (label, name, errors[-10:])
            if step is None:
                checks[name + '_last10_max_error_cents'] = max(errors[-10:])
            else:
                first = next(i for i, (t, _) in enumerate(values)
                             if t >= step / rate and len(errors[i:i+3]) == 3 and max(errors[i:i+3]) <= 50)
                delay = (values[first][0] - step / rate) * 1000
                delays[name.split('_')[0]].append(delay)
                checks[name + '_ms'] = delay
        results[label] = {'checks': checks, 'latency_ms': {
            name: {'min': min(values), 'max': max(values)} for name, values in delays.items()}}
    results['protocol'] = {'sample_rate': rate, 'step_samples': [22050 + 256 * p for p in range(16)],
                           'target_tolerance_cents': 50, 'consecutive_frames': 3,
                           'timestamp': 'first of three correct frames, right edge of analysis window'}
    (args.output / 'checks.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps({label: result['latency_ms'] for label, result in results.items() if label != 'protocol'}, indent=2))


if __name__ == '__main__':
    main()
