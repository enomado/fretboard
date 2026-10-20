#!/usr/bin/env python3
"""Run the pinned Tuner application's unchanged DSP on corpus WAVs (JDK required).

Writes time,frequency CSVs for the collector and the gated/smoothed output.
Score with the release-only pitch_bench::trajectory_rpa_on_corpus test.
Source and JVM dependencies are cached under git-ignored datasets/reference/.
"""
from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import struct
import tarfile
import urllib.request
import wave

ROOT = Path(__file__).resolve().parents[1]
COMMIT = '961de77619c52076304bc3d6a28c51f712d71acb'
KOTLIN = '2.4.10'
SOURCES = [
    'FFT', 'Correlation', 'FrequencySpectrum', 'AutoCorrelation', 'Harmonics',
    'HarmonicPredictor', 'CorrelationBasedFrequency', 'MaximumOfPolynomialFit',
    'AccurateSpectrumPeakFrequency', 'HarmonicStatistics', 'Inharmonicity',
    'AcousticWeighting', 'FrequencyDetectionCollectedResults', 'SampleData',
    'TimeSeries', 'WindowingFunctions', 'OutlierRemovingSmoother', 'FrequencyEvaluatorSimple',
]


def download(url: str, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_suffix(destination.suffix + '.partial')
    urllib.request.urlretrieve(url, partial)
    partial.replace(destination)


def dependency(cache: Path, group: str, artifact: str, version: str) -> Path:
    # Existing Gradle artifacts avoid a second download. No Gradle/Android build is run.
    gradle = Path.home() / '.gradle/caches/modules-2/files-2.1' / group / artifact / version
    existing = list(gradle.glob('*/*.jar'))
    if existing:
        return existing[0]
    jar = cache / f'{artifact}-{version}.jar'
    if not jar.is_file():
        download(f'https://repo.maven.apache.org/maven2/{group.replace(".", "/")}/{artifact}/{version}/{jar.name}', jar)
    return jar


def compile_probe(cache: Path, source: Path) -> str:
    runtime = [dependency(cache, 'org.jetbrains.kotlin', 'kotlin-stdlib', KOTLIN),
               dependency(cache, 'org.jetbrains.kotlinx', 'kotlinx-coroutines-core-jvm', '1.8.0'),
               dependency(cache, 'org.jetbrains', 'annotations', '13.0')]
    compiler = runtime + [dependency(cache, 'org.jetbrains.kotlin', artifact, KOTLIN) for artifact in [
        'kotlin-compiler-embeddable', 'kotlin-script-runtime', 'kotlin-daemon-embeddable', 'kotlin-build-tools-api']]
    compiler.append(dependency(cache, 'org.jetbrains.kotlin', 'kotlin-reflect', '1.6.10'))
    base = source / 'app/src/main/java/de/moekadu/tuner'
    files = [base / 'notedetection' / f'{name}.kt' for name in SOURCES]
    files += [base / 'misc' / f'{name}.kt' for name in ['DefaultValues', 'MemoryPool', 'UpdatableStatistics']]
    files += sorted((ROOT / 'tools/tuner_probe').glob('*.kt'))
    output = cache / 'probe.jar'
    subprocess.run(['java', '-Xmx512m', '-cp', os.pathsep.join(map(str, compiler)),
                    'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler', '-no-stdlib', '-no-reflect',
                    '-classpath', os.pathsep.join(map(str, runtime)), '-d', str(output),
                    *map(str, files)], check=True)
    return os.pathsep.join(map(str, [output, *runtime]))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    inputs = parser.add_mutually_exclusive_group(required=True)
    inputs.add_argument('--instrument', help='comma-separated MedleyDB instrument names')
    inputs.add_argument('--wav', type=Path, action='append', help='explicit PCM16 WAV; may be repeated')
    inputs.add_argument('--probe', action='store_true', help='pure/missing-fundamental controls and two pitch steps')
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--source', type=Path, help='existing checkout at the pinned commit')
    args = parser.parse_args()
    output = args.output.resolve()
    for stage in ['raw', 'line']:
        if (output / stage).exists():
            raise FileExistsError(f'refusing to overwrite {output / stage}')
    cache = ROOT / 'datasets/reference/tuner_probe'
    cache.mkdir(parents=True, exist_ok=True)
    if args.source:
        source = args.source.resolve()
        actual = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
        if actual != COMMIT:
            raise ValueError(f'Tuner source must be {COMMIT}, got {actual}')
        dirty = subprocess.check_output(['git', '-C', str(source), 'status', '--porcelain'], text=True)
        if dirty:
            raise ValueError('Tuner source has local changes')
    else:
        source = cache / COMMIT / 'tuner'
        if not source.is_dir():
            archive = cache / f'{COMMIT}.tar.gz'
            if not archive.is_file():
                download(f'https://codeberg.org/thetwom/Tuner/archive/{COMMIT}.tar.gz', archive)
            with tarfile.open(archive) as tar:
                tar.extractall(cache / COMMIT, filter='data')
        if not source.is_dir():
            raise FileNotFoundError(f'archive did not contain {source}')
    classpath = compile_probe(cache, source)
    wanted = {name.strip().lower() for name in args.instrument.split(',')} if args.instrument else set()
    if args.probe:
        inputs = output / 'inputs'
        inputs.mkdir(parents=True)
        tracks = []
        for name, target in [('pure', 440), ('missing', 440),
                             ('tone_step', 440 * 2 ** (2 / 12)), ('octave_step', 880)]:
            path = inputs / f'{name}.wav'
            tracks.append(path)
            def sample(i):
                t = i / 44100
                if name == 'missing':
                    return 0.3 * (math.sin(2 * math.pi * 880 * t) + math.sin(2 * math.pi * 1320 * t))
                phase = 440 * min(t, .5) + target * max(0, t - .5)
                return 0.6 * math.sin(2 * math.pi * phase)
            with wave.open(str(path), 'wb') as wav:
                wav.setparams((1, 2, 44100, 0, 'NONE', 'not compressed'))
                wav.writeframes(b''.join(struct.pack('<h', int(32767 * sample(i))) for i in range(44100)))
    elif args.wav:
        tracks = [path.resolve() for path in args.wav]
    else:
        rows = [line.split('\t') for line in (ROOT / 'datasets/medleydb_meta/stem_instruments.tsv').read_text().splitlines()]
        tracks = sorted(ROOT / f'datasets/MDB-stem-synth/audio_stems/{stem}.RESYN.wav'
                        for stem, instrument in rows if instrument.lower() in wanted)
    if not tracks:
        raise ValueError(f'no stems for {wanted}')
    if len({track.stem for track in tracks}) != len(tracks):
        raise ValueError('WAV basenames must be unique')
    output.mkdir(parents=True, exist_ok=True)
    track_list = output / 'tracks.txt'
    track_list.write_text(''.join(f'{track}\n' for track in tracks))
    subprocess.run(['java', '-Xmx512m', '-cp', classpath, 'ProbeKt', str(output), str(track_list)], check=True)
    for stage in ['raw', 'line']:
        manifest = dict(detector=f'Tuner {stage}', source_commit=COMMIT,
                        audio='clean corpus audio' if args.instrument else 'provided WAV without added noise',
                        window_samples=4096, hop_samples=1024, window='Tophat',
                        timestamp='right edge of full window; no padding',
                        frequency_range_hz=[16, 16000], instruments=sorted(wanted), tracks=len(tracks))
        (output / stage / 'run.json').write_text(json.dumps(manifest, indent=2) + '\n')
    if args.probe:
        checks = {}
        for stage in ['raw', 'line']:
            for name, target in [('pure', 440), ('missing', 440),
                                 ('tone_step', 440 * 2 ** (2 / 12)), ('octave_step', 880)]:
                pairs = [tuple(map(float, row.split(',')))
                         for row in (output / stage / f'{name}.csv').read_text().splitlines()]
                def correct(pair, tolerance):
                    return pair[1] > 0 and abs(1200 * math.log2(pair[1] / target)) < tolerance
                if name in ['pure', 'missing']:
                    if not all(correct(pair, 5) for pair in pairs[-10:]):
                        raise AssertionError(f'{stage} {name}: control outside 5 cents')
                else:
                    arrival = next(pairs[i][0] for i in range(len(pairs) - 2)
                                   if all(pair[0] >= .5 and correct(pair, 50) for pair in pairs[i:i + 3]))
                    checks[f'{stage}_{name}_ms'] = round((arrival - .5) * 1000, 1)
        (output / 'checks.json').write_text(json.dumps(checks, indent=2) + '\n')
        print(json.dumps(checks), flush=True)
    print(f'{len(tracks)} tracks captured to {output}', flush=True)


if __name__ == '__main__':
    main()
