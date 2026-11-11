// Applies the saved theme before first paint so there is no flash.
(function () {
  try {
    var t = localStorage.getItem("ripple.theme") || "system";
    var dark = t === "dark" || (t === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.dataset.theme = dark ? "dark" : "light";
    document.documentElement.dataset.accent = localStorage.getItem("ripple.accent") || "tidal";
  } catch (e) {
    document.documentElement.dataset.theme = "dark";
  }
})();
