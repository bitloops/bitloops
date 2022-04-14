import { ISSEConnectionToTopicsCache } from './interfaces';
import { SSEConnectionToTopicsCacheType, SSETopicsType } from './definitions';

export default class SSEConnectionToTopicsCache implements ISSEConnectionToTopicsCache {
    private prefixKey = 'sseConnectionToTopics';
    private _cache: SSEConnectionToTopicsCacheType;

    constructor() {
        this._cache = {};
    }

    cacheTopic(connectionId: string, topic: string) {
        const key = this.getCacheKey(connectionId);
        this.initializeConnectionCache(key);
        this._cache[key].topics.push(topic);
    }

    cacheCreds(connectionId: string, creds: any) {
        const key = this.getCacheKey(connectionId);
        this.initializeConnectionCache(key);
        this._cache[key].creds = creds;
    }

    fetch(connectionId: string): SSETopicsType {
        const key = this.getCacheKey(connectionId);
        return this._cache[key];
    }

    private initializeConnectionCache(key: string) {
        if (!this._cache[key]) this._cache[key] = { topics: [], creds: {} };
    }

    private getCacheKey(connectionId: string) {
        return `${this.prefixKey}:${connectionId}`;
    }
}
