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

export {
    ISSEConnectionToTopicsCache,
    ISSETopicToConnectionsCache,
    ISSEConnectionsCache,
    ISubscriptionTopicsCache,
}