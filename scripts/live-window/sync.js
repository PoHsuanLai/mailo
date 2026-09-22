<script>
// Does pressing Sync do anything? The button says "Sync" or "Syncing…", so the label is the
// state. Sampled fast and often: a pass against a store with no credentials fails in well under
// a second, and a probe that looks once a second can miss the whole thing and report silence.
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const button = () =>
    Array.from(document.querySelectorAll(".place")).find((b) => /sync/.test(b.className));

  let tries = 0;
  const start = setInterval(() => {
    const b = button();
    if (!b) { if (++tries > 60) { clearInterval(start); say({ stage: "no sync button" }); } return; }
    clearInterval(start);

    const seen = [];
    const sample = () => {
      const now = button();
      const label = now ? now.textContent : "(gone)";
      if (seen[seen.length - 1] !== label) seen.push(label);
    };
    sample();
    b.click();
    let n = 0;
    const watch = setInterval(() => {
      sample();
      n += 1;
      if (n === 10 || n === 30 || n >= 50) {
        // Every distinct label in order. "Sync" alone means the click changed nothing.
        say({ stage: `labels after ${n} samples`, seen: seen });
        if (n >= 50) clearInterval(watch);
      }
    }, 100);
  }, 100);
})();
</script>
