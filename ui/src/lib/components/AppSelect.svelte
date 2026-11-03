<script lang="ts">
	// A themed wrapper over [`svelte-select`](https://github.com/rob-balfre/svelte-select) (v6, Svelte
	// 5). It carries the app's popover surface, border, focus ring and type, so a dropdown reads like
	// the rest of the UI instead of svelte-select's stock white/blue.
	//
	// The wrapper div is where the component's CSS custom properties are set. svelte-select renders
	// its list in place (floating-ui, not a portal), so the variables inherit all the way down to the
	// list and items.
	import Select from 'svelte-select';

	/** One option. `flag` (an emoji) is drawn before the label; `count` is drawn muted at the end. */
	export type SelectItem = { value: string; label: string; flag?: string; count?: number };

	let {
		items = [] as SelectItem[],
		value = $bindable(''),
		placeholder = '',
		ariaLabel,
		class: className = '',
		onchange
	}: {
		items?: SelectItem[];
		value?: string;
		placeholder?: string;
		ariaLabel?: string;
		class?: string;
		onchange?: () => void;
	} = $props();
</script>

<div class="app-select {className}">
	<Select
		{items}
		bind:value
		valueMode="id"
		{placeholder}
		showChevron
		clearable={false}
		inputAttributes={{ 'aria-label': ariaLabel ?? placeholder }}
		onchange={() => onchange?.()}
	>
		{#snippet item({ item })}
			<span class="flex w-full items-center gap-2">
				{#if item.flag}<span aria-hidden="true">{item.flag}</span>{/if}
				<span class="min-w-0 flex-1 truncate">{item.label}</span>
				{#if item.count != null}
					<span class="shrink-0 text-xs text-muted-foreground">{item.count}</span>
				{/if}
			</span>
		{/snippet}
		<!-- The closed field is `value` alone (a bare id in `id` mode), so the flag has to be looked
		     back up from the items to show it next to the selected name. -->
		{#snippet selection({ selection })}
			{@const chosen = items.find((i) => i.value === selection)}
			<span class="flex items-center gap-2">
				{#if chosen?.flag}<span aria-hidden="true">{chosen.flag}</span>{/if}
				<span class="truncate">{chosen?.label ?? selection}</span>
			</span>
		{/snippet}
	</Select>
</div>

<style>
	.app-select {
		/* Matches the search field: the card tint and a soft shadow, with no hard border at rest.
		   svelte-select defaults to a light 1px outline that reads too bright next to it. */
		--background: color-mix(in oklab, var(--card) 80%, transparent);
		--border: 1px solid transparent;
		--border-hover: 1px solid var(--ring);
		--border-focused: 1px solid var(--ring);
		--border-radius: 9999px;
		--border-radius-focused: 9999px;
		--height: 3rem;
		--font-size: 0.875rem;
		--input-color: var(--foreground);
		--placeholder-color: var(--muted-foreground);
		--selected-item-color: var(--foreground);
		--icons-color: var(--muted-foreground);
		--chevron-color: var(--muted-foreground);
		--chevron-background: transparent;
		--chevron-border: 0;
		--chevron-width: 2.25rem;
		--chevron-height: 3rem;
		--clear-icon-color: var(--muted-foreground);
		--list-background: var(--popover);
		--list-border: 1px solid var(--border);
		--list-border-radius: 0.75rem;
		--list-shadow: var(--shadow-xl, 0 20px 25px -5px rgb(0 0 0 / 0.1));
		/* Above the cards' favourite button (z-10) so the open list is never overlapped by it. */
		--list-z-index: 50;
		--item-color: var(--popover-foreground);
		--item-height: 2rem;
		--item-line-height: 2rem;
		--item-padding: 0 0.75rem;
		--item-hover-bg: var(--accent);
		--item-hover-color: var(--accent-foreground);
		--item-active-background: var(--accent);
		--item-is-active-bg: var(--accent);
		--item-is-active-color: var(--accent-foreground);
		--item-is-not-selectable-color: var(--muted-foreground);
	}

	/* The same soft lift the search field has, since the at-rest border is gone. */
	.app-select :global(.svelte-select) {
		box-shadow: var(--shadow-sm, 0 1px 2px 0 rgb(0 0 0 / 0.05));
	}

	/* svelte-select's `.item` is a block; the item snippet lays out flag + label + count on one line. */
	.app-select :global(.item) {
		display: flex;
		align-items: center;
	}
</style>
