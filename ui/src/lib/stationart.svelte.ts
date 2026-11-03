// Station artwork for the browse card, the player bar and the player view.
//
// Native first: a directory logo is a plain URL, and the original image is the best quality there
// is, so it loads directly. Rust (`radio_favicon`) is the fallback for the two cases where the
// original is not usable — a plain `http://` logo (blocked as mixed content under the secure
// origin) or a host that refuses a hotlink — and for a logo so small that the webview would have
// to stretch it badly, where Rust grows it with xBRZ instead.
import * as api from './api';

// Bounded, insertion-order eviction — the house pattern (see artroom.svelte.ts). A long session
// across many stations would otherwise keep one decoded logo per station for the life of the view.
const MAX = 48;
const cache = new Map<string, string>();
const inflight = new Map<string, Promise<string>>();

function remember(key: string, data: string) {
	cache.set(key, data);
	while (cache.size > MAX) {
		const oldest = cache.keys().next().value;
		if (oldest === undefined) break;
		cache.delete(oldest);
	}
}

/**
 * Follow a piece of station artwork to something the webview can draw. Call it during component
 * init, like `artRoom`. `source` is the URL to show; `size` is the pixel box it is drawn in, used
 * both as the size Rust renders a fallback at and as the yardstick for "this native image is too
 * small to be worth showing as-is". `proxyable` says whether Rust is an option at all — the player
 * shares this with YouTube thumbnails, which have their own step-down and must not go through the
 * logo pipeline. Wire the returned `onload`/`onerror` to the `<img>`.
 */
export function stationArt(
	source: () => string | undefined,
	size?: number,
	proxyable: () => boolean = () => true
) {
	let src = $state<string | undefined>(undefined);
	// True once `src` is the Rust data URL, so the native-load checks stand down.
	let proxied = false;
	// One logo at two sizes is two entries: the card's copy never pays for the full player's.
	const keyOf = (url: string) => `${size ?? ''}:${url}`;

	$effect(() => {
		const url = source();
		proxied = false;
		if (!url) {
			src = undefined;
			return;
		}
		const ready = cache.get(keyOf(url));
		if (ready) {
			src = ready;
			proxied = true;
			return;
		}
		// A plain-http logo never loads from the tauri:// origin; go straight to Rust instead of
		// spending an error event on a request the webview will refuse.
		if (url.startsWith('http://')) {
			src = undefined;
			void resolve(url);
			return;
		}
		src = url; // the original, untouched — the best quality available
	});

	async function resolve(url: string) {
		const key = keyOf(url);
		let ready = cache.get(key);
		if (!ready) {
			let job = inflight.get(key);
			if (!job) {
				job = api.radioFavicon(url, size);
				inflight.set(key, job);
			}
			try {
				ready = await job;
				remember(key, ready);
			} catch {
				ready = undefined;
			} finally {
				inflight.delete(key);
			}
		}
		// The track may have moved on while the fetch was in flight.
		if (source() === url) {
			src = ready;
			proxied = !!ready;
		}
	}

	return {
		get src() {
			return src;
		},
		/** The native image loaded. If it is so small that the webview would stretch it several
		 *  times over, it reads as blur — swap in Rust's xBRZ-grown copy. A big-enough image is
		 *  left exactly as it is. */
		onload(e: Event) {
			if (proxied || !proxyable()) return;
			const img = e.currentTarget as HTMLImageElement;
			const box = size ?? 256;
			// Only when it would be enlarged ~4x: a 1024 box keeps a 256px logo as-is, while a
			// 64px favicon (the case xBRZ exists for) is grown.
			if (Math.max(img.naturalWidth, img.naturalHeight) * 4 < box) {
				const url = source();
				if (url) void resolve(url);
			}
		},
		/** An `<img>` that failed to load: hand it to Rust once, then fall back to the placeholder. */
		onerror() {
			if (proxied) {
				// The Rust copy failed too; show the placeholder rather than loop.
				if (proxyable()) src = undefined;
				return;
			}
			if (!proxyable()) return;
			const url = source();
			if (url) void resolve(url);
		}
	};
}
