<script lang="ts">
	// Internet radio, from the radio-browser.info directory (Rust `radio.rs`). Clicking a station
	// plays its stable id; the "My stations" tab is the one place a stream URL is pasted, and Rust
	// probes and keeps it.
	//
	// The browse page is deliberately stateless across visits — the directory's own cache in Rust is
	// what makes a return quick, and a filter combination is cheap to rebuild. Favorites and custom
	// stations come from SQLite, with their display metadata snapshotted.
	import { onDestroy, onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Add01Icon,
		Cancel01Icon,
		Loading03Icon,
		Radio02Icon,
		Search01Icon,
		FavouriteIcon,
		MusicNote01Icon
	} from '@hugeicons/core-free-icons';
	import AppSelect from '#lib/components/AppSelect.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import RadioStationCard from '#lib/components/RadioStationCard.svelte';
	import AddStationDialog from '#lib/components/AddStationDialog.svelte';
	import MediaCardSkeleton from '#lib/components/MediaCardSkeleton.svelte';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import ErrorState from '#lib/components/ErrorState.svelte';
	import * as api from '#lib/api.js';
	import type { RadioFacets, RadioQuery, RadioStation } from '#lib/api.js';
	import { flagOf } from '#lib/flags.js';
	import { openPlayer, playback, toast } from '#lib/player.svelte.js';
	import { t } from '#lib/i18n.svelte.js';

	const PAGE = 60;
	/** How many of the directory's tags a chip row can usefully show. The long tail is thousands of
	 *  near-empty genres; the head is what anyone filters by. */
	const TAG_CHIPS = 24;

	let stations = $state<RadioStation[]>([]);
	let facets = $state<RadioFacets | null>(null);
	let loading = $state(true);
	let loadingMore = $state(false);
	let error = $state<string | null>(null);
	let moreError = $state(false);
	let hasMore = $state(true);
	let offset = $state(0);

	let name = $state('');
	let country = $state('all');
	let language = $state('all');
	let tag = $state('all');

	// Browse is the directory; the other two are what this machine has saved (SQLite in Rust). The
	// saved lists are snapshotted with their display metadata, so they still open when the directory
	// is down, and they are small enough to keep in memory for the whole visit.
	type View = 'browse' | 'favorites' | 'custom';
	let view = $state<View>('browse');
	let favorites = $state<RadioStation[]>([]);
	let custom = $state<RadioStation[]>([]);
	let savedLoading = $state(true);
	let savedError = $state<string | null>(null);
	let addOpen = $state(false);

	// Only the newest request may write to the page: typing faster than the directory answers is the
	// normal case, and an older reply landing last would replace the newer grid.
	let requestId = 0;
	let debounce: ReturnType<typeof setTimeout> | undefined;

	const nowId = $derived(playback.now?.videoId ?? '');
	const playingId = (s: RadioStation) => `${api.RADIO_PREFIX}${s.stationuuid}`;

	const favoriteIds = $derived(new Set(favorites.map((s) => s.stationuuid)));
	/** What the grid shows: the directory, or one of the saved lists. */
	const visible = $derived(view === 'browse' ? stations : view === 'favorites' ? favorites : custom);
	const emptyCopy = $derived(
		view === 'favorites'
			? { title: t('radio.favorites_empty'), hint: t('radio.favorites_empty_hint') }
			: view === 'custom'
				? { title: t('radio.custom_empty'), hint: t('radio.custom_empty_hint') }
				: { title: t('radio.empty'), hint: t('radio.empty_hint') }
	);

	const clean = (v: string) => (v.trim() ? v.trim() : undefined);
	function query(): RadioQuery {
		return {
			name: clean(name),
			country: country === 'all' ? undefined : country,
			language: language === 'all' ? undefined : language,
			tag: tag === 'all' ? undefined : tag
		};
	}

	async function fetchPage(nextOffset: number, append: boolean) {
		const id = ++requestId;
		if (append) {
			loadingMore = true;
			moreError = false;
		} else {
			loading = true;
			error = null;
			moreError = false;
			stations = [];
			hasMore = true;
		}
		try {
			const res = await api.radioSearch({ ...query(), limit: PAGE, offset: nextOffset });
			if (id !== requestId) return;
			if (append) {
				// Votes can shift between pages; a station that moved up a slot must not appear twice.
				const seen = new Set(stations.map((s) => s.stationuuid));
				stations = [...stations, ...res.filter((s) => !seen.has(s.stationuuid))];
			} else {
				stations = res;
			}
			offset = nextOffset;
			hasMore = res.length === PAGE;
		} catch (e) {
			if (id !== requestId) return;
			if (append) moreError = true;
			else error = String(e);
		} finally {
			if (id === requestId) {
				loading = false;
				loadingMore = false;
			}
		}
	}

	function reload() {
		clearTimeout(debounce);
		void fetchPage(0, false);
	}

	function loadMore() {
		if (loading || loadingMore || !hasMore) return;
		void fetchPage(offset + PAGE, true);
	}

	/** Typing costs one request per pause, not one per keystroke. */
	function onSearchInput() {
		clearTimeout(debounce);
		debounce = setTimeout(reload, 350);
	}

	function clearSearch() {
		name = '';
		reload();
	}

	async function loadFacets() {
		try {
			facets = await api.radioFacets();
		} catch {
			// The filter row is an enhancement: the grid still browses the whole directory without it.
		}
	}

	async function loadSaved() {
		savedLoading = true;
		savedError = null;
		try {
			const [f, c] = await Promise.all([api.radioFavorites(), api.radioCustomStations()]);
			favorites = f;
			custom = c;
		} catch (e) {
			savedError = String(e);
		} finally {
			savedLoading = false;
		}
	}

	onMount(() => {
		void loadFacets();
		void loadSaved();
		void fetchPage(0, false);
	});
	onDestroy(() => {
		clearTimeout(debounce);
		// A reply that lands after the page is gone must not touch its state.
		requestId++;
	});

	// The visible grid is the queue, starting at the station that was clicked, so Next walks the
	// directory (or the saved list) instead of dead-ending on one station.
	function playFrom(list: RadioStation[], station: RadioStation) {
		const index = list.findIndex((s) => s.stationuuid === station.stationuuid);
		if (index < 0) return;
		openPlayer();
		api.playPlaylist(list.map(api.stationToSong), index, undefined, t('radio.source'));
	}

	function play(station: RadioStation) {
		playFrom(stations, station);
	}

	/** The heart on a card: save a directory station, or drop it from the saved list. Rust re-fetches
	 *  the stream URL on save, so the snapshot here is display metadata only. */
	async function toggleFavorite(station: RadioStation) {
		const uuid = station.stationuuid;
		try {
			if (favoriteIds.has(uuid)) {
				if (await api.radioRemoveStation(uuid)) {
					favorites = favorites.filter((s) => s.stationuuid !== uuid);
				}
			} else {
				const saved = await api.radioAddFavorite(station);
				favorites = [saved, ...favorites.filter((s) => s.stationuuid !== saved.stationuuid)];
			}
		} catch (e) {
			toast.error(String(e));
		}
	}

	/** The bin on a custom station (and on a favorite, when removed from its own tab). */
	async function removeStation(station: RadioStation) {
		const uuid = station.stationuuid;
		try {
			if (!(await api.radioRemoveStation(uuid))) return;
			favorites = favorites.filter((s) => s.stationuuid !== uuid);
			custom = custom.filter((s) => s.stationuuid !== uuid);
		} catch (e) {
			toast.error(String(e));
		}
	}

	function sentinel(node: HTMLElement) {
		const io = new IntersectionObserver(
			([e]) => {
				if (e.isIntersecting) loadMore();
			},
			{ rootMargin: '600px 0px' }
		);
		io.observe(node);
		return () => io.disconnect();
	}

	const chip =
		'shrink-0 cursor-pointer rounded-full px-3.5 py-1.5 text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring bg-foreground/5 text-foreground/75 hover:bg-foreground/10 hover:text-foreground';
</script>

<div class="p-6">
	<div class="mb-6 flex items-center justify-between">
		<h1 class="font-heading text-2xl font-bold">{t('radio.title')}</h1>
		{#if view === 'custom'}
			<div class="flex items-center gap-2">
				<Button size="sm" class="gap-2" onclick={() => (addOpen = true)}>
					<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" />
					{t('radio.add_station.button')}
				</Button>
			</div>
		{/if}
	</div>

	<!-- Browse the directory, or the two lists saved on this machine. The counts are loaded with
	     the page, not per tab, because the lists are small and a tab switch should be instant. -->
	<Tabs.Root bind:value={view} onValueChange={(v) => (view = v as 'browse' | 'favorites' | 'custom')}>
		<Tabs.List class="mb-4">
			<Tabs.Trigger value="browse">
				<HugeiconsIcon icon={Radio02Icon} class="h-4 w-4" /> {t('radio.tabs.browse')}
			</Tabs.Trigger>
			<Tabs.Trigger value="favorites">
				<HugeiconsIcon icon={FavouriteIcon} class="h-4 w-4" /> {t('radio.tabs.favorites')}
				{#if favorites.length}<span class="ml-1 tabular-nums opacity-70">{favorites.length}</span>{/if}
			</Tabs.Trigger>
			<Tabs.Trigger value="custom">
				<HugeiconsIcon icon={MusicNote01Icon} class="h-4 w-4" /> {t('radio.tabs.custom')}
				{#if custom.length}<span class="ml-1 tabular-nums opacity-70">{custom.length}</span>{/if}
			</Tabs.Trigger>
		</Tabs.List>

		{#if view === 'browse'}
			<!-- Filters. The search is debounced; the selects and genre chips apply at once. -->
			<div class="mb-3 flex flex-col gap-3">
				<div class="flex flex-wrap items-center gap-2">
					<div class="relative max-w-2xl flex-1">
						<HugeiconsIcon
							icon={Search01Icon}
							class="pointer-events-none absolute left-4 top-1/2 z-10 h-5 w-5 -translate-y-1/2 text-muted-foreground"
						/>
						<input
							bind:value={name}
							oninput={onSearchInput}
							onkeydown={(e) => {
								if (e.key === 'Escape') clearSearch();
							}}
							placeholder={t('radio.search_placeholder')}
							aria-label={t('radio.search_placeholder')}
							class="h-12 w-full rounded-full bg-card/80 pl-12 pr-12 text-base shadow-sm outline-none placeholder:text-muted-foreground focus-within:ring-1 focus-within:ring-ring/20"
						/>
						{#if name}
							<button
								class="absolute right-2.5 top-1/2 z-10 flex h-8 w-8 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
								aria-label={t('a11y.clear_search')}
								title={t('a11y.clear_search')}
								onclick={clearSearch}
							>
								<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
							</button>
						{/if}
					</div>

					{#if facets?.countries?.length}
						<AppSelect
							class="w-44 shrink-0"
							items={[
								{ value: 'all', label: t('common.all') },
								...facets.countries.map((c) => ({
									value: c.name,
									label: c.name,
									flag: flagOf(c.iso_3166_1)
								}))
							]}
							bind:value={country}
							placeholder={t('radio.country')}
							ariaLabel={t('radio.country')}
							onchange={reload}
						/>
					{/if}

					{#if facets?.languages?.length}
						<AppSelect
							class="w-44 shrink-0"
							items={[
								{ value: 'all', label: t('common.all') },
								...facets.languages.map((l) => ({ value: l.name, label: l.name }))
							]}
							bind:value={language}
							placeholder={t('radio.language')}
							ariaLabel={t('radio.language')}
							onchange={reload}
						/>
					{/if}
				</div>

				{#if facets?.tags?.length}
					<div class="flex flex-wrap gap-1.5">
						<button
							class="{chip} {tag === 'all'
								? 'bg-primary text-primary-foreground'
								: ''}"
							onclick={() => {
								tag = 'all';
								reload();
							}}
						>
							{t('radio.all_genres')}
						</button>
						{#each facets.tags.slice(0, TAG_CHIPS) as g (g.name)}
							<button
								class="{chip} {tag === g.name ? 'bg-primary text-primary-foreground capitalize' : 'capitalize'}"
								onclick={() => {
									tag = g.name;
									reload();
								}}
							>
								{g.name}
							</button>
						{/each}
					</div>
				{/if}
			</div>
		{/if}

		{#if view === 'browse'}
			{#if loading}
				<div class="grid grid-cols-[repeat(auto-fill,10rem)] gap-4">
					{#each Array(12) as _, i (i)}
						<MediaCardSkeleton />
					{/each}
				</div>
			{:else if error}
				<ErrorState message={error} onRetry={reload} />
			{:else if !stations.length}
				<div class="flex flex-col items-center gap-2 py-16 text-center">
					<span class="rounded-full bg-muted p-4 text-muted-foreground/50">
						<HugeiconsIcon icon={Radio02Icon} class="h-8 w-8" />
					</span>
					<p class="font-medium">{t('radio.empty')}</p>
					<p class="text-sm text-muted-foreground">{t('radio.empty_hint')}</p>
				</div>
			{:else}
				<div class="content-in grid grid-cols-[repeat(auto-fill,10rem)] gap-4">
					{#each stations as station (station.stationuuid)}
						<RadioStationCard
							{station}
							playing={nowId === playingId(station)}
							favorite={favoriteIds.has(station.stationuuid)}
							onplay={() => play(station)}
							onfavorite={() => toggleFavorite(station)}
						/>
					{/each}
				</div>

				{#if loadingMore}
					<div class="flex justify-center py-6 text-muted-foreground">
						<HugeiconsIcon icon={Loading03Icon} class="h-5 w-5 animate-spin" />
					</div>
				{:else if moreError}
					<div class="flex justify-center py-6">
						<Button variant="outline" size="sm" onclick={() => loadMore()}>
							{t('common.try_again')}
						</Button>
					</div>
				{:else if hasMore}
					<div class="h-1" {@attach sentinel}></div>
				{/if}
			{/if}
		{:else if savedLoading}
			<div class="grid grid-cols-[repeat(auto-fill,10rem)] gap-4">
				{#each Array(6) as _, i (i)}
					<MediaCardSkeleton />
				{/each}
			</div>
		{:else if savedError}
			<ErrorState message={savedError} onRetry={loadSaved} />
		{:else if !visible.length}
			<div class="flex flex-col items-center gap-2 py-16 text-center">
				<span class="rounded-full bg-muted p-4 text-muted-foreground/50">
					<HugeiconsIcon icon={Radio02Icon} class="h-8 w-8" />
				</span>
				<p class="font-medium">{emptyCopy.title}</p>
				<p class="text-sm text-muted-foreground">{emptyCopy.hint}</p>
				{#if view === 'custom'}
					<Button variant="outline" size="sm" class="mt-2" onclick={() => (addOpen = true)}>
						<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" />
						{t('radio.add_station.button')}
					</Button>
				{/if}
			</div>
		{:else}
			<div class="content-in grid grid-cols-[repeat(auto-fill,10rem)] gap-4">
				{#each visible as station (station.stationuuid)}
					<RadioStationCard
						{station}
						playing={nowId === playingId(station)}
						favorite={favoriteIds.has(station.stationuuid)}
						onplay={() => playFrom(visible, station)}
						onfavorite={view === 'custom' ? undefined : () => toggleFavorite(station)}
						onremove={view === 'custom' ? () => removeStation(station) : undefined}
					/>
				{/each}
			</div>
		{/if}
	</Tabs.Root>
</div>

<AddStationDialog
	bind:open={addOpen}
	onsaved={(station) => {
		savedError = null;
		custom = [station, ...custom.filter((s) => s.stationuuid !== station.stationuuid)];
	}}
/>
