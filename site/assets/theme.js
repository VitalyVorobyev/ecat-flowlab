// Theme toggle shared by the site's pages. The choice is a per-viewer convenience.
(function () {
  const root = document.documentElement;
  try {
    const saved = localStorage.getItem("flowlab-theme");
    if (saved === "light" || saved === "dark") root.dataset.theme = saved;
  } catch {
    /* storage unavailable: follow the OS setting */
  }
  function current() {
    if (root.dataset.theme) return root.dataset.theme;
    return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  window.addEventListener("DOMContentLoaded", () => {
    for (const b of document.querySelectorAll("[data-theme-toggle]")) {
      b.addEventListener("click", () => {
        const next = current() === "dark" ? "light" : "dark";
        root.dataset.theme = next;
        try {
          localStorage.setItem("flowlab-theme", next);
        } catch {
          /* ignore */
        }
        window.dispatchEvent(new CustomEvent("themechange"));
      });
    }
  });
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () =>
    window.dispatchEvent(new CustomEvent("themechange")),
  );
})();
