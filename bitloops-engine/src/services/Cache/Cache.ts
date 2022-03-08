import { ILRUCache } from '../interfaces';
abstract class Cache<T> implements ILRUCache<T> {
	protected _cache: Map<string, T>;
	protected _max: number;

	constructor(max: number) {
		if (typeof max !== 'number' || max < 0) console.error('max must be a non-negative number');
		this._max = max;
		this._cache = new Map<string, T>();
	}

	set max(newMax: number) {
		if (typeof newMax !== 'number' || newMax < 0) console.error('max must be a non-negative number');
		this._max = newMax;
		while (this._cache.size > this._max) {
			this._cache.delete(this.oldest());
		}
	}

	get(key: string): T {
		let item = this._cache.get(key);
		if (item) {
			// console.log(`Cache hit for key: ${key}`);
			// re-insert for LRU strategy
			this._cache.delete(key);
			this._cache.set(key, item);
		}
		return item ?? null;
	}

	set(key: string, value: T): void {
		// refresh key for LRU
		if (this._cache.has(key)) this._cache.delete(key);
		// evict oldest
		else if (this._cache.size >= this._max) {
			const oldestKey = this.oldest();
			this._cache.delete(oldestKey);
		}
		this._cache.set(key, value);
	}

	protected oldest(): string {
		return this._cache.keys().next().value;
	}

	getCount(): number {
		return this._cache.size;
	}

	getSize() {
		throw new Error('Method not implemenented.');
		// console.log(process.memoryUsage())
	}

	remove(key: string) {
		this._cache.delete(key);
	}

	clear() {
		this._cache.clear();
	}

	getSnapshot() {
		console.table([...this._cache.entries()]);
		// console.table(Object.fromEntries(this._cache));
	}
}

export default Cache;
