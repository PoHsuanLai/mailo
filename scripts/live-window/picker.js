<script>
// Does choosing a look in the real window change the window?
//
// The look is the Space's now, chosen in the Space editor. Three mechanisms have to work in a
// row, and a static render can check none of them: the click reaches its Rust handler; the
// handler's `document::eval` repaints <html> (`data-theme`, the frame tokens), so WebKit
// re-resolves the palette; and the component re-renders so the pressed button moves.
//
// Each stage reports the attributes, the resolved tokens, and which buttons claim to be pressed.
// Healthy:
//   editor   -> editor_open true, pressed theme "System"
//   pine     -> --f-grad changes to the Pine preset's, --accent follows it (a hint of the Space)
//   dark     -> data_theme "dark",  --paper the dark value, pressed theme "Dark"
//   escape   -> editor_open false, data_theme absent, --f-grad back to what it was at "editor"
// The window runs with XDG_CONFIG_HOME in a scratch directory (see live-window.sh), and Esc
// saves nothing anyway.
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const root = document.documentElement;
  const token = (name) => getComputedStyle(root).getPropertyValue(name).trim();
  const pressed = (sel) => Array.from(document.querySelectorAll(sel))
    .filter((b) => b.getAttribute("aria-pressed") === "true")
    .map((b) => b.textContent.trim());

  const look = (stage) => {
    try {
      say({
        stage,
        editor_open: Boolean(document.querySelector(".editor")),
        data_theme: root.dataset.theme === undefined ? "(absent)" : root.dataset.theme,
        accent: token("--accent"),
        paper: token("--paper"),
        f_grad: token("--f-grad"),
        pressed_theme: pressed('.seg[aria-label="Theme"] button'),
        presets: document.querySelectorAll(".presets button").length,
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
  const escape = () => {
    const app = document.querySelector(".app");
    if (app) {
      app.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    }
  };

  const steps = [
    () => { click(".space-name"); },
    () => look("editor"),
    () => { click('.presets button[aria-label="Pine"]'); },
    () => look("after pine"),
    () => { click('.seg[aria-label="Theme"] button[data-v="Dark"]'); },
    () => look("after dark"),
    () => escape(),
    () => look("after escape"),
  ];

  let tries = 0;
  const wait = setInterval(() => {
    if (!document.querySelector(".space-name") && ++tries < 60) return;
    clearInterval(wait);
    // Spaced out so each click's eval and re-render have landed before the next look.
    steps.forEach((step, i) => setTimeout(step, i * 700));
  }, 100);
})();
</script>
