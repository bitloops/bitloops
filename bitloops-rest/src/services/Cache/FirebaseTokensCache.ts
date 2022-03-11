import { tokenInfo } from './../definitions/index';
import admin from 'firebase-admin';
import FirebaseAdmin from '../FirebaseAdmin';
import { IFirebaseTokensCache } from '../interfaces';
import LRUCache from './LRUCache';

class FirebaseTokensCache extends LRUCache<tokenInfo> implements IFirebaseTokensCache {
	constructor(max: number) {
		super(max);
	}
	fetch(token: string): tokenInfo {
		return this.get(token);
	}
	cache(token: string, tokenInfo: Omit<tokenInfo, 'cached_at'>) {
		tokenInfo['cached_at'] = Date.now();
		this.set(token, tokenInfo as tokenInfo);
	}
}
export default FirebaseTokensCache;
