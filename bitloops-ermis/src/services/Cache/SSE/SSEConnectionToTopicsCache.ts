import { ISSEConnectionToTopicsCache } from './interfaces';
import { SSEConnectionToTopicsCacheType, SSETopicsType } from './definitions';

export default class SSEConnectionToTopicsCache implements ISSEConnectionToTopicsCache {
    private _cache: SSEConnectionToTopicsCacheType;

    constructor() {
        this._cache = {};
    }

    cacheTopic(connectionId: string, topic: string) {
        this.initializeConnectionCache(connectionId);
        this._cache[connectionId].topics.push(topic);
    }

    cacheCreds(connectionId: string, creds: any) {
        this.initializeConnectionCache(connectionId);
        this._cache[connectionId].creds = creds;
    }

    fetch(connectionId: string): SSETopicsType {
        return this._cache[connectionId];
    }

    private initializeConnectionCache(connectionId: string) {
        if (!this._cache[connectionId]) this._cache[connectionId] = { topics: [], creds: {} };
    }
}
