import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { initTelegramApp } from "./telegram/webapp";
import "./styles.css";

initTelegramApp();

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("missing #root element");
}

createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
