// A Web Worker running ngspice (scripts/build-ngspice-wasm.sh's ngspice.mjs beside it)
// for the browser build: `sim::browser` posts {id, netlist} and gets back, for that id,
// what libngspice's callbacks give (see crates/sim/src/browser.rs):
//   {type: 'init', kind, names, complex}   a plot begins: ngspice's vectors in its order
//   {type: 'data', values}                 points: per point re, im of each vector
//   {type: 'stat', text}                   "tran: 42.3%"
//   {type: 'done', stdout, stderr, notLoaded, exited, raw}   raw: the written raw file
//   {type: 'failed', message}              ngspice didn't load
// A run is synchronous (no threads): messages for it are sent as it goes, and the worker
// reads the next run once it ended. Stop terminates the worker.
import create from './ngspice.mjs';

// Compiled once per worker; a new instance (after ngspice asked to exit) reuses it.
let compiled = null;
// The XSPICE code models (side modules beside it, as scripts/build-ngspice-wasm.sh
// builds them; table.cm isn't shipped): fetched once, written into each instance's file
// system for ngspice's `codemodel` (its dlopen) to load.
const CODE_MODELS = ['spice2poly', 'analog', 'digital', 'xtradev', 'xtraevt', 'tlines'];
let codeModels = null;
let ng = null;
// The run going: its id, log, batch of points.
let run = null;

const post = (message, transfer) => postMessage({ id: run.id, ...message }, transfer ?? []);

/** Sends the batched points (by time: the live plot needs no more). */
function flush() {
  if (run.values.length) {
    const values = new Float64Array(run.values);
    post({ type: 'data', values }, [values.buffer]);
    run.values = [];
  }
  run.flushed = performance.now();
}

async function start() {
  compiled ??= WebAssembly.compileStreaming(fetch(new URL('./ngspice.wasm', import.meta.url)));
  codeModels ??= Promise.all(
    CODE_MODELS.map(async (name) => {
      const r = await fetch(new URL(`./${name}.cm`, import.meta.url));
      return r.ok ? [name, new Uint8Array(await r.arrayBuffer())] : null;
    }),
  );
  const module = await compiled;
  const models = (await codeModels).filter((m) => m);
  const m = await create({
    instantiateWasm: (imports, done) => {
      WebAssembly.instantiate(module, imports).then((i) => done(i, module));
      return {};
    },
  });
  const s = { m, exited: false };
  const str = (p) => (p ? m.UTF8ToString(p) : '');
  const i32 = (p) => m.HEAP32[p >> 2];
  const sendChar = m.addFunction((p) => {
    const line = str(p);
    const space = line.indexOf(' ');
    const stream = line.slice(0, space);
    if (run && stream === 'stderr') run.stderr += line.slice(space + 1) + '\n';
    else if (run) run.stdout += (stream === 'stdout' ? line.slice(space + 1) : line) + '\n';
    return 0;
  }, 'iiii');
  const sendStat = m.addFunction((p) => {
    if (run) post({ type: 'stat', text: str(p) });
    return 0;
  }, 'iiii');
  const controlledExit = m.addFunction(() => {
    s.exited = true;
    return 0;
  }, 'iiiiii');
  // vecvaluesall {int veccount, int vecindex, vecvalues **vecsa};
  // vecvalues {char *name, double creal, double cimag, bool is_scale, bool is_complex}.
  const sendData = m.addFunction((all) => {
    if (!run) return 0;
    const count = i32(all);
    const vecs = i32(all + 8);
    for (let k = 0; k < count; k++) {
      const v = i32(vecs + 4 * k);
      run.values.push(m.HEAPF64[(v + 8) >> 3], m.HEAPF64[(v + 16) >> 3]);
    }
    if (performance.now() - run.flushed > 50) flush();
    return 0;
  }, 'iiiii');
  // vecinfoall {char *name, *title, *date, *type; int veccount; vecinfo **vecs};
  // vecinfo {int number; char *vecname; bool is_real; void *pdvec, *pdvecscale}.
  const sendInitData = m.addFunction((all) => {
    if (!run) return 0;
    flush();
    const count = i32(all + 16);
    const vecs = i32(all + 20);
    const names = [];
    const complex = [];
    for (let k = 0; k < count; k++) {
      const v = i32(vecs + 4 * k);
      names.push(str(i32(v + 4)));
      complex.push(m.HEAPU8[v + 8] === 0);
    }
    post({ type: 'init', kind: str(i32(all + 12)), names, complex });
    return 0;
  }, 'iiii');
  const bgRunning = m.addFunction(() => 0, 'iiii');
  if (m._ngSpice_Init(sendChar, sendStat, controlledExit, sendData, sendInitData, bgRunning, 0) !== 0) {
    throw new Error('ngSpice_Init failed');
  }
  s.command = (c) => {
    const p = m.stringToNewUTF8(c);
    const r = m._ngSpice_Command(p);
    m._free(p);
    return r;
  };
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
  m.FS.mkdir('/cm');
  for (const [name, bytes] of models) {
    m.FS.writeFile(`/cm/${name}.cm`, bytes);
    s.command(`codemodel /cm/${name}.cm`);
  }
  // LTspice and PSpice compatibility, as on the desktop.
  s.command('set ngbehavior=ltpsa');
  return s;
}

/** Runs a netlist as `sim::Libngspice::run` does, `run` instead of `bg_run`. */
function simulate(netlist) {
  const s = ng;
  const end = (extra) => {
    flush();
    const message = { type: 'done', stdout: run.stdout, stderr: run.stderr, exited: s.exited, raw: null, ...extra };
    post(message, message.raw ? [message.raw.buffer] : []);
  };
  const loaded = s.circ(netlist.split(/\r?\n/));
  if (s.exited) return end({});
  if (loaded !== 0 || run.stderr.includes('circuit not parsed')) {
    s.command('remcirc');
    return end({ notLoaded: true });
  }
  s.command('run');
  const path = '/tmp/out.raw';
  const written = s.command(`write '${path}'`);
  s.command('destroy all');
  s.command('remcirc');
  let raw = null;
  if (written === 0 && s.m.FS.analyzePath(path).exists) {
    raw = s.m.FS.readFile(path);
    s.m.FS.unlink(path);
  }
  end({ raw });
}

// One run at a time, in order.
let queue = Promise.resolve();
self.onmessage = (e) => {
  queue = queue.then(async () => {
    run = { id: e.data.id, stdout: '', stderr: '', values: [], flushed: performance.now() };
    try {
      if (!ng || ng.exited) {
        ng = await start();
        // Not the run's: what ngspice says starting (no spinit: the code models are
        // loaded by hand).
        run.stdout = run.stderr = '';
      }
      simulate(e.data.netlist);
    } catch (err) {
      ng = null;
      post({ type: 'failed', message: String(err?.message ?? err) });
    }
    run = null;
  });
};
