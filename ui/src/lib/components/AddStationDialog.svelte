<script lang="ts">
	// "Add station": a stream URL the radio-browser directory does not know. Rust probes it before
	// saving, so a typo or an HTML error page is refused here instead of becoming a station that
	// fails silently when it is clicked. The URL is only ever sent to Rust; the UI never gets it
	// back with the station.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { AlertCircleIcon, CheckmarkCircle02Icon, Loading03Icon } from '@hugeicons/core-free-icons';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as api from '#lib/api.js';
	import type { RadioProbe, RadioStation } from '#lib/api.js';
	import { toast } from '#lib/player.svelte.js';
	import { t } from '#lib/i18n.svelte.js';

	let {
		open = $bindable(false),
		onsaved
	}: {
		open?: boolean;
		onsaved?: (station: RadioStation) => void;
	} = $props();

	let name = $state('');
	let url = $state('');
	let checking = $state(false);
	let adding = $state(false);
	let probe = $state<RadioProbe | null>(null);
	let probeError = $state<string | null>(null);

	// A check describes one URL. Editing the box makes it stale, so the badge goes away rather than
	// claiming a stream that is no longer the one in the field.
	function invalidateProbe() {
		probe = null;
		probeError = null;
	}

	// A cancelled dialog opens empty next time, like "New playlist".
	$effect(() => {
		if (!open) {
			name = '';
			url = '';
			probe = null;
			probeError = null;
			checking = false;
			adding = false;
		}
	});

	async function check() {
		const target = url.trim();
		if (!target || checking) return;
		checking = true;
		probe = null;
		probeError = null;
		try {
			probe = await api.radioProbeStream(target);
		} catch (e) {
			probeError = String(e);
		} finally {
			checking = false;
		}
	}

	async function submit(e: Event) {
		e.preventDefault();
		const target = url.trim();
		if (!target || adding) return;
		adding = true;
		try {
			const station = await api.radioAddCustomStation(name.trim(), target);
			onsaved?.(station);
			open = false;
			toast.success(t('radio.add_station.added', { name: station.name }));
		} catch (err) {
			toast.error(String(err));
		} finally {
			adding = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>{t('radio.add_station.title')}</Dialog.Title>
			<Dialog.Description>{t('radio.add_station.desc')}</Dialog.Description>
		</Dialog.Header>
		<form class="flex flex-col gap-4" onsubmit={submit}>
			<label class="flex flex-col gap-1.5">
				<span class="text-xs font-medium text-muted-foreground">
					{t('radio.add_station.name')}
				</span>
				<Input
					bind:value={name}
					placeholder={t('radio.add_station.name_placeholder')}
					aria-label={t('radio.add_station.name')}
				/>
			</label>

			<div class="flex flex-col gap-1.5">
				<span class="text-xs font-medium text-muted-foreground">
					{t('radio.add_station.url')}
				</span>
				<div class="flex gap-2">
					<Input
						bind:value={url}
						oninput={invalidateProbe}
						placeholder={t('radio.add_station.url_placeholder')}
						aria-label={t('radio.add_station.url')}
						autofocus
					/>
					<Button
						type="button"
						variant="outline"
						disabled={!url.trim() || checking}
						onclick={check}
					>
						{#if checking}
							<HugeiconsIcon icon={Loading03Icon} class="h-4 w-4 animate-spin" />
						{:else}
							{t('radio.add_station.check')}
						{/if}
					</Button>
				</div>
			</div>

			{#if probe}
				<p class="flex items-start gap-2 text-xs text-muted-foreground">
					<HugeiconsIcon
						icon={CheckmarkCircle02Icon}
						class="mt-px h-4 w-4 shrink-0 text-primary"
					/>
					<span class="min-w-0">
						{[probe.icyName, probe.contentType].filter(Boolean).join(' · ') ||
							t('radio.add_station.check_ok')}
					</span>
				</p>
			{:else if probeError}
				<p class="flex items-start gap-2 text-xs text-destructive">
					<HugeiconsIcon icon={AlertCircleIcon} class="mt-px h-4 w-4 shrink-0" />
					<span class="min-w-0 break-words">{probeError}</span>
				</p>
			{/if}

			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)}>
					{t('common.cancel')}
				</Button>
				<Button type="submit" disabled={adding || !url.trim()}>
					{adding ? t('common.loading') : t('radio.add_station.add')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
