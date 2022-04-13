import { SSETopicsType, SSEConnectionsType } from './definitions';

export interface ISSEConnectionToTopicsCache {
    cacheTopic(connectionId: string, topic: string);
    cacheCreds(connectionId: string, creds: any);
    fetch(connectionId: string): SSETopicsType;
}

export interface ISSETopicToConnectionsCache {
    cache(topic: string, connectionId: string);
    fetch(topic: string): SSEConnectionsType;
}