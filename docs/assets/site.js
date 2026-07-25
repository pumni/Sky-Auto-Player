(() => {
  "use strict";

  document.documentElement.classList.add("js");

  const menuToggle = document.querySelector(".menu-toggle");
  const siteNav = document.querySelector(".site-nav");

  if (!menuToggle || !siteNav) {
    return;
  }

  const closeMenu = ({ returnFocus = false } = {}) => {
    siteNav.classList.remove("is-open");
    menuToggle.setAttribute("aria-expanded", "false");
    menuToggle.setAttribute("aria-label", menuToggle.dataset.closeLabel || "Open navigation");
    if (returnFocus) {
      menuToggle.focus();
    }
  };

  const openMenu = () => {
    siteNav.classList.add("is-open");
    menuToggle.setAttribute("aria-expanded", "true");
    menuToggle.setAttribute("aria-label", menuToggle.dataset.openLabel || "Close navigation");
  };

  menuToggle.addEventListener("click", () => {
    if (siteNav.classList.contains("is-open")) {
      closeMenu({ returnFocus: true });
    } else {
      openMenu();
    }
  });

  siteNav.addEventListener("click", (event) => {
    if (event.target instanceof Element && event.target.closest("a")) {
      closeMenu();
    }
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && siteNav.classList.contains("is-open")) {
      closeMenu({ returnFocus: true });
    }
  });

  window.addEventListener("resize", () => {
    if (window.matchMedia("(min-width: 56rem)").matches) {
      closeMenu();
    }
  });
})();
