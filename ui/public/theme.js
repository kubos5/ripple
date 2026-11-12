// Applies the saved theme before first paint so there is no flash.
(function () {
  try {
    var t = localStorage.getItem("ripple.theme") || "system";
    var dark = t === "dark" || (t === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.dataset.theme = dark ? "dark" : "light";
    var accent = localStorage.getItem("ripple.accent");
    document.documentElement.dataset.accent = !accent || accent === "tidal" ? "mono" : accent;
    document.documentElement.dataset.font = localStorage.getItem("ripple.font") || "system";
  } catch (e) {
    document.documentElement.dataset.theme = "dark";
  }
})();
