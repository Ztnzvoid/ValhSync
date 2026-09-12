// ValhSync documents: the small amount of behaviour a reference page earns.
//
// Two things, both of which a reader notices only by their absence: knowing
// where they are in a long page, and getting a command out of it without
// selecting it by hand. Everything here degrades to nothing without script --
// the pages are readable, linkable and printable on their own.

(() => {
  "use strict";

  /** Mark the section currently being read in the margin contents. */
  function followAlong() {
    const links = new Map();
    for (const a of document.querySelectorAll("nav.toc a[href^='#']")) {
      const target = document.getElementById(a.hash.slice(1));
      if (target) links.set(target, a);
    }
    if (links.size === 0) return;

    let current = null;
    const mark = (section) => {
      if (section === current) return;
      current = section;
      for (const [, a] of links) a.classList.remove("here");
      const a = links.get(section);
      if (a) a.classList.add("here");
    };

    // The topmost section whose start is above the reading line. Computed
    // from the sections themselves rather than from scroll offsets, so it
    // stays right when the window is resized or a table wraps.
    const pick = () => {
      const line = window.innerHeight * 0.3;
      let best = null;
      for (const [section] of links) {
        if (section.getBoundingClientRect().top <= line) best = section;
      }
      mark(best ?? links.keys().next().value);
    };

    let queued = false;
    const onScroll = () => {
      if (queued) return;
      queued = true;
      requestAnimationFrame(() => {
        queued = false;
        pick();
      });
    };
    addEventListener("scroll", onScroll, { passive: true });
    addEventListener("resize", onScroll, { passive: true });
    pick();
  }

  /** A copy button on every code block. */
  function copyable() {
    const label = document.documentElement.lang === "fr" ? "Copier" : "Copy";
    const done = document.documentElement.lang === "fr" ? "Copié" : "Copied";

    for (const pre of document.querySelectorAll("pre")) {
      const wrap = document.createElement("div");
      wrap.className = "snip";
      pre.parentNode.insertBefore(wrap, pre);
      wrap.appendChild(pre);

      const button = document.createElement("button");
      button.type = "button";
      button.textContent = label;
      // The block is already in the page; the button is an extra way to
      // reach it, not a label for it.
      button.setAttribute("aria-label", `${label}: ${pre.textContent.trim().split("\n")[0]}`);
      wrap.appendChild(button);

      button.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText(pre.textContent.replace(/\s+$/, ""));
          button.textContent = done;
          button.classList.add("done");
          setTimeout(() => {
            button.textContent = label;
            button.classList.remove("done");
          }, 1400);
        } catch {
          // No clipboard permission, or an insecure origin. Select it
          // instead, so the keyboard can finish the job.
          const range = document.createRange();
          range.selectNodeContents(pre);
          const selection = getSelection();
          selection.removeAllRanges();
          selection.addRange(range);
        }
      });
    }
  }

  if (document.readyState === "loading") {
    addEventListener("DOMContentLoaded", () => {
      followAlong();
      copyable();
    });
  } else {
    followAlong();
    copyable();
  }
})();
