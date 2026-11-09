import test from 'node:test';
import assert from 'node:assert/strict';
import {appendKeypadToken, keypadRows, scientificEnter} from '../src/keypad.ts';

test('scientific keypad appends the old tokens and edits one character at a time', () => {
  assert.deepEqual(keypadRows[0], ['7', '8', '9', '/', '*', '^', '⌫']);
  assert.equal(keypadRows.flat().length, 39);
  assert.equal(appendKeypadToken('', 'sin('), 'sin(');
  assert.equal(appendKeypadToken('sin(', '⌫'), 'sin');
  assert.equal(appendKeypadToken('', '⌫'), '');
  assert.equal(appendKeypadToken('sin(pi/2)', 'C'), '');
  assert.equal(appendKeypadToken('10', 'H'), '10H');
  assert.equal(appendKeypadToken('', '0x'), '0x');
  assert.equal(appendKeypadToken('1+', 'e'), '1+e');
  assert.equal(appendKeypadToken('2*', 'pi'), '2*pi');
  assert.equal(scientificEnter('Enter', false, true), 'calculate');
  assert.equal(scientificEnter('Enter', true, true), 'newline');
  assert.equal(scientificEnter('Enter', false, false), 'block');
  assert.equal(scientificEnter('a', false, true), 'pass');
});
