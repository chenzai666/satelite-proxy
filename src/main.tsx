import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// Disable the webview's default right-click context menu (Windows & macOS)
document.addEventListener(
  "contextmenu",
  (e) => {
    e.preventDefault();
  },
  { capture: true },
);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
