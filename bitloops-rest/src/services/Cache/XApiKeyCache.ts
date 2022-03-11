import { IXApiKeyDefinition } from '../definitions';
import { IXApiKeyCache } from '../interfaces';
import Cache from './LRUCache';

class XApiKeyCache extends Cache<IXApiKeyDefinition> implements IXApiKeyCache {
	constructor(max: number) {
		super(max);
	}

	cache(xApiKey: string, xApiKeyRecord: IXApiKeyDefinition) {
		xApiKeyRecord['cached_at'] = Date.now();
		this.set(xApiKey, xApiKeyRecord);
	}

	fetch(xApiKey: string): Promise<IXApiKeyDefinition> {
		return Promise.resolve(this.get(xApiKey));
	}
}

export default XApiKeyCache;
