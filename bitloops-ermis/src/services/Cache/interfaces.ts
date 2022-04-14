import { SubscriptionType } from './definitions';
import {
    ISSEConnectionToTopicsCache,
    ISSETopicToConnectionsCache,
    ISSEConnectionsCache,
} from './SSE/interfaces';

interface ISubscriptionTopicsCache {
    cache(topic: string, subscription: SubscriptionType);
    fetch(topic: string): SubscriptionType;
    delete(topic: string);
}

export {
    ISSEConnectionToTopicsCache,
    ISSETopicToConnectionsCache,
    ISSEConnectionsCache,
    ISubscriptionTopicsCache,
}