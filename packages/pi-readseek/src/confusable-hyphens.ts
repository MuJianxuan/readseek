/**
 * Unicode hyphen-like characters normalized to ASCII "-" during text
 * comparison so edits with typographic hyphens match their originals.
 */
export const CONFUSABLE_HYPHENS_RE =
	/[\u2010\u2011\u2012\u2013\u2014\u2015\u2212\uFE63\uFF0D]/g;
