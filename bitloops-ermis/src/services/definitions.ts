import { IMQ } from './MQ/interfaces';
import {
    ISSEConnectionsCache,
    ISSEConnectionToTopicsCache,
    ISSETopicToConnectionsCache,
    ISubscriptionTopicsCache
} from './Cache/interfaces';

export type Services = {
    sseConnectionToTopicsCache: ISSEConnectionToTopicsCache;
    sseTopicToConnectionsCache: ISSETopicToConnectionsCache;
    sseConnectionsCache: ISSEConnectionsCache;
    subscriptionTopicsCache: ISubscriptionTopicsCache;
    mq: IMQ;
};