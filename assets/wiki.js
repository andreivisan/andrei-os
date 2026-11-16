/* Andrei OS — wiki POC.
 *
 * Articles are plain data with a tiny wiki markup:
 *   [[Title]]          link to another article (shown as "not written yet" if it has no entry)
 *   [[Title|label]]    link with a different label
 *   [[category:Name]]  link to a category page
 *   '''bold'''
 * Backlinks ("What links here"), the contents box, categories and search are all
 * derived from this data, so adding an article is adding one object below.
 *
 * Routes (URL hash):  #slug   #slug/section   #category:name   #special:all   #special:random
 */
(function () {
  'use strict';

  var HOME = 'electronics';

  var ARTICLES = [
    {
      title: 'Electronics',
      info: { 'Type': 'Topic', 'Part of': '[[category:Learning]]', 'Status': 'Current focus', 'Feeds': '[[HUDPod]]', 'Next': '[[Embedded Rust]]' },
      lead: "'''Electronics''' is the current learning focus: the bottom-up path from the [[Transistor|transistor]] to bare-metal [[Embedded Rust|embedded Rust]]. Most of these notes feed the [[HUDPod]] build.",
      sections: [
        { h: 'Fundamentals', text: "[[Ohm's law]] relates voltage, current and resistance. A [[Voltage divider|voltage divider]] uses two resistors in series to scale a voltage down, for example to bring a battery voltage into the input range of an [[ADC]]." },
        { h: 'Components', text: "A [[Decoupling capacitor|decoupling capacitor]] sits next to an IC's power pins and smooths out supply noise. Every part starts with its [[Datasheet|datasheet]]: absolute maximum ratings first, then the pinout." },
        { h: 'Audio', text: "An [[I2S microphone]] sends digital audio over three lines: bit clock, word select and data." },
        { h: 'Power', text: "A single-cell [[LiPo battery]] is nominally 3.7 V and charges to 4.2 V. A [[UPS HAT]] handles charging and the switchover from USB power." },
        { h: 'Practice', text: "[[Soldering]] starts with through-hole parts: heat the pad and the lead together, then feed the solder." }
      ],
      seeAlso: ['Embedded Rust', 'FPGA', 'Study plan'],
      categories: ['Learning', 'Hardware']
    },
    {
      title: "Ohm's law",
      info: { 'Type': 'Concept', 'Part of': '[[Electronics]]' },
      lead: "'''Ohm's law''' states that the voltage across a resistor equals the current through it times its resistance: V = I × R.",
      sections: [
        { h: 'Use', text: "It is the first tool for sizing a resistor, and it is what makes a [[Voltage divider|voltage divider]] predictable." }
      ],
      seeAlso: ['Voltage divider'],
      categories: ['Hardware']
    },
    {
      title: 'Voltage divider',
      info: { 'Type': 'Circuit', 'Part of': '[[Electronics]]' },
      lead: "A '''voltage divider''' is two resistors in series across a supply. The output, taken across the lower resistor R2, is Vin × R2 / (R1 + R2), which follows from [[Ohm's law]].",
      sections: [
        { h: 'Use', text: "Typical use: scaling a [[LiPo battery]] voltage down so an [[ADC]] can measure it." }
      ],
      seeAlso: ["Ohm's law"],
      categories: ['Hardware']
    },
    {
      title: 'Decoupling capacitor',
      info: { 'Type': 'Component', 'Part of': '[[Electronics]]' },
      lead: "A '''decoupling capacitor''' is a small capacitor, often 100 nF, placed as close as possible to an IC's power pins. It supplies short bursts of current locally, so switching noise stays off the supply rail.",
      sections: [
        { h: 'Choosing one', text: "The recommended value and placement are in the part's [[Datasheet|datasheet]]." }
      ],
      seeAlso: ['Datasheet'],
      categories: ['Hardware']
    },
    {
      title: 'Datasheet',
      info: { 'Type': 'Reference', 'Part of': '[[Electronics]]' },
      lead: "A '''datasheet''' is the manufacturer's reference for a part.",
      sections: [
        { h: 'Reading order', text: "Absolute maximum ratings, recommended operating conditions, the pinout, then the application circuit. The application circuit is also where the [[Decoupling capacitor|decoupling capacitors]] are specified." }
      ],
      seeAlso: ['Decoupling capacitor', 'Soldering'],
      categories: ['Hardware']
    },
    {
      title: 'I2S microphone',
      info: { 'Type': 'Component', 'Part of': '[[Electronics]]', 'Used in': '[[HUDPod]]' },
      lead: "An '''I2S microphone''' is a MEMS microphone with a digital output.",
      sections: [
        { h: 'Signals', text: "I2S uses three lines: a bit clock, a word-select line that marks the left and right channels, and a data line." }
      ],
      seeAlso: ['Electronics', 'HUDPod'],
      categories: ['Hardware']
    },
    {
      title: 'LiPo battery',
      info: { 'Type': 'Component', 'Part of': '[[Electronics]]', 'Used in': '[[HUDPod]]' },
      lead: "A '''LiPo battery''' is a lithium-polymer cell: nominally 3.7 V, 4.2 V when fully charged.",
      sections: [
        { h: 'Charging', text: "It needs a dedicated charger circuit; a [[UPS HAT]] bundles one." },
        { h: 'Measuring', text: "Read the charge level through a [[Voltage divider|voltage divider]]." }
      ],
      seeAlso: ['Voltage divider'],
      categories: ['Hardware']
    },
    {
      title: 'Soldering',
      info: { 'Type': 'Skill', 'Part of': '[[Electronics]]' },
      lead: "'''Soldering''' joins a component lead to a pad with molten solder.",
      sections: [
        { h: 'Through-hole', text: "Heat the pad and the lead together, feed solder into the joint, take the solder away, then the iron." },
        { h: 'Limits', text: "The maximum soldering temperature of a part is in its [[Datasheet|datasheet]]." }
      ],
      seeAlso: ['Datasheet'],
      categories: ['Hardware']
    },
    {
      title: 'Embedded Rust',
      info: { 'Type': 'Topic', 'Part of': '[[category:Learning]]' },
      lead: "'''Embedded Rust''' is Rust without the standard library, running directly on a microcontroller.",
      sections: [
        { h: 'Where it fits', text: "It is the step after [[Electronics]] on the [[Study plan|study plan]], and sits next to [[FPGA]] work." }
      ],
      seeAlso: ['Electronics', 'FPGA'],
      categories: ['Learning', 'Rust']
    },
    {
      title: 'HUDPod',
      info: { 'Type': 'Project', 'Part of': '[[category:Projects]]' },
      lead: "'''HUDPod''' is the perception pod: a camera and a microphone in a unit that clips onto the glasses.",
      sections: [
        { h: 'Hardware notes', text: "[[I2S microphone]], [[LiPo battery]] and [[Soldering|soldering]]. Background reading: [[Electronics]]." }
      ],
      seeAlso: ['Electronics'],
      categories: ['Projects', 'Hardware']
    }
  ];

  // ---------- helpers ----------
  function slug(s) { return s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, ''); }
  function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }

  var BY_SLUG = {};
  ARTICLES.forEach(function (a) { a.slug = slug(a.title); BY_SLUG[a.slug] = a; });

  var LINK_RE = /\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g;

  function rawText(a) {
    var parts = [a.lead];
    a.sections.forEach(function (s) { parts.push(s.text); });
    Object.keys(a.info || {}).forEach(function (k) { parts.push(a.info[k]); });
    (a.seeAlso || []).forEach(function (t) { parts.push('[[' + t + ']]'); });
    return parts.join('\n');
  }
  function plainText(a) {
    return [a.lead].concat(a.sections.map(function (s) { return s.text; })).join(' ')
      .replace(LINK_RE, function (_, t, l) { return l || t; }).replace(/'''/g, '');
  }
  function linksOf(a) {
    var out = {}, m;
    LINK_RE.lastIndex = 0;
    var text = rawText(a);
    while ((m = LINK_RE.exec(text))) { if (!/^category:/i.test(m[1])) out[slug(m[1])] = true; }
    return out;
  }
  function backlinks(target) {
    return ARTICLES.filter(function (a) { return a.slug !== target && linksOf(a)[target]; });
  }
  function categoriesAll() {
    var set = {};
    ARTICLES.forEach(function (a) { (a.categories || []).forEach(function (c) { set[c] = true; }); });
    return Object.keys(set).sort();
  }

  function link(target, label) {
    if (/^category:/i.test(target)) {
      var name = target.slice(9);
      return '<a href="#category:' + esc(slug(name)) + '">' + esc(label || name) + '</a>';
    }
    var s = slug(target);
    return BY_SLUG[s]
      ? '<a href="#' + s + '">' + esc(label || target) + '</a>'
      : '<a class="is-missing" href="#' + s + '" title="Not written yet">' + esc(label || target) + '</a>';
  }
  function inline(text) {
    return esc(text)
      .replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, function (_, t, l) {
        // text was escaped above; undo for the lookup, link() escapes again
        var un = function (x) { return x.replace(/&quot;/g, '"').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&'); };
        return link(un(t), l ? un(l) : undefined);
      })
      .replace(/'''(.+?)'''/g, '<strong>$1</strong>');
  }

  // ---------- views ----------
  var body = document.getElementById('wiki-body');
  var tabLinks = document.getElementById('tab-links');
  var tabArticle = document.getElementById('tab-article');
  var tabAll = document.getElementById('tab-all');

  function footer(a, slugName) {
    var back = backlinks(slugName).map(function (b) { return link(b.title); }).join('') || '<span>Nothing yet</span>';
    var html = '<footer class="wiki-foot"><div id="sec-links-here"><span>What links here</span>' + back + '</div>';
    if (a && a.categories && a.categories.length) {
      html += '<div><span>Categories</span>' + a.categories.map(function (c) { return link('category:' + c); }).join('') + '</div>';
    }
    return html + '</footer>';
  }

  function viewArticle(a) {
    var toc = a.sections.map(function (s) { return s.h; }).concat(a.seeAlso && a.seeAlso.length ? ['See also'] : []);
    var html = '<article class="article"><header><h1>' + esc(a.title) + '</h1><p>' + inline(a.lead) + '</p></header>';
    if (toc.length > 2) {
      html += '<nav class="toc" aria-label="Contents"><div class="label">Contents</div>' +
        toc.map(function (h, i) { return '<a href="#' + a.slug + '/' + slug(h) + '"><span>' + (i + 1) + '</span>' + esc(h) + '</a>'; }).join('') + '</nav>';
    }
    a.sections.forEach(function (s) {
      html += '<section id="sec-' + slug(s.h) + '"><h2>' + esc(s.h) + '</h2>' +
        s.text.split(/\n\n+/).map(function (p) { return '<p>' + inline(p) + '</p>'; }).join('') + '</section>';
    });
    if (a.seeAlso && a.seeAlso.length) {
      html += '<section id="sec-see-also"><h2>See also</h2><ul>' +
        a.seeAlso.map(function (t) { return '<li>' + link(t) + '</li>'; }).join('') + '</ul></section>';
    }
    html += footer(a, a.slug) + '</article>';
    if (a.info) {
      html += '<aside class="infobox" aria-label="Summary"><div class="infobox__title">' + esc(a.title) + '</div><dl>' +
        Object.keys(a.info).map(function (k) { return '<dt>' + esc(k) + '</dt><dd>' + inline(a.info[k]) + '</dd>'; }).join('') + '</dl></aside>';
    }
    return html;
  }

  function titleFromSlug(s) {
    var t = s.replace(/-/g, ' ');
    return t.charAt(0).toUpperCase() + t.slice(1);
  }

  function viewMissing(slugName) {
    var title = titleFromSlug(slugName);
    return '<article class="article"><header><h1>' + esc(title) + '</h1>' +
      '<p>This article has not been written yet. Other articles already point here, so it is a gap worth filling.</p></header>' +
      footer(null, slugName) + '</article>';
  }

  function viewList(title, intro, list) {
    return '<article class="article"><header><h1>' + esc(title) + '</h1><p>' + intro + '</p></header>' +
      '<ul class="all-list">' + list.map(function (a) { return '<li>' + link(a.title) + '</li>'; }).join('') + '</ul></article>';
  }

  function route() {
    var hash = decodeURIComponent(location.hash.slice(1));
    var parts = hash.split('/');
    var name = parts[0] || HOME;
    var section = parts[1];
    var title, html;

    if (name === 'special:random') {
      var pick = ARTICLES[Math.floor(Math.random() * ARTICLES.length)];
      location.replace('#' + pick.slug);
      return;
    }
    if (name === 'special:all') {
      var sorted = ARTICLES.slice().sort(function (a, b) { return a.title.localeCompare(b.title); });
      title = 'All articles';
      html = viewList(title, sorted.length + ' articles. Categories: ' +
        categoriesAll().map(function (c) { return link('category:' + c); }).join(', ') + '.', sorted);
    } else if (name.indexOf('category:') === 0) {
      var cat = name.slice(9);
      var members = ARTICLES.filter(function (a) { return (a.categories || []).some(function (c) { return slug(c) === cat; }); });
      var catName = (categoriesAll().filter(function (c) { return slug(c) === cat; })[0]) || cat;
      title = 'Category: ' + catName;
      html = viewList(title, members.length + ' articles in this category.', members);
    } else if (BY_SLUG[name]) {
      title = BY_SLUG[name].title;
      html = viewArticle(BY_SLUG[name]);
    } else {
      title = titleFromSlug(name);
      html = viewMissing(name);
    }

    body.innerHTML = html;
    document.title = 'Andrei OS — Wiki: ' + title;
    var isPage = name.indexOf(':') === -1;
    tabArticle.setAttribute('href', '#' + (isPage ? name : HOME));
    tabLinks.setAttribute('href', '#' + (isPage ? name : HOME) + '/links-here');
    tabLinks.hidden = !isPage;
    if (isPage) { tabArticle.setAttribute('aria-current', 'page'); } else { tabArticle.removeAttribute('aria-current'); }
    if (name === 'special:all') { tabAll.setAttribute('aria-current', 'page'); } else { tabAll.removeAttribute('aria-current'); }

    var target = section && document.getElementById('sec-' + section);
    if (target) { target.scrollIntoView({ block: 'start' }); } else { window.scrollTo(0, 0); }
  }

  // ---------- search ----------
  var input = document.getElementById('wiki-q');
  var box = document.getElementById('wiki-results');
  var active = -1;

  function search(q) {
    var needle = q.trim().toLowerCase();
    if (!needle) { return []; }
    return ARTICLES.map(function (a) {
      var text = plainText(a);
      var inTitle = a.title.toLowerCase().indexOf(needle);
      var inText = text.toLowerCase().indexOf(needle);
      if (inTitle === -1 && inText === -1) { return null; }
      var snippet;
      if (inText === -1) {
        snippet = esc(text.slice(0, 90)) + (text.length > 90 ? '…' : '');
      } else {
        var from = Math.max(0, inText - 40), to = Math.min(text.length, inText + needle.length + 50);
        snippet = (from > 0 ? '…' : '') + esc(text.slice(from, inText)) +
          '<mark>' + esc(text.slice(inText, inText + needle.length)) + '</mark>' +
          esc(text.slice(inText + needle.length, to)) + (to < text.length ? '…' : '');
      }
      // title matches first, then prefix matches, then body matches
      var score = inTitle === 0 ? 0 : inTitle > 0 ? 1 : 2;
      return { a: a, snippet: snippet, score: score };
    }).filter(Boolean).sort(function (x, y) { return x.score - y.score || x.a.title.localeCompare(y.a.title); }).slice(0, 8);
  }

  function showResults() {
    var q = input.value;
    var hits = search(q);
    active = -1;
    if (!q.trim()) { box.hidden = true; box.innerHTML = ''; return; }
    box.innerHTML = hits.length
      ? hits.map(function (h) { return '<a href="#' + h.a.slug + '"><span class="hit">' + esc(h.a.title) + '</span><span class="snippet">' + h.snippet + '</span></a>'; }).join('')
      : '<p><span class="snippet">No article matches. Start it: ' + link(q.trim()) + '</span></p>';
    box.hidden = false;
  }
  function closeResults() { box.hidden = true; active = -1; }

  input.addEventListener('input', showResults);
  input.addEventListener('focus', showResults);
  input.addEventListener('keydown', function (e) {
    var items = box.querySelectorAll('a');
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      if (!items.length) { return; }
      e.preventDefault();
      active = (active + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
      Array.prototype.forEach.call(items, function (el, i) { el.classList.toggle('is-active', i === active); });
    } else if (e.key === 'Enter') {
      var go = items[active] || items[0];
      if (go) { location.hash = go.getAttribute('href'); input.value = ''; closeResults(); input.blur(); }
    } else if (e.key === 'Escape') {
      closeResults(); input.blur();
    }
  });
  box.addEventListener('click', function (e) {
    if (e.target.closest('a')) { input.value = ''; closeResults(); }
  });
  document.addEventListener('click', function (e) {
    if (!e.target.closest('.wiki-search')) { closeResults(); }
  });
  document.addEventListener('keydown', function (e) {
    var typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement.tagName);
    if (e.key === '/' && !typing) { e.preventDefault(); input.focus(); }
  });

  window.addEventListener('hashchange', route);
  route();
})();
