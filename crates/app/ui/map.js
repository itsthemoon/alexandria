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
    mapView.addControl(new maplibregl.AttributionControl({ compact: true, customAttribution: '© OpenStreetMap contributors · Protomaps' }), 'bottom-right');
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

function runMapSearch() {
  const q = document.getElementById('mapsearch').value.trim().toLowerCase();
  const sq = streetQuery(q);
  const box = document.getElementById('mapresults');
  box.textContent = '';
  box.classList.remove('on');
  if (!q || !mapView || !mapView.isStyleLoaded()) return;
  let feats = [];
  try {
    feats = mapView.queryRenderedFeatures({ layers: ['place-label', 'road-label'] });
    feats = feats.concat(mapView.querySourceFeatures('pm', { sourceLayer: 'roads' }));
    feats = feats.concat(mapView.querySourceFeatures('pm', { sourceLayer: 'places' }));
    feats = feats.concat(mapView.querySourceFeatures('pm', { sourceLayer: 'pois' }));
  } catch (_) { try { feats = mapView.queryRenderedFeatures(); } catch (__) { return; } }
  const seen = new Map();
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
  const rows = [...seen.values()]
    .sort((a, b) => (b.pop || 0) - (a.pop || 0))
    .slice(0, 12);
  if (!rows.length) {
    const d = document.createElement('div');
    d.className = 'empty';
    d.textContent = 'Nothing found on this map. Try a street or place name — house addresses are not included.';
    box.appendChild(d);
  } else {
    for (const r of rows) {
      const d = document.createElement('div');
      d.textContent = `${r.name} · ${r.kind}`;
      d.onclick = async () => {
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
      };
      box.appendChild(d);
    }
  }
  box.classList.add('on');
}
