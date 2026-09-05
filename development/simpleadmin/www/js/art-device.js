(function () {
  'use strict';
  function init() {
    const languageSelect = document.getElementById('headerLanguage');
    const lang = window.SimpleAdmin.Lang;
    languageSelect.value = lang.getCurrentLanguage();
    languageSelect.addEventListener('change', () => lang.setLanguage(languageSelect.value));
    const syncLanguage = () => {
      languageSelect.value = lang.getCurrentLanguage();
      languageSelect.setAttribute('aria-label', lang.t('语言'));
      Object.values(window.SimpleAdmin.Vue.apps || {}).forEach(app => {
        if ('language' in app) app.language = lang.getCurrentLanguage();
      });
    };
    window.addEventListener('simpleadmin:language-changed', syncLanguage);
    syncLanguage();
    if (window.SimpleAdminIcons) window.SimpleAdminIcons();
    const sidebar = document.getElementById('simpleadminSidebar');
    const backdrop = document.getElementById('artSidebarBackdrop');
    document.querySelectorAll('.sa-menu-link').forEach(link => { link.title = link.textContent.trim(); });
    const closeMenu = () => { sidebar.classList.remove('open'); backdrop.hidden = true; };
    document.getElementById('artMenuButton').addEventListener('click', () => {
      if (innerWidth < 992) { sidebar.classList.toggle('open'); backdrop.hidden = !sidebar.classList.contains('open'); }
      else document.body.classList.toggle('art-collapsed');
    });
    backdrop.addEventListener('click', closeMenu);
    document.addEventListener('keydown', event => { if (event.key === 'Escape') closeMenu(); });
    document.getElementById('artRefreshButton').addEventListener('click', () => location.reload());
    document.getElementById('artThemeButton').addEventListener('click', () => {
      const toggle = document.querySelector('.sa-page.active .sa-titlebar-theme-toggle');
      if (toggle) toggle.click();
    });
    document.getElementById('artFullscreenButton').addEventListener('click', async () => {
      try { if (document.fullscreenElement) await document.exitFullscreen(); else await document.documentElement.requestFullscreen(); } catch (_) {}
    });
    const pageName = () => {
      closeMenu();
      const title = document.querySelector('.sa-page.active h1');
      document.getElementById('artCurrentPage').textContent = title ? title.textContent : '总览';
    };
    window.addEventListener('simpleadmin:page-changed', pageName);
    pageName();
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init, { once: true }); else init();
})();
