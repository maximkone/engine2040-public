#!/usr/bin/env node
// Task 8 referee: no numeric cast, and the work the cast was in still stands.
// The mutation removes the cast by emptying the body: red.
//
//   node tools/tasks/08-cast-removal/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/08-cast-removal/referee.mjs --tree <dir>

import { cli, defineTask } from '../lib.mjs';

const FILE = 'cast.rs.txt';
const SIGNATURE = 'pub fn tick_count(n: u8) -> f32 {';
const ARITHMETIC = 'scaled * 0.5';
const NUMERIC_AS = /\bas\s+(?:f32|f64|u8|u16|u32|u64|usize|i8|i16|i32|i64|isize)\b/;

function source({ convert, body = ['    let scaled = ' + convert + ';', `    ${ARITHMETIC}`] }) {
  return ['// Fixture: one conversion in a sim helper.', SIGNATURE, ...body, '}', ''].join('\n');
}

const start = { [FILE]: source({ convert: 'n as f32' }) };
const fixed = { [FILE]: source({ convert: 'f32::from(n)' }) };
// The wrong fix: the cast is gone because the function no longer does anything.
const mutation = { [FILE]: source({ convert: 'f32::from(n)', body: ['    todo!()'] }) };

export const task = defineTask({
  id: '08-cast-removal',
  title: 'remove a cast without deleting the work',
  variants: { start, fixed, mutation },
  checks(files) {
    const failures = [];
    const text = files[FILE];
    if (text === undefined) return [`${FILE} is missing`];
    if (!text.includes(SIGNATURE)) failures.push(`${FILE} has no \`${SIGNATURE}\``);
    if (NUMERIC_AS.test(text)) failures.push(`${FILE} still has a numeric cast: ${NUMERIC_AS.exec(text)[0]}`);
    if (!text.includes('f32::from(n)')) failures.push(`${FILE} does not convert with f32::from(n)`);
    if (!text.includes(ARITHMETIC)) failures.push(`${FILE} lost the arithmetic \`${ARITHMETIC}\`: removing the cast removed the work`);
    return failures;
  },
});

cli(task);
