// Runs the Emscripten build of ngspice (scripts/build-ngspice-wasm.sh) in Node: an
// operating point and a transient of an RC low pass, through the shared API as the app
// uses it (ngSpice_Init, ngSpice_Circ, ngSpice_Command), the results read back from a
// raw file in Emscripten's in-memory file system. Then an error that makes ngspice ask to
// exit: the instance is spent, and a fresh one (from the same compiled module) runs on.
// Usage: node scripts/ngspice-wasm-test.mjs [DIR]   (DIR holds ngspice.mjs and .wasm)
import { pathToFileURL } from 'node:url';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';

const dir = path.resolve(process.argv[2] ?? 'target/ngspice/wasm32-emscripten');
const { default: create } = await import(pathToFileURL(path.join(dir, 'ngspice.mjs')).href);
let t = performance.now();
const compiled = await WebAssembly.compile(await readFile(path.join(dir, 'ngspice.wasm')));
console.log(`compile: ${(performance.now() - t).toFixed(0)} ms`);

// The XSPICE code models (side modules), loaded into each instance's file system.
const codeModels = [];
for (const f of (await readdir(dir)).filter((f) => f.endsWith('.cm'))) {
  codeModels.push([f, await readFile(path.join(dir, f))]);
}

let out = '';
const fail = (msg) => { console.error(`FAIL: ${msg}\n--- ngspice output ---\n${out}`); process.exit(1); };

/** A started ngspice: one per module instance, spent once ngspice asks to exit. */
async function start() {
  const m = await create({
    instantiateWasm: (imports, done) => {
      WebAssembly.instantiate(compiled, imports).then((i) => done(i, compiled));
      return {};
    },
  });
  const s = { m, points: 0, exited: false };
  const sendChar = m.addFunction((p) => { out += m.UTF8ToString(p) + '\n'; return 0; }, 'iiii');
  const sendStat = m.addFunction(() => 0, 'iiii');
  const controlledExit = m.addFunction(() => { s.exited = true; return 0; }, 'iiiiii');
  const sendData = m.addFunction(() => { s.points++; return 0; }, 'iiiii');
  const sendInitData = m.addFunction(() => 0, 'iiii');
  const bgRunning = m.addFunction(() => 0, 'iiii');
  if (m._ngSpice_Init(sendChar, sendStat, controlledExit, sendData, sendInitData, bgRunning, 0) !== 0) fail('init');
  s.command = (c) => { const p = m.stringToNewUTF8(c); const r = m._ngSpice_Command(p); m._free(p); return r; };
  /** Loads `lines` as the circuit (a NULL-terminated array of strings). */
  s.circ = (lines) => {
    const ptrs = lines.map((l) => m.stringToNewUTF8(l));
    const array = m._malloc(4 * (ptrs.length + 1));
    ptrs.forEach((p, i) => { m.HEAP32[(array >> 2) + i] = p; });
    m.HEAP32[(array >> 2) + ptrs.length] = 0;
    const r = m._ngSpice_Circ(array);
    ptrs.forEach((p) => m._free(p));
    m._free(array);
    return r;
  };
  /** Runs the netlist and returns the raw file ngspice writes. */
  s.run = (lines) => {
    if (s.circ(lines) !== 0) fail('circuit not loaded');
    const t0 = performance.now();
    if (s.command('run') !== 0) fail('run');
    const ms = performance.now() - t0;
    if (s.command("write '/tmp/out.raw'") !== 0) fail('write');
    s.command('destroy all');
    s.command('remcirc');
    const raw = m.FS.readFile('/tmp/out.raw');
    m.FS.unlink('/tmp/out.raw');
    return { raw, ms };
  };
  m.FS.mkdir('/cm');
  for (const [name, bytes] of codeModels) {
    m.FS.writeFile(`/cm/${name}`, bytes);
    if (s.command(`codemodel /cm/${name}`) !== 0 || /Error/.test(out.split('\n').slice(-3).join('\n'))) {
      fail(`code model ${name} not loaded`);
    }
  }
  s.command('set ngbehavior=ltpsa');
  return s;
}

/** The values of `name` in a binary raw file's first plot. */
function vector(raw, name) {
  const text = new TextDecoder('latin1').decode(raw);
  const start = text.indexOf('Binary:\n') + 'Binary:\n'.length;
  const header = text.slice(0, start);
  const vars = Number(/No\. Variables:\s*(\d+)/.exec(header)[1]);
  const count = Number(/No\. Points:\s*(\d+)/.exec(header)[1]);
  const names = header.split('Variables:\n')[1].split('\n').slice(0, vars).map((l) => l.trim().split(/\s+/)[1]);
  const index = names.indexOf(name);
  if (index < 0) fail(`no ${name} in ${names}`);
  const view = new DataView(raw.buffer, raw.byteOffset + start);
  return Array.from({ length: count }, (_, i) => view.getFloat64((i * vars + index) * 8, true));
}

t = performance.now();
let s = await start();
console.log(`instance: ${(performance.now() - t).toFixed(0)} ms`);

// Operating point: a divider, 10 V over 1k and 3k → 7.5 V.
const divider = (r2) => ['* divider', 'V1 in 0 10', 'R1 in out 1k', `R2 out 0 ${r2}`, '.op', '.end'];
const op = s.run(divider('3k'));
const vop = vector(op.raw, 'v(out)')[0];
if (Math.abs(vop - 7.5) > 1e-9) fail(`op: v(out) = ${vop}`);
console.log(`op: v(out) = ${vop} V (${op.ms.toFixed(1)} ms)`);

// Transient: an RC low pass (1k, 1µ: τ = 1 ms) stepped to 1 V, at 5 τ ≈ 0.9933 V. Every
// accepted point is also streamed (SendData), as the live plot needs.
const tran = s.run(['* rc', 'V1 in 0 PULSE(0 1 0 1n 1n 1 2)', 'R1 in out 1k', 'C1 out 0 1u', '.tran 10u 5m', '.end']);
const time = vector(tran.raw, 'time');
const vout = vector(tran.raw, 'v(out)');
const expected = 1 - Math.exp(-time.at(-1) / 1e-3);
if (Math.abs(vout.at(-1) - expected) > 1e-3) fail(`tran: v(out) at ${time.at(-1)} = ${vout.at(-1)}, expected ${expected}`);
if (s.points < time.length) fail(`tran: ${s.points} streamed points, ${time.length} in the raw file`);
console.log(`tran: ${time.length} points, v(out) at ${time.at(-1)} s = ${vout.at(-1).toFixed(4)} V (${tran.ms.toFixed(1)} ms)`);

// Code models: a POLY source (spice2poly, as vendor op amp models use) and an XSPICE
// gain block (analog.cm).
const poly = s.run(['* poly', 'V1 in 0 1.5', 'R1 in 0 1k', 'E1 out 0 POLY(1) in 0 0.5 2', 'R2 out 0 1k', '.op', '.end']);
const vpoly = vector(poly.raw, 'v(out)')[0];
if (Math.abs(vpoly - 3.5) > 1e-9) fail(`poly: v(out) = ${vpoly}, expected 3.5`);
const gain = s.run(['* gain', 'V1 in 0 0.7', 'R1 in 0 1k', 'A1 in out amp', '.model amp gain(gain=3)', 'R2 out 0 1k', '.op', '.end']);
const vgain = vector(gain.raw, 'v(out)')[0];
if (Math.abs(vgain - 2.1) > 1e-9) fail(`gain: v(out) = ${vgain}, expected 2.1`);
// A digital inverter between bridges (digital.cm, xtradev's bridges are in digital).
const inv = s.run([
  '* inverter', 'V1 in 0 0', 'R1 in 0 1k', 'A1 [in] [d] adc', 'A2 d nd inv', 'A3 [nd] [out] dac', 'R2 out 0 1k',
  '.model adc adc_bridge(in_low=0.5 in_high=0.5)', '.model inv d_inverter', '.model dac dac_bridge(out_low=0 out_high=3.3)',
  '.op', '.end',
]);
const vinv = vector(inv.raw, 'v(out)')[0];
if (Math.abs(vinv - 3.3) > 1e-6) fail(`inverter: v(out) = ${vinv}, expected 3.3`);
console.log(`code models (${codeModels.length}): POLY ${vpoly} V, gain block ${vgain} V, inverter of 0: ${vinv} V`);

// A line ngspice can't recover from: it asks to exit, the instance is spent.
s.circ(['* broken', 'R1 a 0', '.op', '.end']);
if (!s.exited) fail('expected ngspice to ask to exit');
t = performance.now();
s = await start();
console.log(`exited after an error; new instance: ${(performance.now() - t).toFixed(0)} ms`);
const again = s.run(divider('1k'));
const v2 = vector(again.raw, 'v(out)')[0];
if (Math.abs(v2 - 5) > 1e-9) fail(`run after an error: v(out) = ${v2}`);
console.log('op in the new instance: ok');
console.log((/ngspice-\d+/.exec(out) ?? ['(no version)'])[0]);
