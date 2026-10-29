let browseOffset = 0;
let browseTimer = null;

async function loadBrowse(reset) {
  if (reset) browseOffset = 0;
  const list = $('browslist');
  $('browsemeta').textContent = 'Loading…';
  try {
    const page = await invoke('browse', {
      pack: $('pack').value || null,
      sort: $('browsesort').value,
      q: $('browsefilter').value,
      offset: browseOffset,
      limit: 100,
    });
    list.textContent = '';
    for (const it of page.items) {
      const li = el('li', '', it.title);
      li.onclick = () => openArticle(it.url);
      list.appendChild(li);
    }
    const from = page.total ? browseOffset + 1 : 0;
    const to = Math.min(browseOffset + page.items.length, page.total);
    $('browsemeta').textContent = page.total
      ? `${from.toLocaleString()}–${to.toLocaleString()} of ${page.total.toLocaleString()} articles`
      : 'No titles match that filter.';
    $('browseprev').disabled = browseOffset === 0;
    $('browsenext').disabled = to >= page.total;
  } catch (e) {
    $('browsemeta').textContent = String(e);
  }
}
$('browsesort').onchange = () => loadBrowse(true);
$('browsefilter').addEventListener('input', () => {
  clearTimeout(browseTimer);
  browseTimer = setTimeout(() => loadBrowse(true), 250);
});
$('browseprev').onclick = () => { browseOffset = Math.max(0, browseOffset - 100); loadBrowse(false); };
$('browsenext').onclick = () => { browseOffset += 100; loadBrowse(false); };

function renderRich(node, text) {
  node.textContent = '';
  text.split('**').forEach((part, i) => {
    if (!part) return;
    node.appendChild(i % 2 ? el('b', '', part) : document.createTextNode(part));
  });
}

let asking = false;

async function askModel(q) {
  if (asking) return;
  asking = true;
  $('answer').classList.remove('off');
  const answer = $('answertext');
  answer.textContent = '';
  answer.classList.add('thinking');
  answer.dataset.wait = 'Reading the library\u2026';
  $('sources').textContent = '';
  const ch = new Channel();
  ch.onmessage = (t) => {
    if ($('q').value !== q) return;
    answer.classList.remove('thinking');
    renderRich(answer, t);
  };
  try {
    const res = await invoke('ask', { question: q, pack: null, onText: ch });
    if ($('q').value !== q) return;
    answer.classList.remove('thinking');
    renderRich(answer, res.text);
    res.sources.forEach((s, i) => {
      const b = el('button', 'source');
      b.append(el('span', 'n', String(i + 1)), el('span', '', s.title));
      b.onclick = () => openArticle(s.url, s.pack);
      $('sources').appendChild(b);
    });
  } catch (e) {
    answer.classList.remove('thinking');
    answer.textContent = String(e);
  } finally {
    asking = false;
  }
}

let tTimer = null;
let tLast = '';

async function loadLanguages() {
  const langs = await invoke('languages').catch(() => ['English']);
  const sel = $('tlang');
  for (const l of langs) sel.appendChild(Object.assign(el('option', '', l), { value: l }));
  const mine = new Intl.DisplayNames(['en'], { type: 'language' }).of((navigator.language || 'en').split('-')[0]);
  sel.value = langs.includes(mine) ? mine : 'English';
}

async function liveTranslate(now) {
  clearTimeout(tTimer);
  const run = async () => {
    const text = $('tin').value;
    const key = `${$('tlang').value}\u0000${text}`;
    if (!text.trim()) { $('tout').textContent = ''; $('tstatus').textContent = ''; tLast = ''; return; }
    if (key === tLast) return;
    tLast = key;
    const out = $('tout');
    $('tstatus').textContent = 'Translating…';
    out.classList.add('stale');
    let first = true;
    const ch = new Channel();
    ch.onmessage = (t) => {
      if (tLast !== key) return;
      if (first) { out.classList.remove('stale'); first = false; }
      out.textContent = t;
    };
    try {
      const res = await invoke('translate', { text, target: $('tlang').value, onText: ch });
      if (tLast !== key) return;
      out.classList.remove('stale');
      renderRich(out, res);
      $('tstatus').textContent = '';
    } catch (e) {
      if (tLast !== key) return;
      out.classList.remove('stale');
      $('tstatus').textContent = String(e) === 'cancelled' ? '' : String(e);
    }
  };
  if (now) run(); else tTimer = setTimeout(run, 650);
}
$('tin').addEventListener('input', () => liveTranslate(false));
$('tlang').onchange = () => liveTranslate(true);
loadLanguages();

function toUint8(data) {
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  if (Array.isArray(data)) return new Uint8Array(data);
  if (data && typeof data === 'object' && typeof data.length === 'number') {
    const out = new Uint8Array(data.length);
    for (let i = 0; i < data.length; i++) out[i] = data[i] || 0;
    return out;
  }
  return new Uint8Array(data);
}

function mimeFor(name) {
  const ext = name.split('.').pop().toLowerCase();
  return ext === 'png' ? 'image/png' : ext === 'jpg' || ext === 'jpeg' ? 'image/jpeg'
    : ext === 'webp' ? 'image/webp' : ext === 'gif' ? 'image/gif' : 'application/octet-stream';
}

function fileSize(n) {
  if (n > 1e9) return (n / 1e9).toFixed(1) + ' GB';
  if (n > 1e6) return (n / 1e6).toFixed(1) + ' MB';
  if (n > 1e3) return (n / 1e3).toFixed(0) + ' KB';
  return n + ' B';
}

async function loadPersonal() {
  const grid = $('personalgrid');
  grid.textContent = '';
  grid.className = '';
  try {
    const items = await invoke('personal_list');
    if (!items.length) {
      grid.className = 'empty';
      grid.textContent = 'Nothing here yet. Add photos of documents, prescriptions or maps you want with you.';
      return;
    }
    for (const it of items) {
      const card = el('button', 'filecard');
      card.title = it.name;
      if (it.kind === 'image') {
        try {
          const bytes = await invoke('personal_read', { name: it.name });
          const img = el('img');
          img.src = URL.createObjectURL(new Blob([toUint8(bytes)], { type: mimeFor(it.name) }));
          img.alt = it.name;
          img.onerror = () => { img.remove(); card.classList.add('nopreview'); };
          card.appendChild(img);
        } catch (_) { card.classList.add('nopreview'); }
      } else {
        card.appendChild(el('span', 'fileicon', it.kind === 'pdf' ? 'PDF' : it.kind === 'text' ? 'TXT' : 'FILE'));
      }
      card.append(el('span', 'filelabel', it.name), el('span', 'filesize', fileSize(it.size)));
      card.onclick = () => {
        if (it.kind === 'image') showImage(it.name);
        else if (it.kind === 'pdf' || it.kind === 'text') {
          invoke('personal_open', { name: it.name }).catch(e => { $('personalview').textContent = String(e); });
        } else {
          invoke('personal_reveal', { name: it.name }).catch(e => { $('personalview').textContent = String(e); });
        }
      };
      grid.appendChild(card);
    }
  } catch (e) {
    grid.textContent = String(e);
  }
}

async function showImage(name) {
  const view = $('personalview');
  view.textContent = '';
  try {
    const bytes = await invoke('personal_read', { name });
    const img = el('img', 'lightbox');
    img.src = URL.createObjectURL(new Blob([toUint8(bytes)], { type: mimeFor(name) }));
    img.alt = name;
    img.onclick = () => img.remove();
    view.appendChild(img);
  } catch (e) { view.textContent = String(e); }
}

$('personalfiles').onchange = async (e) => {
  for (const f of [...e.target.files]) {
    try {
      const bytes = [...new Uint8Array(await f.arrayBuffer())];
      await invoke('personal_save', { name: f.name, bytes });
    } catch (err) {
      $('personalview').textContent = `${f.name}: ${err}`;
    }
  }
  e.target.value = '';
  loadPersonal();
};
