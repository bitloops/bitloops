import { MQSubscription } from '../MQ/definitions';

export type SubscriptionType = MQSubscription;

export type SubscriptionTopicsCacheType = Record<string, SubscriptionType>;

export enum CacheTypeName {
    SSEConnectionsCache = 'sseConnectionsCache',
    SSEConnectionToTopicsCache = 'sseConnectionToTopicsCache',
    SSETopicToConnectionsCache = 'sseTopicToConnectionsCache',
    SubscriptionTopicsCache = 'subscriptionTopicsCache',
}