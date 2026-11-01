// NovumChat JS bridge — the small set of browser capabilities the Rust/WASM UI
// calls through wasm-bindgen (`window.NovumBridge`). Keep it dependency-free.
(function () {
  'use strict';

  const tauri = () => window.__TAURI__ && window.__TAURI__.core;
  const mathCache = new Map();
  const codeCache = new Map();
  let blockRemote = true;
  let mermaidLoading = null;

  function remember(map, key, value) {
    if (map.size > 3000) map.clear();
    map.set(key, value);
    return value;
  }

  function isRemote(url) {
    return /^\s*(https?:)?\/\//i.test(url || '');
  }

  // ── Diagram palette ─────────────────────────────────────────────────────
  // Mermaid's theming engine only understands hex colours, so the palette is
  // read from the live design tokens and blended here. Diagrams then follow
  // light/dark mode and the chosen accent.
  function hex(value) {
    const c = (value || '').trim().toLowerCase();
    if (/^#[0-9a-f]{6}$/.test(c)) return c;
    if (/^#[0-9a-f]{3}$/.test(c)) return '#' + [...c.slice(1)].map((x) => x + x).join('');
    return null;
  }

  function mix(a, b, t) {
    const pa = parseInt(a.slice(1), 16);
    const pb = parseInt(b.slice(1), 16);
    return '#' + [16, 8, 0]
      .map((s) => Math.round(((pa >> s) & 255) * (1 - t) + ((pb >> s) & 255) * t))
      .map((v) => v.toString(16).padStart(2, '0'))
      .join('');
  }

  function diagramTheme(dark) {
    const css = getComputedStyle(document.documentElement);
    const token = (name, fallback) => hex(css.getPropertyValue(name)) || fallback;
    const bg = token('--bg', dark ? '#0e1b2b' : '#f2f5f9');
    const surface = token('--surface', dark ? '#15263b' : '#ffffff');
    const surface2 = token('--surface-2', dark ? '#1b2e47' : '#edf1f6');
    const text = token('--text', dark ? '#eaf1fb' : '#0f1b2d');
    const text2 = token('--text-2', dark ? '#c2cfe2' : '#3c4a60');
    const muted = token('--muted', dark ? '#8b9cb6' : '#687589');
    const line = token('--line-strong', dark ? '#2f4870' : '#c9d2df');
    const accent = token('--accent', dark ? '#5bd0ff' : '#1373c2');
    const spectrum = ['--s1', '--s2', '--s3', '--s4', '--s5'].map((n) => token(n, accent));
    const tint = (c, t) => mix(surface, c, t);
    const vars = {
      darkMode: dark,
      background: surface,
      fontFamily: '"Instrument Sans Variable", ui-sans-serif, system-ui, sans-serif',
      fontSize: '14px',
      textColor: text,
      titleColor: text,
      lineColor: muted,
      primaryColor: tint(accent, dark ? 0.2 : 0.1),
      primaryTextColor: text,
      primaryBorderColor: tint(accent, dark ? 0.7 : 0.6),
      secondaryColor: tint(spectrum[1], dark ? 0.2 : 0.12),
      secondaryTextColor: text,
      secondaryBorderColor: tint(spectrum[1], 0.6),
      tertiaryColor: surface2,
      tertiaryTextColor: text2,
      tertiaryBorderColor: line,
      clusterBkg: surface2,
      clusterBorder: line,
      edgeLabelBackground: surface,
      noteBkgColor: tint(spectrum[2], dark ? 0.2 : 0.14),
      noteTextColor: text,
      noteBorderColor: tint(spectrum[2], 0.6),
      pieStrokeColor: surface,
      pieOuterStrokeColor: line,
      pieTitleTextColor: text,
      pieLegendTextColor: text2,
      pieSectionTextColor: dark ? bg : text,
    };
    const series = [...spectrum, accent];
    series.forEach((c, i) => { vars['pie' + (i + 1)] = c; });
    series.concat(series).slice(0, 8).forEach((c, i) => { vars['git' + i] = c; });
    return vars;
  }

  // ── Sanitiser ───────────────────────────────────────────────────────────
  function setupPurify() {
    if (!window.DOMPurify || window.__novumPurify) return;
    window.__novumPurify = true;
    DOMPurify.addHook('afterSanitizeAttributes', (node) => {
      if (node.tagName === 'A') {
        node.removeAttribute('target');
        node.setAttribute('rel', 'noopener noreferrer');
      }
      if (blockRemote) {
        // Remote media can leak conversation contents through its URL.
        const tag = node.tagName;
        if (tag === 'IMG' || tag === 'VIDEO' || tag === 'AUDIO' || tag === 'SOURCE') {
          const src = node.getAttribute('src');
          if (src && isRemote(src)) {
            node.setAttribute('data-src', src);
            node.removeAttribute('src');
            node.setAttribute('data-action', 'load-image');
            node.classList.add('nc-img-held');
            node.setAttribute('alt', node.getAttribute('alt') || 'Remote image — click to load');
          }
          node.removeAttribute('srcset');
          node.removeAttribute('poster');
        }
        const style = node.getAttribute && node.getAttribute('style');
        if (style && /url\s*\(/i.test(style)) node.removeAttribute('style');
      }
    });
  }

  const PURIFY = {
    FORBID_TAGS: ['style', 'form', 'input', 'textarea', 'select', 'option', 'iframe', 'frame', 'frameset',
      'object', 'embed', 'link', 'meta', 'base', 'noscript', 'dialog', 'portal'],
    FORBID_ATTR: ['autofocus', 'formaction', 'srcdoc'],
    ALLOW_DATA_ATTR: true,
  };

  // ── Artifacts (sandboxed apps) ──────────────────────────────────────────
  const PREVIEW_CSP = "default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' blob: data:; " +
    "style-src 'unsafe-inline' data:; img-src data: blob:; font-src data:; media-src data: blob:; " +
    "connect-src 'none'; frame-src 'none'; worker-src blob:; form-action 'none'";

  function injectHead(html, extra) {
    if (/<head[^>]*>/i.test(html)) return html.replace(/<head[^>]*>/i, (m) => m + extra);
    if (/<html[^>]*>/i.test(html)) return html.replace(/<html[^>]*>/i, (m) => m + '<head>' + extra + '</head>');
    return '<!doctype html><html><head>' + extra + '</head><body>' + html + '</body></html>';
  }

  function artifactDoc(code, kind, allowNetwork, dark) {
    const csp = allowNetwork ? '' : `<meta http-equiv="Content-Security-Policy" content="${PREVIEW_CSP}">`;
    const base = '<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">';
    if (kind === 'svg') {
      const bg = dark ? '#0e1b2b' : '#ffffff';
      return `<!doctype html><html><head>${base}${csp}<style>html,body{margin:0;height:100%;display:grid;place-items:center;background:${bg}}svg{max-width:100%;max-height:100vh}</style></head><body>${code}</body></html>`;
    }
    return injectHead(code, base + csp);
  }

  function jsRunnerDoc(code, dark) {
    const safe = String(code).replace(/<\/script/gi, '<\\/script');
    const fg = dark ? '#eaf1fb' : '#0f1b2d';
    const err = dark ? '#ff8a8a' : '#c62f45';
    return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${PREVIEW_CSP}">
<style>body{margin:0;padding:10px 12px;font:13px/1.5 ui-monospace,'JetBrains Mono Variable',monospace;color:${fg};background:transparent;white-space:pre-wrap;word-break:break-word}
.e{color:${err}}.w{color:#d9a21b}.r{opacity:.6}</style></head><body><div id="o"></div><script>
(function(){var o=document.getElementById('o');function fmt(v){try{if(typeof v==='string')return v;if(v instanceof Error)return v.stack||String(v);return JSON.stringify(v,null,2)}catch(e){return String(v)}}
function out(cls,args){var d=document.createElement('div');d.className=cls;d.textContent=Array.prototype.map.call(args,fmt).join(' ');o.appendChild(d);parent.postMessage({novumRunner:1,h:document.documentElement.scrollHeight},'*')}
console.log=function(){out('',arguments)};console.info=console.log;console.debug=console.log;console.warn=function(){out('w',arguments)};console.error=function(){out('e',arguments)};
window.onerror=function(m,s,l,c,e){out('e',[e&&e.stack?e.stack:m]);};window.addEventListener('unhandledrejection',function(e){out('e',['Unhandled rejection: '+fmt(e.reason)])});
window.__done=function(r){if(r!==undefined)out('r',['← '+fmt(r)]);if(!o.childNodes.length)out('r',['(no output)'])};})();
</script><script>
try{var __r=(function(){${safe}
})();Promise.resolve(__r).then(window.__done)}catch(e){console.error(e)}
</script></body></html>`;
  }

  // ── Event transport ─────────────────────────────────────────────────────
  function connectEvents(onEvent) {
    const t = window.__TAURI__;
    if (t && t.event) {
      t.event.listen('novum://event', (e) => onEvent(JSON.stringify(e.payload)));
      return;
    }
    let first = true;
    const open = () => {
      const es = new EventSource('/api/events', { withCredentials: true });
      es.onopen = () => {
        if (!first) onEvent('{"type":"resync"}');
        first = false;
      };
      es.onmessage = (e) => onEvent(e.data);
      es.onerror = () => {
        if (es.readyState === EventSource.CLOSED) {
          setTimeout(open, 2000);
        }
      };
    };
    open();
  }

  window.NovumBridge = {
    isTauri: () => !!tauri(),

    // RPC: params/results travel as JSON strings to keep the Rust side simple.
    invoke: async (method, paramsJson) => {
      const params = paramsJson ? JSON.parse(paramsJson) : {};
      const t = tauri();
      if (t) {
        const result = await t.invoke('rpc', { method, params });
        return JSON.stringify(result === undefined ? null : result);
      }
      const resp = await fetch('/api/rpc/' + encodeURIComponent(method), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify(params),
      });
      const text = await resp.text();
      if (!resp.ok) {
        let message = text;
        try { message = JSON.parse(text).error || text; } catch (_) { /* plain text */ }
        if (resp.status === 401) message = 'Not authorised: open the link with ?token=… printed by `novumchat --serve`.';
        throw new Error(message);
      }
      return text || 'null';
    },

    listen: (cb) => connectEvents(cb),

    // ── Rendering helpers ─────────────────────────────────────────────────
    renderMath: (tex, display) => {
      const key = (display ? 'D' : 'I') + tex;
      if (mathCache.has(key)) return mathCache.get(key);
      if (!window.katex) return null;
      try {
        const html = katex.renderToString(tex, {
          displayMode: !!display,
          throwOnError: false,
          output: 'htmlAndMathml',
          strict: 'ignore',
          trust: false,
          maxExpand: 2000,
        });
        return remember(mathCache, key, html);
      } catch (e) {
        return null;
      }
    },

    highlight: (code, lang) => {
      const key = lang + '\u0000' + code;
      if (codeCache.has(key)) return codeCache.get(key);
      if (!window.hljs) return null;
      try {
        let html;
        if (lang && hljs.getLanguage(lang)) {
          html = hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
        } else if (!lang || lang === 'text' || lang === 'plaintext') {
          return null;
        } else {
          return null;
        }
        // Long blocks are cached only once complete (cheap enough otherwise).
        return code.length < 200000 ? remember(codeCache, key, html) : html;
      } catch (e) {
        return null;
      }
    },

    sanitize: (html, allowRemote) => {
      if (!window.DOMPurify) return '';
      setupPurify();
      blockRemote = !allowRemote;
      return DOMPurify.sanitize(html, PURIFY);
    },

    sanitizeSvg: (svg) => window.DOMPurify ? DOMPurify.sanitize(svg, { USE_PROFILES: { svg: true, svgFilters: true } }) : '',

    artifactDoc,
    jsRunnerDoc,

    mermaid: async (code, dark) => {
      if (!window.mermaid) {
        if (!mermaidLoading) {
          mermaidLoading = new Promise((resolve, reject) => {
            const s = document.createElement('script');
            s.src = 'vendor/mermaid.min.js';
            s.onload = resolve;
            s.onerror = () => reject(new Error('Mermaid failed to load'));
            document.head.appendChild(s);
          });
        }
        await mermaidLoading;
      }
      // SVG <text> labels (not HTML in <foreignObject>) survive the SVG
      // sanitiser. The wider wrap keeps ordinary labels on one line; Mermaid
      // otherwise breaks words longer than 120px mid-word.
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: 'strict',
        theme: 'base',
        themeVariables: diagramTheme(!!dark),
        htmlLabels: false,
        flowchart: { htmlLabels: false, wrappingWidth: 240 },
        fontFamily: '"Instrument Sans Variable", ui-sans-serif, system-ui, sans-serif',
      });
      const id = 'nc-mmd-' + Math.random().toString(36).slice(2);
      const { svg } = await mermaid.render(id, code);
      return svg;
    },

    // ── Platform helpers ──────────────────────────────────────────────────
    copyText: async (text) => {
      try {
        if (navigator.clipboard && window.isSecureContext !== false) {
          await navigator.clipboard.writeText(text);
          return true;
        }
      } catch (_) { /* fall back */ }
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('readonly', '');
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      let ok = false;
      try { ok = document.execCommand('copy'); } catch (_) { ok = false; }
      ta.remove();
      return ok;
    },

    readFileBase64: (file) => new Promise((resolve, reject) => {
      const r = new FileReader();
      r.onload = () => {
        const s = String(r.result || '');
        const i = s.indexOf('base64,');
        resolve(i >= 0 ? s.slice(i + 7) : s);
      };
      r.onerror = () => reject(r.error || new Error('could not read file'));
      r.readAsDataURL(file);
    }),

    objectUrl: (blobOrFile) => URL.createObjectURL(blobOrFile),
    revokeUrl: (url) => { try { URL.revokeObjectURL(url); } catch (_) { /* ignore */ } },

    dataUrlToObjectUrl: (b64, mime) => {
      const bin = atob(b64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      return URL.createObjectURL(new Blob([bytes], { type: mime || 'application/octet-stream' }));
    },

    download: (filename, content, mime) => {
      const url = URL.createObjectURL(new Blob([content], { type: mime || 'text/plain' }));
      const a = document.createElement('a');
      a.href = url;
      a.download = filename;
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 2000);
    },

    canSpeak: () => 'speechSynthesis' in window && typeof SpeechSynthesisUtterance !== 'undefined',
    speak: (text, onEnd) => {
      if (!('speechSynthesis' in window)) return false;
      speechSynthesis.cancel();
      const u = new SpeechSynthesisUtterance(text);
      u.onend = () => onEnd && onEnd();
      u.onerror = () => onEnd && onEnd();
      speechSynthesis.speak(u);
      return true;
    },
    stopSpeaking: () => { if ('speechSynthesis' in window) speechSynthesis.cancel(); },

    prefersDark: () => !!(window.matchMedia && matchMedia('(prefers-color-scheme: dark)').matches),
    onSchemeChange: (cb) => {
      if (!window.matchMedia) return;
      const mq = matchMedia('(prefers-color-scheme: dark)');
      const handler = () => cb(mq.matches);
      if (mq.addEventListener) mq.addEventListener('change', handler); else mq.addListener(handler);
    },
    prefersReducedMotion: () => !!(window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches),

    store: (key, value) => { try { localStorage.setItem(key, value); } catch (_) { /* private mode */ } },
    load: (key) => { try { return localStorage.getItem(key); } catch (_) { return null; } },

    vibrate: (ms) => { try { if (navigator.vibrate) navigator.vibrate(ms); } catch (_) { /* unsupported */ } },
  };
})();
