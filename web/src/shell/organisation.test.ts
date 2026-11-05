import { describe, expect, it } from 'vitest';
import { MAX_ORGANISATION_CHARS, organisationName } from './organisation';

const health = (organisation: unknown) => ({ status: 'ok', version: '0.1.0', organisation });

describe('organisationName', () => {
  it('is the name the server sent, without surrounding space', () => {
    expect(organisationName(health('Acme Ltd'))).toBe('Acme Ltd');
    expect(organisationName(health('  Acme Ltd \n'))).toBe('Acme Ltd');
    expect(organisationName(health('Société Générale – 研究所'))).toBe('Société Générale – 研究所');
  });

  it('is nothing when the server sent no name, or an empty one', () => {
    expect(organisationName({ status: 'ok', version: '0.1.0' })).toBeUndefined();
    expect(organisationName(health(''))).toBeUndefined();
    expect(organisationName(health('   '))).toBeUndefined();
    expect(organisationName(health(null))).toBeUndefined();
  });

  it('is nothing for anything that is not text', () => {
    for (const value of [1, true, {}, ['Acme'], undefined]) expect(organisationName(health(value))).toBeUndefined();
    for (const body of [null, undefined, 'Acme', 7, ['Acme'], {}]) expect(organisationName(body)).toBeUndefined();
  });

  it('is nothing for a name longer than the limit, counted in characters', () => {
    const atLimit = '研'.repeat(MAX_ORGANISATION_CHARS);
    expect(organisationName(health(atLimit))).toBe(atLimit);
    expect(organisationName(health(`${atLimit}研`))).toBeUndefined();
    const astral = '😀'.repeat(MAX_ORGANISATION_CHARS);
    expect(organisationName(health(astral))).toBe(astral);
    expect(organisationName(health(`${astral}😀`))).toBeUndefined();
  });

  it('is nothing for a name with a control or invisible formatting character', () => {
    for (const bad of ['a\nb', 'a\tb', 'a\u0000b', 'a\u001b[31mb', 'a‮b', 'a​b', 'a⁦b', 'a﻿b', 'a؜b']) {
      expect(organisationName(health(bad)), JSON.stringify(bad)).toBeUndefined();
    }
  });

  it('keeps markup as text for the page to escape', () => {
    expect(organisationName(health('<b>Acme</b> & Sons'))).toBe('<b>Acme</b> & Sons');
  });
});
