import React from "react";
import ReactDOM from "react-dom/client";

/// `?gallery` renders the specimen page in a plain browser tab. Both branches
/// import dynamically, or `App.css` follows into the gallery and its `:root`
/// tokens collide with the library's.
const showGallery = new URLSearchParams(window.location.search).has("gallery");

async function mount() {
  const el = document.getElementById("root");
  if (!el) throw new Error("root element missing");
  const root = ReactDOM.createRoot(el);

  // Substituted with `false` in a production build, so Rollup drops the
  // branch and its import rather than ship a dead chunk.
  if (import.meta.env.DEV && showGallery) {
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
