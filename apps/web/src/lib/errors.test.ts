// Taken over from Roedy Mail (`apps/web/src/lib/errors.test.ts`).
import { describe, expect, it } from 'vitest';
import { ApiError, NetworkError } from '#lib/api.ts';
import { ERRS, errorCodeOf, failText, gatewayErrorIn, gatewayErrorOf, isOffline, loadFailText } from './errors.ts';

describe('errorCodeOf', () => {
  it('maps HTTP statuses to the error pages', () => {
    expect(errorCodeOf(404)).toBe('404');
    expect(errorCodeOf(403)).toBe('403');
    expect(errorCodeOf(500)).toBe('500');
    expect(errorCodeOf(502)).toBe('502');
    expect(errorCodeOf(503)).toBe('503');
    expect(errorCodeOf(504)).toBe('504');
    expect(errorCodeOf(400)).toBe('500');
    expect(errorCodeOf(501)).toBe('500');
  });
});

describe('ERRS', () => {
  it('returns the texts of the error pages', () => {
    expect(ERRS['404'].title()).toBe('Page not found');
    expect(ERRS.offline.text()).toBe('Check your internet connection and try again.');
    expect(ERRS['500'].retry).toBe(true);
    expect(ERRS['503'].title()).toBe('App not ready yet');
    expect(ERRS['403'].retry).toBe(false);
  });
});

describe('errors of the gateway', () => {
  it('maps the codes of docs/app-contract.md › Errors to their pages', () => {
    expect(gatewayErrorOf('no_app_access')).toBe('403');
    expect(gatewayErrorOf('origin_mismatch')).toBe('403');
    expect(gatewayErrorOf('not_found')).toBe('404');
    expect(gatewayErrorOf('app_unreachable')).toBe('502');
    expect(gatewayErrorOf('app_not_ready')).toBe('503');
    expect(gatewayErrorOf('app_timeout')).toBe('504');
    for (const unknown of ['', 'internal', 'toString', '__proto__', null, undefined]) {
      expect(gatewayErrorOf(unknown), String(unknown)).toBeNull();
    }
  });

  it('reads the code the gateway put into the page', () => {
    const meta = document.createElement('meta');
    meta.name = 'roedy-error';
    meta.content = 'app_timeout';
    document.head.append(meta);
    try {
      expect(gatewayErrorIn()).toBe('504');
      meta.content = 'something_else';
      expect(gatewayErrorIn()).toBeNull();
    } finally {
      meta.remove();
    }
    expect(gatewayErrorIn()).toBeNull();
  });
});

describe('text of the error toast', () => {
  const answered = new ApiError(500, { error: 'internal' });

  it('counts as offline only without a server response, not for errors in the code', () => {
    expect(isOffline(new NetworkError(new TypeError('Failed to fetch')))).toBe(true);
    expect(isOffline(new TypeError('Cannot read properties of undefined'))).toBe(false);
    expect(isOffline(undefined)).toBe(false);
    expect(isOffline(answered)).toBe(false);
    expect(isOffline(new ApiError(401, { error: 'unauthorized' }))).toBe(false);
  });

  it('names “No connection” without a connection, otherwise the text of the caller', () => {
    expect(failText(answered)).toBe('That didn’t work. Please try again.');
    expect(failText(answered, 'The list could not be refreshed.')).toBe('The list could not be refreshed.');
    expect(failText(new NetworkError(new TypeError('Failed to fetch')))).toBe('No connection. Please try again.');
    expect(failText(new NetworkError(new TypeError('Failed to fetch')), 'The list could not be refreshed.')).toBe(
      'No connection. Please try again.',
    );
  });

  it('names the wait time after 429 on a "Couldn’t load …" card, otherwise like the toast', () => {
    expect(loadFailText(new ApiError(429, { error: 'rate_limited', retry_after: 30 }))).toBe(
      'Too many requests. Try again in 30 seconds.',
    );
    expect(loadFailText(answered)).toBe('That didn’t work. Please try again.');
    expect(loadFailText(new NetworkError(new TypeError('Failed to fetch')))).toBe('No connection. Please try again.');
  });
});
