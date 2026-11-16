/* Andrei OS — client behaviour. Pages are server-rendered; this file only
 * adds interactions (tasks, Kanban drag & drop, search, library edits). */
(function () {
  'use strict';

  // ---------------------------------------------------------------- helpers
  function $(sel, root) { return (root || document).querySelector(sel); }
  function $$(sel, root) { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); }
  function esc(s) {
    return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }
  function slug(s) {
    return s.normalize('NFKD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
  }
  function localDate(d) {
    d = d || new Date();
    var m = String(d.getMonth() + 1).padStart(2, '0'), day = String(d.getDate()).padStart(2, '0');
    return d.getFullYear() + '-' + m + '-' + day;
  }

  function toast(msg, kind) {
    var box = $('.toasts');
    if (!box) { return; }
    var t = document.createElement('div');
    t.className = 'toast' + (kind ? ' toast--' + kind : '');
    t.textContent = msg;
    box.appendChild(t);
    setTimeout(function () { t.remove(); }, kind === 'error' || kind === 'warn' ? 9000 : 3500);
  }

  // Same-origin JSON only: the server refuses anything else (CSRF guard).
  function api(method, url, body) {
    var opts = { method: method, headers: { 'X-Requested-With': 'fetch', 'Accept': 'application/json' } };
    if (body !== undefined) {
      opts.headers['Content-Type'] = 'application/json';
      opts.body = JSON.stringify(body);
    }
    return fetch(url, opts).then(function (r) {
      return r.json().catch(function () { return {}; }).then(function (data) {
        if (!r.ok) { throw new Error(data.error || (r.status + ' ' + r.statusText)); }
        if (data.warning) { toast(data.warning, 'warn'); }
        return data;
      });
    });
  }
  function fail(e) { toast(e.message || String(e), 'error'); }
  function reloadSoon(ms) { setTimeout(function () { location.reload(); }, ms || 0); }

  $$('[data-dialog-close]').forEach(function (b) {
    b.addEventListener('click', function () { b.closest('dialog').close(); });
  });

  // ---------------------------------------------------------------- tasks
  function taskEl(el) { return el.closest('[data-task-id]'); }

  document.addEventListener('change', function (e) {
    if (!e.target.matches('[data-task-toggle]')) { return; }
    var row = taskEl(e.target), done = e.target.checked;
    row.classList.toggle('is-done', done);
    api('PATCH', '/api/tasks/' + row.dataset.taskId, { status: done ? 'done' : 'todo' })
      .then(function () { row.dataset.status = done ? 'done' : 'todo'; })
      .catch(function (err) { e.target.checked = !done; row.classList.toggle('is-done', !done); fail(err); });
  });

  var dialog = $('#task-dialog');
  var form = dialog && $('[data-task-form]', dialog);

  function openTask(el) {
    if (!dialog) { return; }
    var d = el.dataset;
    form.elements.id.value = d.taskId;
    form.elements.title.value = d.title || '';
    form.elements.notes.value = d.notes || '';
    form.elements.status.value = d.status || 'todo';
    form.elements.due_date.value = d.due || '';
    form.elements.due_time.value = d.time || '';
    form.elements.target.value = d.project ? 'project:' + d.project : (d.area ? 'area:' + d.area : '');
    var src = d.source || '';
    var note = /^Reminders/.test(src) ? ' — checking it done here also completes it in Reminders'
      : /^Notes/.test(src) ? ' — from an Apple Notes checklist' : '';
    $('[data-task-source]', dialog).textContent = src ? 'Source: ' + src + note : '';
    dialog.showModal();
    form.elements.title.focus();
  }

  document.addEventListener('click', function (e) {
    var open = e.target.closest('[data-task-open]');
    if (open) { openTask(taskEl(open)); return; }
    var card = e.target.closest('.card[data-task-id]');
    if (card && !card.dataset.justDragged) { openTask(card); }
  });
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Enter' && e.target.matches('.card[data-task-id]')) { openTask(e.target); }
  });

  if (form) {
    form.addEventListener('submit', function (e) {
      e.preventDefault();
      var f = form.elements, target = f.target.value;
      var body = {
        title: f.title.value,
        notes: f.notes.value,
        status: f.status.value,
        due_date: f.due_date.value || null,
        due_time: f.due_time.value || null
      };
      if (target.indexOf('project:') === 0) { body.project_id = Number(target.slice(8)); }
      else if (target.indexOf('area:') === 0) { body.area = target.slice(5); }
      else { body.project_id = null; body.area = null; }
      api('PATCH', '/api/tasks/' + f.id.value, body).then(function () { dialog.close(); reloadSoon(); }).catch(fail);
    });
    $('[data-task-delete]', form).addEventListener('click', function () {
      if (!confirm('Delete this task?')) { return; }
      api('DELETE', '/api/tasks/' + form.elements.id.value).then(function () { dialog.close(); reloadSoon(); }).catch(fail);
    });
  }

  $$('[data-add-task]').forEach(function (f) {
    f.addEventListener('submit', function (e) {
      e.preventDefault();
      var title = f.elements.title.value.trim();
      if (!title) { return; }
      var body = { title: title };
      if (f.elements.project_id) { body.project_id = Number(f.elements.project_id.value); }
      if (f.elements.area) { body.area = f.elements.area.value; }
      if (f.elements.due_date) { body.due_date = f.elements.due_date.value === 'today' ? localDate() : f.elements.due_date.value; }
      f.elements.title.disabled = true;
      api('POST', '/api/tasks', body).then(function () { reloadSoon(); })
        .catch(function (err) { f.elements.title.disabled = false; fail(err); });
    });
  });

  // ---------------------------------------------------------------- Kanban
  var dragging = null;

  function cardAfter(zone, y) {
    var cards = $$('.card:not(.is-dragging)', zone);
    for (var i = 0; i < cards.length; i++) {
      var r = cards[i].getBoundingClientRect();
      if (y < r.top + r.height / 2) { return cards[i]; }
    }
    return null;
  }
  function recount() {
    $$('[data-column]').forEach(function (c) {
      $('[data-count]', c).textContent = $$('.card', c).length;
    });
  }

  $$('[data-board] .card').forEach(function (card) {
    card.addEventListener('dragstart', function (e) {
      dragging = card;
      card.classList.add('is-dragging');
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', card.dataset.taskId);
    });
    card.addEventListener('dragend', function () {
      card.classList.remove('is-dragging');
      $$('.is-over').forEach(function (z) { z.classList.remove('is-over'); });
      card.dataset.justDragged = '1';
      setTimeout(function () { delete card.dataset.justDragged; }, 50);
      dragging = null;
    });
  });

  $$('[data-board] [data-dropzone]').forEach(function (zone) {
    zone.addEventListener('dragover', function (e) {
      if (!dragging) { return; }
      e.preventDefault();
      zone.classList.add('is-over');
      var after = cardAfter(zone, e.clientY);
      if (after) { zone.insertBefore(dragging, after); } else { zone.appendChild(dragging); }
    });
    zone.addEventListener('dragleave', function (e) {
      if (!zone.contains(e.relatedTarget)) { zone.classList.remove('is-over'); }
    });
    zone.addEventListener('drop', function (e) {
      if (!dragging) { return; }
      e.preventDefault();
      zone.classList.remove('is-over');
      var card = dragging;
      var status = zone.closest('[data-column]').dataset.column;
      var prev = card.previousElementSibling, next = card.nextElementSibling;
      var p = prev ? Number(prev.dataset.position) : null, n = next ? Number(next.dataset.position) : null;
      var pos = p !== null && n !== null ? (p + n) / 2 : p !== null ? p + 1 : n !== null ? n - 1 : 1;
      var oldStatus = card.dataset.status;
      card.dataset.position = pos;
      card.dataset.status = status;
      recount();
      api('PATCH', '/api/tasks/' + card.dataset.taskId, { status: status, position: pos })
        .then(function () { if (oldStatus !== status && (oldStatus === 'done' || status === 'done')) { toast(status === 'done' ? 'Done' : 'Reopened'); } })
        .catch(function (err) { fail(err); reloadSoon(1500); });
    });
  });

  // ---------------------------------------------------------------- search
  function searchBox(input, box, onPick) {
    var active = -1, timer = null, seq = 0;
    function close() { box.hidden = true; active = -1; }
    function render(q, results) {
      active = -1;
      if (!q.trim()) { close(); box.innerHTML = ''; return; }
      box.innerHTML = results.length
        ? results.map(function (r) {
            var snip = esc(r.snippet || '').replace(/\u0001/g, '<mark>').replace(/\u0002/g, '</mark>');
            return '<a href="/wiki/' + esc(r.slug) + '" data-slug="' + esc(r.slug) + '"><span class="hit">' + esc(r.title) +
              '</span><span class="snippet">' + (snip || esc(r.folder)) + '</span></a>';
          }).join('')
        : '<p><span class="snippet">No article matches. <a class="is-missing" href="/wiki/' + esc(slug(q)) + '">' + esc(q.trim()) + '</a> is not written yet.</span></p>';
      box.hidden = false;
    }
    function run() {
      var q = input.value, my = ++seq;
      if (!q.trim()) { render('', []); return; }
      fetch('/api/search?q=' + encodeURIComponent(q), { headers: { Accept: 'application/json' } })
        .then(function (r) { return r.json(); })
        .then(function (d) { if (my === seq) { render(q, d.results || []); } })
        .catch(function () {});
    }
    input.addEventListener('input', function () { clearTimeout(timer); timer = setTimeout(run, 120); });
    input.addEventListener('focus', function () { if (input.value.trim()) { run(); } });
    input.addEventListener('keydown', function (e) {
      var items = $$('a[data-slug]', box);
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        if (!items.length) { return; }
        e.preventDefault();
        active = (active + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
        items.forEach(function (el, i) { el.classList.toggle('is-active', i === active); });
      } else if (e.key === 'Enter') {
        e.preventDefault();
        var go = items[active] || items[0];
        if (go) { onPick(go.dataset.slug, go); }
      } else if (e.key === 'Escape') {
        close(); input.blur();
      }
    });
    box.addEventListener('click', function (e) {
      var a = e.target.closest('a[data-slug]');
      if (a) { e.preventDefault(); onPick(a.dataset.slug, a); }
    });
    document.addEventListener('click', function (e) {
      if (!box.contains(e.target) && e.target !== input) { close(); }
    });
  }

  var wikiInput = $('[data-wiki-search]');
  if (wikiInput) {
    searchBox(wikiInput, $('#wiki-results'), function (s) { location.href = '/wiki/' + s; });
  }
  document.addEventListener('keydown', function (e) {
    var typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement.tagName) || document.activeElement.isContentEditable;
    if (e.key === '/' && !typing && wikiInput) { e.preventDefault(); wikiInput.focus(); }
  });

  $$('[data-pin-form]').forEach(function (f) {
    f.addEventListener('submit', function (e) { e.preventDefault(); });
    searchBox($('[data-pin-input]', f), $('[data-pin-results]', f), function (s) {
      api('POST', '/api/projects/' + f.dataset.project + '/notes', { slug: s }).then(function () { reloadSoon(); }).catch(fail);
    });
  });
  document.addEventListener('click', function (e) {
    var b = e.target.closest('[data-unpin]');
    if (!b) { return; }
    api('DELETE', '/api/projects/' + b.dataset.project + '/notes/' + b.dataset.unpin).then(function () { reloadSoon(); }).catch(fail);
  });

  // ---------------------------------------------------------------- focus & sync
  var focusSel = $('[data-focus-select]');
  if (focusSel) {
    focusSel.addEventListener('change', function () {
      api('POST', '/api/settings/focus', { value: focusSel.value }).then(function () { reloadSoon(); }).catch(fail);
    });
  }
  $$('[data-set-focus]').forEach(function (b) {
    b.addEventListener('click', function () {
      api('POST', '/api/settings/focus', { value: b.dataset.setFocus }).then(function () {
        toast(b.dataset.setFocus ? 'Learning focus set' : 'Learning focus back to automatic');
        reloadSoon(600);
      }).catch(fail);
    });
  });

  $$('[data-sync]').forEach(function (b) {
    b.addEventListener('click', function () {
      if (b.classList.contains('is-busy')) { return; }
      b.classList.add('is-busy');
      var label = $('[data-sync-label]', b);
      if (label) { label.textContent = 'Syncing…'; }
      b.disabled = true;
      api('POST', '/api/sync').then(function (r) {
        if (r.errors && r.errors.length) { toast(r.errors.join(' · '), 'error'); }
        else { toast((r.lines || []).join(' · ') || 'Synced'); }
        if (r.warnings && r.warnings.length) { toast(r.warnings.length + ' warning(s), see the sync log', 'warn'); }
        reloadSoon(1200);
      }).catch(function (err) { fail(err); b.disabled = false; b.classList.remove('is-busy'); });
    });
  });

  // ---------------------------------------------------------------- library
  var titleDialog = $('#title-dialog');
  $$('[data-add-title]').forEach(function (b) {
    b.addEventListener('click', function () {
      var f = $('[data-title-form]', titleDialog);
      f.elements.kind.value = b.dataset.addTitle;
      f.elements.title.value = '';
      titleDialog.showModal();
      f.elements.title.focus();
    });
  });
  if (titleDialog) {
    $('[data-title-form]', titleDialog).addEventListener('submit', function (e) {
      e.preventDefault();
      var f = e.target, btn = $('.btn--primary', f);
      btn.disabled = true;
      api('POST', '/api/library', { kind: f.elements.kind.value, title: f.elements.title.value })
        .then(function (d) { location.href = '/library/' + d.id; })
        .catch(function (err) { btn.disabled = false; fail(err); });
    });
  }

  var item = $('[data-item]');
  if (item) {
    var id = item.dataset.item, url = '/api/library/' + id;
    var scoreForm = $('[data-score-form]', item);
    scoreForm.addEventListener('submit', function (e) {
      e.preventDefault();
      var v = scoreForm.elements.score.value.trim();
      var n = v === '' ? null : Number(v.replace(',', '.'));
      if (n !== null && (isNaN(n) || n < 0 || n > 10)) { toast('Score must be between 0 and 10', 'error'); return; }
      api('PATCH', url, { user_score: n }).then(function () { reloadSoon(); }).catch(fail);
    });
    $('[data-item-status]', item).addEventListener('change', function (e) {
      api('PATCH', url, { status: e.target.value || null }).then(function () { toast('Saved'); }).catch(fail);
    });
    var sum = $('[data-summary-form]', item);
    if (sum) {
      sum.addEventListener('submit', function (e) {
        e.preventDefault();
        api('PATCH', url, { summary: sum.elements.summary.value }).then(function () { toast('Summary saved'); }).catch(fail);
      });
    }
    $('[data-lookup-form]', item).addEventListener('submit', function (e) {
      e.preventDefault();
      var btn = $('.btn', e.target);
      btn.disabled = true;
      btn.textContent = 'Looking up…';
      api('PATCH', url, { lookup_override: e.target.elements.lookup.value || null })
        .then(function () { reloadSoon(); })
        .catch(function (err) { btn.disabled = false; btn.textContent = 'Look up'; fail(err); });
    });
    var del = $('[data-item-delete]', item);
    if (del) {
      del.addEventListener('click', function () {
        if (!confirm('Remove this title from the library?')) { return; }
        api('DELETE', url).then(function () { location.href = '/library'; }).catch(fail);
      });
    }
  }
})();
