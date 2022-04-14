import { AuthVerificationRequest, RawReply } from '../../../routes/definitions';
import { SSETopicsType, SSEConnectionsType, SSEConnectionsCredsType } from './definitions';

export interface ISSEConnectionToTopicsCache {
    cacheTopic(connectionId: string, topic: string);
    cacheCreds(connectionId: string, creds: any);
    fetch(connectionId: string): SSETopicsType;
}

export interface ISSETopicToConnectionsCache {
    cache(topic: string, connectionId: string);
    fetch(topic: string): SSEConnectionsType;
}

export interface ISSEConnectionsCache {
    cache(connectionId: string, connection: RawReply, creds: AuthVerificationRequest);
    fetch(connectionId: string): SSEConnectionsCredsType;
    delete(connectionId: string)
}