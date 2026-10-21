// Capture a compact, reproducible oracle for the Rust calculation, using R2's
// preserved WASM. No corpus or WAVs needed; the same signal recipe lives in
// yinfft/tests.rs. Regenerate only when intentionally changing the reference.
// node tools/yinfft_oracle_fixture.cjs AUBIO_JS AUBIO_WASM OUTPUT_JSON
const fs = require('fs');
const path = require('path');
const [js, wasm, output] = process.argv.slice(2);
if (!output) throw new Error('expected AUBIO_JS AUBIO_WASM OUTPUT_JSON');

function signal(rate, hz, amplitude, kind) {
  let state = 1;
  return Float32Array.from({length: 2048}, (_, i) => {
    const phase = 2 * Math.PI * hz * i / rate;
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    const noise = (state >>> 16) / 32768 - 1;
    let value;
    switch (kind) {
      case 'pure': value = Math.sin(phase); break;
      case 'missing': value = (Math.sin(2 * phase) + Math.sin(3 * phase)) / 2; break;
      case 'harmonic_noise': value = (Math.sin(phase) + .8 * Math.sin(2 * phase) + .6 * Math.sin(3 * phase) + .3 * noise) / 2.4; break;
      case 'noise': value = noise; break;
      case 'dc': value = 1; break;
      default: throw new Error(kind);
    }
    return amplitude * value;
  });
}

require(path.resolve(js))({wasmBinary: fs.readFileSync(wasm)}).then(module => {
  try {
    const cases = [];
    for (const rate of [8000, 16000, 44100, 48000]) {
      const detector = new module.Pitch('yinfft', 2048, 1, rate);
      try {
        for (const hz of [55, 65.40639, 110, 220, 440, 880, 1300, 1760, 2637, 3500, 4186]) {
          for (const kind of ['pure', 'missing', 'harmonic_noise']) {
            if (hz * (kind === 'pure' ? 1 : 3) >= rate / 2) continue;
            for (const amplitude of [.6, .004, .005]) {
              const samples = signal(rate, hz, amplitude, kind);
              cases.push({rate, hz, amplitude, kind, expected_hz: detector.do(samples)});
            }
          }
        }
        for (const kind of ['noise', 'dc']) {
          cases.push({rate, hz: 440, amplitude: .6, kind, expected_hz: detector.do(signal(rate, 440, .6, kind))});
        }
      } finally { detector.delete(); }
    }
    fs.writeFileSync(output, JSON.stringify({
      reference: 'R2 preserved aubio WASM, explicit yinfft; tools/yinfft_oracle_fixture.cjs',
      window_samples: 2048, cases,
    }, null, 2) + '\n', {flag: 'wx'});
    console.log(`${cases.length} oracle windows captured`);
  } catch (error) { console.error(error); process.exitCode = 1; }
});
