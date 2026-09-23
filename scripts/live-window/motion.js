<script>
// Hover, archive, undo, in the real window.
//
// The component tests prove the tree agrees with itself; this asks WebKit. It rests the
// pointer on a row for 500 ms and reports whether a card opened, archives a row and reports the
// animations WebKit is running on it, then pulls the toast's tab and reports the row count.
//
// Healthy:
//   "hover"   -> card true (a `.hc` is on the page), card_text starts with the row's subject
//   "archive" -> going true, animations ["fold"], store already moved (rows_after_requery
//                is one fewer once the row has gone)
//   "undo"    -> rows equal to "before", toast gone
(function () {
  const say = (what) =>
    fetch("http://127.0.0.1:18099/report", { method: "POST", body: JSON.stringify(what) })
      .catch(() => {});
  const rows = () => document.querySelectorAll(".list .row").length;
  const point = (el, type, bubbles) => {
    const box = el.getBoundingClientRect();
    el.dispatchEvent(new PointerEvent(type, {
      bubbles, clientX: box.left + 20, clientY: box.top + 12, pointerType: "mouse",
    }));
  };
  const names = (el) => (el && el.getAnimations ? el.getAnimations() : [])
    .map((a) => a.animationName || (a.effect && a.effect.getKeyframes && "unnamed") || "?");

  let tries = 0;
  const wait = setInterval(() => {
    const first = document.querySelector(".list .row[data-hc]");
    if (!first) {
      if (++tries > 80) { clearInterval(wait); say({ stage: "no rows" }); }
      return;
    }
    clearInterval(wait);
    // Past the list's first-show rise, so the hover is the only thing moving.
    setTimeout(() => {
      const before = rows();
      const row = document.querySelector(".list .row[data-hc]");
      const subject = row.querySelector(".row-sub").textContent;
      point(row, "pointerover", true);
      point(row, "pointerenter", false);
      setTimeout(() => {
        const card = document.querySelector(".hc");
        say({
          stage: "hover",
          rested_ms: 500,
          card: Boolean(card),
          card_text: card ? card.textContent.slice(0, 80) : null,
          subject,
          unread_after_hover: row.getAttribute("data-read"),
        });
        point(row, "pointerleave", false);
        const archive = row.querySelector('.strip [data-op="archive"]');
        if (!archive) { say({ stage: "no archive button" }); return; }
        archive.click();
        setTimeout(() => {
          const going = document.querySelector(".list .row.going");
          say({
            stage: "archive",
            going: Boolean(going),
            data_op: going && going.getAttribute("data-op"),
            animations: names(going),
            toast: (document.querySelector(".toast") || {}).textContent || null,
          });
          setTimeout(() => {
            const after = rows();
            const tab = document.querySelector(".toast .tab");
            if (!tab) { say({ stage: "no toast tab", rows_after_exit: after }); return; }
            tab.click();
            setTimeout(() => {
              say({
                stage: "undo",
                rows_before: before,
                rows_after_exit: after,
                rows: rows(),
                toast: Boolean(document.querySelector(".toast")),
                returned: Boolean(document.querySelector(".list .row.returning")),
              });
            }, 700);
          }, 1200);
        }, 80);
      }, 500);
    }, 900);
  }, 100);
})();
</script>
