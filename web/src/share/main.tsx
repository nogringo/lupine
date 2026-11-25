import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Shared } from "./Shared";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Shared />
  </StrictMode>,
);
