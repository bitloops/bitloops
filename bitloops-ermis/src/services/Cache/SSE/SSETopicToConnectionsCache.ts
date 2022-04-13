import { ISSETopicToConnectionsCache } from './interfaces';
import { SSETopicToConnectionsCacheType, SSEConnectionsType } from './definitions';

export default class SSETopicToConnectionsCache implements ISSETopicToConnectionsCache {
    private _cache: SSETopicToConnectionsCacheType;

    constructor() {
        this._cache = {};
    }

    cache(topic: string, connectionId: string) {
        if (!this._cache[topic]) this._cache[topic] = [];
        this._cache[topic].push(connectionId);
    }

    fetch(topic: string): SSEConnectionsType {
        return this._cache[topic];
    }
}
