import { IMQ } from './MQ/interfaces';
import { ISSEConnectionsCache } from './Cache/interfaces';

export type Services = {
    sseConnectionsCache: ISSEConnectionsCache;
    mq?: IMQ;
};