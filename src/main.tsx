import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { logActivity, reportRendererFailures } from "./activity";
import "./styles.css";

// Before the first render, so an exception thrown by it is the first thing recorded rather
// than the one failure nothing can see.
const label = getCurrentWindow().label;
reportRendererFailures(label);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

logActivity(`${label} window rendered`);
