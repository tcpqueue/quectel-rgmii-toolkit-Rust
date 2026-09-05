(function (global) {
  'use strict';

  const root = global.SimpleAdmin || (global.SimpleAdmin = {});
  const pages = [
    { id: 'dashboard', selector: '#dashboardApp', factoryName: 'dashboard', title: '首页' },
    { id: 'network', selector: '#networkApp', factoryName: 'network', title: '网络' },
    { id: 'settings', selector: '#settingsApp', factoryName: 'settings', title: '设置' },
    { id: 'sms', selector: '#smsApp', factoryName: 'sms', title: '短信' },
    { id: 'deviceinfo', selector: '#deviceinfoApp', factoryName: 'deviceinfo', title: '设备信息' },
    { id: 'console', selector: null, factoryName: null, title: '控制台', frameSelector: '#consoleFrame', frameSrc: '/console' }
  ];
  const mountedPages = new Set();
  const revealTimers = new WeakMap();
  const pageIds = new Set(pages.map((page) => page.id));

  function disableBrowserScrollRestore() {
    try {
      if ('scrollRestoration' in global.history) {
        global.history.scrollRestoration = 'manual';
      }
    } catch (_) {
      // Ignore browsers that block history access.
    }
  }

  function setElementScrollTop(element) {
    if (!element) return;
    try {
      element.scrollTop = 0;
      element.scrollLeft = 0;
    } catch (_) {
      // Ignore read-only scroll containers.
    }
  }

  function collectScrollContainers() {
    const containers = [
      document.scrollingElement,
      document.documentElement,
      document.body
    ];
    [
      '.sa-shell',
      '.sa-sidebar',
      '.sa-main',
      '.sa-content',
      '.sa-page',
      '.sa-page .sa-page-app'
    ].forEach((selector) => {
      document.querySelectorAll(selector).forEach((element) => containers.push(element));
    });
    return Array.from(new Set(containers.filter(Boolean)));
  }

  function resetScrollPosition() {
    collectScrollContainers().forEach(setElementScrollTop);
  }

  function resetScrollPositionAfterBrowserRestore() {
    document.documentElement.classList.add('sa-no-scroll-motion');
    resetScrollPosition();
    global.setTimeout(() => {
      resetScrollPosition();
      document.documentElement.classList.remove('sa-no-scroll-motion');
    }, 0);
  }

  function beginPageSwitch() {
    document.documentElement.classList.add('sa-page-switching', 'sa-no-scroll-motion');
    resetScrollPosition();
  }

  function finishPageSwitchAtTop() {
    resetScrollPosition();
    document.documentElement.classList.remove('sa-page-switching');
    global.setTimeout(() => {
      resetScrollPosition();
      document.documentElement.classList.remove('sa-no-scroll-motion');
    }, 0);
  }

  function updateAddressHash(id, replace) {
    const hash = `#${id}`;
    if (global.location.hash === hash) return;
    if (global.history && typeof global.history[replace ? 'replaceState' : 'pushState'] === 'function') {
      try {
        global.history[replace ? 'replaceState' : 'pushState']({ simpleadminPage: id }, '', hash);
        return;
      } catch (_) {
        // Fall back to location hash on browsers that block history access.
      }
    }
    // Do not fall back to location.hash here: native hash navigation may restore or
    // animate scroll position. Keeping the current URL is safer than showing a scroll.
  }

  function normalizePage(value) {
    const id = String(value || '').replace(/^#/, '');
    return pageIds.has(id) ? id : 'dashboard';
  }

  function getPage(id) {
    return pages.find((page) => page.id === id) || pages[0];
  }

  function setActiveLink(id) {
    document.querySelectorAll('[data-page-link]').forEach((link) => {
      const active = link.getAttribute('data-page-link') === id;
      link.classList.toggle('active', active);
      if (active) link.setAttribute('aria-current', 'page');
      else link.removeAttribute('aria-current');
    });
  }

  function clearRevealRunning(section) {
    if (!section) return;
    const timer = revealTimers.get(section);
    if (timer) {
      global.clearTimeout(timer);
      revealTimers.delete(section);
    }
    section.classList.remove('sa-reveal-running');
  }

  function setActiveSection(id) {
    document.querySelectorAll('.sa-page').forEach((section) => {
      const active = section.getAttribute('data-page') === id;
      section.classList.toggle('active', active);
      if (!active) clearRevealRunning(section);
    });
  }

  function collectRevealItems(section) {
    const selectors = [
      ':scope > .sa-page-titlebar',
      ':scope .sa-page-app > .sa-section-stack > .sa-stack-item',
      ':scope .sa-page-app > .sa-panel',
      ':scope .sa-page-app .sa-band-mode-panel',
      ':scope .sa-page-app .sa-data-table tr',
      ':scope .sa-page-app .sa-sms-summary-row'
    ];
    return Array.from(new Set(section.querySelectorAll(selectors.join(','))));
  }

  function applyProgressiveReveal(id) {
    const section = document.querySelector(`.sa-page[data-page="${id}"]`);
    if (!section) return;
    clearRevealRunning(section);
    const items = collectRevealItems(section);
    if (items.length === 0) return;
    section.classList.add('sa-reveal-running');
    items.forEach((item, index) => {
      item.classList.remove('sa-reveal-visible');
      item.classList.add('sa-reveal-item');
      item.style.setProperty('--sa-reveal-index', String(index));
      item.style.setProperty('--sa-reveal-delay', `${Math.min(index, 12) * 42}ms`);
    });
    nextFrame(() => {
      items.forEach((item) => {
        item.classList.add('sa-reveal-visible');
      });
      const maxDelay = Math.min(Math.max(items.length - 1, 0), 12) * 42;
      const timer = global.setTimeout(() => {
        clearRevealRunning(section);
      }, maxDelay + 520);
      revealTimers.set(section, timer);
    });
  }

  function nextFrame(callback) {
    if (typeof global.requestAnimationFrame === 'function') {
      global.requestAnimationFrame(callback);
      return;
    }
    global.setTimeout(callback, 16);
  }

  function scheduleProgressiveReveal(id) {
    nextFrame(() => {
      nextFrame(() => applyProgressiveReveal(id));
    });
  }

  function mountPage(id) {
    if (mountedPages.has(id)) return;
    const page = getPage(id);
    if (page.frameSelector) {
      const frame = document.querySelector(page.frameSelector);
      if (frame && !frame.getAttribute('src')) {
        frame.setAttribute('src', frame.getAttribute('data-console-src') || page.frameSrc);
      }
      mountedPages.add(id);
      return;
    }
    const factory = root.Pages && root.Pages[page.factoryName];
    if (typeof factory !== 'function') {
      console.warn('SimpleAdmin page factory missing:', page.factoryName);
      return;
    }
    root.Vue.mount(factory, page.selector);
    mountedPages.add(id);
  }

  function applyTitle(id) {
    const page = getPage(id);
    if (root.Brand && typeof root.Brand.setPageTitle === 'function') {
      root.Brand.setPageTitle(page.title);
      return;
    }
    document.title = page.title;
  }

  function closeMobileSidebar() {
    const sidebar = document.getElementById('simpleadminSidebar');
    if (sidebar) sidebar.classList.remove('open');
  }

  function showPage(value, options) {
    const id = normalizePage(value);
    disableBrowserScrollRestore();
    beginPageSwitch();
    setActiveSection(id);
    setActiveLink(id);
    mountPage(id);
    applyTitle(id);
    closeMobileSidebar();
    if (!options || options.updateHash !== false) {
      updateAddressHash(id, options && options.replaceHash === true);
    }
    if (root.Lang && typeof root.Lang.apply === 'function') {
      root.Lang.apply(document);
    }
    finishPageSwitchAtTop();
    scheduleProgressiveReveal(id);
    if (typeof global.dispatchEvent === 'function') {
      global.dispatchEvent(new CustomEvent('simpleadmin:page-changed', {
        detail: { page: id }
      }));
    }
  }

  function bindNavigation() {
    document.querySelectorAll('[data-logout-link]').forEach((link) => {
      link.addEventListener('click', (event) => {
        event.preventDefault();
        if (root.Logout && typeof root.Logout.start === 'function') {
          root.Logout.start();
        } else {
          global.location.href = '/api/logout';
        }
      });
    });

    document.querySelectorAll('[data-page-link]').forEach((link) => {
      link.addEventListener('click', (event) => {
        event.preventDefault();
        showPage(link.getAttribute('data-page-link'));
      });
    });

    const sidebar = document.getElementById('simpleadminSidebar');
    if (sidebar) {
      document.querySelectorAll('.sa-sidebar-toggle').forEach((sidebarToggle) => {
        if (sidebarToggle.dataset.simpleadminSidebarBound === '1') return;
        sidebarToggle.dataset.simpleadminSidebarBound = '1';
        sidebarToggle.addEventListener('click', () => {
          sidebar.classList.toggle('open');
        });
      });
    }

    global.addEventListener('popstate', () => {
      showPage(global.location.hash, { updateHash: false });
    });

    global.addEventListener('hashchange', () => {
      showPage(global.location.hash, { updateHash: false });
    });
  }

  function init() {
    disableBrowserScrollRestore();
    document.documentElement.classList.add('sa-progressive-ready');
    bindNavigation();
    showPage(global.location.hash || 'dashboard', { updateHash: false });
  }

  global.addEventListener('load', resetScrollPositionAfterBrowserRestore);
  global.addEventListener('pageshow', resetScrollPositionAfterBrowserRestore);

  root.Spa = { showPage, mountPage, resetScrollPosition };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init, { once: true });
  } else {
    init();
  }
})(window);
