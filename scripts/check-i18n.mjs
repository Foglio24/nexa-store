#!/usr/bin/env node
// Checks the Italian dictionary against the messages used in the interface code.
//
//   node scripts/check-i18n.mjs          check (exit 1 on problems)
//   node scripts/check-i18n.mjs --list   print every English message found in the code
//
// Messages are written in English in the code as t('…') / tr('…'); Italian lives in src/i18n/it.ts, one
// JSON-quoted `"English": "Italiano",` pair per line. Category labels/descriptions and the sidebar
// labels are looked up with a variable, so they are collected from their definitions.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const src = join(root, 'src');
const dictionaryFile = join(src, 'i18n', 'it.ts');

function files(directory) {
  return readdirSync(directory).flatMap(name => {
    const path = join(directory, name);
    return statSync(path).isDirectory() ? files(path) : /\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}

const unescape = (text, quote) => text.replace(/\\(.)/g, (_, c) => c === 'n' ? '\n' : c === 't' ? '\t' : c);
const found = new Map(); // message -> first place
const note = (message, path, line) => { if (!found.has(message)) found.set(message, `${relative(root, path)}:${line}`); };
const lineOf = (text, index) => text.slice(0, index).split('\n').length;

for (const path of files(src)) {
  if (path === dictionaryFile) continue;
  const text = readFileSync(path, 'utf8');
  for (const match of text.matchAll(/(?<![\w.$])(?:t|tr)\(\s*(['"])((?:\\.|(?!\1)[^\\\n])*)\1/g)) note(unescape(match[2], match[1]), path, lineOf(text, match.index));
  const relativePath = relative(src, path);
  if (relativePath === join('lib', 'categories.ts')) {
    for (const match of text.matchAll(/\b(?:label|description):\s*(['"])((?:\\.|(?!\1)[^\\\n])*)\1/g)) note(unescape(match[2], match[1]), path, lineOf(text, match.index));
  }
  if (relativePath === 'App.tsx') {
    for (const match of text.matchAll(/\{\s*id:\s*'[a-z]+',\s*label:\s*'([^']*)'/g)) note(match[1], path, lineOf(text, match.index));
  }
}

if (process.argv.includes('--list')) {
  for (const [message, place] of found) console.log(JSON.stringify(message), '//', place);
  process.exit(0);
}

/** Words that are written the same in Italian. */
const SAME_IN_ITALIAN = new Set(['Internet', 'App', 'A–Z']);
const dictionary = new Map();
const problems = [];
readFileSync(dictionaryFile, 'utf8').split('\n').forEach((line, index) => {
  const match = line.match(/^\s*("(?:\\.|[^"\\])*"):\s*("(?:\\.|[^"\\])*"),\s*$/);
  if (!match) return;
  const english = JSON.parse(match[1]);
  const italian = JSON.parse(match[2]);
  if (dictionary.has(english)) problems.push(`it.ts:${index + 1}: duplicate entry: ${english}`);
  dictionary.set(english, italian);
  const count = text => (text.match(/\{\}/g) ?? []).length;
  if (count(english) !== count(italian)) problems.push(`it.ts:${index + 1}: placeholders differ: ${english}`);
  if (english === italian && !SAME_IN_ITALIAN.has(english)) problems.push(`it.ts:${index + 1}: not translated: ${english}`);
});
for (const [message, place] of found) if (!dictionary.has(message)) problems.push(`${place}: no Italian text for: ${message}`);
for (const message of dictionary.keys()) if (!found.has(message)) problems.push(`it.ts: entry not used in the code: ${message}`);

if (problems.length) { console.error(problems.join('\n')); console.error(`\n${problems.length} problem(s).`); process.exit(1); }
console.log(`i18n ok: ${found.size} messages, all with an Italian text.`);
