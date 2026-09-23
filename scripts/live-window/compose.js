<script>
// Does the composer's editor work in the real window?
//
// The component tests replay the glue's messages into Rust. They cannot prove WebKitGTK sends
// them: that beforeinput is cancelable here, that the hidden wire reaches Rust, that the caret
// comes back, or that KEEP_FOCUS leaves the editor alone. This opens a new message, types into
// it through the same events a keyboard makes, and reports what the document became.
//
// Healthy: "typed" shows hello and a bold world; "composed" gains the committed text once;
// "focus" names the editor after KEEP_FOCUS has had time to run twice.
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const wait = (ms) => new Promise((done) => setTimeout(done, ms));
  const body = () => document.querySelector(".cpage .c-body");
  const doc = () => (body() ? body().dataset.doc : null);
  const type = async (b, text) => {
    for (const ch of text) {
      const event = new InputEvent("beforeinput", {
        inputType: "insertText", data: ch, bubbles: true, cancelable: true,
      });
      const went = b.dispatchEvent(event);
      if (went) { say({ stage: "not cancelled", ch }); }
      await wait(40);
    }
  };

  let tries = 0;
  const start = setInterval(async () => {
    const app = document.querySelector(".app");
    if (!app) {
      if (++tries > 60) { clearInterval(start); say({ stage: "no app" }); }
      return;
    }
    clearInterval(start);
    app.focus();
    app.dispatchEvent(new KeyboardEvent("keydown", { key: "c", bubbles: true }));
    await wait(600);
    const b = body();
    if (!b) { say({ stage: "no composer page", html: document.querySelector(".reader")?.className }); return; }
    b.focus();
    window.mailoCaret.set(b, 0, 0);
    await wait(100);
    await type(b, "hello **world**");
    await wait(300);
    say({ stage: "typed", doc: doc(), caret: b.dataset.caret, seq: b.dataset.seq, selection: window.mailoCaret.get(b) });

    // Pinyin, as an IME would deliver it. The browser writes the preedit itself; Rust must wait.
    const synthesized = typeof CompositionEvent === "function";
    if (synthesized) {
      b.dispatchEvent(new CompositionEvent("compositionstart", { data: "", bubbles: true }));
      b.dispatchEvent(new InputEvent("beforeinput", {
        inputType: "insertCompositionText", data: "ni", isComposing: true, bubbles: true, cancelable: true,
      }));
      await wait(80);
      const during = doc();
      b.dispatchEvent(new CompositionEvent("compositionend", { data: "\u4f60\u597d", bubbles: true }));
      await wait(300);
      say({ stage: "composed", during, doc: doc(), caret: b.dataset.caret });
    } else {
      say({ stage: "composed", error: "CompositionEvent cannot be constructed here" });
    }

    await wait(600);
    const here = document.activeElement;
    say({
      stage: "focus",
      activeTag: here && here.tagName,
      activeClass: here && here.className,
      inEditor: !!(here && here.closest && here.closest(".c-body")),
    });
  }, 100);
})();
</script>
