import "./styles.css";

import { initTheme, Toaster, TooltipProvider } from "@vitavision/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/App";

initTheme();

const root = document.getElementById("root");
if (!root) throw new Error("index.html has no #root");

createRoot(root).render(
  <StrictMode>
    <TooltipProvider>
      <App />
      <Toaster />
    </TooltipProvider>
  </StrictMode>,
);
