// The page's entry point. Inside Tauri it only loads the app. On a dev server
// opened in an ordinary browser there is no backend, so it first installs the
// stand-in from tests/drive/fake-backend.js — that is how the UI gets driven.
// `import.meta.env.DEV` is false in a packaged build, which drops the stand-in
// and its fixture from the bundle.
const backend = import.meta.env.DEV && !window.__TAURI_INTERNALS__
  ? import("../drive/fake-backend.js")
  : Promise.resolve();
backend.then(() => import("./main.js"));
