import React from "react";
import { createRoot } from "react-dom/client";
import UIRoot from "./UIRoot";

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("Root element #root not found");
}

createRoot(rootEl).render(<UIRoot />);