import { ISSEConnectionToTopicsCache } from './interfaces';
import { SSEConnectionToTopicsCacheType, SSETopicsType } from './definitions';

export default class SSEConnectionToTopicsCache implements ISSEConnectionToTopicsCache {
    private prefixKey = 'sseConnectionToTopics';
    private _cache: SSEConnectionToTopicsCacheType;

    constructor() {
        this._cache = {};
    }

    cache(connectionId: string, topic: string) {
        const key = this.getCacheKey(connectionId);
        if (!this._cache[key]) this._cache[key] = [];
        this._cache[key].push(topic);
    }

    fetch(connectionId: string): SSETopicsType {
        const key = this.getCacheKey(connectionId);
        return this._cache[key];
    }

    fetchAll(): Array<string> {
        return Object.keys(this._cache);
    }

    deleteTopicFromConnectionId(connectionId: string, topic: string) {
        const key = this.getCacheKey(connectionId);
        if (!this._cache[key]) return;
        const index = this._cache[key].indexOf(topic);
        if (index === -1) return;
        this._cache[key].splice(index, 1);

        // if the connectionId is empty, delete it
        if (this._cache[key].length === 0) {
            delete this._cache[key];
        }
    }

    delete(connectionId: string) {
        const key = this.getCacheKey(connectionId);
        delete this._cache[key];
    }

    private getCacheKey(connectionId: string) {
        return `${this.prefixKey}:${connectionId}`;
    }
}
