// Adapter for the public WASM pitch frontend served by Singing Carrots.
// Usage: node tools/singing_carrots_probe.cjs AUBIO_JS AUBIO_WASM OUTPUT TRACK_LIST
// Uses the site's call: new Pitch("default", 4096, 1, sampleRate), one full block per call.
const fs = require('fs');
const path = require('path');
const [js, wasm, output, trackList] = process.argv.slice(2);
if (!trackList) throw new Error('expected AUBIO_JS AUBIO_WASM OUTPUT TRACK_LIST');
const Aubio = require(path.resolve(js));

function readWav(filename) {
  const data = fs.readFileSync(filename);
  if (data.toString('ascii', 0, 4) !== 'RIFF' || data.toString('ascii', 8, 12) !== 'WAVE')
    throw new Error(`not RIFF WAVE: ${filename}`);
  let format, rate, bits, audio;
  for (let offset = 12; offset + 8 <= data.length;) {
    const kind = data.toString('ascii', offset, offset + 4);
    const size = data.readUInt32LE(offset + 4);
    const start = offset + 8;
    if (kind === 'fmt ') {
      format = data.readUInt16LE(start);
      if (format === 65534 && size >= 40) format = data.readUInt16LE(start + 24);
      if (data.readUInt16LE(start + 2) !== 1) throw new Error('expected mono WAV');
      rate = data.readUInt32LE(start + 4);
      bits = data.readUInt16LE(start + 14);
    } else if (kind === 'data') audio = data.subarray(start, start + size);
    offset = start + size + (size % 2);
  }
  if (!audio || !rate || !((format === 1 && bits === 16) || (format === 3 && bits === 32)) || audio.length % (bits / 8))
    throw new Error(`expected PCM16 or float32 mono WAV: ${filename}`);
  const samples = Float32Array.from({length: audio.length / (bits / 8)}, (_, i) =>
    format === 1 ? audio.readInt16LE(i * 2) / 32768 : audio.readFloatLE(i * 4));
  return {samples, rate};
}

Aubio({wasmBinary: fs.readFileSync(wasm)}).then(module => {
  try {
    fs.mkdirSync(output, {recursive:true});
    const tracks = fs.readFileSync(trackList, 'utf8').split(/\r?\n/).filter(Boolean);
    if (!tracks.length) throw new Error('empty track list');
    for (const filename of tracks) {
      const {samples, rate} = readWav(filename);
      const detector = new module.Pitch('default', 4096, 1, rate);
      const destination = path.join(output, path.basename(filename, '.wav') + '.csv');
      const fd = fs.openSync(destination, 'wx');
      try {
        for (let start = 0; start + 4096 <= samples.length; start += 4096) {
          const frequency = detector.do(samples.subarray(start, start + 4096));
          if (!Number.isFinite(frequency) || frequency < 0) throw new Error('invalid pitch');
          fs.writeSync(fd, `${(start + 4096) / rate},${frequency}\n`);
        }
      } finally { fs.closeSync(fd); detector.delete(); }
      console.log(path.basename(filename));
    }
    fs.writeFileSync(path.join(output, 'run.json'), JSON.stringify({
      detector:'Singing Carrots published WASM frontend', audio:process.env.PITCH_CAPTURE_AUDIO || 'provided WAV without added noise',
      method:'default', window_samples:4096, hop_samples:4096,
      timestamp:'right edge of full block; no padding', tracks:tracks.length,
      source:'https://singingcarrots.com/js/tuner/tuner.js?v=1016',
      captured_at:new Date().toISOString()
    }, null, 2) + '\n', {flag:'wx'});
  } catch (error) { console.error(error); process.exitCode = 1; }
});
