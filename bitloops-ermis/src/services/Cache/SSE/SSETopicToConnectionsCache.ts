import { ISSETopicToConnectionsCache } from './interfaces';
import { SSETopicToConnectionsCacheType, SSEConnectionsType } from './definitions';

export default class SSETopicToConnectionsCache implements ISSETopicToConnectionsCache {
    private prefixKey = 'sseTopicToConnections';
    private _cache: SSETopicToConnectionsCacheType;

    constructor() {
        this._cache = {};
    }

    cache(topic: string, connectionId: string) {
        const key = this.getCacheKey(topic);
        if (!this._cache[key]) this._cache[key] = [];
        this._cache[key].push(connectionId);
    }

    fetch(topic: string): SSEConnectionsType {
        const key = this.getCacheKey(topic);
        return this._cache[key];
    }

    private getCacheKey(topic: string) {
        return `${this.prefixKey}:${topic}`;
    }
}
