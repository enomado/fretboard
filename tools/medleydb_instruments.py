# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Map every MDB-stem-synth stem to its instrument, from MedleyDB's public metadata.

    uv run tools/medleydb_instruments.py     # writes datasets/medleydb_meta/stem_instruments.tsv

MDB-stem-synth ships audio and f0 only; which instrument a stem is lives in MedleyDB's
per-song `<Artist>_<Song>_METADATA.yaml` (github.com/marl/medleydb). This fetches the YAML
for every song in the corpus (cached in `datasets/medleydb_meta/`, git-ignored like the
corpus) and writes `stem_id<TAB>instrument`, which `pitch_bench` reads for
`PITCH_BENCH_INSTRUMENT=violin`.

The YAML is parsed by pattern, not by a YAML library, to stay dependency-free: the files
are machine-written and uniform — `  S<nn>:` opens a stem, its first `    instrument:` line
names it (the deeper `raw:` entries are indented further and never match).
"""

import pathlib
import re
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent / "datasets"
AUDIO = ROOT / "MDB-stem-synth" / "audio_stems"
META = ROOT / "medleydb_meta"
URL = "https://raw.githubusercontent.com/marl/medleydb/master/medleydb/data/Metadata/{song}_METADATA.yaml"
SUFFIX = ".RESYN.wav"


def main() -> None:
    # The Zenodo tarball was packed on a Mac: every stem has an AppleDouble twin `._<stem>`
    # with the same extension. `pitch_bench` never sees them (they have no annotation), so
    # skip them here too rather than ask MedleyDB about a song called `._Artist`.
    stems = sorted(p.name[: -len(SUFFIX)] for p in AUDIO.glob(f"*{SUFFIX}") if not p.name.startswith("._"))
    songs = sorted({re.sub(r"_STEM_\d+$", "", stem) for stem in stems})
    META.mkdir(exist_ok=True)

    instrument: dict[str, str] = {}
    for song in songs:
        cached = META / f"{song}.yaml"
        if not cached.exists():
            with urllib.request.urlopen(URL.format(song=song)) as response:
                cached.write_bytes(response.read())
        stem = None
        for line in cached.read_text().splitlines():
            opened = re.match(r"  S(\d+):$", line)
            if opened:
                stem = f"{song}_STEM_{opened.group(1)}"
                continue
            named = re.match(r"    instrument: (.+)$", line)
            if named and stem:
                instrument[stem] = named.group(1).strip()
                stem = None

    missing = [stem for stem in stems if stem not in instrument]
    assert not missing, f"no instrument in MedleyDB metadata for {missing}"
    out = META / "stem_instruments.tsv"
    out.write_text("".join(f"{stem}\t{instrument[stem]}\n" for stem in stems))
    print(f"{len(stems)} stems → {out}")


if __name__ == "__main__":
    main()
