(function () {
  'use strict';

  function initDarkMode() {
    if (window.SimpleAdmin && window.SimpleAdmin.UI) {
      window.SimpleAdmin.UI.initDarkMode('darkModeToggle');
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initDarkMode);
  } else {
    initDarkMode();
  }

  // Most pages mount the whole body area through Vue. If dark-mode.js runs
  // before the page app mounts, the original button is replaced and its click
  // listener is lost. Re-bind after every page Vue mount.
  window.addEventListener('simpleadmin:vue-mounted', initDarkMode);
})();
