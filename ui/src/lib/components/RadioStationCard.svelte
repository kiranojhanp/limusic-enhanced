<script lang="ts">
	// One station as a tile on the radio page. Everything here is directory metadata (radio.rs); the
	// card never sees a stream URL — clicking it plays `RADIO:<stationuuid>` and Rust resolves that.
	//
	// The tile is a positioned wrapper with one invisible button covering it, rather than a <button>
	// around the whole card: the favorite/remove action is a real nested button, which HTML does not
	// allow inside another one.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Delete02Icon, FavouriteIcon, PlayIcon, Radio02Icon } from '@hugeicons/core-free-icons';
	import type { RadioStation } from '#lib/api.js';
	import { flagOf } from '#lib/flags.js';
	import { stationArt } from '#lib/stationart.svelte.js';
	import { t } from '#lib/i18n.svelte.js';

	let {
		station,
		playing = false,
		favorite = false,
		onplay,
		onfavorite,
		onremove
	}: {
		station: RadioStation;
		playing?: boolean;
		favorite?: boolean;
		onplay: () => void;
		onfavorite?: () => void;
		onremove?: () => void;
	} = $props();

	// The original favicon loads directly (best quality). Rust only steps in for one the webview
	// cannot load — a 404, a hotlink-blocked host, or a plain `http://` logo (blocked as mixed
	// content by the secure origin) — or for one far too small to fill the tile, which Rust grows
	// with xBRZ. The tile draws ~120 CSS px, so 256 is the box at 2x.
	const art = stationArt(() => station.favicon, 256);

	const flag = $derived(flagOf(station.countrycode));
	const place = $derived(station.country || station.language);
	// "MP3 · 128 kbps"; either half can be missing.
	const quality = $derived(
		[station.codec.toUpperCase(), station.bitrate ? `${station.bitrate} kbps` : '']
			.filter(Boolean)
			.join(' · ')
	);
	// The directory stores tags as a space-separated list (with the odd comma thrown in by hand).
	// Three fit under a name; the rest are on the page's filter row.
	const tags = $derived(
		[...new Set(station.tags
			.split(/[,\s]+/)
			.map((tag) => tag.trim())
			.filter(Boolean))].slice(0, 3)
	);
</script>

<div class="group relative flex w-full flex-col gap-2 rounded-xl p-2 transition-colors hover:bg-accent/10">
	<!-- Hover lift shadow: rasterized once, composited cheaply -->
	<div class="pointer-events-none absolute inset-0 -z-10 rounded-xl opacity-0 shadow-xl ring-1 ring-foreground/5 transition-opacity duration-300 group-hover:opacity-100"></div>
	<!-- One click target for the whole tile. Below the content, which lets pointer events through. -->
	<button
		type="button"
		class="absolute inset-0 z-0 cursor-pointer rounded-xl focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
		onclick={onplay}
		aria-label={t('radio.play_station', { name: station.name })}
	></button>

	<div
		class="pointer-events-none relative aspect-square w-full overflow-hidden rounded-lg bg-muted ring-1 ring-inset ring-foreground/5 {playing
			? 'ring-2 ring-primary/60'
			: ''}"
	>
		{#if art.src}
			<!-- object-contain, not cover: station logos are whatever shape the broadcaster drew, and
			     cropping a wordmark to a square makes it unreadable. no-referrer because a fair number
			     of hosts refuse hotlinked images. -->
			<img
				src={art.src}
				alt=""
				loading="lazy"
				decoding="async"
				draggable="false"
				referrerpolicy="no-referrer"
				class="h-full w-full object-contain p-5"
				onload={art.onload}
				onerror={art.onerror}
			/>
		{:else}
			<div class="flex h-full w-full items-center justify-center text-muted-foreground/40">
				<HugeiconsIcon icon={Radio02Icon} class="h-10 w-10" />
			</div>
		{/if}

		{#if playing}
			<span
				class="absolute left-2 top-2 flex items-center gap-1 rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-semibold tracking-wide text-primary-foreground"
			>
				<span class="size-1.5 animate-pulse rounded-full bg-primary-foreground"></span>
				{t('radio.live')}
			</span>
		{/if}

		<!-- Same affordance as a media card: the whole tile is the button, this only says so. -->
		<span
			class="absolute bottom-2 right-2 flex h-9 w-9 items-center justify-center rounded-full bg-primary text-primary-foreground opacity-0 shadow-lg transition-opacity duration-200 group-hover:opacity-100 group-focus-visible:opacity-100"
		>
			<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" />
		</span>
	</div>

	<div class="pointer-events-none min-w-0 px-0.5">
		<!-- A fixed two-line box: card names wrap to one line or two, and without this the meta
		     and tags rows below sit at different heights across a row. -->
		<div class="line-clamp-2 min-h-10 text-sm font-medium leading-snug" title={station.name}>
			{station.name}
		</div>
		<div class="mt-0.5 flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
			{#if flag}<span aria-hidden="true">{flag}</span>{/if}
			<span class="truncate">{[place, quality].filter(Boolean).join(' · ')}</span>
		</div>
		{#if tags.length}
			<div class="mt-1.5 flex flex-wrap gap-1">
				{#each tags as tag (tag)}
					<span
						class="inline-flex max-w-full truncate rounded-full bg-muted/80 px-2 py-0.5 text-[10px] font-medium capitalize text-muted-foreground"
					>
						{tag}
					</span>
				{/each}
			</div>
		{/if}
	</div>

	{#if onfavorite}
		<button
			type="button"
			class="absolute right-3 top-3 z-10 flex size-8 cursor-pointer items-center justify-center rounded-full bg-background/85 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
			aria-label={favorite ? t('radio.unfavorite', { name: station.name }) : t('radio.favorite', { name: station.name })}
			aria-pressed={favorite}
			onclick={(e) => {
				e.stopPropagation();
				onfavorite();
			}}
		>
			<HugeiconsIcon
				icon={FavouriteIcon}
				class="h-4 w-4 {favorite ? 'fill-current text-primary' : ''}"
			/>
		</button>
	{/if}

	{#if onremove}
		<button
			type="button"
			class="absolute right-3 top-3 z-10 flex size-8 cursor-pointer items-center justify-center rounded-full bg-background/85 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:text-destructive focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
			aria-label={t('radio.remove_station', { name: station.name })}
			onclick={(e) => {
				e.stopPropagation();
				onremove();
			}}
		>
			<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" />
		</button>
	{/if}
</div>
