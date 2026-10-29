const invoke = window.__TAURI__.core.invoke;
const Channel = window.__TAURI__.core.Channel;
const $ = (id) => document.getElementById(id);

let info = null;
let searchTimer = null;
let searchRun = 0;
let articleText = '';
let articleStack = [];
let currentArticle = null;
let fontScale = 1;
let badPacks = new Set();

const SUGGESTIONS = [['First aid', 'First aid'], ['Clean water', 'Water purification'], ['CPR (restart a heart)', 'Cardiopulmonary resuscitation'], ['How do I treat a burn?', 'How do I treat a burn?'], ['Solar power', 'Solar power'], ['Knots', 'Knot'], ['Fire', 'Fire']];

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

const BLANK = '<!doctype html><html style="background:#0e0f11"></html>';

function busy() {
  const home = !$('tab-home').classList.contains('off');
  const reading = !$('reader').classList.contains('off');
  document.body.classList.toggle('busy', !home || reading || !!$('q').value.trim());
}

function showTab(name) {
  for (const b of document.querySelectorAll('.rail [data-tab]')) b.classList.toggle('on', b.dataset.tab === name);
  for (const s of document.querySelectorAll('.tab')) s.classList.toggle('off', s.id !== `tab-${name}`);
  busy();
  if (name === 'browse' && !$('browslist').childElementCount) loadBrowse(true);
  if (name === 'files') loadPersonal();
  if (name === 'translate') setTimeout(() => $('tin').focus(), 0);
  if (name === 'home' && $('reader').classList.contains('off')) setTimeout(() => $('q').focus(), 0);
  if (name === 'map') {
    if (!mapView) bootMap();
    else setTimeout(() => mapView.resize(), 30);
  }
}
for (const b of document.querySelectorAll('.rail [data-tab]')) b.onclick = () => showTab(b.dataset.tab);

function packLabel(path) {
  return (info?.packs || []).find(p => p.path === path)?.label || '';
}

function gb(n) {
  return n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.max(1, Math.round(n / 1e6))} MB`;
}

function published() {
  const d = new Date(`${info.date}T00:00:00`);
  return isNaN(d) ? info.date : d.toLocaleDateString(undefined, { year: 'numeric', month: 'short' });
}

function paras(lines) {
  const body = $('sheetbody');
  body.textContent = '';
  for (const l of lines) {
    if (Array.isArray(l)) {
      const ul = el('ul');
      for (const i of l) ul.appendChild(el('li', '', i));
      body.appendChild(ul);
    } else body.appendChild(el('p', '', l));
  }
}

function renderStatus(error) {
  const bad = !!error || badPacks.size > 0;
  $('status').classList.toggle('bad', bad);
  $('statuslabel').textContent = bad ? 'Source problem' : 'Sources verified';
  $('statusmark').setAttribute('d', bad ? 'M10 6.5v4.5M10 13.6v.1' : 'M7 10l2 2 4-4');
  $('sheettitle').textContent = bad ? (error ? 'This library can\u2019t be verified' : 'Some sources can\u2019t be verified') : 'Sources verified';
  if (error) {
    paras(['Alexandria could not confirm this library is exactly what it published. It may be damaged or changed, so nothing from it is shown.', 'To fix it, download the library again.']);
    $('sheetmeta').textContent = String(error);
  } else if (bad) {
    paras(['These don\u2019t match what Alexandria published:', [...badPacks].map(p => packLabel(p) || p), 'They may be damaged or changed. Alexandria won\u2019t show them, so you never read something wrong.', 'Everything else is still verified and safe to use.', 'To fix it, download the library again.']);
  } else {
    paras(['Every guide, article and map here is exactly what Alexandria published. Nothing is missing, broken or changed.', 'Alexandria checks the library when it opens, and checks each article again when you open it. If anything has been changed, even by accident, it warns you and won\u2019t show it.']);
  }
  if (info && !error) {
    const n = (info.packs || []).length + (info.maps || []).length;
    const size = (info.files || []).reduce((a, f) => a + f.size, 0);
    $('sheetmeta').textContent = `Published ${published()} \u00b7 ${n} collections \u00b7 ${gb(size)}`;
  }
}

function reportProblem(pack, e) {
  if (!pack || !/verif|signature|hash|mismatch/i.test(String(e))) return;
  badPacks.add(pack);
  renderStatus();
}

function openSheet() {
  $('sheet').classList.remove('off');
  $('sheetok').focus();
}
function closeSheet() {
  $('sheet').classList.add('off');
  $('status').focus();
}
$('status').onclick = openSheet;
$('sheetok').onclick = closeSheet;
$('sheet').onclick = (e) => { if (e.target === $('sheet')) closeSheet(); };
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !$('sheet').classList.contains('off')) closeSheet(); });

async function boot(path) {
  try {
    info = await invoke('library_info', { path: path || null });
    badPacks = new Set();
    const libsel = $('library');
    libsel.textContent = '';
    for (const l of info.libraries || []) libsel.appendChild(Object.assign(el('option', '', l.label), { value: l.path }));
    libsel.value = info.path;
    libsel.classList.toggle('off', (info.libraries || []).length < 2);
    const sel = $('pack');
    sel.textContent = '';
    for (const p of info.packs || []) sel.appendChild(Object.assign(el('option', '', p.label), { value: p.path }));
    sel.classList.toggle('off', (info.packs || []).length < 2);
    mapPacks = info.maps || [];
    const ms = $('mapselect');
    ms.textContent = '';
    for (const p of mapPacks) ms.appendChild(Object.assign(el('option', '', p.label), { value: p.path }));
    if (mapPacks.some(p => p.path === 'planet.pmtiles')) ms.value = 'planet.pmtiles';
    else if (mapPacks.some(p => p.path === 'world.pmtiles')) ms.value = 'world.pmtiles';
    ms.classList.toggle('off', mapPacks.length < 2);
    renderStatus();
  } catch (e) {
    info = null;
    renderStatus(e);
  }
}

$('library').onchange = async (e) => {
  await boot(e.target.value);
  closeArticle();
  if (mapView) { mapView.remove(); mapView = null; }
  $('browslist').textContent = '';
  clearSearch();
  showTab('home');
};
$('pack').onchange = () => {
  $('browslist').textContent = '';
  loadBrowse(true);
};

function fitArt() {
  const pre = $('art');
  const lines = pre.textContent.split('\n');
  const cols = Math.max(...lines.map(l => l.length));
  if (!cols) return;
  const box = pre.parentElement;
  pre.style.fontSize = `${Math.max(box.clientWidth / (cols * 0.602), box.clientHeight / (lines.length * 1.2))}px`;
}
fetch('art.txt').then(r => r.text()).then(t => { $('art').textContent = t.replace(/\n+$/, ''); fitArt(); }).catch(() => {});
addEventListener('resize', fitArt);

const isQuestion = (q) => /\?\s*$|^(how|what|why|when|where|who|which|can|should|is|are|do|does)\b/i.test(q.trim());

const STOP = new Set(['how', 'do', 'does', 'what', 'why', 'when', 'where', 'who', 'which', 'can', 'should', 'is', 'are', 'the', 'to', 'of', 'an', 'in', 'on', 'for', 'and', 'or', 'my', 'me', 'it', 'you', 'your']);

function rankHits(q, lists) {
  const sing = (w) => w.length > 3 && w.endsWith('s') && !w.endsWith('ss') ? w.slice(0, -1) : w;
  const toks = (t) => t.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(w => w.length > 1).map(sing);
  const qt = toks(q);
  const score = (t) => {
    const tt = toks(t);
    if (!qt.length) return 0;
    if (tt.join(' ') === qt.join(' ')) return 3;
    if (qt.every((w, i) => tt[i] === w)) return 2;
    if (qt.every(w => tt.includes(w))) return 1;
    const key = qt.filter(w => !STOP.has(w));
    return key.length ? 0.9 * key.filter(w => tt.includes(w)).length / key.length : 0;
  };
  const all = [];
  lists.forEach((hits, pi) => hits.forEach((h, rank) => all.push({ ...h, pack: info.packs[pi].path, s: score(h.title), rank, pi })));
  const repair = /\b(fix|repair|replac|broken|battery|screen|disassembl|teardown)/i.test(q);
  const offTopic = (h) => isQuestion(q) && !repair && /ifixit/.test(h.pack) && h.s === 0;
  const full = all.some(h => h.all) ? all.filter(h => h.all) : all;
  const keep = full.filter(h => !offTopic(h));
  keep.sort((a, b) => b.s - a.s || a.rank - b.rank || a.pi - b.pi);
  const seen = new Set();
  return keep.filter(h => !seen.has(h.title.toLowerCase()) && seen.add(h.title.toLowerCase())).slice(0, 24);
}

async function runSearch(q) {
  const run = ++searchRun;
  busy();
  const empty = !q.trim();
  $('chips').classList.toggle('off', !empty);
  $('count').classList.toggle('off', empty);
  if (empty || !info) { $('results').textContent = ''; return; }
  let failed = 0;
  const lists = await Promise.all(info.packs.map(p => invoke('search_articles', { q, pack: p.path, limit: 6 }).catch((e) => { failed++; reportProblem(p.path, e); return []; })));
  if (run !== searchRun) return;
  const hits = rankHits(q, lists);
  const list = $('results');
  list.textContent = '';
  const hidden = failed ? ' Some collections can\u2019t be verified, so they aren\u2019t shown.' : '';
  $('count').textContent = (hits.length ? `${hits.length} articles about this.` : 'Nothing found. Try fewer or different words.') + hidden;
  for (const h of hits) {
    const li = el('li');
    const b = el('button', 'result');
    b.append(el('small', '', packLabel(h.pack)), el('b', '', h.title));
    if (h.snippet) b.append(el('span', '', h.snippet.replace(/\s+/g, ' ')));
    b.onclick = () => openArticle(h.url, h.pack);
    li.appendChild(b);
    list.appendChild(li);
  }
}

function clearSearch() {
  $('q').value = '';
  $('answer').classList.add('off');
  runSearch('');
}

$('q').addEventListener('input', (e) => {
  clearTimeout(searchTimer);
  const q = e.target.value;
  if (!q.trim()) $('answer').classList.add('off');
  searchTimer = setTimeout(() => runSearch(q), 220);
});
$('q').addEventListener('keydown', (e) => { if (e.key === 'Escape') clearSearch(); });
$('searchform').onsubmit = (e) => {
  e.preventDefault();
  clearTimeout(searchTimer);
  const q = $('q').value;
  if (!q.trim()) return;
  runSearch(q);
  if (isQuestion(q)) askModel(q);
  else $('answer').classList.add('off');
};

for (const [label, s] of SUGGESTIONS) {
  const b = el('button', 'chip', label);
  b.onclick = () => { $('q').value = s; $('searchform').requestSubmit(); };
  $('chips').appendChild(b);
}

const mac = navigator.platform.toUpperCase().includes('MAC');
if (mac) document.body.classList.add('mac');
document.addEventListener('keydown', (e) => {
  if ((mac ? e.metaKey : e.ctrlKey) && e.key.toLowerCase() === 'k') {
    e.preventDefault();
    closeArticle();
    showTab('home');
    $('q').select();
  }
});

function closeArticle() {
  currentArticle = null;
  articleStack = [];
  articleText = '';
  $('reader').classList.add('off');
  $('home').classList.remove('off');
  $('body').srcdoc = BLANK;
  busy();
}

function applyFont() {
  const doc = $('body').contentDocument;
  if (doc && doc.body) doc.body.style.fontSize = `${17 * fontScale}px`;
}
$('body').addEventListener('load', applyFont);

setInterval(() => {
  const w = $('body').contentWindow;
  let h = '';
  try { h = w.location.hash; } catch (_) { return; }
  if (!h.startsWith('#a:')) return;
  w.location.hash = '';
  let url = h.slice(3);
  try { url = decodeURIComponent(url); } catch (_) { return; }
  if (url && currentArticle) openArticle(url, currentArticle.pack);
}, 120);

async function openArticle(url, pack, push = true) {
  showTab('home');
  $('home').classList.add('off');
  $('reader').classList.remove('off');
  busy();
  if (push && currentArticle && currentArticle.url !== url) articleStack.push(currentArticle);
  $('title').textContent = 'Opening…';
  $('body').srcdoc = BLANK;
  $('path').textContent = '';
  try {
    const a = await invoke('read_article', { url, pack: pack || null });
    $('title').textContent = a.title;
    $('path').textContent = packLabel(pack);
    $('body').srcdoc = a.html;
    articleText = a.text;
    currentArticle = { url: a.url, title: a.title, pack };
  } catch (e) {
    reportProblem(pack, e);
    $('title').textContent = 'This article can\u2019t be shown';
    $('path').textContent = String(e);
    $('body').srcdoc = BLANK;
  }
}

$('back').onclick = () => {
  const prev = articleStack.pop();
  if (prev) openArticle(prev.url, prev.pack, false);
  else {
    closeArticle();
    $('q').focus();
  }
};
$('smaller').onclick = () => { fontScale = Math.max(0.8, fontScale - 0.1); applyFont(); };
$('larger').onclick = () => { fontScale = Math.min(1.6, fontScale + 0.1); applyFont(); };
$('totranslate').onclick = () => {
  if (!articleText) return;
  $('tin').value = articleText.slice(0, 6000);
  showTab('translate');
  liveTranslate(true);
};
$('toask').onclick = () => {
  if (!currentArticle) return;
  const t = `${currentArticle.title}: `;
  closeArticle();
  $('q').value = t;
  $('answer').classList.add('off');
  runSearch('');
  $('q').focus();
  $('q').setSelectionRange(t.length, t.length);
  busy();
};

boot();
busy();
