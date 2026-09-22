<script>
// Do frame delivery and re-rendering stall together?
//
// FINDINGS F140 says the VirtualDom stops being polled after the first render, and blames the
// tao waker discarding its `UserWindowEvent::Poll`. Reading dioxus-desktop 0.7.10 offers a
// second candidate: `WebviewInstance::poll_vdom` returns at its first line while
// `poll_edits_flushed` is pending, and that flag clears only when the page acknowledges the
// last batch of edits -- which `dioxus-interpreter-js`'s `rafEdits` does from inside a
// `requestAnimationFrame` callback. A surface the compositor never presents gets no frame
// callbacks, so on that reading the acknowledgement never arrives and the dom is never polled
// again.
//
// The two explanations make different predictions, and only a run that measures BOTH signals
// can tell them apart -- which is why this probe types as well as counts. Measuring frames
// alone proves nothing: on a machine where the window happens to work, frames tick and the
// list updates, and every hypothesis survives.
//
//   frames stall AND the list freezes, together, run after run  -> the rAF path is the gate
//   frames keep ticking WHILE the list freezes                  -> the waker is, as F140 says
//   both healthy                                                -> F140 does not reproduce
//                                                                  here, and the environment
//                                                                  is the variable
//
// The third outcome is the one to be careful with: it is not evidence for either mechanism,
// only evidence that this session is not the failing session.
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const subjects = () =>
    Array.from(document.querySelectorAll(".row .subject")).map((e) => e.textContent);

  let ticks = 0;   // setInterval, which even a throttled surface still gets
  let frames = 0;  // requestAnimationFrame, which it does not
  const began = performance.now();

  // Chained, not a single callback: rAF fires once per callback, so re-arming is the only way
  // to learn whether frames keep coming or stop once the window is first presented.
  const arm = () => requestAnimationFrame(() => { frames += 1; arm(); });
  arm();
  setInterval(() => { ticks += 1; }, 250);

  const look = (stage, extra) => say(Object.assign({
    stage,
    at_ms: Math.round(performance.now() - began),
    interval_ticks: ticks,
    raf_ticks: frames,
    visibility: document.visibilityState,
    has_focus: document.hasFocus(),
    subjects: subjects(),
  }, extra || {}));

  const type = (box, text) => {
    // Controlled input: set through the native setter so the framework's own listener sees a
    // real change, then dispatch the event the component listens for.
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype, "value").set;
    setter.call(box, text);
    box.dispatchEvent(new Event("input", { bubbles: true }));
  };

  let tries = 0;
  const start = setInterval(() => {
    const box = document.querySelector(".search");
    if (!box) {
      if (++tries > 60) { clearInterval(start); look("no search box ever appeared"); }
      return;
    }
    clearInterval(start);
    look("mounted");
    type(box, "label:travel");
    setTimeout(() => {
      look("after label:travel", { typed: box.value });
      type(box, "label:nosuchlabel");
      setTimeout(() => {
        look("after label:nosuchlabel");
        type(box, "");
        setTimeout(() => look("after clearing"), 900);
      }, 900);
    }, 900);
  }, 100);
})();
</script>
