<script>
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype, "value").set;

  let tries = 0;
  const start = setInterval(() => {
    const app = document.querySelector(".app");
    if (!app) {
      if (++tries > 60) { clearInterval(start); say({ stage: "no app" }); }
      return;
    }
    clearInterval(start);
    app.focus();
    app.dispatchEvent(new KeyboardEvent("keydown", {
      key: "t", ctrlKey: true, bubbles: true,
    }));
    setTimeout(() => {
      const box = document.querySelector(".cmdk .inp");
      if (!box) {
        say({
          stage: "no command field",
          active: document.activeElement && document.activeElement.className,
        });
        return;
      }
      box.focus();
      setter.call(box, "dana");
      box.dispatchEvent(new Event("input", { bubbles: true }));
      // Two KEEP_FOCUS ticks. The field has to still be the active element after them.
      setTimeout(() => {
        const active = document.activeElement;
        say({
          stage: "after dana",
          activeTag: active && active.tagName,
          activeClass: active && active.className,
          activeInCmdk: !!(active && active.closest && active.closest(".cmdk")),
          items: document.querySelectorAll(".cmdk .it").length,
        });
      }, 600);
    }, 400);
  }, 100);
})();
</script>
