import fs from 'fs';
const src = fs.readFileSync(new URL('../ui/app.js', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const start = src.indexOf('function rewriteLinks');
if (start < 0) { console.log('FAIL rewriteLinks missing'); process.exit(1); }
const end = src.indexOf('\n}\n', start) + 2;
const rewriteLinks = new Function(src.slice(start, end) + 'return rewriteLinks;')();
let bad = 0;
const check = (name, ok) => { console.log((ok ? 'ok   ' : 'FAIL ') + name); if (!ok) bad++; };
check('an article link becomes a top-level article-scheme link that carries its pack', rewriteLinks('<a class="al" href="#a:Kinshasa">x</a>', 'wikipedia_en_all.zim') === '<a class="al" target="_top" href="alexandria-article:wikipedia_en_all.zim/Kinshasa">x</a>');
check('percent-encoded targets survive', rewriteLinks('<a class="al" href="#a:S%C3%A3o_Paulo">x</a>', 'wikipedia_en_all.zim').includes('alexandria-article:wikipedia_en_all.zim/S%C3%A3o_Paulo'));
check('packs with slashes are percent-encoded', rewriteLinks('<a class="al" href="#a:Kinshasa">x</a>', 'sub/wikipedia_en_all.zim').includes('alexandria-article:sub%2Fwikipedia_en_all.zim/Kinshasa'));
check('inert links are left alone', rewriteLinks('<a class="ext">web</a>', 'wikipedia_en_all.zim') === '<a class="ext">web</a>');
check('plain text is left alone', rewriteLinks('<p>href="#a:hi" is just text</p>', 'wikipedia_en_all.zim') === '<p>href="#a:hi" is just text</p>');
process.exit(bad ? 1 : 0);
