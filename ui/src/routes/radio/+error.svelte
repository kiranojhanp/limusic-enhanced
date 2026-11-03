<script lang="ts">
	let { error }: { error?: { message?: string; status?: number } } = $props();
</script>

<div class="p-8 text-center">
	<h2 class="text-2xl font-bold">Radio page error</h2>
	<p class="text-destructive"><code>{error?.status} {error?.message ?? (error ? String(error) : '')}</code></p>
</div>
