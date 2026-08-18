import React from "react";
import ReactDOM from "react-dom/client";

/// `?gallery` renders the design system's specimen page instead of the app.
/// It runs in a plain browser tab with no Tauri bridge, which is what makes the
/// UI reviewable without a native build.
///
/// Both branches import dynamically, and that is load-bearing rather than
/// stylistic: a static `import App` would pull `App.css` into the gallery too,
/// where its `:root` tokens and `body` rules collide with the library's. The
/// specimen page would then be judging whichever stylesheet happened to load
/// last. Each entry point loads exactly one design system.
const showGallery = new URLSearchParams(window.location.search).has("gallery");

async function mount() {
  const el = document.getElementById("root");
  if (!el) throw new Error("root element missing");
  const root = ReactDOM.createRoot(el);

  if (showGallery) {
    const { Gallery } = await import("@ui/gallery/Gallery");
    root.render(
      <React.StrictMode>
        <Gallery />
      </React.StrictMode>,
    );
    return;
  }

  const { default: App } = await import("./shell/App");
  root.render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void mount();
