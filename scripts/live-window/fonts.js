<script>
// Do the embedded faces load in the real window?
//
// The style tests prove each @font-face is a well-formed WOFF2 data URI. They cannot prove
// WebKitGTK decodes it: a face that fails to load falls back to the next name in the stack
// with no error anywhere, and the window just looks slightly wrong. This asks the engine.
//
// Healthy: every face "loaded", and each check true.
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const wanted = [
    "700 16px 'Bricolage Grotesque'",
    "400 14px Karla",
    "italic 400 14px Karla",
    "400 11px 'Space Mono'",
    "700 11px 'Space Mono'",
  ];
  let tries = 0;
  const wait = setInterval(() => {
    if (!document.querySelector(".app") && ++tries < 60) return;
    clearInterval(wait);
    Promise.all(wanted.map((f) => document.fonts.load(f, "Inbox 09:41").then((got) => [f, got.length])))
      .then((loaded) => {
        const faces = [];
        document.fonts.forEach((face) => faces.push(face.family + " " + face.style + " " + face.weight + " " + face.status));
        say({
          loaded,
          checks: wanted.map((f) => [f, document.fonts.check(f, "Inbox 09:41")]),
          body_font: getComputedStyle(document.body).fontFamily,
          faces,
        });
      })
      .catch((e) => say({ error: String(e) }));
  }, 100);
})();
</script>
