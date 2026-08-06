import { useState } from "react";

/**
 * Scaffolding: a hydrated React island, here to prove the Astro/React/Tailwind
 * wiring works end to end. Replace it with the real thing.
 */
export default function IslandCheck() {
	const [count, setCount] = useState(0);

	return (
		<div className="flex items-center gap-4 rounded-lg border border-neutral-200 p-4 dark:border-neutral-800">
			<button
				type="button"
				onClick={() => setCount((n) => n + 1)}
				className="rounded-md bg-neutral-900 px-3 py-1.5 text-sm font-medium text-white transition hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300"
			>
				Count up
			</button>
			<p className="text-sm text-neutral-600 dark:text-neutral-400">
				Clicked <span className="font-mono text-neutral-900 dark:text-neutral-100">{count}</span> times
			</p>
		</div>
	);
}
