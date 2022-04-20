import { IMQ } from './MQ/interfaces';
import { IDatabase } from './Database/interfaces';
import {
    ISSEConnectionsCache,
    ISSEConnectionToTopicsCache,
    ISSETopicToConnectionsCache,
    ISubscriptionTopicsCache,
    IXApiKeyCache
} from './Cache/interfaces';

export type Services = {
    sseConnectionToTopicsCache: ISSEConnectionToTopicsCache;
    sseTopicToConnectionsCache: ISSETopicToConnectionsCache;
    sseConnectionsCache: ISSEConnectionsCache;
    subscriptionTopicsCache: ISubscriptionTopicsCache;
    xApiKeyCache: IXApiKeyCache;
    mq: IMQ;
    db: IDatabase;
};