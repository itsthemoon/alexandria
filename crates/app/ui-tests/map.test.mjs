import fs from 'fs';
const src = fs.readFileSync(new URL('../ui/map.js', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const start = src.indexOf('function placeMatch');
if (start < 0) { console.log('FAIL placeMatch missing'); process.exit(1); }
const end = src.indexOf('\n}\n', start) + 2;
const placeMatch = new Function(src.slice(start, end) + 'return placeMatch;')();
let bad = 0;
const check = (name, ok) => { console.log((ok ? 'ok   ' : 'FAIL ') + name); if (!ok) bad++; };
check('English name matches and is shown', placeMatch({ name: 'القاهرة', 'name:en': 'Cairo' }, 'cairo') === 'Cairo');
check('local name still matches', placeMatch({ name: 'München', 'name:en': 'Munich' }, 'münchen') === 'Munich');
check('no match gives null', placeMatch({ name: 'Paris' }, 'cairo') === null);
check('missing properties give null', placeMatch(null, 'cairo') === null);
const streetStart = src.indexOf('function streetRows');
const streetEnd = src.indexOf('async function runMapSearch');
if (streetStart < 0 || streetEnd < 0) { console.log('FAIL street helpers missing'); process.exit(1); }
const { streetRows, mergeStreets, streetNear } = new Function(
  src.slice(streetStart, streetEnd) + 'return { streetRows, mergeStreets, streetNear };'
)();
check('world view ranks streets by town size, not distance', JSON.stringify(streetNear(2, { lat: 20, lng: 0 })) === '{}');
check('zoomed in ranks streets by distance', JSON.stringify(streetNear(10, { lat: 48.8, lng: 2.3 })) === '{"lat":48.8,"lon":2.3}');
const flat = streetRows([{ name: 'Lexington Avenue', lat: 40.76, lon: -73.97, place: 'New York, US' }]);
check('street coordinates go lon then lat', flat[0].c[0] === -73.97 && flat[0].c[1] === 40.76);
check('streets are tagged as streets', flat[0].kind === 'street');
check('a place label is carried through', flat[0].place === 'New York, US');
check('a row without a place has none', streetRows([{ name: 'X', lat: 0, lon: 0 }])[0].place === null);
const seen = new Map();
mergeStreets(seen, [{ name: 'Bedford Avenue', lat: 40.71, lon: -73.96 }, { name: 'bedford avenue', lat: 40.72, lon: -73.95 }]);
check('one result per street name', seen.size === 1);
const keep = new Map([['court street', { name: 'Court Street', c: [0, 0], kind: 'street' }]]);
mergeStreets(keep, [{ name: 'Court Street', lat: 40.6, lon: -73.9 }]);
check('a street already on the map keeps its place', keep.get('court street').c[0] === 0);
const ranked = mergeStreets(new Map(), [{ name: 'First', lat: 1, lon: 2 }, { name: 'Second', lat: 3, lon: 4 }]);
check('sidecar order survives sorting', ranked.get('first').pop > ranked.get('second').pop);
process.exit(bad ? 1 : 0);
