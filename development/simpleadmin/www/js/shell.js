// Page chrome: phone tab bar and "more" sheet, appearance and language pickers, dialogs.
(function (global) {
  'use strict';
  const root = global.SimpleAdmin;
  const MORE_PAGES = ['deviceinfo', 'console'];

  function init() {
    const lang = root.Lang;
    const languageSelects = document.querySelectorAll('[data-language-select]');
    const themeSelects = document.querySelectorAll('[data-theme-select]');
    const moreSheet = document.getElementById('moreSheet');

    const syncLanguage = () => {
      const current = lang.getCurrentLanguage();
      languageSelects.forEach((select) => { select.value = current; });
      Object.values(root.Vue.apps || {}).forEach((app) => {
        if ('language' in app) app.language = current;
      });
    };
    languageSelects.forEach((select) => {
      select.addEventListener('change', () => lang.setLanguage(select.value));
    });
    global.addEventListener('simpleadmin:language-changed', syncLanguage);
    syncLanguage();

    const syncTheme = () => {
      const preference = root.Theme.preference();
      themeSelects.forEach((select) => { select.value = preference; });
    };
    themeSelects.forEach((select) => {
      select.addEventListener('change', () => { root.Theme.set(select.value); syncTheme(); });
    });
    syncTheme();

    // Tab bar and sheet rows switch pages through the router.
    document.querySelectorAll('[data-tab]').forEach((button) => {
      button.addEventListener('click', () => {
        const page = button.getAttribute('data-tab');
        if (page === 'more') {
          moreSheet.showModal();
          return;
        }
        if (moreSheet.open) moreSheet.close();
        root.Spa.showPage(page);
      });
    });
    const syncTabs = (event) => {
      const page = (event && event.detail && event.detail.page) || 'dashboard';
      document.querySelectorAll('.sa-tabbar [data-tab]').forEach((button) => {
        const tab = button.getAttribute('data-tab');
        const active = tab === page || (tab === 'more' && MORE_PAGES.includes(page));
        button.classList.toggle('active', active);
        if (active) button.setAttribute('aria-current', 'page');
        else button.removeAttribute('aria-current');
      });
    };
    global.addEventListener('simpleadmin:page-changed', syncTabs);
    syncTabs({ detail: { page: (document.querySelector('.sa-page.active') || {}).dataset?.page } });

    // Generic dialogs: [data-open-dialog="id"] opens, [data-close] inside closes, backdrop click closes.
    document.querySelectorAll('[data-open-dialog]').forEach((button) => {
      button.addEventListener('click', () => {
        const dialog = document.getElementById(button.getAttribute('data-open-dialog'));
        if (dialog) dialog.showModal();
      });
    });
    document.querySelectorAll('dialog.ui-sheet').forEach((dialog) => {
      dialog.addEventListener('click', (event) => {
        if (event.target === dialog || event.target.closest('[data-close]')) dialog.close();
      });
    });

    // Icon-only sidebar on narrow windows: keep the names as tooltips.
    document.querySelectorAll('.sa-menu-link').forEach((link) => { link.title = link.textContent.trim(); });
    global.addEventListener('simpleadmin:language-changed', () => {
      document.querySelectorAll('.sa-menu-link').forEach((link) => { link.title = link.textContent.trim(); });
    });

    if (global.SimpleAdminIcons) global.SimpleAdminIcons();
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init, { once: true });
  else init();
})(window);
