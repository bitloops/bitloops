import { ISSEConnectionsCache } from './interfaces';
import { SSEConnectionsCacheType, SSEConnectionsCredsType } from './definitions';
import { AuthVerificationRequest, RawReply } from '../../../routes/definitions';

export default class SSEConnectionsCache implements ISSEConnectionsCache {
    private prefixKey = 'sseConnections';
    private _cache: SSEConnectionsCacheType;

    constructor() {
        this._cache = {};
    }

    cache(connectionId: string, connection: RawReply, creds: AuthVerificationRequest) {
        const key = this.getCacheKey(connectionId);
        this._cache[key] = {
            creds,
            connection
        }
    }

    fetch(connectionId: string): SSEConnectionsCredsType {
        const key = this.getCacheKey(connectionId);
        return this._cache[key];
    }

    fetchAll(): Array<string> {
        return Object.keys(this._cache);
    }

    delete(connectionId: string) {
        const key = this.getCacheKey(connectionId);
        delete this._cache[key];
    }

    private getCacheKey(connectionId: string) {
        return `${this.prefixKey}:${connectionId}`;
    }
}
