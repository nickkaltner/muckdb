(() => {
  "use strict";
  // Keep every original gallery item and its arrow-key navigation.
  const lightbox = document.getElementById("lightbox");
  const image = document.getElementById("lightbox-image");
  const caption = document.getElementById("lightbox-caption");
  const shots = [...document.querySelectorAll(".shot")];
  const closeButton = lightbox.querySelector(".lightbox-close");
  let current = 0;
  let opener;
  const close = () => {
    lightbox.classList.remove("open");
    document.body.classList.remove("has-lightbox");
    document
      .querySelectorAll("body > :not(.lightbox):not(script)")
      .forEach((el) => (el.inert = false));
    opener?.focus({ preventScroll: true });
  };
  const show = (index) => {
    current = (index + shots.length) % shots.length;
    const button = shots[current];
    image.src = button.dataset.shot;
    image.alt = button.querySelector("img").alt;
    caption.textContent = button.dataset.caption;
    lightbox.classList.add("open");
    document.body.classList.add("has-lightbox");
    document
      .querySelectorAll("body > :not(.lightbox):not(script)")
      .forEach((el) => (el.inert = true));
  };
  shots.forEach((button, index) =>
    button.addEventListener("click", () => {
      opener = button;
      show(index);
      closeButton.focus();
    }),
  );
  lightbox.addEventListener("click", (event) => {
    if (event.target === lightbox) close();
  });
  closeButton.addEventListener("click", close);
  lightbox
    .querySelector(".lightbox-prev")
    .addEventListener("click", () => show(current - 1));
  lightbox
    .querySelector(".lightbox-next")
    .addEventListener("click", () => show(current + 1));
  document.addEventListener("keydown", (event) => {
    if (!lightbox.classList.contains("open")) return;
    if (event.key === "Escape") close();
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      show(current - 1);
    }
    if (event.key === "ArrowRight") {
      event.preventDefault();
      show(current + 1);
    }
    if (event.key === "Tab") {
      const buttons = [...lightbox.querySelectorAll("button")];
      const first = buttons[0],
        last = buttons[buttons.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
  });

  // One brief explanatory entrance; static content is the default, including without JS.
  const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
  const replay = document.querySelector(".replay-intro");
  let animations = [];
  function intro() {
    animations.forEach((animation) => animation.cancel());
    animations = [];
    if (reducedMotion.matches) return;
    const cues = [
      [".hero-copy", 0, 0],
      [".hero-workbench .terminal", 100, 0],
      [".hero-map", 360, -3],
      [".hero-timeline", 560, 3],
    ];
    for (const [selector, delay, rotation] of cues) {
      const element = document.querySelector(selector);
      animations.push(
        element.animate(
          [
            {
              opacity: 0,
              transform: `translateY(18px) rotate(${rotation}deg)`,
            },
            { opacity: 1, transform: `translateY(0) rotate(${rotation}deg)` },
          ],
          {
            duration: 800,
            delay,
            easing: "cubic-bezier(.23,1,.32,1)",
            fill: "backwards",
          },
        ),
      );
    }
    const underline = document.querySelector(".visible-word svg");
    animations.push(
      underline.animate(
        [
          { opacity: 0, transform: "scaleX(.25)", transformOrigin: "left" },
          { opacity: 1, transform: "scaleX(1)", transformOrigin: "left" },
        ],
        {
          duration: 650,
          delay: 650,
          easing: "cubic-bezier(.23,1,.32,1)",
          fill: "backwards",
        },
      ),
    );
  }
  replay.hidden = reducedMotion.matches;
  replay.addEventListener("click", intro);
  reducedMotion.addEventListener("change", () => {
    replay.hidden = reducedMotion.matches;
    if (reducedMotion.matches)
      animations.forEach((animation) => animation.cancel());
  });
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) animations.forEach((animation) => animation.finish());
  });
  // Let local typography settle before revealing the composition.
  document.fonts.ready.then(intro);
})();
