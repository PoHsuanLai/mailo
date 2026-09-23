<script>
// Does the palette reach the real window?
//
// Every other check of the stylesheet is static: the style tests parse `STYLE`, and `dump()`
// renders the components to a file that headless Chrome photographs. None of that is WebKitGTK,
// and none of it is the head script that stamps the Space's `data-theme`, `data-motion` and
// frame tokens onto <html> at launch. This asks the running window what it actually resolved.
//
// Read-only, and it needs nothing to re-render: the first frame is what carries the appearance,
// so this is checkable whatever F140/F141 turn out to be about.
//
// What a healthy report looks like, with nothing persisted yet:
//   data_theme absent (the first Space follows the desktop), data_motion "standard",
//   data_accent absent (the six accent hues are retired; the card's accent is the Space's),
//   --paper, --accent and --f-grad resolved to real values rather than "",
//   body_background the computed --paper, not the WebKit default of transparent / white.
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});

  const root = document.documentElement;
  const token = (name) => getComputedStyle(root).getPropertyValue(name).trim();
  // A probe that throws reports nothing, which reads exactly like a window that never ran it.
  // So a failure is itself a report.
  const look = (stage) => { try { lookUnguarded(stage); } catch (e) { say({ stage, error: String(e) }); } };
  const lookUnguarded = (stage) => {
    const app = document.querySelector(".app");
    const current = document.querySelector('.item[aria-current="true"]');
    say({
      stage,
      data_theme: root.dataset.theme === undefined ? "(absent)" : root.dataset.theme,
      data_motion: root.dataset.motion === undefined ? "(absent)" : root.dataset.motion,
      data_accent: root.dataset.accent === undefined ? "(absent)" : root.dataset.accent,
      prefers_dark: window.matchMedia("(prefers-color-scheme: dark)").matches,
      paper: token("--paper"),
      ink: token("--ink"),
      accent: token("--accent"),
      f_grad: token("--f-grad"),
      frame: token("--frame"),
      // The first look runs from the custom head, before <body> exists.
      body_background: document.body ? getComputedStyle(document.body).backgroundColor : "(no body yet)",
      app_found: Boolean(app),
      // The seal on the current place is an inset box-shadow in --seal; if the tokens are
      // live, its colour is in the computed shadow.
      current_place_shadow: current ? getComputedStyle(current).boxShadow : "(no .place.on)",
    });
  };

  // Once immediately, which is what the head script alone decided, and once after mount.
  look("head");
  let tries = 0;
  const wait = setInterval(() => {
    if (document.querySelector(".app") || ++tries > 60) {
      clearInterval(wait);
      look("mounted");
    }
  }, 100);
})();
</script>
