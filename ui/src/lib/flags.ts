/** A country's two-letter code (`iso_3166_1` / `countrycode`) to its flag emoji, built from the two
 *  regional-indicator letters. Empty when the code is not two A–Z letters, so callers can skip it
 *  rather than render a placeholder. */
export function flagOf(code: string): string {
	const cc = code.trim().toUpperCase();
	if (!/^[A-Z]{2}$/.test(cc)) return '';
	return String.fromCodePoint(...[...cc].map((c) => 0x1f1e6 + c.charCodeAt(0) - 65));
}
