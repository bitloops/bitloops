import { IXApiKeyDefinition } from '../Database/definitions';
import { SubscriptionType } from './definitions';
import {
    ISSEConnectionToTopicsCache,
    ISSETopicToConnectionsCache,
    ISSEConnectionsCache,
} from './SSE/interfaces';

export interface ICache<T> {
    fetch(id: string): T;
    fetchAll(): Array<string>;
}

interface ISubscriptionTopicsCache extends ICache<SubscriptionType> {
    cache(topic: string, subscription: SubscriptionType);
    delete(topic: string);
}

export interface IXApiKeyCache {
    cache(xApiKey: string, xApiKeyRecord: IXApiKeyDefinition);
    fetch(xApiKey: string): Promise<IXApiKeyDefinition>;
}

export interface ILRUCache<T> {
    get(key: string): T;
    set(key: string, value: T): void;
}

export {
    ISSEConnectionToTopicsCache,
    ISSETopicToConnectionsCache,
    ISSEConnectionsCache,
    ISubscriptionTopicsCache,
}