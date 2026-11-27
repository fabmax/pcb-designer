// The loading screen's progress (index.html's #loading): Trunk's initializer
// (`data-initializer` on the rust link) streams the app's wasm through these callbacks.
// `total` is the wasm's size at build time, so it's right even when the server compresses
// the download. Once the wasm is in, it compiles and the app starts; web.rs removes
// #loading when it runs, or shows why it couldn't start.
export default function () {
  const bar = () => document.getElementById('loading-bar');
  const text = () => document.getElementById('loading-text');
  const mb = (bytes) => (bytes / 1e6).toFixed(1);
  return {
    onProgress({ current, total }) {
      const fraction = total ? Math.min(current / total, 1) : 0;
      bar()?.style.setProperty('width', `${(fraction * 100).toFixed(1)}%`);
      const t = text();
      if (t) t.textContent = `Loading PCB Designer… ${mb(current)} of ${mb(total)} MB`;
    },
    onComplete() {
      const t = text();
      if (t) t.textContent = 'Starting PCB Designer…';
    },
    onFailure(error) {
      const loading = document.getElementById('loading');
      if (loading) loading.innerHTML = '<p></p>';
      loading?.querySelector('p')?.append(`PCB Designer could not be loaded: ${error?.message ?? error}`);
    },
  };
}
