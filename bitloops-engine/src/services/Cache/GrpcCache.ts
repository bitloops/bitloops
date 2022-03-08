import { Client, closeClient } from '@grpc/grpc-js';
import Cache from './Cache';

class GRPCCache extends Cache<Client> {
	constructor(max: number) {
		super(max);
	}

	override set(key: string, value: Client): void {
		// refresh key for LRU
		if (this._cache.has(key)) this._cache.delete(key);
		// evict oldest
		else if (this._cache.size >= this.max) {
			const oldestKey = this.oldest();
			const oldestClient: Client = this._cache.get(oldestKey);
			closeClient(oldestClient);
			this._cache.delete(oldestKey);
		}
		this._cache.set(key, value);
	}
}

export default GRPCCache;
