<script>
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const syncButton = () =>
    Array.from(document.querySelectorAll(".place")).find((b) =>
      /sync/i.test(b.className) || /sync/i.test(b.textContent));

  let tries = 0;
  const start = setInterval(() => {
    const b = syncButton();
    if (!b) { if (++tries > 60) { clearInterval(start); say({ stage: "no sync button" }); } return; }
    clearInterval(start);
    say({ stage: "before", label: b.textContent, disabled: b.disabled });
    b.click();
    let n = 0;
    const watch = setInterval(() => {
      const now = syncButton();
      say({ stage: "after click", at: ++n, label: now ? now.textContent : "(gone)" });
      if (n >= 6) clearInterval(watch);
    }, 1500);
  }, 100);
})();
</script>
