/**
 * Page-side half of scripts/i18n-sweep.js — evaluated inside the renderer.
 *
 * Walks the live DOM after a runtime language switch (no reload) and reports
 * every place where copy did not follow the language. Returns an array of
 * { state, lang, defectCount, defects[] } records.
 */
(async () => {
  const HAN = /[\u4e00-\u9fff]/;
  // simplified-only glyphs that never appear in Japanese copy
  const SIMP = /[们这点时电话设视图务钮击换网络载删缓导显应让从过还进远连择换复贴领开关项为个么没对头长见]/;
  const PH = /\{[a-zA-Z_][a-zA-Z0-9_]*\}/;
  // language names in the picker are intentionally native, and user data is
  // never translated
  const EXCLUDE = '[data-no-i18n], #setLang';

  const records = [];
  const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  const pathOf = (el) => {
    const parts = [];
    let n = el;
    while (n && n.nodeType === 1 && n !== document.documentElement) {
      let s = n.tagName.toLowerCase();
      if (n.id) s += `#${n.id}`;
      const cls = typeof n.className === 'string' ? n.className.trim() : '';
      if (cls) s += `.${cls.split(/\s+/).slice(0, 3).join('.')}`;
      parts.unshift(s);
      n = n.parentElement;
    }
    return parts.join(' > ');
  };

  const scan = (label, lang) => {
    const defects = [];
    const push = (kind, el, text) => defects.push({ kind, path: pathOf(el), text: String(text).slice(0, 100) });
    const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT, null);
    let n = w.nextNode();
    while (n) {
      const host = n.nodeType === 1 ? n : n.parentElement;
      const skip = !!(host && host.closest && host.closest(EXCLUDE));
      if (!skip) {
        if (n.nodeType === 3) {
          const v = n.nodeValue;
          if (v === '') push('empty-text', host, '');
          if (v && HAN.test(v) && lang === 'en') push('han', host, v.trim());
          if (v && SIMP.test(v) && lang === 'ja') push('simp', host, v.trim());
          if (v && PH.test(v)) push('placeholder', host, v.trim());
          if (v && v.indexOf('[object Object]') !== -1) push('object', host, v.trim());
        } else {
          for (const a of ['title', 'placeholder', 'aria-label']) {
            const v = n.getAttribute(a);
            if (!v) continue;
            if (HAN.test(v) && lang === 'en') push(`han@${a}`, n, v);
            if (SIMP.test(v) && lang === 'ja') push(`simp@${a}`, n, v);
            if (PH.test(v)) push(`placeholder@${a}`, n, v);
            if (v.indexOf('[object Object]') !== -1) push(`object@${a}`, n, v);
          }
        }
      }
      n = w.nextNode();
    }
    if (HAN.test(document.title) && lang === 'en') push('han@title', document.body, document.title);
    if (SIMP.test(document.title) && lang === 'ja') push('simp@title', document.body, document.title);
    records.push({ state: label, lang, defectCount: defects.length, defects });
  };

  const setLang = async (lang) => {
    state.settings.lang = lang;
    if (window.sweepInfo && window.sweepInfo.setLang) window.sweepInfo.setLang(lang);
    applyLanguage();
    await frame();
  };
  const close = () => {
    if (!document.querySelector('#modalLayer').hidden) closeModal();
  };

  const ds = (window.sweepInfo && window.sweepInfo.dataset) || 'paired';
  records.push({ meta: { dataset: ds, lang: window.I18N.getLang() } });

  if (ds === 'empty') {
    // onboarding modal is opened by init() shortly after boot
    for (const lang of ['en', 'ja']) {
      await setLang(lang);
      scan('onboarding', lang);
    }
    // and again with the dialog already open while the language flips
    await setLang('zh');
    await setLang('en');
    scan('onboarding-hotswitch-en', 'en');
    await setLang('ja');
    scan('onboarding-hotswitch-ja', 'ja');
    return records;
  }

  for (const lang of ['en', 'ja']) {
    close();
    applyTheme('dark', { persist: false });
    await setLang(lang);
    scan('main', lang);

    close();
    openSettings();
    await frame();
    scan('settings', lang);

    const light = document.querySelector('[data-theme-set="light"]');
    if (light) {
      light.click();
      await frame();
      scan('settings-light', lang);
      const dark = document.querySelector('[data-theme-set="dark"]');
      if (dark) dark.click();
      await frame();
    }

    close();
    openAddPhone();
    await frame();
    scan('pair-code', lang);

    const importTab = document.querySelector('[data-tab="import"]');
    if (importTab) {
      importTab.click();
      await frame();
      scan('pair-import', lang);
    }

    close();
    openAbout();
    await frame();
    scan('about', lang);
  }

  // runtime strings that only exist inside toasts, the context menu or an
  // in-flight pairing: the dialog states above never render them
  for (const lang of ['en', 'ja']) {
    close();
    applyTheme('dark', { persist: false });
    await setLang(lang);

    openSettings();
    await frame();
    const sw = document.querySelector('#modalCard [data-toggle]');
    if (sw) {
      sw.click();
      await sleep(80);
      await frame();
      scan('settings-toast', lang);
    }
    close();
    await frame();

    const chip = document.querySelector('#deviceChips [data-device]');
    if (chip) {
      const r = chip.getBoundingClientRect();
      chip.dispatchEvent(new MouseEvent('contextmenu', {
        bubbles: true, clientX: r.left + 8, clientY: r.top + 8,
      }));
      await frame();
      scan('ctx-menu', lang);
      document.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      await frame();
    }

    if (window.sweepInfo && window.sweepInfo.setPairMode) window.sweepInfo.setPairMode('pending');
    openAddPhone();
    await frame();
    const nameInput = document.querySelector('#pName');
    const urlInput = document.querySelector('#pUrl');
    const codeInput = document.querySelector('#pCode');
    if (nameInput && urlInput && codeInput) {
      nameInput.value = 'Sweep';
      urlInput.value = 'https://192.168.1.5:8787';
      codeInput.value = '123456';
      document.querySelector('#btnPairGo').click();
      await sleep(120);
      await frame();
      scan('pair-processing', lang);
    }
    if (window.sweepInfo && window.sweepInfo.setPairMode) window.sweepInfo.setPairMode('reject');
    close();
    await frame();

    const themeBtn = document.querySelector('#btnTheme');
    themeBtn.click();
    await sleep(80);
    await frame();
    scan('theme-toast', lang);
    themeBtn.click();
    await frame();
  }

  // dialog already open while the language flips
  close();
  applyTheme('dark', { persist: false });
  await setLang('zh');
  openSettings();
  await frame();
  await setLang('en');
  scan('settings-hotswitch-en', 'en');
  await setLang('ja');
  scan('settings-hotswitch-ja', 'ja');
  close();

  // crash page last: it replaces document.body
  for (const lang of ['en', 'ja']) {
    window.I18N.setLang(lang);
    renderCrash(new Error('sweep failure'));
    await frame();
    scan('crash', lang);
  }

  return records;
})()
