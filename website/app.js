// This page contains screenshots only; it never opens or reads a vault.
(() => {
  "use strict";
  const root = document.documentElement;
  const themeButton = document.querySelector(".theme-toggle");
  const themeMeta = document.querySelector('meta[name="theme-color"]');
  const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
  let savedTheme = null;
  try { savedTheme = localStorage.getItem("secback-website-theme"); } catch { /* Storage may be unavailable for file URLs. */ }
  function applyTheme(theme) {
    root.dataset.theme = theme;
    const dark = theme === "dark";
    themeButton.setAttribute("aria-label", dark ? "切换为浅色主题" : "切换为深色主题");
    themeButton.querySelector("use").setAttribute("href", dark ? "#sun" : "#moon");
    themeMeta.content = dark ? "#101a17" : "#f5f8f5";
  }
  applyTheme(savedTheme === "dark" || savedTheme === "light" ? savedTheme : systemTheme.matches ? "dark" : "light");
  themeButton.addEventListener("click", () => {
    savedTheme = root.dataset.theme === "dark" ? "light" : "dark";
    applyTheme(savedTheme);
    try { localStorage.setItem("secback-website-theme", savedTheme); } catch { /* Theme remains usable without persistence. */ }
  });
  systemTheme.addEventListener("change", event => { if (!savedTheme) applyTheme(event.matches ? "dark" : "light"); });

  const menu = document.querySelector(".menu-toggle");
  const navigation = document.getElementById("navigation");
  const mobileLayout = window.matchMedia("(max-width: 800px)");
  function closeMenu(returnFocus = false) {
    navigation.classList.remove("is-open");
    menu.setAttribute("aria-expanded", "false");
    menu.setAttribute("aria-label", "打开导航");
    if (returnFocus) menu.focus();
  }
  menu.addEventListener("click", () => {
    const open = menu.getAttribute("aria-expanded") !== "true";
    navigation.classList.toggle("is-open", open);
    menu.setAttribute("aria-expanded", String(open));
    menu.setAttribute("aria-label", open ? "关闭导航" : "打开导航");
  });
  navigation.addEventListener("click", event => { if (event.target.closest("a")) closeMenu(); });
  document.addEventListener("click", event => { if (!event.target.closest(".nav-wrap")) closeMenu(); });
  document.addEventListener("keydown", event => { if (event.key === "Escape" && menu.getAttribute("aria-expanded") === "true") closeMenu(true); });
  document.addEventListener("focusin", event => { if (!event.target.closest(".nav-wrap")) closeMenu(); });
  mobileLayout.addEventListener("change", () => closeMenu());

  const tabs = Array.from(document.querySelectorAll('[role="tab"]'));
  const previewTheme = document.querySelector(".preview-theme");
  function selectTab(tab, focus = false) {
    tabs.forEach(item => {
      const selected = item === tab;
      item.setAttribute("aria-selected", String(selected));
      item.tabIndex = selected ? 0 : -1;
      document.getElementById(item.getAttribute("aria-controls")).hidden = !selected;
    });
    previewTheme.hidden = tab.id !== "desktop-tab";
    if (focus) tab.focus();
  }
  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => selectTab(tab));
    tab.addEventListener("keydown", event => {
      let next;
      if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
      else if (event.key === "ArrowLeft") next = (index + tabs.length - 1) % tabs.length;
      else if (event.key === "Home") next = 0;
      else if (event.key === "End") next = tabs.length - 1;
      else return;
      event.preventDefault();
      selectTab(tabs[next], true);
    });
  });
  const image = document.getElementById("desktop-image");
  const swatches = Array.from(document.querySelectorAll("[data-preview-theme]"));
  swatches.forEach(button => button.addEventListener("click", () => {
    const theme = button.dataset.previewTheme;
    image.src = `assets/windows-${theme}.png`;
    image.alt = `SecBack ${theme === "dark" ? "深色" : "浅色"}桌面界面：左侧项目列表，右侧分组展示账户，底部固定编辑与删除操作`;
    swatches.forEach(item => item.setAttribute("aria-pressed", String(item === button)));
  }));
})();
