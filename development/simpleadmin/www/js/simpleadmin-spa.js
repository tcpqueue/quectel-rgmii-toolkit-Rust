(function (global) {
  'use strict';

  const root = global.SimpleAdmin || (global.SimpleAdmin = {});
  const pages = [
    { id: 'dashboard', selector: '#dashboardApp', factoryName: 'dashboard', title: '首页' },
    { id: 'events', selector: null, factoryName: null, title: '网络事件' },
    { id: 'network', selector: '#networkApp', factoryName: 'network', title: '网络' },
    { id: 'settings', selector: '#settingsApp', factoryName: 'settings', title: '设置' },
    { id: 'sms', selector: '#smsApp', factoryName: 'sms', title: '短信' },
    { id: 'notify', selector: '#forwardingApp', factoryName: 'forwarding', title: '消息通知' },
    { id: 'deviceinfo', selector: '#deviceinfoApp', factoryName: 'deviceinfo', title: '设备信息' },
    { id: 'console', selector: null, factoryName: null, title: '控制台', frameSelector: '#consoleFrame', frameSrc: '/console' }
  ];
  // Menu entries. Several views can share one page (one Vue app): the page then shows only
  // the parts marked data-views="<view>". A group is one tab-bar entry and one sub-menu.
  const views = [
    { id: 'dashboard', page: 'dashboard', group: 'overview', title: '总览' },
    { id: 'events', page: 'events', group: 'overview', title: '网络事件' },
    { id: 'bands', page: 'network', group: 'network', title: '频段与频点' },
    { id: 'cells', page: 'network', group: 'network', title: '小区锁定' },
    { id: 'connection', page: 'network', group: 'network', title: '拨号与 SIM' },
    { id: 'sms', page: 'sms', group: 'message', title: '短信' },
    { id: 'notify', page: 'notify', group: 'message', title: '消息通知' },
    { id: 'watchdog', page: 'settings', group: 'system', title: '断网恢复' },
    { id: 'lan', page: 'settings', group: 'system', title: '网络功能' },
    { id: 'at', page: 'settings', group: 'system', title: 'AT 终端' },
    { id: 'system', page: 'settings', group: 'system', title: '系统设置' },
    { id: 'deviceinfo', page: 'deviceinfo', group: 'more', title: '设备信息' },
    { id: 'console', page: 'console', group: 'more', title: '控制台' }
  ];
  // Tab-bar groups open the view last shown in them, or this one the first time.
  const groupDefaults = { overview: 'dashboard', network: 'bands', message: 'sms', system: 'system' };
  // Addresses from before the menu split (bookmarks, a page reloaded after an online
  // update): each opens the last view of the group that took its content over.
  const legacy = { network: 'bands', settings: 'system', forwarding: 'notify' };
  const lastView = {};
  const mountedPages = new Set();
  let currentPage = '';
  let currentView = '';
  const revealTimers = new WeakMap();
  const viewIds = new Set(views.map((view) => view.id));

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

  function normalizeView(value) {
    const id = String(value || '').replace(/^#/, '');
    if (viewIds.has(id)) return id;
    if (legacy[id]) return lastView[getView(legacy[id]).group] || legacy[id];
    return 'dashboard';
  }

  // A tab-bar entry. Group names are checked before view names: the "system" group and the
  // "system" view share a name, and the tab must reopen the group's last view.
  function showGroup(name) {
    if (!groupDefaults[name]) return showPage(name);
    return showPage(lastView[name] || groupDefaults[name]);
  }

  function getView(id) {
    return views.find((view) => view.id === id) || views[0];
  }

  function getPage(id) {
    return pages.find((page) => page.id === id) || pages[0];
  }

  // One rule per shared view hides the parts of its page that belong to the other views.
  function installViewStyles() {
    const rules = views
      .filter((view) => views.some((other) => other !== view && other.page === view.page))
      .map((view) => `.sa-page[data-view="${view.id}"] [data-views]:not([data-views~="${view.id}"]){display:none!important}`);
    const style = document.createElement('style');
    style.textContent = rules.join('\n');
    document.head.appendChild(style);
  }

  // Sub-menu above each grouped page, for phones where the sidebar is hidden.
  function buildSubnav() {
    document.querySelectorAll('.sa-page').forEach((section) => {
      const pageId = section.getAttribute('data-page');
      const own = views.find((view) => view.page === pageId);
      if (!own || own.group === 'more') return;
      const members = views.filter((view) => view.group === own.group);
      if (members.length < 2) return;
      const nav = document.createElement('nav');
      nav.className = 'sa-subnav';
      nav.setAttribute('aria-label', own.group);
      members.forEach((view) => {
        const button = document.createElement('button');
        button.type = 'button';
        button.setAttribute('data-view-link', view.id);
        button.textContent = view.title;
        button.addEventListener('click', () => showPage(view.id));
        nav.appendChild(button);
      });
      const titlebar = section.querySelector('.sa-page-titlebar');
      if (titlebar) titlebar.after(nav);
    });
  }

  function setActiveLink(id) {
    document.querySelectorAll('[data-view-link]').forEach((button) => {
      const active = button.getAttribute('data-view-link') === id;
      button.classList.toggle('active', active);
      if (active) button.setAttribute('aria-current', 'page');
      else button.removeAttribute('aria-current');
    });
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

  function setActiveSection(id, view) {
    document.querySelectorAll('.sa-page').forEach((section) => {
      const active = section.getAttribute('data-page') === id;
      section.classList.toggle('active', active);
      if (active) section.setAttribute('data-view', view.id);
      if (!active) clearRevealRunning(section);
    });
    // Shared pages show the view's name as their heading.
    const heading = document.getElementById(`page-title-${id}`);
    if (heading && views.filter((v) => v.page === id).length > 1) {
      heading.setAttribute('data-simpleadmin-i18n-key', view.title);
      heading.textContent = view.title;
    }
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
    if (!page.factoryName) {
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
    const page = getView(id);
    const title = root.Lang && typeof root.Lang.t === 'function' ? root.Lang.t(page.title) : page.title;
    if (root.Brand && typeof root.Brand.setPageTitle === 'function') {
      root.Brand.setPageTitle(title);
      return;
    }
    document.title = title;
  }

  function closeMobileSidebar() {
    const sidebar = document.getElementById('simpleadminSidebar');
    if (sidebar) sidebar.classList.remove('open');
  }

  function showPage(value, options) {
    const viewId = normalizeView(value);
    const view = getView(viewId);
    const id = view.page;
    currentPage = id;
    currentView = viewId;
    lastView[view.group] = viewId;
    disableBrowserScrollRestore();
    beginPageSwitch();
    setActiveSection(id, view);
    setActiveLink(viewId);
    mountPage(id);
    applyTitle(viewId);
    closeMobileSidebar();
    if (!options || options.updateHash !== false || String(value).replace(/^#/, '') !== viewId) {
      updateAddressHash(viewId, (options && options.replaceHash === true) || (options && options.updateHash === false));
    }
    if (root.Lang && typeof root.Lang.apply === 'function') {
      root.Lang.apply(document);
    }
    finishPageSwitchAtTop();
    scheduleProgressiveReveal(id);
    if (typeof global.dispatchEvent === 'function') {
      global.dispatchEvent(new CustomEvent('simpleadmin:page-changed', {
        detail: { page: id, view: viewId, group: view.group }
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

    // Back/forward fires both popstate and hashchange; switch pages only once.
    const followAddress = () => {
      const hash = global.location.hash.replace(/^#/, '');
      const viewId = normalizeView(hash);
      if (viewId === currentView) {
        // An old address for the view already shown: correct the address bar only.
        if (hash && hash !== viewId) updateAddressHash(viewId, true);
        return;
      }
      showPage(hash, { updateHash: false });
    };
    global.addEventListener('popstate', followAddress);
    global.addEventListener('hashchange', followAddress);
    global.addEventListener('simpleadmin:language-changed', () => {
      if (currentView) applyTitle(currentView);
    });
  }

  function init() {
    installViewStyles();
    buildSubnav();
    disableBrowserScrollRestore();
    document.documentElement.classList.add('sa-progressive-ready');
    bindNavigation();
    showPage(global.location.hash || 'dashboard', { updateHash: false });
  }

  global.addEventListener('load', resetScrollPositionAfterBrowserRestore);
  global.addEventListener('pageshow', resetScrollPositionAfterBrowserRestore);

  root.Spa = { showPage, showGroup, mountPage, resetScrollPosition, views, current: () => currentView };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init, { once: true });
  } else {
    init();
  }
})(window);
