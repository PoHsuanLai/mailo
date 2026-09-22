<script>
// Does choosing a look in the real window change the window?
//
// Three different mechanisms have to work in a row, and a static render can check none of them:
// the click reaches its Rust handler; the handler's `document::eval` stamps `data-theme` /
// `data-accent` onto <html>, so WebKit re-resolves the palette; and the component re-renders so
// the pressed button moves. The last is exactly what F140 said did not happen, which makes this
// a second, independent reading of F141 as well as a check of the picker.
//
// Each stage reports the attributes, one resolved token, and which buttons claim to be pressed.
// Healthy:
//   pine     -> data_accent "pine",  --accent the pine value,  pressed hue "pine"
//   dark     -> data_theme "dark",   --paper the dark value,   pressed theme "Dark"
//   system   -> data_theme absent,   pressed theme "System"
// The window runs with XDG_CONFIG_HOME in a scratch directory (see live-window.sh), so the
// choices it saves do not touch the look of whoever runs this.
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const root = document.documentElement;
  const token = (name) => getComputedStyle(root).getPropertyValue(name).trim();
  const pressed = (sel, attr) => Array.from(document.querySelectorAll(sel))
    .filter((b) => b.getAttribute("aria-pressed") === "true")
    .map((b) => b.getAttribute(attr) || b.textContent.trim());

  const look = (stage) => {
    try {
      say({
        stage,
        data_theme: root.dataset.theme === undefined ? "(absent)" : root.dataset.theme,
        data_accent: root.dataset.accent === undefined ? "(absent)" : root.dataset.accent,
        accent: token("--accent"),
        paper: token("--paper"),
        pressed_hue: pressed(".swatch", "data-hue"),
        pressed_theme: pressed(".theme-choice button", null),
        swatches: document.querySelectorAll(".swatch").length,
      });
    } catch (e) {
      say({ stage, error: String(e) });
    }
  };

  const click = (sel) => {
    const el = document.querySelector(sel);
    if (!el) { say({ stage: "missing", selector: sel }); return false; }
    el.click();
    return true;
  };
  const themeButton = (label) =>
    Array.from(document.querySelectorAll(".theme-choice button"))
      .find((b) => b.textContent.trim() === label);

  const steps = [
    () => look("mounted"),
    () => { click('.swatch[data-hue="pine"]'); },
    () => look("after pine"),
    () => { const b = themeButton("Dark"); if (b) b.click(); else say({ stage: "no Dark button" }); },
    () => look("after dark"),
    () => { const b = themeButton("System"); if (b) b.click(); else say({ stage: "no System button" }); },
    () => look("after system"),
  ];

  let tries = 0;
  const wait = setInterval(() => {
    if (!document.querySelector(".appearance") && ++tries < 60) return;
    clearInterval(wait);
    // Spaced out so each click's eval and re-render have landed before the next look.
    steps.forEach((step, i) => setTimeout(step, i * 700));
  }, 100);
})();
</script>
