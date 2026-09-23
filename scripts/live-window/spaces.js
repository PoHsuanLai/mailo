<script>
// Does Ctrl 2 switch Space in the real window, and does the frame cross-fade?
//
// The tests read the script a switch evals; only WebKit can say whether it ran. This asks the
// running window for the computed `--f-grad` before and after Ctrl 2, and for the two layers'
// opacity partway through the 380ms fade, when both should be between 0 and 1.
//
// The seeded store has one account, so the first run makes one Space. The probe makes a second
// the way a person would: "+" in the foot, which switches to it and opens the editor; Esc
// closes the editor and keeps the Space; Ctrl 1 goes back. Then it measures Ctrl 2.
//
// Healthy:
//   "before"  -> space "Space 1", f_grad the first Space's gradient
//   "mid"     -> both layers' opacity strictly between 0 and 1, one rising, one falling
//   "after"   -> space "Space 2", f_grad a different gradient, front layer opacity 1
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const root = document.documentElement;
  const grad = () => getComputedStyle(root).getPropertyValue("--f-grad").trim();
  const layers = () => Array.from(document.querySelectorAll(".app .layer"))
    .map((layer) => Number(getComputedStyle(layer).opacity));
  const current = () => {
    const on = document.querySelector('.sp[aria-pressed="true"]');
    return on ? on.getAttribute("aria-label") : "(no pressed dot)";
  };
  const press = (key, ctrl) => {
    const app = document.querySelector(".app");
    if (!app) { return; }
    app.focus();
    app.dispatchEvent(new KeyboardEvent("keydown", { key, ctrlKey: ctrl, bubbles: true }));
  };
  const look = (stage, extra) => {
    try {
      say(Object.assign({
        stage,
        space: current(),
        dots: document.querySelectorAll(".sp").length,
        f_grad: grad(),
        layer_opacity: layers(),
        data_theme: root.dataset.theme === undefined ? "(absent)" : root.dataset.theme,
        data_accent: root.dataset.accent === undefined ? "(absent)" : root.dataset.accent,
        editor_open: Boolean(document.querySelector(".editor")),
      }, extra || {}));
    } catch (e) {
      say({ stage, error: String(e) });
    }
  };

  let tries = 0;
  const wait = setInterval(() => {
    if (!document.querySelector(".side-foot") && ++tries < 60) { return; }
    clearInterval(wait);
    look("mounted");
    const plus = document.querySelector('.foot-btn[aria-label="New Space"]');
    if (!plus) { say({ stage: "no New Space button" }); return; }
    plus.click();
    setTimeout(() => {
      look("after +");
      press("Escape", false);
      setTimeout(() => {
        press("1", true);
        setTimeout(() => {
          look("before");
          press("2", true);
          setTimeout(() => look("mid"), 150);
          setTimeout(() => look("after"), 900);
        }, 900);
      }, 500);
    }, 900);
  }, 100);
})();
</script>
