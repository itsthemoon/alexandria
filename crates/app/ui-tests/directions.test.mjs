import fs from 'fs';
const src = fs.readFileSync(new URL('../ui/map.js', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const start = src.indexOf('function formatDistanceM');
const end = src.indexOf('function renderSteps');
if (start < 0 || end < 0) { console.log('FAIL directions helpers missing'); process.exit(1); }
const h = new Function(
  src.slice(start, end) + 'return { formatDistanceM, formatKm, formatDurationS, routeSummary, routeGeoJSON, routeBounds, routePadding, routeMarkerData, stepItems, drawRoute, clearDirections };'
)();
let bad = 0;
const check = (name, ok) => { console.log((ok ? 'ok   ' : 'FAIL ') + name); if (!ok) bad++; };
check('short distances show metres', h.formatDistanceM(100) === '100 m');
check('a half mile shows one decimal', h.formatDistanceM(800) === '0.5 mi');
check('a mile rounds to one decimal', h.formatDistanceM(1609) === '1.0 mi');
check('long distances drop the decimal', h.formatDistanceM(182000) === '113 mi');
check('metres under a kilometre', h.formatKm(450) === '450 m');
check('small kilometres keep a decimal', h.formatKm(5200) === '5.2 km');
check('large kilometres round', h.formatKm(182000) === '182 km');
check('durations under an hour are minutes', h.formatDurationS(300) === '5 min');
check('durations over an hour show hours and minutes', h.formatDurationS(7500) === '2 hr 5 min');
check('summary shows both units and time', h.routeSummary({ distance_m: 182000, duration_s: 7500 }) === '182 km · 113 mi · 2 hr 5 min');
const geo = h.routeGeoJSON([[-73.9, 40.6], [-73.8, 40.7]]);
check('route is a LineString feature collection', geo.type === 'FeatureCollection' && geo.features[0].geometry.type === 'LineString');
check('coordinates pass through as lon/lat', JSON.stringify(geo.features[0].geometry.coordinates) === '[[-73.9,40.6],[-73.8,40.7]]');
check('bounds span the line corners', JSON.stringify(h.routeBounds([[-73.9, 40.7], [-73.8, 40.6], [-73.85, 40.65]])) === '[[-73.9,40.6],[-73.8,40.7]]');
const steps = h.stepItems({ steps: [{ text: 'Head north', distance_m: 800, at: [-73.9, 40.6] }, { text: 'Turn right', distance_m: 1609, at: [-73.8, 40.7] }] });
check('steps are numbered in order', steps[0].n === 1 && steps[1].n === 2);
check('step distance is formatted', steps[0].dist === '0.5 mi' && steps[1].dist === '1.0 mi');
check('step keeps its point for fly-to', JSON.stringify(steps[1].at) === '[-73.8,40.7]');
check('a route without steps gives an empty list', h.stepItems({ steps: [] }).length === 0);
check('a route with no steps field gives an empty list', h.stepItems({}).length === 0);
const line = [[-73.9, 40.7], [-73.85, 40.65], [-73.8, 40.6]];
const marks = h.routeMarkerData(line);
check('markers are two point features', marks.type === 'FeatureCollection' && marks.features.length === 2 && marks.features.every(f => f.geometry.type === 'Point'));
check('start marker sits on the first coordinate', JSON.stringify(marks.features[0].geometry.coordinates) === '[-73.9,40.7]' && marks.features[0].properties.kind === 'start');
check('end marker sits on the last coordinate', JSON.stringify(marks.features[1].geometry.coordinates) === '[-73.8,40.6]' && marks.features[1].properties.kind === 'end');
const panelRect = { width: 320, right: 450, left: 130 };
const mapRect = { left: 116 };
const container = { getBoundingClientRect: () => mapRect };
const panel = { getBoundingClientRect: () => panelRect };
check('left padding clears the panel plus a margin', h.routePadding(container, panel).left === 358);
check('left padding is at least the panel width', h.routePadding(container, panel).left >= panelRect.width);
check('hidden panel falls back to even padding', h.routePadding(container, { getBoundingClientRect: () => ({ width: 0, right: 0 }) }).left === 80);
const sources = {};
const layers = [];
const removed = [];
let lastFit = null;
const fakeMap = {
  getSource: (id) => sources[id] || null,
  addSource: (id, def) => { sources[id] = def; },
  addLayer: (l) => layers.push(l),
  getLayer: (id) => layers.find(l => l.id === id) || null,
  removeLayer: (id) => removed.push(id),
  removeSource: (id) => removed.push(id),
  getContainer: () => ({ getBoundingClientRect: () => mapRect, querySelector: () => panel }),
  fitBounds: (b, o) => { lastFit = { b, o }; },
};
globalThis.mapView = fakeMap;
globalThis.document = { getElementById: () => ({ textContent: '' }) };
h.drawRoute({ line });
check('drawRoute frames with left padding at least the panel width', !!lastFit && lastFit.o.padding.left >= panelRect.width);
check('drawRoute draws start and end circle layers', layers.some(l => l.id === 'route-start' && l.type === 'circle') && layers.some(l => l.id === 'route-end' && l.type === 'circle'));
check('route line uses the bright accent colour', layers.find(l => l.id === 'route').paint['line-color'] === '#f1f2f4');
check('markers sit on a geojson source', sources['route-pts'] && sources['route-pts'].type === 'geojson' && sources['route-pts'].data.features.length === 2);
h.clearDirections();
check('Clear removes the marker layers and source', removed.includes('route-end') && removed.includes('route-start') && removed.includes('route-pts'));
check('Clear removes the route line too', removed.includes('route') && removed.includes('route-casing'));
process.exit(bad ? 1 : 0);
