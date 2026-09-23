<script>
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const subjects = () =>
    Array.from(document.querySelectorAll(".row .row-sub")).map((e) => e.textContent);

  const type = (box, text) => {
    // React-style controlled input: set through the native setter so the framework's own
    // listener sees a real change, then dispatch the event the component listens for.
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype, "value").set;
    setter.call(box, text);
    box.dispatchEvent(new Event("input", { bubbles: true }));
  };

  let tries = 0;
  const start = setInterval(() => {
    const box = document.querySelector("input.search");
    if (!box) {
      if (++tries > 60) { clearInterval(start); say({ stage: "no search box ever appeared" }); }
      return;
    }
    clearInterval(start);
    say({ stage: "mounted", subjects: subjects() });
    type(box, "label:travel");
    setTimeout(() => {
      say({ stage: "after label:travel", typed: box.value, subjects: subjects() });
      type(box, "label:nosuchlabel");
      setTimeout(() => {
        say({ stage: "after label:nosuchlabel", subjects: subjects() });
        type(box, "");
        setTimeout(() => say({ stage: "after clearing", subjects: subjects() }), 600);
      }, 600);
    }, 600);
  }, 100);
})();
</script>
