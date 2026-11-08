let mapView = null;
let mapPacks = [];
let roadIndex = [];
let mapInfo = null;

let tileRegistered = false;
function registerTiles() {
  if (tileRegistered) return;
  tileRegistered = true;
  maplibregl.addProtocol('alexglyph', async (params) => {
    const m = /^alexglyph:\/\/(.+)\/(\d+-\d+\.pbf)$/.exec(params.url);
    if (!m) throw new Error('bad glyph url');
    const buf = await window.__TAURI__.core.invoke('map_glyphs', { fontstack: m[1], range: m[2] });
    return { data: buf instanceof ArrayBuffer ? buf : new Uint8Array(buf).buffer };
  });
  maplibregl.addProtocol('alex', async (params) => {
    const m = /^alex:\/\/(\d+)\/(\d+)\/(\d+)$/.exec(params.url);
    if (!m) throw new Error('bad tile url');
    const buf = await window.__TAURI__.core.invoke('map_tile', { z: +m[1], x: +m[2], y: +m[3] });
    return { data: buf instanceof ArrayBuffer ? buf : new Uint8Array(buf).buffer };
  });
}

function rasterStyle(info) {
  return {
    version: 8,
    sources: { base: { type: 'raster', tiles: ['alex://{z}/{x}/{y}'], tileSize: 256, minzoom: info.min_zoom, maxzoom: info.max_zoom } },
    layers: [{ id: 'base', type: 'raster', source: 'base' }],
  };
}

function vectorStyle(info) {
  return {
    version: 8,
    glyphs: 'alexglyph://{fontstack}/{range}.pbf',
    sources: { pm: { type: 'vector', tiles: ['alex://{z}/{x}/{y}'], minzoom: info.min_zoom, maxzoom: info.max_zoom } },
    layers: [
      { id: 'bg', type: 'background', paint: { 'background-color': '#0e0f11' } },
      { id: 'earth', type: 'fill', source: 'pm', 'source-layer': 'earth', paint: { 'fill-color': '#16171a' } },
      { id: 'water', type: 'fill', source: 'pm', 'source-layer': 'water', paint: { 'fill-color': '#2d333c' } },
      { id: 'landuse', type: 'fill', source: 'pm', 'source-layer': 'landuse', minzoom: 8, paint: { 'fill-color': '#1c1d21', 'fill-opacity': 0.7 } },
      { id: 'buildings', type: 'fill', source: 'pm', 'source-layer': 'buildings', minzoom: 13, paint: { 'fill-color': '#25272b', 'fill-outline-color': '#303237' } },
      { id: 'boundaries', type: 'line', source: 'pm', 'source-layer': 'boundaries', paint: { 'line-color': '#7c8088', 'line-width': ['interpolate', ['linear'], ['zoom'], 1, 0.4, 6, 1.2], 'line-dasharray': [3, 2] } },
      { id: 'road-minor', type: 'line', source: 'pm', 'source-layer': 'roads', filter: ['match', ['get', 'kind'], ['minor', 'path', 'track'], true, false], paint: { 'line-color': '#3a3c41', 'line-width': ['interpolate', ['linear'], ['zoom'], 10, 0.5, 15, 3] } },
      { id: 'road-major', type: 'line', source: 'pm', 'source-layer': 'roads', filter: ['match', ['get', 'kind'], ['highway', 'major', 'medium'], true, false], paint: { 'line-color': '#9a9ea6', 'line-width': ['interpolate', ['linear'], ['zoom'], 6, 0.5, 15, 6] } },
      { id: 'road-label', type: 'symbol', source: 'pm', 'source-layer': 'roads', minzoom: 13, filter: ['all', ['!=', ['get', 'name'], ''], ['!=', ['get', 'kind'], 'path']], layout: { 'symbol-placement': 'line', 'text-field': ['coalesce', ['get', 'name:en'], ['get', 'name']], 'text-font': ['Noto Sans Regular'], 'text-size': 11 }, paint: { 'text-color': '#c9ccd2', 'text-halo-color': '#0b0b0c', 'text-halo-width': 1.2 } },
      { id: 'place-label', type: 'symbol', source: 'pm', 'source-layer': 'places', filter: ['!=', ['get', 'name'], ''], layout: { 'text-field': ['coalesce', ['get', 'name:en'], ['get', 'name']], 'text-font': ['Noto Sans Regular'], 'text-size': ['match', ['get', 'kind'], 'city', 15, 'town', 13, 'village', 12, 11] }, paint: { 'text-color': '#eceef1', 'text-halo-color': '#0b0b0c', 'text-halo-width': 1.4 } },
    ],
  };
}

async function bootMap(pack, target) {
  const status = document.getElementById('mapstatus');
  try {
    if (mapView) { mapView.remove(); mapView = null; }
    resetDirections();
    const chosen = pack || document.getElementById('mapselect').value || null;
    mapInfo = await window.__TAURI__.core.invoke('map_info', { pack: chosen });
    roadIndex = [];
    if (chosen) {
      fetch(chosen.replace(/\.pmtiles$/, '') + '.roads.json')
        .then(r => r.ok ? r.json() : [])
        .then(rows => { roadIndex = rows; })
        .catch(() => {});
    }
    registerTiles();
    const world = mapInfo.max_zoom <= 5 || mapInfo.global;
    const center = world ? [0, 20] : [(mapInfo.min_lon + mapInfo.max_lon) / 2, (mapInfo.min_lat + mapInfo.max_lat) / 2];
    mapView = new maplibregl.Map({
      container: 'map',
      style: mapInfo.kind === 'mvt' ? vectorStyle(mapInfo) : rasterStyle(mapInfo),
      center: target ? target.center : center,
      zoom: target ? Math.min(target.zoom, mapInfo.max_zoom + 1) : world ? 2 : Math.min(mapInfo.max_zoom, 14),
      minZoom: mapInfo.min_zoom,
      attributionControl: false,
    });
    mapView.addControl(new maplibregl.NavigationControl({ showCompass: false }), 'top-right');
    mapView.addControl(new maplibregl.ScaleControl({ maxWidth: 120, unit: 'imperial' }), 'bottom-left');
    mapView.addControl(new maplibregl.AttributionControl({ compact: true, customAttribution: '© OpenStreetMap contributors · Protomaps · Overture Maps' }), 'bottom-right');
    mapView.on('error', (e) => {
      const err = e && e.error;
      const msg = (err && err.message) || (typeof err === 'string' ? err : 'a map tile could not be read');
      status.textContent = 'Map problem: ' + msg;
    });
    document.getElementById('mapmeta').textContent = (mapInfo.max_zoom >= 12 ? 'Street level' : 'Overview') + ' · zoom ' + mapInfo.min_zoom + '–' + mapInfo.max_zoom;
    setTimeout(() => mapView.resize(), 50);
    document.getElementById('mapselect').onchange = () => bootMap(document.getElementById('mapselect').value);
    wireMapSearch();
  } catch (e) {
    document.getElementById('mapmeta').textContent = 'no map pack';
    status.textContent = 'Map problem: ' + e;
  }
}


function wireMapSearch() {
  const inp = document.getElementById('mapsearch');
  const box = document.getElementById('mapresults');
  let t = null;
  inp.addEventListener('input', () => {
    clearTimeout(t);
    t = setTimeout(runMapSearch, 140);
  });
  inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') runMapSearch(); });
}

function streetQuery(raw) {
  const abbr = {ct:'court',st:'street',rd:'road',dr:'drive',ln:'lane',blvd:'boulevard',ave:'avenue',pkwy:'parkway',cir:'circle',pl:'place',ter:'terrace',trl:'trail',hwy:'highway'};
  return raw.replace(/^\d+\s+/, '').replace(/\b(ct|st|rd|dr|ln|blvd|ave|pkwy|cir|pl|ter|trl|hwy)\b/g, m => abbr[m]);
}

function placeMatch(p, q) {
  const names = [p && p['name:en'], p && p.name].filter(Boolean);
  return names.some(n => n.toLowerCase().includes(q)) ? names[0] : null;
}

function streetRows(rows) {
  return (rows || []).map(r => ({ name: r.name, c: [r.lon, r.lat], kind: 'street', place: r.place || null }));
}

function mergeStreets(seen, rows) {
  streetRows(rows).forEach((r, i) => {
    const key = r.name.toLowerCase();
    if (!seen.has(key)) seen.set(key, { ...r, pop: 1000 - i });
  });
  return seen;
}

function streetNear(zoom, c) {
  return zoom >= 8 ? { lat: c.lat, lon: c.lng } : {};
}

async function collectStreetResults(q) {
  const sq = streetQuery(q);
  let feats = [];
  try {
    feats = mapView.queryRenderedFeatures({ layers: ['place-label', 'road-label'] });
    feats = feats.concat(mapView.querySourceFeatures('pm', { sourceLayer: 'roads' }));
    feats = feats.concat(mapView.querySourceFeatures('pm', { sourceLayer: 'places' }));
    feats = feats.concat(mapView.querySourceFeatures('pm', { sourceLayer: 'pois' }));
  } catch (_) { try { feats = mapView.queryRenderedFeatures(); } catch (__) { return []; } }
  const seen = new Map();
  try {
    const far = await window.__TAURI__.core.invoke('streets_search', { q: sq, limit: 8, ...streetNear(mapView.getZoom(), mapView.getCenter()) });
    mergeStreets(seen, far);
  } catch (_) {}
  for (const f of feats) {
    const name = placeMatch(f.properties, q);
    if (!name) continue;
    const key = name.toLowerCase();
    if (seen.has(key)) continue;
    const g = f.geometry;
    let c = null;
    if (g.type === 'Point') c = g.coordinates;
    else if (g.type === 'LineString' && g.coordinates.length) c = g.coordinates[Math.floor(g.coordinates.length / 2)];
    else if (g.type === 'MultiLineString' && g.coordinates.length) { const a = g.coordinates[0]; c = a[Math.floor(a.length / 2)]; }
    if (!c) continue;
    seen.set(key, { name, c, kind: f.sourceLayer === 'roads' || (f.layer && f.layer.id === 'road-label') ? 'street' : 'place' });
  }
  for (const g of roadIndex) {
    const gn = g[0].toLowerCase();
    if (!gn.includes(q) && !(sq !== q && gn.includes(sq))) continue;
    const key = gn;
    if (seen.has(key)) continue;
    seen.set(key, { name: g[0], c: [g[1], g[2]], kind: 'street', pop: 0 });
  }
  return [...seen.values()].sort((a, b) => (b.pop || 0) - (a.pop || 0)).slice(0, 12);
}

function renderStreetResults(box, rows, onPick) {
  box.textContent = '';
  box.classList.remove('on');
  if (!rows.length) {
    const d = document.createElement('div');
    d.className = 'empty';
    d.textContent = 'Nothing found on this map. Try a street or place name — house addresses are not included.';
    box.appendChild(d);
  } else {
    for (const r of rows) {
      const d = document.createElement('div');
      d.textContent = r.place ? `${r.name} · ${r.place}` : `${r.name} · ${r.kind}`;
      d.onclick = () => onPick(r);
      box.appendChild(d);
    }
  }
  box.classList.add('on');
}

async function runMapSearch() {
  const q = document.getElementById('mapsearch').value.trim().toLowerCase();
  const box = document.getElementById('mapresults');
  if (!q || !mapView || !mapView.isStyleLoaded()) {
    box.textContent = '';
    box.classList.remove('on');
    return;
  }
  const rows = await collectStreetResults(q);
  renderStreetResults(box, rows, async (r) => {
    box.classList.remove('on');
    const zoom = r.kind === 'street' ? 16 : 13;
    const best = await window.__TAURI__.core.invoke('map_for_point', { lon: r.c[0], lat: r.c[1] }).catch(() => null);
    const sel = document.getElementById('mapselect');
    if (best && best !== sel.value) {
      sel.value = best;
      await bootMap(best, { center: r.c, zoom });
    } else {
      mapView.flyTo({ center: r.c, zoom: Math.min(zoom, mapInfo.max_zoom + 1) });
    }
  });
}

let dirFrom = null;
let dirTo = null;

function setRoads(has) {
  const d = document.getElementById('directions');
  if (d) d.classList.toggle('off', !has);
}

function resetDirections() {
  dirFrom = null;
  dirTo = null;
  const f = document.getElementById('dirfrom');
  const t = document.getElementById('dirto');
  if (f) f.value = '';
  if (t) t.value = '';
  const out = document.getElementById('dirout');
  if (out) out.textContent = '';
}

function formatDistanceM(m) {
  const mi = m / 1609.344;
  if (mi < 0.1) return `${Math.round(m)} m`;
  if (mi < 10) return `${mi.toFixed(1)} mi`;
  return `${Math.round(mi)} mi`;
}

function formatKm(m) {
  if (m < 1000) return `${Math.round(m)} m`;
  if (m < 10000) return `${(m / 1000).toFixed(1)} km`;
  return `${Math.round(m / 1000)} km`;
}

function formatDurationS(s) {
  const mins = Math.round(s / 60);
  if (mins < 60) return `${mins} min`;
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return `${h} hr ${m} min`;
}

function routeSummary(route) {
  return `${formatKm(route.distance_m)} · ${formatDistanceM(route.distance_m)} · ${formatDurationS(route.duration_s)}`;
}

function routeGeoJSON(line) {
  return { type: 'FeatureCollection', features: [{ type: 'Feature', properties: {}, geometry: { type: 'LineString', coordinates: line } }] };
}

function routeBounds(line) {
  const first = line[0];
  let minLon = first[0], minLat = first[1], maxLon = first[0], maxLat = first[1];
  for (const c of line) {
    minLon = Math.min(minLon, c[0]);
    minLat = Math.min(minLat, c[1]);
    maxLon = Math.max(maxLon, c[0]);
    maxLat = Math.max(maxLat, c[1]);
  }
  return [[minLon, minLat], [maxLon, maxLat]];
}

function routePadding(container, panel) {
  const pad = { top: 40, right: 60, bottom: 60, left: 80 };
  if (!container || !panel) return pad;
  const p = panel.getBoundingClientRect();
  if (!p.width) return pad;
  pad.left = Math.ceil(p.right - container.getBoundingClientRect().left) + 24;
  return pad;
}

function routeMarkerData(line) {
  return { type: 'FeatureCollection', features: [
    { type: 'Feature', properties: { kind: 'start' }, geometry: { type: 'Point', coordinates: line[0] } },
    { type: 'Feature', properties: { kind: 'end' }, geometry: { type: 'Point', coordinates: line[line.length - 1] } },
  ] };
}

function stepItems(route) {
  return (route.steps || []).map((s, i) => ({ n: i + 1, text: s.text, dist: formatDistanceM(s.distance_m), at: s.at }));
}

function clearRouteLine() {
  if (mapView && mapView.getLayer('route')) {
    mapView.removeLayer('route');
    mapView.removeLayer('route-casing');
    mapView.removeSource('route');
  }
}

function clearRouteMarkers() {
  if (mapView && mapView.getLayer('route-end')) {
    mapView.removeLayer('route-end');
    mapView.removeLayer('route-start');
    mapView.removeSource('route-pts');
  }
}

function clearDirections() {
  clearRouteLine();
  clearRouteMarkers();
  const out = document.getElementById('dirout');
  if (out) out.textContent = '';
}

function drawRouteMarkers(line) {
  if (!line.length) return;
  const src = mapView.getSource('route-pts');
  if (src) {
    src.setData(routeMarkerData(line));
    return;
  }
  mapView.addSource('route-pts', { type: 'geojson', data: routeMarkerData(line) });
  mapView.addLayer({ id: 'route-start', type: 'circle', source: 'route-pts', filter: ['==', ['get', 'kind'], 'start'], paint: { 'circle-radius': 6, 'circle-color': '#f1f2f4', 'circle-stroke-width': 2, 'circle-stroke-color': '#0b0b0c' } });
  mapView.addLayer({ id: 'route-end', type: 'circle', source: 'route-pts', filter: ['==', ['get', 'kind'], 'end'], paint: { 'circle-radius': 6.5, 'circle-color': '#0b0b0c', 'circle-stroke-width': 2.5, 'circle-stroke-color': '#f1f2f4' } });
}

function drawRoute(route) {
  const line = route.line || [];
  const geo = routeGeoJSON(line);
  const src = mapView.getSource('route');
  if (src) {
    src.setData(geo);
  } else {
    mapView.addSource('route', { type: 'geojson', data: geo });
    mapView.addLayer({ id: 'route-casing', type: 'line', source: 'route', layout: { 'line-cap': 'round', 'line-join': 'round' }, paint: { 'line-color': '#0b0b0c', 'line-width': 8 } });
    mapView.addLayer({ id: 'route', type: 'line', source: 'route', layout: { 'line-cap': 'round', 'line-join': 'round' }, paint: { 'line-color': '#f1f2f4', 'line-width': 4.5 } });
  }
  drawRouteMarkers(line);
  if (line.length > 1) {
    const container = mapView.getContainer();
    mapView.fitBounds(routeBounds(line), { padding: routePadding(container, container.querySelector('#directions')), duration: 400 });
  }
}

function renderSteps(route) {
  const out = document.getElementById('dirout');
  out.textContent = '';
  const sum = document.createElement('div');
  sum.className = 'dirsum';
  sum.textContent = routeSummary(route);
  out.appendChild(sum);
  const items = stepItems(route);
  if (!items.length) {
    const e = document.createElement('div');
    e.className = 'dirempty';
    e.textContent = 'No turn-by-turn steps for this route.';
    out.appendChild(e);
    return;
  }
  const ol = document.createElement('ol');
  ol.className = 'dirsteps';
  for (const it of items) {
    const li = document.createElement('li');
    const b = document.createElement('button');
    b.className = 'dirstep';
    const n = document.createElement('span');
    n.className = 'dirnum';
    n.textContent = String(it.n);
    const tx = document.createElement('span');
    tx.className = 'dirtext';
    tx.textContent = it.text;
    const d = document.createElement('span');
    d.className = 'dirdist';
    d.textContent = it.dist;
    b.append(n, tx, d);
    b.onclick = () => mapView.flyTo({ center: it.at, zoom: Math.max(mapView.getZoom(), 15) });
    li.appendChild(b);
    ol.appendChild(li);
  }
  out.appendChild(ol);
}

async function runDirections() {
  const out = document.getElementById('dirout');
  if (!dirFrom || !dirTo) {
    out.textContent = '';
    const e = document.createElement('div');
    e.className = 'dirempty';
    e.textContent = 'Choose a From and a To first.';
    out.appendChild(e);
    return;
  }
  out.textContent = '';
  const wait = document.createElement('div');
  wait.className = 'dirempty';
  wait.textContent = 'Finding a route…';
  out.appendChild(wait);
  try {
    const route = await window.__TAURI__.core.invoke('route', { from: [dirFrom[1], dirFrom[0]], to: [dirTo[1], dirTo[0]] });
    drawRoute(route);
    renderSteps(route);
  } catch (e) {
    clearRouteLine();
    clearRouteMarkers();
    out.textContent = '';
    const err = document.createElement('div');
    err.className = 'dirempty';
    err.textContent = String(e);
    out.appendChild(err);
  }
}

async function dirLookup(input, box, onPick) {
  const q = input.value.trim().toLowerCase();
  box.textContent = '';
  box.classList.remove('on');
  if (!q || !mapView || !mapView.isStyleLoaded()) return;
  const rows = await collectStreetResults(q);
  renderStreetResults(box, rows, (r) => {
    box.classList.remove('on');
    onPick(r);
  });
}

function wireDirSearch(inputId, resultsId, clearPoint, onPick, onEnter) {
  const input = document.getElementById(inputId);
  const box = document.getElementById(resultsId);
  let t = null;
  input.addEventListener('input', () => {
    clearPoint();
    clearTimeout(t);
    t = setTimeout(() => dirLookup(input, box, onPick), 140);
  });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (onEnter) onEnter(); else dirLookup(input, box, onPick);
    }
  });
}

function wireDirections() {
  wireDirSearch('dirfrom', 'dirfromresults', () => { dirFrom = null; }, (r) => {
    dirFrom = r.c;
    document.getElementById('dirfrom').value = r.name;
  }, null);
  wireDirSearch('dirto', 'dirtoresults', () => { dirTo = null; }, (r) => {
    dirTo = r.c;
    document.getElementById('dirto').value = r.name;
  }, () => runDirections());
  document.getElementById('dirgo').onclick = runDirections;
  document.getElementById('dirclear').onclick = clearDirections;
}

wireDirections();
