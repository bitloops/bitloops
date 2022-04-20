import { IPublicKeysCache } from '../interfaces';
import Cache from './LRUCache';

export type CachedPublicKey = {
	cached_at: number;
	pk: string;
};
class PublicKeysCache extends Cache<CachedPublicKey> implements IPublicKeysCache {
	constructor(max: number) {
		super(max);
	}
	fetch(providerId: string) {
		return this.get(providerId);
	}
	cache(providerId: string, pk: string) {
		const storedValue = {
			cached_at: Date.now(),
			pk,
		};
		this.set(providerId, storedValue);
	}
}
export default PublicKeysCache;
