import { ISubscriptionTopicsCache } from './interfaces';
import { SubscriptionTopicsCacheType, SubscriptionType } from './definitions';

export default class SubscriptionTopicsCache implements ISubscriptionTopicsCache {
    private prefixKey = 'subscriptionTopics';
    private _cache: SubscriptionTopicsCacheType;

    constructor() {
        this._cache = {};
    }

    cache(topic: string, subscription: SubscriptionType) {
        const key = this.getCacheKey(topic);
        this._cache[key] = subscription;
    }

    fetch(topic: string): SubscriptionType {
        const key = this.getCacheKey(topic);
        return this._cache[key];
    }

    fetchAll(): Array<string> {
        return Object.keys(this._cache);
    }

    delete(topic: string) {
        const key = this.getCacheKey(topic);
        delete this._cache[key];
    }

    private getCacheKey(topic: string) {
        return `${this.prefixKey}:${topic}`;
    }
}
