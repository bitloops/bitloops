import { IMQ } from './MQ/interfaces';
import {
    ISSEConnectionToTopicsCache,
    ISSETopicToConnectionsCache
} from './Cache/interfaces';

export type Services = {
    sseConnectionToTopicsCache: ISSEConnectionToTopicsCache;
    sseTopicToConnectionsCache: ISSETopicToConnectionsCache;
    mq: IMQ;
};