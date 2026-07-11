/* controller-nav.js — Plutonium controller navigation layer
 * Injected into launcher/assets/index.html by plutonium-launcher.exe on every run.
 *
 * This file is ALWAYS written from the Rust binary (embedded via include_str!).
 * Edit here; the binary re-embeds it at compile time.
 *
 * Milestone state: M3 — full spatial keyboard navigation, confirmed working
 * on real hardware. Ultralight exposes no Gamepad API (navigator.getGamepads
 * is absent), so all input arrives via the Rust controller helper injecting
 * synthetic keyboard events (see src/gamepad.rs).
 */

(function () {
  'use strict';

  // ── Spatial keyboard navigation (M3) ─────────────────────────────────────
  // Finds all focusable/clickable elements in the Vue app, maintains a visible
  // highlight, and routes arrow keys / Enter / Esc through them.
  //
  // Key bindings (fed by keyboard directly OR injected by the Rust controller helper):
  //   ArrowUp/Down/Left/Right — move focus to nearest element in that direction
  //   Enter                   — activate (click) focused element
  //   Escape / Backspace      — go back (simulate browser back / click a close button)
  //   Tab / Shift+Tab         — cycle focus forward / backward (fallback nav)

  // This app has no <button>/[role=button]/[tabindex] anywhere — its clickable
  // widgets are plain <div>s with these classes (confirmed via the on-screen
  // cursor:pointer dump: .avatarImage.clickable, .button, .row all present;
  // bare div/img entries in that dump were just inheriting cursor:pointer from
  // these ancestors, not independent targets).
  var FOCUSABLE_SEL = [
    'button:not([disabled])',
    '[role="button"]',
    'a[href]',
    'input:not([disabled]):not([type="hidden"])',
    'select:not([disabled])',
    '[tabindex]:not([tabindex="-1"])',
    '.button',
    '.clickable',
    '.row',
  ].join(',');

  var FOCUS_STYLE_ID = 'ctrl-nav-focus-style';
  var focused = null;

  function installFocusStyle() {
    if (document.getElementById(FOCUS_STYLE_ID)) return;
    var s = document.createElement('style');
    s.id = FOCUS_STYLE_ID;
    s.textContent = [
      '.ctrl-nav-focused {',
      '  outline: 2px solid #00e5ff !important;',
      '  outline-offset: 2px !important;',
      '  box-shadow: 0 0 8px rgba(0,229,255,0.6) !important;',
      '}',
    ].join('\n');
    document.head.appendChild(s);
  }

  function getFocusable() {
    return Array.from(document.querySelectorAll(FOCUSABLE_SEL))
      .filter(function (el) {
        var r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      });
  }

  function setFocus(el) {
    if (focused) focused.classList.remove('ctrl-nav-focused');
    focused = el;
    if (!el) return;
    el.classList.add('ctrl-nav-focused');
    el.focus({ preventScroll: false });
    el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }

  function center(el) {
    var r = el.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  }

  function dist(a, b) {
    var dx = a.x - b.x, dy = a.y - b.y;
    return Math.sqrt(dx * dx + dy * dy);
  }

  /** Move focus in a direction ('up'|'down'|'left'|'right'). */
  function moveFocus(dir) {
    var items = getFocusable();
    if (!items.length) return;

    if (!focused || !document.body.contains(focused)) {
      setFocus(items[0]);
      return;
    }

    var from = center(focused);
    var best = null, bestScore = Infinity;

    items.forEach(function (el) {
      if (el === focused) return;
      var to = center(el);
      var dx = to.x - from.x, dy = to.y - from.y;

      // Must be in the right half-plane for the direction.
      var inPlane = (dir === 'up'    && dy < -10)
                 || (dir === 'down'  && dy >  10)
                 || (dir === 'left'  && dx < -10)
                 || (dir === 'right' && dx >  10);
      if (!inPlane) return;

      // Score: primary axis distance + 0.3× perpendicular distance (prioritise alignment).
      var primary = dir === 'up' || dir === 'down' ? Math.abs(dy) : Math.abs(dx);
      var perp    = dir === 'up' || dir === 'down' ? Math.abs(dx) : Math.abs(dy);
      var score   = primary + 0.3 * perp;

      if (score < bestScore) { bestScore = score; best = el; }
    });

    if (best) setFocus(best);
  }

  function activate() {
    if (!focused) return;
    // Don't activate text inputs on Enter — let the browser handle it.
    var tag = focused.tagName.toLowerCase();
    if (tag === 'input' || tag === 'select' || tag === 'textarea') return;
    focused.click();
  }

  function goBack() {
    // Try to find a visible "back" or "close" button first.
    var backBtn = document.querySelector(
      '[aria-label="back"], [aria-label="close"], .back-btn, .close-btn, .btn-back'
    );
    if (backBtn) { backBtn.click(); return; }
    history.back();
  }

  // This Ultralight build doesn't populate KeyboardEvent.key/.code (confirmed
  // via the on-screen lastKeyDown diagnostic: real arrow-key presses arrive
  // with key=undefined code=undefined, only the legacy numeric keyCode is
  // set). Map keyCode as the primary source, e.key as a bonus if some future
  // build does populate it.
  var KEYCODE_TO_NAME = {
    37: 'ArrowLeft', 38: 'ArrowUp', 39: 'ArrowRight', 40: 'ArrowDown',
    13: 'Enter', 27: 'Escape', 8: 'Backspace', 9: 'Tab',
  };

  function onKeyDown(e) {
    var key = e.key || KEYCODE_TO_NAME[e.keyCode];

    // Never intercept when a text input is focused — login fields need the keyboard.
    var active = document.activeElement;
    if (active) {
      var tag = active.tagName.toLowerCase();
      if (tag === 'input' || tag === 'textarea' || tag === 'select') return;
    }

    switch (key) {
      case 'ArrowUp':    e.preventDefault(); moveFocus('up');    break;
      case 'ArrowDown':  e.preventDefault(); moveFocus('down');  break;
      case 'ArrowLeft':  e.preventDefault(); moveFocus('left');  break;
      case 'ArrowRight': e.preventDefault(); moveFocus('right'); break;
      case 'Enter':      e.preventDefault(); activate();         break;
      case 'Escape':
      case 'Backspace':  e.preventDefault(); goBack();           break;
      case 'Tab':
        // Tab/Shift+Tab: cycle through focusables.
        e.preventDefault();
        var items = getFocusable();
        if (!items.length) break;
        var idx = focused ? items.indexOf(focused) : -1;
        var next = e.shiftKey
          ? items[(idx - 1 + items.length) % items.length]
          : items[(idx + 1) % items.length];
        setFocus(next);
        break;
    }
  }

  // Re-scan on DOM changes (Vue route transitions, dynamic menus).
  function observeDom() {
    var mo = new MutationObserver(function () {
      // If previously focused element is gone, clear highlight.
      if (focused && !document.body.contains(focused)) {
        focused.classList.remove('ctrl-nav-focused');
        focused = null;
      }
    });
    mo.observe(document.body, { childList: true, subtree: true });
  }

  // ── Bootstrap ─────────────────────────────────────────────────────────────
  function init() {
    installFocusStyle();
    document.addEventListener('keydown', onKeyDown, true);
    observeDom();
    console.log('[controller-nav] loaded. Arrow keys / Enter / Esc active.');
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
