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
process.exit(bad ? 1 : 0);
