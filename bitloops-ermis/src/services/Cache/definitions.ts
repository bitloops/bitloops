import { MQSubscription } from '../MQ/definitions';

export type SubscriptionType = MQSubscription;

export type SubscriptionTopicsCacheType = Record<string, SubscriptionType>;