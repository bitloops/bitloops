import admin from 'firebase-admin';
import FirebaseAdmin from '../FirebaseAdmin';
import { IFirebaseConnectionsCache } from '../interfaces';
import LRUCache from './LRUCache';

class FirebaseConnectionsCache extends LRUCache<FirebaseAdmin> implements IFirebaseConnectionsCache {
	constructor(max: number) {
		super(max);
	}
	fetch(connectionId: string) {
		return this.get(connectionId);
	}
	cache(connectionId: string, connection: FirebaseAdmin) {
		connection['cached_at'] = Date.now();
		this.set(connectionId, connection);
	}
}
export default FirebaseConnectionsCache;
